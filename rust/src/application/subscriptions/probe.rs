//! Explicit, bounded Claude usage probes. Only the opt-in route sends a prompt.
use super::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Mutex};
pub trait SubscriptionProbeHooks: Send + Sync {
    fn dismiss<'a>(&'a self, binding: SessionBinding) -> CoreFuture<'a, Result<(), SessionError>>;
    fn wrapper_pid(&self, binding: SessionBinding) -> Option<u32>;
    fn recover_process(&self, pid: u32) -> io::Result<()>;
}
#[derive(Clone, Serialize, Deserialize)]
struct Record {
    version: u32,
    provider: String,
    profile_id: String,
    label: String,
    #[serde(default, skip_serializing_if = "zero")]
    session_id: i64,
    #[serde(default, skip_serializing_if = "zero_pid")]
    pid: u32,
    started_at: String,
    cwd: PathBuf,
}
impl crate::proto::wire::GoWire for Record {
    const GO_TYPE: &'static str = "UsageProbeRecord";
    const SCHEMAS: &'static [crate::proto::wire::Schema] = &[crate::proto::wire::Schema {
        name: "UsageProbeRecord",
        fields: &[
            crate::proto::wire::Field {
                name: "version",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "provider",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "profile_id",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "label",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "session_id",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "pid",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "started_at",
                kind: "time.Time",
            },
            crate::proto::wire::Field {
                name: "cwd",
                kind: "string",
            },
        ],
    }];
}
fn zero(v: &i64) -> bool {
    *v == 0
}
fn zero_pid(v: &u32) -> bool {
    *v == 0
}
struct Active {
    record: Record,
    cancel: TaskCancellation,
}
#[derive(Default)]
pub(super) struct ProbeManager {
    active: Mutex<BTreeMap<String, Active>>,
}
impl ProbeManager {
    pub(super) fn running(&self, provider: &str, id: &str) -> bool {
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_key(&format!("{provider}:{id}"))
    }
}
struct Claim {
    manager: Arc<ProbeManager>,
    key: String,
    dir: Dir,
    name: String,
}
impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(active) = self
            .manager
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.key)
        {
            active.cancel.cancel();
        }
        let _ = self.dir.remove_file(&self.name);
    }
}
// A cleanup permit is acquired before any native launch. It remains admissible
// during Hub shutdown and owns cleanup even if the request future is aborted.
struct Cleanup {
    permit: Option<crate::hub::task_owner::OwnedTaskPermit>,
    claim: Option<Claim>,
    binding: Arc<Mutex<Option<SessionBinding>>>,
    hooks: Arc<dyn SubscriptionProbeHooks>,
    paths: RuntimePaths,
    profile: PathBuf,
    cwd: PathBuf,
    warning: Arc<dyn Fn(&'static str) + Send + Sync>,
}
impl Cleanup {
    fn schedule(&mut self) -> Option<crate::hub::task_owner::TaskWaiter<()>> {
        let permit = self.permit.take()?;
        let claim = self.claim.take();
        if let Some(claim) = &claim
            && let Some(active) = claim
                .manager
                .active
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&claim.key)
        {
            active.cancel.cancel();
        }
        let binding = *self.binding.lock().unwrap_or_else(|p| p.into_inner());
        let hooks = self.hooks.clone();
        let paths = self.paths.clone();
        let profile = self.profile.clone();
        let cwd = self.cwd.clone();
        let warning = self.warning.clone();
        Some(permit.start(async move {
            if let Some(binding) = binding {
                if hooks.dismiss(binding).await.is_err() {
                    warning("usage probe dismiss failed");
                }
                if cleanup_transcripts(&paths, &profile, &cwd).is_err() {
                    warning("usage probe transcript cleanup failed");
                }
            }
            drop(claim);
        }))
    }
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = self.schedule();
    }
}
fn compact(raw: &str) -> String {
    proto_lower(
        &raw.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>(),
    )
}
fn proto_lower(raw: &str) -> String {
    crate::proto::unicode::simple_lower(raw)
}
pub fn confirm_dialog(raw: &str) -> bool {
    let compact = compact(raw);
    [
        "itrustthisfolder",
        "isthisaprojectyoucreated",
        "allowexternalclaude.md",
        "allowexternalimports",
        "disableexternalimports",
    ]
    .iter()
    .any(|needle| compact.contains(needle))
}
pub fn dialog_key(lines: &[String]) -> Option<&'static str> {
    let target = lines.iter().position(|line| {
        ["yes,itrustthisfolder", "yes,allowexternalimports"]
            .iter()
            .any(|needle| compact(line).contains(needle))
    })?;
    let cursor = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.trim().starts_with(['❯', '›', '>']))
        .min_by_key(|(i, _)| i.abs_diff(target))
        .map(|(i, _)| i)?;
    Some(if cursor == target {
        "\r"
    } else if cursor < target {
        "\x1b[B"
    } else {
        "\x1b[A"
    })
}
fn active_dir(root: &Path) -> io::Result<Dir> {
    Dir::open_or_create_private(&root.join("active"))
}
fn marker_name(id: &str) -> String {
    format!("claude-{id}.json")
}
fn write_record(dir: &Dir, record: &Record) -> io::Result<()> {
    dir.replace(
        &marker_name(&record.profile_id),
        &serde_json::to_vec(record).map_err(io::Error::other)?,
        0o600,
    )
}
pub fn cleanup_transcripts(paths: &RuntimePaths, profile: &Path, cwd: &Path) -> io::Result<usize> {
    let name =
        crate::hub::preference_media::clean_native_path(&cwd.to_string_lossy(), cfg!(windows))
            .replace(['\\', '/', ':'], "-");
    let target = profile.join("projects").join(name);
    crate::profile::subscriptions::check_path(paths, &target)?;
    let entries = match std::fs::read_dir(&target) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut files = vec![];
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|ext| proto_lower(&ext.to_string_lossy()) != "jsonl")
        {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        files.push((meta.modified()?, entry.file_name(), path));
    }
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let mut count = 0;
    for (_, _, path) in files.into_iter().skip(3) {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        count += 1;
    }
    Ok(count)
}
impl SubscriptionService {
    pub fn update_probe_pid(&self, label: &str, pid: u32) {
        if pid == 0 {
            return;
        }
        let record = {
            let mut active = self.probes.active.lock().unwrap_or_else(|p| p.into_inner());
            let Some(active) = active
                .values_mut()
                .find(|active| active.record.label == label)
            else {
                return;
            };
            active.record.pid = pid;
            active.record.clone()
        };
        if active_dir(&record.cwd)
            .and_then(|dir| write_record(&dir, &record))
            .is_err()
        {
            (self.warning)("usage probe pid marker update failed");
        }
    }
    pub async fn probe(
        &self,
        provider: &str,
        raw: &str,
        delete: bool,
        now: Timestamp,
        hub_cancel: &TaskCancellation,
    ) -> Result<Value, ServiceError> {
        let provider = provider.trim();
        if provider != "claude" {
            return Err(ServiceError::bad(
                "usage probe supports Claude profiles only",
            ));
        }
        let id = config::normalize_subscription_id(raw);
        config::validate_subscription_id(&id).map_err(|e| ServiceError::bad(e.to_string()))?;
        let key = format!("claude:{id}");
        if delete {
            let active = self.probes.active.lock().unwrap_or_else(|p| p.into_inner());
            let Some(active) = active.get(&key) else {
                return Err(ServiceError::new(
                    404,
                    "not_found",
                    "usage probe is not running",
                ));
            };
            active.cancel.cancel();
            return Ok(json!({"ok":true,"cancelled":true,"provider":"claude","id":id}));
        }
        let resolved = self
            .resolve(provider, &id)?
            .ok_or_else(|| ServiceError::bad("subscription id is required"))?;
        let root = self.paths.root().join("usage-probe");
        let dir = active_dir(&root).map_err(|_| {
            ServiceError::new(500, "config_dir_error", "cannot prepare usage probe")
        })?;
        let cleanup_permit = self
            .effect_permit()
            .map_err(|_| ServiceError::new(503, "shutdown", "Hub task admission stopped"))?;
        let cancel = TaskCancellation::default();
        let datetime = chrono::DateTime::from_timestamp(
            now.unix_nanos().div_euclid(1_000_000_000) as i64,
            now.unix_nanos().rem_euclid(1_000_000_000) as u32,
        )
        .ok_or_else(|| ServiceError::bad("invalid probe timestamp"))?;
        let record = Record {
            version: 1,
            provider: provider.into(),
            profile_id: id.clone(),
            label: format!(
                "usage-probe-{id}-{}",
                datetime
                    .with_timezone(&chrono::Local)
                    .format("%Y%m%d%H%M%S.%9f")
            ),
            session_id: 0,
            pid: 0,
            started_at: crate::proto::time::format_rfc3339_nano(now).unwrap_or_default(),
            cwd: root.clone(),
        };
        {
            let mut active = self.probes.active.lock().unwrap_or_else(|p| p.into_inner());
            if active.contains_key(&key) {
                return Err(ServiceError::new(
                    409,
                    "probe_running",
                    "usage probe is already running for this profile",
                ));
            }
            active.insert(
                key.clone(),
                Active {
                    record: record.clone(),
                    cancel: cancel.clone(),
                },
            );
        }
        let claim = Claim {
            manager: self.probes.clone(),
            key: key.clone(),
            dir,
            name: marker_name(&id),
        };
        let binding_slot = Arc::new(Mutex::new(None));
        let mut cleanup = Cleanup {
            permit: Some(cleanup_permit),
            claim: Some(claim),
            binding: binding_slot.clone(),
            hooks: self.probe_hooks.clone(),
            paths: self.paths.clone(),
            profile: resolved.path.clone(),
            cwd: root.clone(),
            warning: self.warning.clone(),
        };
        write_record(&cleanup.claim.as_ref().expect("owned claim").dir, &record).map_err(|_| {
            ServiceError::new(500, "probe_marker_error", "cannot record usage probe state")
        })?;
        cleanup_transcripts(&self.paths, &resolved.path, &root)
            .map_err(|_| ServiceError::new(502, "probe_failed", "usage could not be retrieved"))?;
        let model = config::effective_usage_probe_model(
            &self
                .config
                .snapshot()
                .map_err(super::save_error)?
                .config
                .user_prefs
                .usage_probe_model,
        );
        let spec = WrappedSpawnSpec {
            registration_metadata: Default::default(),
            spawn_attempt: None,
            registration_proof: None,
            provider: "claude".into(),
            cwd: root.clone(),
            model,
            model_selection: String::new(),
            risk_confirmed: false,
            label: record.label.clone(),
            permission_mode: String::new(),
            sandbox: String::new(),
            ask_for_approval: String::new(),
            route: String::new(),
            utf8_session: false,
            effort: String::new(),
            execution_mode: String::new(),
            permission_preset: String::new(),
            initial_prompt: String::new(),
            subscription_profile_id: id.clone(),
            subscription_login: false,
            usage_probe: true,
            grants: Default::default(),
            cancellation: cancel.clone(),
        };
        let work = async {
            match self
                .core
                .spawn_and_wait(
                    spec,
                    Duration::from_secs(20),
                    &HttpWaitCancellation::default(),
                )
                .await
            {
                SpawnWaitOutcome::Registered(binding) => {
                    *binding_slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(binding);
                    let record = {
                        let mut active =
                            self.probes.active.lock().unwrap_or_else(|p| p.into_inner());
                        let entry = active.get_mut(&key).expect("probe claim retained");
                        entry.record.session_id = binding.session.0;
                        if let Some(pid) = self.probe_hooks.wrapper_pid(binding) {
                            entry.record.pid = pid;
                        }
                        entry.record.clone()
                    };
                    if write_record(
                        &active_dir(&root).map_err(|_| {
                            SessionError::Transport("probe marker unavailable".into())
                        })?,
                        &record,
                    )
                    .is_err()
                    {
                        (self.warning)("usage probe marker update failed");
                    }
                    self.drive_probe(binding, &cancel).await
                }
                _ => Err(SessionError::Transport(
                    "usage probe wrapper did not register".into(),
                )),
            }
        };
        let result = tokio::select! {result=tokio::time::timeout(Duration::from_secs(60),work)=>match result{Ok(result)=>result,Err(_)=>Err(SessionError::TimedOut)},_=hub_cancel.token().cancelled()=>Err(SessionError::Cancelled),_=cancel.token().cancelled()=>Err(SessionError::Cancelled)};
        cancel.cancel();
        if let Some(waiter) = cleanup.schedule()
            && waiter.wait().await.is_err()
        {
            (self.warning)("usage probe cleanup owner ended");
        }
        match result {
            Ok(()) => Ok(json!({"ok":true,"provider":"claude","id":id})),
            Err(error) => {
                (self.warning)("usage probe failed");
                let (status, code) = if matches!(error, SessionError::TimedOut) {
                    (504, "probe_timeout")
                } else if matches!(error, SessionError::Cancelled) {
                    (408, "probe_cancelled")
                } else {
                    (502, "probe_failed")
                };
                Err(ServiceError::new(
                    status,
                    code,
                    "usage could not be retrieved",
                ))
            }
        }
    }
    async fn drive_probe(
        &self,
        binding: SessionBinding,
        cancel: &TaskCancellation,
    ) -> Result<(), SessionError> {
        let mut sent = false;
        let mut moves = 0;
        loop {
            if self.core.session_usage(binding)?.is_some_and(|s| {
                s.claude_5h_present
                    || s.claude_7d_present
                    || s.rl_5h_pct != 0.0
                    || s.rl_5h_reset != 0
                    || s.rl_7d_pct != 0.0
                    || s.rl_7d_reset != 0
            }) {
                return Ok(());
            }
            if cancel.token().is_cancelled() {
                return Err(SessionError::Cancelled);
            }
            if !sent {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                loop {
                    let Ok((details, _)) = self.core.initial_prompt_observation(binding) else {
                        break;
                    };
                    if details.last_output_at.is_some_and(|last| {
                        Timestamp::now().unix_nanos() - last.unix_nanos() >= 300_000_000
                    }) || tokio::time::Instant::now() > deadline
                    {
                        break;
                    }
                    tokio::select! {_=tokio::time::sleep(Duration::from_millis(50))=>{},_=cancel.token().cancelled()=>return Err(SessionError::Cancelled)};
                }
                if self.core.session_usage(binding)?.is_some_and(|s| {
                    s.claude_5h_present
                        || s.claude_7d_present
                        || s.rl_5h_pct != 0.0
                        || s.rl_5h_reset != 0
                        || s.rl_7d_pct != 0.0
                        || s.rl_7d_reset != 0
                }) {
                    return Ok(());
                }
            }
            let (_, lines) = self.core.initial_prompt_observation(binding)?;
            if confirm_dialog(&lines.join("")) {
                let key = dialog_key(&lines).ok_or_else(|| {
                    SessionError::Transport("unrecognised usage probe startup dialog".into())
                })?;
                if key != "\r" && moves >= 6 {
                    return Err(SessionError::Transport(
                        "usage probe startup cursor limit".into(),
                    ));
                }
                let receipt = self
                    .core
                    .submit(
                        binding,
                        InputRequest {
                            bytes: key.as_bytes().to_vec(),
                            authority: InputAuthority::Internal,
                        },
                        Timestamp::now(),
                        cancel,
                    )
                    .await;
                written_receipt(receipt)?;
                if key == "\r" {
                    sent = false;
                    moves = 0;
                } else {
                    moves += 1;
                }
                tokio::select! {_=tokio::time::sleep(Duration::from_millis(400))=>{},_=cancel.token().cancelled()=>return Err(SessionError::Cancelled)};
                continue;
            }
            if !sent {
                let receipt = self
                    .core
                    .submit(
                        binding,
                        InputRequest {
                            bytes: b"\x1b[200~ok\x1b[201~\r".to_vec(),
                            authority: InputAuthority::Internal,
                        },
                        Timestamp::now(),
                        cancel,
                    )
                    .await;
                written_receipt(receipt)?;
                sent = true;
                continue;
            }
            tokio::select! {_=tokio::time::sleep(Duration::from_millis(100))=>{},_=cancel.token().cancelled()=>return Err(SessionError::Cancelled)};
        }
    }
    pub fn recover_probes(&self) {
        let root = self.paths.root().join("usage-probe");
        let Ok(dir) = active_dir(&root) else {
            (self.warning)("usage probe recovery unavailable");
            return;
        };
        if let Ok(entries) = dir.entries() {
            for name in entries {
                if !proto_lower(&name).ends_with(".json") {
                    continue;
                }
                if let Ok(bytes) = dir.read(&name, 1024 * 1024)
                    && let Ok(record) = crate::proto::wire::decode::<Record>(&bytes)
                    && record.version == 1
                    && record.provider == "claude"
                    && config::validate_subscription_id(&config::normalize_subscription_id(
                        &record.profile_id,
                    ))
                    .is_ok()
                    && record.cwd == root
                    && record.label.starts_with("usage-probe-")
                    && record.pid > 0
                    && record.pid != std::process::id()
                {
                    let _ = self.probe_hooks.recover_process(record.pid);
                }
                let _ = dir.remove_file(&name);
            }
        }
        if let Ok(snapshot) = self.config.snapshot() {
            for profile in snapshot
                .config
                .subscriptions
                .get("claude")
                .into_iter()
                .flatten()
                .filter(|p| p.is_enabled())
            {
                if let Ok(path) = config::resolve_subscription_profile_dir(
                    &self.paths,
                    "claude",
                    profile,
                    Some(&self.home),
                ) && cleanup_transcripts(&self.paths, &path, &root).is_err()
                {
                    (self.warning)("usage probe transcript cleanup failed");
                }
            }
        }
    }
}
#[cfg(test)]
mod tests;

fn written_receipt(receipt: InputReceipt) -> Result<(), SessionError> {
    match receipt.disposition {
        InputDisposition::TransportWritten { .. } => {
            if matches!(
                receipt.submit,
                SubmitReceipt::PendingEnter | SubmitReceipt::UnconfirmedAfterRetry
            ) {
                Err(SessionError::Transport(
                    "usage probe submit not confirmed".into(),
                ))
            } else {
                Ok(())
            }
        }
        InputDisposition::MissingSession => Err(SessionError::NotFound(receipt.binding.session)),
        InputDisposition::StaleBinding => Err(SessionError::StaleBinding),
        InputDisposition::AuthenticationExpired => Err(SessionError::AuthenticationExpired),
        InputDisposition::Deferred { .. } => {
            Err(SessionError::Transport("usage probe input deferred".into()))
        }
        InputDisposition::Failed { .. } => Err(SessionError::Transport(
            "usage probe input delivery failed".into(),
        )),
    }
}
