//! Shared connection ownership for launcher UI and the Hub Servers endpoints.
use super::{ActiveConnection, ConnectorConfig, LauncherStore, Profile};
use crate::process::Cancellation;
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherError {
    pub status: u16,
    pub code: &'static str,
    pub detail: String,
}
impl LauncherError {
    pub fn new(status: u16, code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            code,
            detail: detail.into(),
        }
    }
}
impl std::fmt::Display for LauncherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}
impl std::error::Error for LauncherError {}
struct LiveConnection {
    id: u64,
    cancel: Cancellation,
}
#[derive(Default)]
struct State {
    next: u64,
    closed: bool,
    tasks: BTreeMap<u64, tokio::task::JoinHandle<()>>,
    request: Option<String>,
    result: Option<Result<String, String>>,
    inflight: Option<LiveConnection>,
    connections: BTreeMap<String, LiveConnection>,
}
pub struct ConnectionManager {
    pub store: LauncherStore,
    pub connector: ConnectorConfig,
    state: Mutex<State>,
    pub connect_timeout: Duration,
}
#[derive(Serialize)]
pub struct ActiveResponse {
    #[serde(flatten)]
    pub connection: ActiveConnection,
    pub owned: bool,
}
#[derive(Serialize)]
pub struct ProfilesResponse {
    pub ok: bool,
    pub profiles: Option<Vec<Profile>>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub last_used: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub active: Vec<ActiveResponse>,
}
impl ConnectionManager {
    pub fn new(store: LauncherStore, connector: ConnectorConfig) -> Arc<Self> {
        Arc::new(Self {
            store,
            connector,
            state: Mutex::new(State::default()),
            connect_timeout: Duration::from_secs(120),
        })
    }
    pub fn with_timeout(
        store: LauncherStore,
        connector: ConnectorConfig,
        connect_timeout: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            connector,
            state: Mutex::new(State::default()),
            connect_timeout,
        })
    }
    pub async fn profiles(&self) -> Result<ProfilesResponse, LauncherError> {
        let store = self.store.clone();
        let profiles = tokio::task::spawn_blocking(move || store.load_profiles())
            .await
            .map_err(|_| LauncherError::new(500, "load_failed", "profile read interrupted"))?
            .map_err(|e| LauncherError::new(500, "load_failed", e.to_string()))?;
        let active = self.store.active_pruned().await.unwrap_or_default();
        let state = self.state.lock().await;
        Ok(ProfilesResponse {
            ok: true,
            profiles: profiles.profiles,
            last_used: profiles.last_used,
            active: active
                .into_iter()
                .map(|connection| ActiveResponse {
                    owned: connection.pid == i64::from(std::process::id())
                        && state.connections.contains_key(&connection.profile),
                    connection,
                })
                .collect(),
        })
    }
    pub async fn replace_profiles(
        &self,
        profiles: Option<Vec<Profile>>,
    ) -> Result<(), LauncherError> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.replace_profiles_staged(profiles))
            .await
            .map_err(|_| LauncherError::new(500, "save_failed", "profile save interrupted"))?
    }
    /// The caller decides whether to trim `name`: launcher UI does not; Hub does.
    pub async fn connect(self: &Arc<Self>, name: &str) -> Result<(), LauncherError> {
        if name.is_empty() {
            return Err(LauncherError::new(400, "bad_request", "name is required"));
        }
        let store = self.store.clone();
        let file = tokio::task::spawn_blocking(move || store.load_profiles())
            .await
            .map_err(|_| LauncherError::new(500, "load_failed", "profile read interrupted"))?
            .map_err(|e| LauncherError::new(500, "load_failed", e.to_string()))?;
        file.validate()
            .map_err(|e| LauncherError::new(400, "invalid_profile", e))?;
        let profile = file
            .list()
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .ok_or_else(|| {
                LauncherError::new(
                    404,
                    "not_found",
                    format!("profile {} not found", super::quote(name)),
                )
            })?;
        let mut state = self.state.lock().await;
        if state.closed {
            return Err(LauncherError::new(
                503,
                "shutting_down",
                "connection manager is shutting down",
            ));
        }
        let mut previous_tasks = Vec::new();
        if let Some(old) = state.inflight.take() {
            old.cancel.cancel();
            if let Some(task) = state.tasks.remove(&old.id) {
                previous_tasks.push(task);
            }
        }
        if let Some(old) = state.connections.remove(name) {
            old.cancel.cancel();
            if let Some(task) = state.tasks.remove(&old.id) {
                previous_tasks.push(task);
            }
            let store = self.store.clone();
            let name = name.to_owned();
            let _ = tokio::task::spawn_blocking(move || store.unregister_active(&name)).await;
        }
        state.next += 1;
        let id = state.next;
        let cancel = Cancellation::default();
        state.inflight = Some(LiveConnection {
            id,
            cancel: cancel.clone(),
        });
        state.request = Some(name.into());
        state.result = None;
        let owner = self.clone();
        let task = tokio::spawn(async move {
            // Do not let the prior attempt's remote cleanup race and kill a
            // newly established replacement using the same binary/root/port.
            for task in previous_tasks {
                let _ = task.await;
            }
            owner.clone().run_connection(profile, id, cancel).await;
            owner.state.lock().await.tasks.remove(&id);
        });
        state.tasks.insert(id, task);
        drop(state);
        Ok(())
    }
    pub async fn status(&self) -> Value {
        let state = self.state.lock().await;
        match (&state.request, &state.result) {
            (None, _) => json!({"ok":true,"status":"idle"}),
            (Some(name), None) => json!({"ok":true,"status":"connecting","name":name}),
            (_, Some(Err(error))) => json!({"ok":false,"status":"error","error":error}),
            (_, Some(Ok(url))) => json!({"ok":true,"status":"connected","hub_url":url}),
        }
    }
    async fn run_connection(self: Arc<Self>, profile: Profile, id: u64, cancel: Cancellation) {
        if cancel.is_cancelled() {
            self.fail(id, "Connection cancelled".into()).await;
            return;
        }
        if let Some(sink) = &self.connector.output {
            sink(
                crate::process::OutputStream::Stdout,
                super::close_behavior_notice(&profile).as_bytes(),
            );
        }
        let mut connection = match self.connector.start(profile.clone()) {
            Ok(c) => c,
            Err(error) => {
                self.fail(id, error).await;
                return;
            }
        };
        let ready = tokio::select! {result=connection.ready()=>result,_=tokio::time::sleep(self.connect_timeout)=>Err("Connection timed out".into()),_=cancel.cancelled()=>Err("Connection cancelled".into())};
        let url = match ready {
            Ok(url) => url,
            Err(error) => {
                connection.cancel();
                let _ = connection.wait().await;
                self.fail(id, error).await;
                return;
            }
        };
        {
            let mut state = self.state.lock().await;
            if state.inflight.as_ref().is_none_or(|live| live.id != id) {
                connection.cancel();
                drop(state);
                let _ = connection.wait().await;
                return;
            }
            state.inflight = None;
            state.connections.insert(
                profile.name.clone(),
                LiveConnection {
                    id,
                    cancel: cancel.clone(),
                },
            );
            state.result = Some(Ok(url.clone()));
            // Publish persistence under the same ownership check. A superseded
            // watcher cannot unregister the replacement's (profile,PID) entry.
            let store = self.store.clone();
            let name = profile.name.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = store.set_last_used(&name);
                store.register_active(&name, &url)
            })
            .await;
        }
        tokio::select! {_=connection.wait()=>{},_=cancel.cancelled()=>{connection.cancel();let _=connection.wait().await;}}
        let mut state = self.state.lock().await;
        if state
            .connections
            .get(&profile.name)
            .is_some_and(|live| live.id == id)
        {
            state.connections.remove(&profile.name);
            let store = self.store.clone();
            let name = profile.name.clone();
            let _ = tokio::task::spawn_blocking(move || store.unregister_active(&name)).await;
        }
    }
    async fn fail(&self, id: u64, error: String) {
        let mut state = self.state.lock().await;
        if state.inflight.as_ref().is_some_and(|live| live.id == id) {
            if let Some(live) = state.inflight.take() {
                live.cancel.cancel();
            }
            state.result = Some(Err(error));
        }
    }
    pub async fn owns(&self, name: &str) -> bool {
        self.state.lock().await.connections.contains_key(name)
    }
    async fn cancel_owned(&self, name: &str) {
        let mut state = self.state.lock().await;
        if let Some(live) = state.connections.remove(name) {
            live.cancel.cancel();
            let store = self.store.clone();
            let name = name.to_owned();
            let _ = tokio::task::spawn_blocking(move || store.unregister_active(&name)).await;
        }
    }
    pub async fn disconnect(
        &self,
        name: &str,
        mode: &str,
        cancel: &Cancellation,
        owner_label: &str,
    ) -> Result<Option<String>, LauncherError> {
        let name = name.trim();
        let mode = mode.trim();
        if name.is_empty() {
            return Err(LauncherError::new(400, "bad_request", "name is required"));
        }
        if !matches!(mode, "all" | "web" | "disconnect") {
            return Err(LauncherError::new(400, "bad_request", "invalid mode"));
        }
        let active = self
            .store
            .active_pruned()
            .await
            .map_err(|e| LauncherError::new(500, "active_failed", e.to_string()))?;
        let owned = self.owns(name).await;
        let target = select_active_connection(&active, name, owned).ok_or_else(|| {
            LauncherError::new(
                404,
                "not_found",
                format!("profile {} is not active", super::quote(name)),
            )
        })?;
        if mode == "disconnect" && !owned {
            return Err(LauncherError::new(
                400,
                "bad_request",
                format!("not owned by this {owner_label}"),
            ));
        }
        let mut warnings = Vec::new();
        if mode == "all"
            && let Err(error) =
                super::post_hub_endpoint(&target.hub_url, "/api/kill-all", cancel).await
        {
            warnings.push(error);
        }
        let remote = if matches!(mode, "all" | "web") {
            super::post_hub_endpoint(&target.hub_url, "/api/shutdown", cancel).await
        } else {
            Ok(())
        };
        if owned {
            self.cancel_owned(name).await;
        }
        if let Err(error) = remote {
            warnings.push(error);
            return Err(LauncherError::new(502, "remote_error", warnings.join("; ")));
        }
        Ok(if warnings.is_empty() {
            None
        } else {
            Some(warnings.join("; "))
        })
    }
    pub async fn close_all(&self) {
        let tasks = {
            let mut state = self.state.lock().await;
            state.closed = true;
            if let Some(live) = state.inflight.take() {
                live.cancel.cancel();
            }
            for (_, live) in std::mem::take(&mut state.connections) {
                live.cancel.cancel();
            }
            std::mem::take(&mut state.tasks)
        };
        // Scoped remote cleanup is part of termination, not a fire-and-forget
        // task which the binary's Tokio runtime could abandon on shutdown.
        for (_, task) in tasks {
            let _ = task.await;
        }
        let store = self.store.clone();
        let _ = tokio::task::spawn_blocking(move || store.unregister_all()).await;
    }
}
pub fn select_active_connection<'a>(
    records: &'a [ActiveConnection],
    name: &str,
    owned: bool,
) -> Option<&'a ActiveConnection> {
    if owned
        && let Some(record) = records
            .iter()
            .find(|r| r.profile == name && r.pid == i64::from(std::process::id()))
    {
        return Some(record);
    }
    records.iter().find(|r| r.profile == name)
}
/// Shared blocking-lifetime CLI flow. The executable owner supplies SIGINT /
/// SIGTERM cancellation and browser opening; tests supply synthetic callbacks.
pub async fn connect_cli(
    store: &LauncherStore,
    config: &ConnectorConfig,
    profile: Profile,
    cancel: &Cancellation,
    browser: &(dyn Fn(&str) -> std::io::Result<()> + Send + Sync),
) -> Result<(), String> {
    if let Ok(active) = store.active_pruned().await
        && let Some(record) = active.iter().find(|r| r.profile == profile.name)
    {
        cli_output(
            config,
            crate::process::OutputStream::Stdout,
            &format!(
                "Profile {} is already connected — reusing {}\n",
                super::quote(&profile.name),
                record.hub_url
            ),
        );
        cli_browser(config, browser, &record.hub_url);
        return Ok(());
    }
    let lock_store = store.clone();
    let lock_name = profile.name.clone();
    let acquired = tokio::task::spawn_blocking(move || lock_store.try_acquire_connect(&lock_name))
        .await
        .map_err(|_| "acquire startup lock interrupted")?
        .map_err(|e| format!("acquire startup lock: {e}"))?;
    let Some(mut startup_lock) = acquired else {
        cli_output(
            config,
            crate::process::OutputStream::Stdout,
            &format!(
                "Profile {} is already starting — waiting for Hub URL...\n",
                super::quote(&profile.name)
            ),
        );
        if let Some(record) = store
            .wait_for_active(&profile.name, Duration::from_secs(30), cancel)
            .await
        {
            cli_output(
                config,
                crate::process::OutputStream::Stdout,
                &format!(
                    "Profile {} is connected — reusing {}\n",
                    super::quote(&profile.name),
                    record.hub_url
                ),
            );
            cli_browser(config, browser, &record.hub_url);
        } else if !cancel.is_cancelled() {
            cli_output(
                config,
                crate::process::OutputStream::Stdout,
                &format!(
                    "Profile {} is still starting; no new terminal was opened.\n",
                    super::quote(&profile.name)
                ),
            );
        }
        return Ok(());
    };
    if let Some(sink) = &config.output {
        sink(
            crate::process::OutputStream::Stdout,
            super::close_behavior_notice(&profile).as_bytes(),
        );
    }
    let mut connection = config.start(profile.clone())?;
    let ready = tokio::select! {result=connection.ready()=>result,_=cancel.cancelled()=>{connection.cancel();let _=connection.wait().await;return Ok(());}};
    match ready {
        Ok(url) => {
            cli_browser(config, browser, &url);
            let record_store = store.clone();
            let name = profile.name.clone();
            let _ = tokio::task::spawn_blocking(move || record_store.register_active(&name, &url))
                .await;
            let _ = startup_lock.release();
        }
        Err(error) => {
            connection.cancel();
            let _ = connection.wait().await;
            return Err(error);
        }
    }
    let result = tokio::select! {result=connection.wait()=>result,_=cancel.cancelled()=>{connection.cancel();connection.wait().await}};
    let record_store = store.clone();
    let _ =
        tokio::task::spawn_blocking(move || record_store.unregister_active(&profile.name)).await;
    if result.is_ok() && !cancel.is_cancelled() {
        cli_output(
            config,
            crate::process::OutputStream::Stdout,
            "Connection closed — exiting.\n",
        );
    }
    result
}
fn cli_output(config: &ConnectorConfig, stream: crate::process::OutputStream, value: &str) {
    if let Some(sink) = &config.output {
        sink(stream, value.as_bytes());
    }
}
fn cli_browser(
    config: &ConnectorConfig,
    browser: &(dyn Fn(&str) -> std::io::Result<()> + Send + Sync),
    url: &str,
) {
    if let Err(error) = browser(url) {
        cli_output(
            config,
            crate::process::OutputStream::Stderr,
            &format!("many-ai-cli-launcher: failed to open browser for {url}: {error}\n"),
        );
    }
}
