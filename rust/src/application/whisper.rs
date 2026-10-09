//! Actual managed Whisper installer and process owner over explicit runtime paths.
mod archive;
pub mod manifest;
pub mod native;
#[cfg(test)]
mod tests;
use crate::{
    application::event_observer::EventWarning,
    config::{ConfigStore, RuntimePaths, VoiceWhisperConfig},
    files::safe_fs::Dir,
    hub::task_owner::HubTaskHandle,
    process::{Cancellation, ProcessPlan},
    proto::{
        core::CoreFuture,
        time::{Timestamp, format_go_rfc3339_layout},
    },
};
use serde::Serialize;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone, Debug)]
pub struct WhisperError {
    pub status: u16,
    pub code: &'static str,
    pub detail: String,
}
impl std::fmt::Display for WhisperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}
impl std::error::Error for WhisperError {}
impl WhisperError {
    fn new(status: u16, code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            code,
            detail: detail.into(),
        }
    }
}
impl From<io::Error> for WhisperError {
    fn from(_: io::Error) -> Self {
        Self::new(
            500,
            "internal_error",
            "managed Whisper filesystem operation failed",
        )
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct InstallState {
    pub installing: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub phase: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub current: String,
    #[serde(skip_serializing_if = "zero_f64")]
    pub progress: f64,
    #[serde(skip_serializing_if = "zero_u64")]
    pub bytes_done: u64,
    #[serde(skip_serializing_if = "zero_u64")]
    pub bytes_total: u64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub updated_at: String,
}
fn zero_f64(value: &f64) -> bool {
    *value == 0.0
}
fn zero_u64(value: &u64) -> bool {
    *value == 0
}
#[derive(Serialize)]
pub struct Status {
    pub ok: bool,
    pub supported: bool,
    pub platform: String,
    pub arch: String,
    pub managed: bool,
    pub installed: bool,
    pub running: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub server_url: String,
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub install_dir: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub binary_path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model_path: String,
    pub binary_version: String,
    pub install: InstallState,
    pub models: &'static [manifest::Model],
    #[serde(skip_serializing_if = "String::is_empty")]
    pub manual_only_message: String,
}
pub type Progress = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;
pub struct Download {
    pub url: String,
    pub file_name: String,
    pub sha256: String,
    pub maximum_bytes: u64,
}
#[derive(Clone, Debug)]
pub struct ProcessExit {
    pub failed: bool,
}
#[derive(Clone)]
pub struct WhisperProcess {
    pub cancellation: Cancellation,
    pub done: tokio::sync::watch::Receiver<Option<ProcessExit>>,
}
impl WhisperProcess {
    pub async fn wait(&self) {
        let mut done = self.done.clone();
        while done.borrow().is_none() {
            if done.changed().await.is_err() {
                break;
            }
        }
    }
}
pub trait WhisperIo: Send + Sync {
    fn download<'a>(
        &'a self,
        spec: Download,
        directory: Arc<Dir>,
        progress: Progress,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<(), WhisperError>>;
    fn room(&self, directory: &Dir, required: u64) -> Result<(), WhisperError>;
    fn start<'a>(
        &'a self,
        plan: ProcessPlan,
        log: Arc<Dir>,
        tasks: HubTaskHandle,
    ) -> CoreFuture<'a, Result<WhisperProcess, WhisperError>>;
    fn ready<'a>(
        &'a self,
        port: u16,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<(), WhisperError>>;
}
pub struct WhisperDependencies {
    pub paths: RuntimePaths,
    pub config: Arc<ConfigStore>,
    pub tasks: HubTaskHandle,
    pub environment: Vec<String>,
    pub platform: String,
    pub arch: String,
    pub io: Arc<dyn WhisperIo>,
    pub warning: EventWarning,
    pub runtime_payload: Vec<(String, Vec<u8>)>,
}
struct Running {
    id: u64,
    process: WhisperProcess,
    url: String,
}
#[derive(Default)]
struct State {
    install: InstallState,
    starting: bool,
    running: Option<Running>,
    next: u64,
    closed: bool,
    install_done: Option<tokio::sync::watch::Receiver<bool>>,
}
pub struct WhisperManager {
    deps: WhisperDependencies,
    root: Arc<Dir>,
    state: Mutex<State>,
    install_cancel: Cancellation,
}
struct StartReservation<'a>(&'a WhisperManager);
struct InstallCompletion {
    owner: std::sync::Weak<WhisperManager>,
    done: tokio::sync::watch::Sender<bool>,
}
impl Drop for InstallCompletion {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            let mut state = owner.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.install.installing {
                state.install.installing = false;
                state.install.phase = "error".into();
                state.install.error = "Whisper install owner cancelled".into();
                state.install.updated_at = now();
            }
        }
        let _ = self.done.send(true);
    }
}
impl Drop for StartReservation<'_> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().starting = false;
    }
}
fn now() -> String {
    format_go_rfc3339_layout(
        Timestamp::now(),
        chrono::Local::now().offset().local_minus_utc(),
        false,
    )
    .unwrap_or_default()
}
impl WhisperManager {
    pub fn new(deps: WhisperDependencies) -> Result<Arc<Self>, WhisperError> {
        let root = Arc::new(Dir::open_or_create_private_components(deps.paths.root())?);
        for (name, _) in &deps.runtime_payload {
            crate::files::safe_fs::basename(name)?;
        }
        Ok(Arc::new(Self {
            deps,
            root,
            state: Mutex::new(State::default()),
            install_cancel: Cancellation::default(),
        }))
    }
    fn base(&self) -> PathBuf {
        self.deps.paths.root().join("whisper")
    }
    fn names(&self) -> &[&str] {
        manifest::binary(&self.deps.platform, &self.deps.arch)
            .map(|entry| entry.server_names)
            .unwrap_or(&[
                "whisper-server.exe",
                "server.exe",
                "whisper-server",
                "server",
            ])
    }
    fn baked(&self) -> Option<PathBuf> {
        let value = self.deps.environment.iter().rev().find_map(|entry| {
            entry
                .split_once('=')
                .filter(|(key, _)| {
                    if cfg!(windows) {
                        key.eq_ignore_ascii_case("MANY_AI_CLI_WHISPER_SERVER")
                    } else {
                        *key == "MANY_AI_CLI_WHISPER_SERVER"
                    }
                })
                .map(|(_, value)| value.trim())
        })?;
        let path = PathBuf::from(value);
        let name = path.file_name()?.to_str()?;
        let stem = Path::new(name).file_stem()?.to_str()?;
        if !self.names().iter().any(|candidate| {
            name.eq_ignore_ascii_case(candidate)
                || Path::new(candidate)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| stem.eq_ignore_ascii_case(name))
        }) {
            return None;
        }
        if self.deps.paths.is_trial()
            && crate::profile::subscriptions::check_path(&self.deps.paths, &path).is_err()
        {
            return None;
        }
        path.is_file().then_some(path)
    }
    fn find_binary(&self) -> Option<PathBuf> {
        if let Some(path) = self.baked() {
            return Some(path);
        }
        let directory = Dir::open(&self.base().join("bin")).ok()?;
        for name in self.names() {
            if directory.open_file(name, false).is_ok() {
                return Some(directory.path().join(name));
            }
        }
        fn walk(directory: &Dir, names: &[&str], depth: usize) -> Option<PathBuf> {
            if depth > 64 {
                return None;
            }
            for name in directory.entries().ok()? {
                if names
                    .iter()
                    .any(|candidate| name.eq_ignore_ascii_case(candidate))
                    && directory.open_file(&name, false).is_ok()
                {
                    return Some(directory.path().join(name));
                }
                if let Ok(child) = directory.child_dir(&name, false)
                    && let Some(path) = walk(&child, names, depth + 1)
                {
                    return Some(path);
                }
            }
            None
        }
        walk(&directory, self.names(), 0)
    }
    pub fn supported(&self) -> bool {
        self.baked().is_some() || manifest::binary(&self.deps.platform, &self.deps.arch).is_some()
    }
    fn configuration(&self) -> Result<VoiceWhisperConfig, WhisperError> {
        self.deps
            .config
            .snapshot()
            .map(|snapshot| snapshot.config.voice.whisper)
            .map_err(|_| {
                WhisperError::new(500, "internal_error", "Whisper configuration unavailable")
            })
    }
    fn publish(&self, update: impl FnOnce(&mut VoiceWhisperConfig)) -> Result<(), WhisperError> {
        let mut snapshot = self.deps.config.snapshot().map_err(|_| {
            WhisperError::new(500, "save_failed", "Whisper configuration unavailable")
        })?;
        update(&mut snapshot.config.voice.whisper);
        self.deps
            .config
            .publish_then_persist_legacy(snapshot.revision, snapshot.config)
            .map_err(|_| {
                WhisperError::new(500, "save_failed", "Whisper configuration save failed")
            })?;
        Ok(())
    }
    pub fn status(&self) -> Result<Status, WhisperError> {
        let cfg = self.configuration()?;
        let model = manifest::model(&cfg.model).unwrap_or(manifest::MODELS[0]);
        let binary = self.find_binary();
        let model_path = self.base().join("models").join(model.file_name);
        let installed = binary.is_some()
            && self
                .root
                .child_dir("whisper", false)
                .and_then(|base| base.child_dir("models", false))
                .and_then(|models| models.open_file(model.file_name, false))
                .is_ok();
        let state = self.state.lock().unwrap();
        Ok(Status {
            ok: true,
            supported: self.supported(),
            platform: self.deps.platform.clone(),
            arch: self.deps.arch.clone(),
            managed: cfg.managed,
            installed,
            running: state.running.is_some(),
            server_url: state
                .running
                .as_ref()
                .map(|running| running.url.clone())
                .unwrap_or_else(|| cfg.server_url.trim().into()),
            model: model.id.into(),
            install_dir: self.base().to_string_lossy().into_owned(),
            binary_path: binary
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            model_path: model_path.to_string_lossy().into_owned(),
            binary_version: manifest::binary(&self.deps.platform, &self.deps.arch)
                .map(|entry| entry.version)
                .unwrap_or(manifest::RELEASE)
                .into(),
            install: state.install.clone(),
            models: manifest::MODELS,
            manual_only_message: if self.supported() {
                String::new()
            } else {
                manifest::MANUAL_ONLY.into()
            },
        })
    }
    fn progress(self: &Arc<Self>, phase: &'static str, current: String, fallback: u64) -> Progress {
        let weak = Arc::downgrade(self);
        Arc::new(move |done, total| {
            if let Some(owner) = weak.upgrade() {
                let total = total.filter(|total| *total > 0).unwrap_or(fallback);
                owner.state.lock().unwrap().install = InstallState {
                    installing: true,
                    phase: phase.into(),
                    current: current.clone(),
                    progress: if total == 0 {
                        0.0
                    } else {
                        (done as f64 / total as f64).min(1.0)
                    },
                    bytes_done: done,
                    bytes_total: total,
                    updated_at: now(),
                    ..Default::default()
                };
            }
        })
    }
    /// False means an existing source install was already running.
    pub fn install(self: &Arc<Self>, id: &str) -> Result<bool, WhisperError> {
        if !self.supported() {
            return Err(WhisperError::new(
                400,
                "unsupported_platform",
                manifest::UNSUPPORTED,
            ));
        }
        let model = manifest::model(id).unwrap_or(manifest::MODELS[0]);
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err(WhisperError::new(
                503,
                "whisper_unavailable",
                "Whisper owner stopped",
            ));
        }
        if state.install.installing {
            return Ok(false);
        }
        let permit = self.deps.tasks.effect_permit().map_err(|_| {
            WhisperError::new(
                503,
                "whisper_unavailable",
                "Whisper install owner unavailable",
            )
        })?;
        state.install = InstallState {
            installing: true,
            phase: "queued".into(),
            current: model.id.into(),
            updated_at: now(),
            ..Default::default()
        };
        let (done, wait) = tokio::sync::watch::channel(false);
        state.install_done = Some(wait);
        drop(state);
        let owner = self.clone();
        let owner_cancel = permit.cancellation();
        let completion = InstallCompletion {
            owner: Arc::downgrade(self),
            done,
        };
        drop(permit.start(async move {
            let _completion=completion;
            let result = tokio::select! {
                result=owner.install_inner(model)=>result,
                _=owner_cancel.token().cancelled()=>Err(WhisperError::new(499,"cancelled","Whisper install owner cancelled")),
            };
            let mut state = owner.state.lock().unwrap();
            match result {
                Ok(()) => {
                    state.install = InstallState {
                        phase: "done".into(),
                        current: model.id.into(),
                        progress: 1.0,
                        updated_at: now(),
                        ..Default::default()
                    }
                }
                Err(error) => {
                    state.install.installing = false;
                    state.install.phase = "error".into();
                    state.install.progress = 0.0;
                    state.install.error = error.detail;
                    state.install.updated_at = now();
                }
            }
        }));
        Ok(true)
    }
    async fn install_inner(self: &Arc<Self>, model: manifest::Model) -> Result<(), WhisperError> {
        let base = Arc::new(self.root.child_dir("whisper", true)?);
        let bin = Arc::new(base.child_dir("bin", true)?);
        let models = Arc::new(base.child_dir("models", true)?);
        let temp = Arc::new(base.child_dir("tmp", true)?);
        if self.find_binary().is_none() {
            let binary =
                manifest::binary(&self.deps.platform, &self.deps.arch).ok_or_else(|| {
                    WhisperError::new(400, "unsupported_platform", manifest::UNSUPPORTED)
                })?;
            let name = url::Url::parse(binary.url)
                .ok()
                .and_then(|url| {
                    url.path_segments()
                        .and_then(|mut parts| parts.next_back())
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "whisper-download.zip".into());
            let cleanup = archive::Cleanup::new(temp.clone(), name.clone());
            let progress = self.progress(
                "binary",
                format!("whisper.cpp {}", binary.version),
                binary.size_bytes,
            );
            progress(0, Some(binary.size_bytes));
            self.deps
                .io
                .download(
                    Download {
                        url: binary.url.into(),
                        file_name: name.clone(),
                        sha256: binary.sha256.into(),
                        maximum_bytes: binary.size_bytes + manifest::EXTRA_ROOM,
                    },
                    temp.clone(),
                    progress,
                    &self.install_cancel,
                )
                .await?;
            archive::extract(
                temp.open_file(&name, false)?,
                bin.clone(),
                binary,
                &self.install_cancel,
            )?;
            drop(cleanup);
            if self.find_binary().is_none() {
                return Err(WhisperError::new(
                    500,
                    "whisper_install_failed",
                    "whisper-server not found in release archive",
                ));
            }
        }
        self.runtime(&bin)?;
        if models.open_file(model.file_name, false).is_err() {
            self.deps
                .io
                .room(&models, model.size_bytes + manifest::EXTRA_ROOM)?;
            let progress = self.progress("model", model.id.into(), model.size_bytes);
            progress(0, Some(model.size_bytes));
            self.deps
                .io
                .download(
                    Download {
                        url: model.url.into(),
                        file_name: model.file_name.into(),
                        sha256: model.sha256.into(),
                        maximum_bytes: model.size_bytes + manifest::EXTRA_ROOM,
                    },
                    models,
                    progress,
                    &self.install_cancel,
                )
                .await?;
        }
        if self.install_cancel.is_cancelled() {
            return Err(WhisperError::new(
                499,
                "cancelled",
                "Whisper install cancelled",
            ));
        }
        self.publish(|cfg| {
            cfg.managed = true;
            cfg.model = model.id.into();
            if cfg.timeout_seconds <= 0 {
                cfg.timeout_seconds = 60;
            }
            if cfg.language.trim().is_empty() {
                cfg.language = "ja".into();
            }
        })
    }
    fn runtime(&self, bin: &Dir) -> Result<(), WhisperError> {
        for (name, bytes) in &self.deps.runtime_payload {
            if name.starts_with('.') || name.to_lowercase().ends_with(".md") {
                continue;
            }
            if bin.open_file(name, false).is_err() {
                bin.create_new(name, bytes, 0o700)?;
            }
        }
        Ok(())
    }
    pub fn ensure<'a>(
        self: &'a Arc<Self>,
        cfg: &'a VoiceWhisperConfig,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<VoiceWhisperConfig, WhisperError>> {
        Box::pin(async move {
            if !cfg.managed {
                return Ok(cfg.clone());
            }
            if !self.supported() {
                return Err(WhisperError::new(
                    400,
                    "unsupported_platform",
                    manifest::UNSUPPORTED,
                ));
            }
            let model = manifest::model(&cfg.model).ok_or_else(|| {
                WhisperError::new(
                    400,
                    "whisper_bad_model",
                    format!("unknown Whisper model {}", cfg.model.trim()),
                )
            })?;
            let binary = self.find_binary().ok_or_else(|| {
                WhisperError::new(
                    400,
                    "whisper_not_installed",
                    "Whisper server is not installed",
                )
            })?;
            let base = Arc::new(self.root.child_dir("whisper", true)?);
            let bin = base.child_dir("bin", true)?;
            self.runtime(&bin).map_err(start_error)?;
            let models = base.child_dir("models", false).map_err(|_| {
                WhisperError::new(
                    400,
                    "whisper_not_installed",
                    "Whisper model is not installed",
                )
            })?;
            models.open_file(model.file_name, false).map_err(|_| {
                WhisperError::new(
                    400,
                    "whisper_not_installed",
                    "Whisper model is not installed",
                )
            })?;
            let url = self
                .start(
                    cfg,
                    binary,
                    models.path().join(model.file_name),
                    base,
                    cancel,
                )
                .await
                .map_err(start_error)?;
            let mut result = cfg.clone();
            result.server_url = url;
            Ok(result)
        })
    }
    async fn start(
        self: &Arc<Self>,
        cfg: &VoiceWhisperConfig,
        binary: PathBuf,
        model: PathBuf,
        base: Arc<Dir>,
        cancel: &Cancellation,
    ) -> Result<String, WhisperError> {
        let id = {
            let mut state = self.state.lock().unwrap();
            if let Some(running) = &state.running {
                return Ok(running.url.clone());
            }
            if state.closed {
                return Err(WhisperError::new(
                    503,
                    "whisper_unavailable",
                    "Whisper owner stopped",
                ));
            }
            if state.starting {
                return Err(WhisperError::new(
                    409,
                    "whisper_starting",
                    "whisper server is starting",
                ));
            }
            state.starting = true;
            state.next = state.next.wrapping_add(1);
            state.next
        };
        let _reservation = StartReservation(self);
        let selected = u16::try_from(cfg.server_port)
            .ok()
            .filter(|port| *port != 0);
        let listener = if let Some(port) = selected {
            match tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await {
                Ok(listener) => listener,
                Err(_) => tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?,
            }
        } else {
            tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?
        };
        let port = listener.local_addr()?.port();
        drop(listener);
        let url = format!("http://127.0.0.1:{port}");
        let plan = ProcessPlan {
            cwd: binary.parent().unwrap_or(self.deps.paths.root()).into(),
            executable: binary,
            args: vec![
                "-m".into(),
                model.into_os_string(),
                "--host".into(),
                "127.0.0.1".into(),
                "--port".into(),
                port.to_string().into(),
                "-t".into(),
                std::thread::available_parallelism()
                    .map(|value| value.get())
                    .unwrap_or(1)
                    .to_string()
                    .into(),
            ],
            env: self
                .deps
                .environment
                .iter()
                .filter_map(|entry| {
                    entry
                        .split_once('=')
                        .map(|(key, value)| (key.into(), Some(value.into())))
                })
                .collect(),
            stdin: Vec::new(),
            timeout: Duration::ZERO,
            output_cap: 8 * 1024 * 1024,
            pipe_drain_timeout: Duration::from_secs(2),
        };
        let process = self
            .deps
            .io
            .start(plan, base, self.deps.tasks.clone())
            .await?;
        {
            let mut state = self.state.lock().unwrap();
            if state.closed {
                process.cancellation.cancel();
                return Err(WhisperError::new(
                    503,
                    "whisper_unavailable",
                    "Whisper owner stopped",
                ));
            }
            state.running = Some(Running {
                id,
                process: process.clone(),
                url: url.clone(),
            });
        }
        let permit = match self.deps.tasks.effect_permit() {
            Ok(permit) => permit,
            Err(_) => {
                self.stop_exact(id).await;
                return Err(WhisperError::new(
                    503,
                    "whisper_unavailable",
                    "Whisper process monitor unavailable",
                ));
            }
        };
        let weak = Arc::downgrade(self);
        let waiter = process.clone();
        drop(permit.start(async move {
            waiter.wait().await;
            if let Some(owner) = weak.upgrade() {
                let mut state = owner.state.lock().unwrap();
                if state
                    .running
                    .as_ref()
                    .is_some_and(|running| running.id == id)
                {
                    state.running = None;
                    if waiter
                        .done
                        .borrow()
                        .as_ref()
                        .is_some_and(|exit| exit.failed)
                    {
                        state.install.installing = false;
                        state.install.phase = "error".into();
                        state.install.progress = 0.0;
                        state.install.error =
                            "managed Whisper process exited unsuccessfully".into();
                        state.install.updated_at = now();
                    } else {
                        state.install.error.clear();
                    }
                }
            }
        }));
        if let Err(error) = self.deps.io.ready(port, cancel).await {
            self.stop_exact(id).await;
            return Err(error);
        }
        if self.state.lock().unwrap().closed {
            self.stop_exact(id).await;
            return Err(WhisperError::new(
                503,
                "whisper_unavailable",
                "Whisper owner stopped",
            ));
        }
        {
            let mut state = self.state.lock().unwrap();
            state.install.error.clear();
            if state.install.phase == "error" {
                state.install.phase.clear();
            }
            state.install.updated_at = now();
        }
        if self
            .publish(|cfg| {
                cfg.managed = true;
                cfg.server_url = url.clone();
                cfg.server_port = i64::from(port);
            })
            .is_err()
        {
            (self.deps.warning)(
                "persist managed Whisper config",
                &crate::proto::core::SessionError::InvalidRequest(
                    "Whisper configuration save failed".into(),
                ),
            );
        }
        Ok(url)
    }
    async fn stop_exact(&self, id: u64) {
        let process = {
            let mut state = self.state.lock().unwrap();
            if state
                .running
                .as_ref()
                .is_some_and(|running| running.id == id)
            {
                state.running.take().map(|running| running.process)
            } else {
                None
            }
        };
        if let Some(process) = process {
            process.cancellation.cancel();
            if tokio::time::timeout(Duration::from_secs(5), process.wait())
                .await
                .is_err()
            {
                (self.deps.warning)(
                    "stop managed Whisper process",
                    &crate::proto::core::SessionError::InvalidRequest(
                        "managed Whisper did not exit within the stop budget".into(),
                    ),
                );
            }
        }
    }
    pub async fn stop(&self) -> Result<(), WhisperError> {
        let id = self
            .state
            .lock()
            .unwrap()
            .running
            .as_ref()
            .map(|running| running.id);
        if let Some(id) = id {
            self.stop_exact(id).await;
        }
        self.publish(|cfg| {
            if cfg.managed {
                cfg.server_url.clear();
            }
        })
    }
    pub async fn uninstall(&self) -> Result<(), WhisperError> {
        if self.state.lock().unwrap().install.installing {
            return Err(WhisperError::new(
                409,
                "install_in_progress",
                "cannot uninstall while install is running",
            ));
        }
        let id = self
            .state
            .lock()
            .unwrap()
            .running
            .as_ref()
            .map(|running| running.id);
        if let Some(id) = id {
            self.stop_exact(id).await;
        }
        match self.root.remove_tree("whisper") {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(WhisperError::new(
                    500,
                    "remove_failed",
                    "managed Whisper removal failed",
                ));
            }
        };
        self.state.lock().unwrap().install = InstallState::default();
        self.publish(|cfg| {
            cfg.managed = false;
            cfg.server_url.clear();
            cfg.server_port = 0;
        })
    }
    pub async fn select_and_start(
        self: &Arc<Self>,
        id: &str,
        cancel: &Cancellation,
    ) -> Result<(), WhisperError> {
        if !id.trim().is_empty() {
            let model = manifest::model(id).ok_or_else(|| {
                WhisperError::new(
                    400,
                    "whisper_bad_model",
                    format!("unknown Whisper model {}", id.trim()),
                )
            })?;
            if self.configuration()?.model != model.id {
                self.publish(|cfg| cfg.model = model.id.into())?;
            }
        }
        let cfg = self.configuration()?;
        self.ensure(&cfg, cancel).await?;
        Ok(())
    }
    pub async fn shutdown(&self) {
        let (process, mut install) = {
            let mut state = self.state.lock().unwrap();
            state.closed = true;
            (
                state.running.take().map(|running| running.process),
                state.install_done.clone(),
            )
        };
        self.install_cancel.cancel();
        if let Some(process) = process {
            process.cancellation.cancel();
            process.wait().await;
        }
        if let Some(wait) = install.as_mut() {
            while !*wait.borrow() {
                if wait.changed().await.is_err() {
                    break;
                }
            }
        }
    }
}
fn start_error(mut error: WhisperError) -> WhisperError {
    if error.status >= 500 && error.code != "whisper_unavailable" {
        error.status = 502;
        error.code = "whisper_start_failed";
    }
    error
}
