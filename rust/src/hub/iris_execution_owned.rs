//! Explicit, opt-in owned dispatch for isolated fake-port acceptance.
//! Native evidence stays native: neither payload equality nor local cessation
//! grants task success, global effects proof, or automatic handoff authority.
use super::{
    iris_execution_grants::GrantLoader,
    iris_execution_journal::{Binding, Journal},
    iris_execution_mcp::{MAX_BYTES, parse_json},
    iris_execution_owner::{
        OwnerStore, OwnerTicket, STOPPED_AWAITING_APPROVAL, STOPPED_RUNTIME_LIMIT, check_binding,
        secure_open,
    },
    iris_execution_verification_store::VerificationStore,
    iris_execution_verifier::{DEFAULT_MAX_RUNTIME_SEC, FolderPolicy, PinnedPolicy},
};
use crate::{
    application::{
        iris_execution_dispatcher::{
            DispatchError, DispatcherConfig, DispatcherHooks, GrantCheck, LaunchAdmission,
            LaunchSupervisor, LaunchTools, OwnedDispatcher,
        },
        iris_execution_launch::OwnedLaunchPort,
    },
    proto::core::SessionBinding,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const PROFILE: &str = "iris-owned-dispatch/1";
/// Separate opt-in version for one bounded real-provider acceptance. It never
/// widens the synthetic profile above and has its own fixed policy source.
pub const REAL_BOUNDED_PROFILE: &str = "iris-owned-real-bounded/1";
/// The single provider/subscription pair the real bounded profile admits. This
/// pair is the only place that names it; every gate calls real_bounded_binding.
pub const REAL_BOUNDED_PROVIDER: &str = "claude";
pub const REAL_BOUNDED_PROFILE_ID: &str = "claude-hiroshi";
/// Separate opt-in version for approved real project work in a copied work
/// folder. Same single pair, its own policy source (`folder-change-record/1`),
/// `folder-diff` collector grants only, and a Hub-side watchdog stop.
pub const REAL_PROJECT_PROFILE: &str = "iris-owned-real-project/1";
pub const REAL_PROJECT_POLICY_FILE: &str = "owned-real-project-policies.json";
type ToolResult = Result<Value, (String, String)>;

/// Exact provider/profile identity of the real bounded profile.
pub fn real_bounded_binding(binding: &Binding) -> bool {
    binding.provider == REAL_BOUNDED_PROVIDER && binding.profile_id == REAL_BOUNDED_PROFILE_ID
}

/// Each kind has a distinct profile string, policy source and provider gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnedProfileKind {
    SyntheticDispatch,
    RealBounded,
    RealProject,
}
impl OwnedProfileKind {
    pub fn profile(self) -> &'static str {
        match self {
            Self::SyntheticDispatch => PROFILE,
            Self::RealBounded => REAL_BOUNDED_PROFILE,
            Self::RealProject => REAL_PROJECT_PROFILE,
        }
    }
    fn policy_file(self) -> &'static str {
        match self {
            Self::SyntheticDispatch => "owned-policies.json",
            Self::RealBounded => "owned-real-policies.json",
            Self::RealProject => REAL_PROJECT_POLICY_FILE,
        }
    }
    fn admits(self, binding: &Binding) -> bool {
        match self {
            Self::SyntheticDispatch => {
                binding.provider == "synthetic" && binding.profile_id == "synthetic"
            }
            Self::RealBounded | Self::RealProject => real_bounded_binding(binding),
        }
    }
    /// The real project profile pins only `folder-change-record/1`; the other
    /// profiles only the fixed-payload verifier, exactly as before.
    fn accepts(self, policy: &PinnedPolicy) -> bool {
        policy.folder().is_some() == (self == Self::RealProject)
    }
}

/// Watchdog limits of the real project profile. Production uses the
/// default (60 s of continuous permission prompt, 1 s observation step).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RealProjectWatch {
    pub awaiting_limit: Duration,
    pub poll_interval: Duration,
}
impl Default for RealProjectWatch {
    fn default() -> Self {
        Self {
            awaiting_limit: Duration::from_secs(60),
            poll_interval: Duration::from_secs(1),
        }
    }
}
impl RealProjectWatch {
    fn valid(&self) -> bool {
        !self.awaiting_limit.is_zero()
            && self.awaiting_limit <= Duration::from_secs(60)
            && !self.poll_interval.is_zero()
            && self.poll_interval <= Duration::from_secs(1)
            && self.poll_interval <= self.awaiting_limit
    }
}

/// In-memory watchdog observation of one running attempt (lost on restart).
#[derive(Clone, Default)]
struct WatchEntry {
    awaiting_approval: Option<bool>,
    awaiting_since_ms: Option<u64>,
    observed_at_ms: Option<u64>,
}

struct WatchState {
    config: RealProjectWatch,
    entries: Mutex<HashMap<String, WatchEntry>>,
}

fn attempt_key(b: &Binding) -> String {
    json!([b.principal_id, b.workspace_id, b.attempt_id]).to_string()
}

/// Deterministic per binding, so a repeated watchdog stop replays the same
/// journal receipt instead of creating another stop request.
pub fn auto_stop_request_id(b: &Binding) -> String {
    let digest = super::iris_execution_owner::sha(json!(b).to_string().as_bytes());
    format!("iris-auto-stop-{}", &digest[..48])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    profile: String,
    enabled: bool,
    policies: Vec<Value>,
}

#[derive(Clone)]
pub struct OwnedProfile {
    journal: Arc<Mutex<Journal>>,
    owners: Arc<Mutex<OwnerStore>>,
    verifications: Arc<Mutex<VerificationStore>>,
    dispatcher: Option<Arc<OwnedDispatcher>>,
    runtime_root: PathBuf,
    kind: OwnedProfileKind,
    // Serializes only new admission composition. No guard crosses await.
    admission: Arc<Mutex<()>>,
    // Real project watchdog observations; None for the other kinds.
    watch: Option<Arc<WatchState>>,
}

fn error(code: &str) -> (String, String) {
    (
        code.into(),
        "Owned dispatch conditions were not satisfied".into(),
    )
}

fn now_ms() -> Result<u64, (String, String)> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| error("OWNED_CLOCK"))?
        .as_millis()
        .try_into()
        .map_err(|_| error("OWNED_CLOCK"))
}

fn read_manifest(root: &Path, kind: OwnedProfileKind) -> Result<Manifest, (String, String)> {
    // secure_open rejects path aliases/reparse points and validates the final
    // opened handle. Only this fixed operator source can select a policy.
    let relative = Path::new("iris-execution").join(kind.policy_file());
    let mut file = secure_open(root, &relative).map_err(|_| error("OWNED_POLICY_SOURCE"))?;
    let meta = file.metadata().map_err(|_| error("OWNED_POLICY_SOURCE"))?;
    if !meta.is_file() || meta.len() > MAX_BYTES as u64 {
        return Err(error("OWNED_POLICY_SOURCE"));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("OWNED_POLICY_SOURCE"))?;
    let value = parse_json(&bytes).map_err(|_| error("OWNED_POLICY_SOURCE"))?;
    let manifest: Manifest =
        serde_json::from_value(value).map_err(|_| error("OWNED_POLICY_SOURCE"))?;
    if manifest.schema_version != 1
        || manifest.profile != kind.profile()
        || manifest.policies.len() > 128
    {
        return Err(error("OWNED_POLICY_SOURCE"));
    }
    let mut seen = HashSet::new();
    for document in &manifest.policies {
        let raw = serde_json::to_vec(document).map_err(|_| error("OWNED_POLICY_SOURCE"))?;
        // Historical policies may remain queryable after their expiry. Only
        // the selected new-admission policy is parsed against current time.
        let policy = PinnedPolicy::parse(&raw, 0).map_err(|_| error("OWNED_POLICY_SOURCE"))?;
        if !kind.admits(policy.binding())
            || !kind.accepts(&policy)
            || !seen.insert(
                serde_json::to_vec(policy.binding()).map_err(|_| error("OWNED_POLICY_SOURCE"))?,
            )
        {
            return Err(error("OWNED_POLICY_SOURCE"));
        }
    }
    Ok(manifest)
}

/// Read-only check of a profile's fixed policy source for trusted Hub
/// composition, with the same strictness as admission. Returns only a fixed
/// code; neither the path nor any file content can reach the caller.
pub fn check_policy_source(root: &Path, kind: OwnedProfileKind) -> Result<(), &'static str> {
    let root = root.canonicalize().map_err(|_| "OWNED_RUNTIME_ROOT")?;
    read_manifest(&root, kind)
        .map(|_| ())
        .map_err(|_| "OWNED_POLICY_SOURCE")
}

/// Exec-boundary check that a launch belongs to the real bounded profile: the
/// exact pair, its enabled fixed policy source, and a current (unexpired)
/// policy for this complete binding. Only a yes/no leaves this function.
pub fn real_bounded_launch_admitted(root: &Path, binding: &Binding, at_ms: u64) -> bool {
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    current_policy(&root, OwnedProfileKind::RealBounded, binding, at_ms).is_ok()
}

/// Exec-boundary policy of the real project profile: the exact pair, the
/// enabled fixed source with a current policy for this complete binding,
/// and the same bytes (spec hash) the Hub pinned at admission, read from
/// `iris-execution/verification.sqlite3` without writing it. Only the
/// operator's folder limits leave this function; any failure is `None`.
pub fn real_project_launch_policy(
    root: &Path,
    binding: &Binding,
    at_ms: u64,
) -> Option<FolderPolicy> {
    let root = root.canonicalize().ok()?;
    let raw = current_policy(&root, OwnedProfileKind::RealProject, binding, at_ms).ok()?;
    let policy = PinnedPolicy::parse(&raw, at_ms).ok()?;
    let pinned = VerificationStore::read_pinned_policy(
        &root.join("iris-execution").join("verification.sqlite3"),
        binding,
    )
    .ok()?;
    (pinned.spec_hash() == policy.spec_hash() && pinned.binding() == binding)
        .then(|| policy.folder().cloned())
        .flatten()
}

fn current_policy(
    root: &Path,
    kind: OwnedProfileKind,
    binding: &Binding,
    at: u64,
) -> Result<Vec<u8>, (String, String)> {
    if !kind.admits(binding) {
        return Err(error("OWNED_PROVIDER_UNAVAILABLE"));
    }
    let manifest = read_manifest(root, kind)?;
    if !manifest.enabled {
        return Err(error("OWNED_ADMISSION_DISABLED"));
    }
    for document in manifest.policies {
        let raw = serde_json::to_vec(&document).map_err(|_| error("OWNED_POLICY_SOURCE"))?;
        let policy = PinnedPolicy::parse(&raw, 0).map_err(|_| error("OWNED_POLICY_SOURCE"))?;
        if policy.binding() == binding {
            PinnedPolicy::parse(&raw, at).map_err(|_| error("OWNED_POLICY_EXPIRED"))?;
            return Ok(raw);
        }
    }
    Err(error("OWNED_POLICY_NOT_APPROVED"))
}

/// The synthetic profile never claims live provider execution. The real
/// bounded profile claims it only when admission is enabled and the strict
/// policy source contains at least one policy, each already restricted to the
/// single real pair. Neither enables automatic handoff.
fn capability_document(
    kind: OwnedProfileKind,
    admitted: bool,
    manifest: Option<&Manifest>,
) -> Value {
    let live = match kind {
        OwnedProfileKind::SyntheticDispatch => false,
        OwnedProfileKind::RealBounded | OwnedProfileKind::RealProject => {
            admitted && manifest.is_some_and(|m| m.enabled && !m.policies.is_empty())
        }
    };
    json!({"profile":kind.profile(),"admission_enabled":admitted,"query_by_request":true,
        "automatic_handoff":false,"live_provider_execution":live})
}

/// The real bounded profile's `verification` field. The fixed-payload verdict
/// (or null with a fixed reason code) is always paired with the collector's
/// folder effects, so a verdict is never read without them. Pass or fail, it is
/// a record only: it verifies neither the task, the effects nor remote jobs.
fn real_bounded_verification_document(
    receipt: &Value,
    collection: &Value,
    reason: Option<&str>,
) -> Value {
    json!({"kind":"real_bounded_verification","verdict":receipt["verdict"],"reason":reason,
        "folder_effects":collection["effects"],"folder_reason":collection["reason"],
        "receipt":receipt,"task_outcome_verified":false,"effects_verified":false,
        "remote_jobs_stopped":false})
}

/// What a real project collection never observes; always listed.
const UNOBSERVED: [&str; 4] = ["outside-approved-cwd", "network", "remote-jobs", "acl"];

/// The real project profile's `verification` field (shared contract with
/// Iris): a fixed key set, never a verdict. While a condition is missing the
/// record fields are null and `reason` is the waiting code; once filled,
/// `reason` is the watchdog stop code or null. A record only: the three
/// flags are always false and the check is never run by the collector.
fn real_project_record(reason: Option<&str>, collection: Option<&Value>) -> Value {
    let field = |name: &str| collection.map_or(Value::Null, |c| c[name].clone());
    json!({"kind":"real_project_record","reason":reason,
        "folder_effects":field("effects"),"folder_reason":field("reason"),
        "change_count":field("change_count"),"changes_sha256":field("changes_sha256"),
        "check":{"status":"not-run-by-collector"},"unobserved":UNOBSERVED,
        "task_outcome_verified":false,"effects_verified":false,"remote_jobs_stopped":false})
}

/// Hub-side watchdog of one running real project attempt. It observes Core's
/// permission-prompt state of the exact session and the run time since the
/// owner bound the provider root. When the prompt has been continuously open
/// for the limit, or the run time reaches the pinned `max_runtime_sec`, it
/// saves the reason, then requests the owner-bound stop through the same
/// dispatcher path as `owned_stop` (deterministic request ID, no retry, no
/// handoff). Confirmation is only the owner's later `local_tree_ceased`.
struct Watchdog {
    state: Arc<WatchState>,
    port: Arc<dyn OwnedLaunchPort>,
    owners: Arc<Mutex<OwnerStore>>,
    verifications: Arc<Mutex<VerificationStore>>,
    dispatcher: Option<Weak<OwnedDispatcher>>,
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok().flatten()
}

impl Watchdog {
    fn set(&self, binding: &Binding, entry: WatchEntry) {
        if let Ok(mut entries) = self.state.entries.lock() {
            entries.insert(attempt_key(binding), entry);
        }
    }

    async fn run(self, ticket: OwnerTicket, session: SessionBinding) {
        let binding = ticket.binding.clone();
        let store = self.verifications.clone();
        let pin_binding = binding.clone();
        // The pinned policy's limit. A missing pin cannot have launched; the
        // default (never above the policy maximum) stops earlier, not later.
        let max_runtime_ms = blocking(move || {
            store.lock().ok()?.policy_pin(&pin_binding).ok()?["folder_policy"]["max_runtime_sec"]
                .as_u64()
        })
        .await
        .unwrap_or(DEFAULT_MAX_RUNTIME_SEC)
        .saturating_mul(1000);
        let limit_ms = self.state.config.awaiting_limit.as_millis() as u64;
        let mut entry = WatchEntry::default();
        loop {
            tokio::time::sleep(self.state.config.poll_interval).await;
            let owners = self.owners.clone();
            let query_binding = binding.clone();
            let Some(owner) =
                blocking(move || owners.lock().ok()?.query(&query_binding).ok()).await
            else {
                return;
            };
            if owner["ticket"] != json!(ticket) || owner["local_tree_ceased"] == true {
                return;
            }
            let Ok(now) = now_ms() else { return };
            let awaiting = self.port.awaiting_approval(session);
            entry.awaiting_approval = awaiting;
            entry.observed_at_ms = Some(now);
            entry.awaiting_since_ms = match awaiting {
                Some(true) => Some(entry.awaiting_since_ms.unwrap_or(now)),
                _ => None,
            };
            self.set(&binding, entry.clone());
            let runtime_ms = owner["root_started_at_ms"]
                .as_u64()
                .map(|start| now.saturating_sub(start));
            let reason = if entry
                .awaiting_since_ms
                .is_some_and(|since| now.saturating_sub(since) >= limit_ms)
            {
                STOPPED_AWAITING_APPROVAL
            } else if runtime_ms.is_some_and(|ms| ms >= max_runtime_ms) {
                STOPPED_RUNTIME_LIMIT
            } else {
                continue;
            };
            self.stop(&ticket, reason, now, entry.awaiting_since_ms, runtime_ms)
                .await;
            return;
        }
    }

    async fn stop(
        &self,
        ticket: &OwnerTicket,
        reason: &str,
        now: u64,
        awaiting_since_ms: Option<u64>,
        runtime_ms: Option<u64>,
    ) {
        let request_id = auto_stop_request_id(&ticket.binding);
        let document = json!({"kind":"auto_stop","ticket":ticket,"reason":reason,
            "stop_request_id":request_id,"requested_at_ms":now,
            "awaiting_since_ms":awaiting_since_ms,"runtime_ms":runtime_ms,
            "awaiting_limit_ms":self.state.config.awaiting_limit.as_millis() as u64,
            "task_outcome":"unknown","effects_verified":false,"task_outcome_verified":false,
            "remote_jobs_stopped":false});
        let owners = self.owners.clone();
        let record_ticket = ticket.clone();
        // The reason is saved before the stop request. A provider that has
        // already ended (AUTO_STOP_ORDER) needs no stop; any other storage
        // failure still stops, because stopping is the safe direction.
        let recorded = tokio::task::spawn_blocking(move || {
            owners
                .lock()
                .map_err(|_| "OWNED_STORAGE".to_string())?
                .record_auto_stop(&record_ticket, &document)
                .map_err(|e| e.code)
        })
        .await;
        if matches!(&recorded, Ok(Err(code)) if code == "AUTO_STOP_ORDER") {
            return;
        }
        let Some(dispatcher) = self.dispatcher.as_ref().and_then(Weak::upgrade) else {
            return;
        };
        let _ = dispatcher.request_stop(&request_id, &ticket.binding).await;
    }
}

fn identity(
    binding: &Binding,
    principal: &str,
    workspace: &str,
    host: &str,
) -> Result<(), (String, String)> {
    check_binding(binding).map_err(|_| error("INVALID_BINDING"))?;
    if binding.principal_id != principal
        || binding.workspace_id != workspace
        || binding.execution_host_id != host
    {
        return Err(error("PRINCIPAL_MISMATCH"));
    }
    Ok(())
}

fn arguments(
    kind: OwnedProfileKind,
    tool: &str,
    args: &Value,
) -> Result<(Option<Binding>, Option<String>), (String, String)> {
    let object = args.as_object().ok_or_else(|| error("INVALID_ARGUMENT"))?;
    if args["profile"] != kind.profile() {
        return Err(error("OWNED_PROFILE_REQUIRED"));
    }
    let expected: &[&str] = match tool {
        "owned_start" | "owned_stop" => &["profile", "binding", "request_id"],
        "owned_poll" | "owned_evidence" => &["profile", "binding"],
        "owned_request" => &["profile", "request_id"],
        _ => return Err(error("EXECUTOR_TOOL")),
    };
    if object.len() != expected.len()
        || object.keys().any(|name| !expected.contains(&name.as_str()))
    {
        return Err(error("INVALID_ARGUMENT"));
    }
    let binding = if expected.contains(&"binding") {
        Some(
            serde_json::from_value(args["binding"].clone())
                .map_err(|_| error("INVALID_BINDING"))?,
        )
    } else {
        None
    };
    let request_id = if expected.contains(&"request_id") {
        let id = args["request_id"]
            .as_str()
            .ok_or_else(|| error("INVALID_ARGUMENT"))?;
        if !super::iris_execution_owner::id(id) {
            return Err(error("INVALID_ARGUMENT"));
        }
        Some(id.to_owned())
    } else {
        None
    };
    Ok((binding, request_id))
}

impl OwnedProfile {
    /// The synthetic `iris-owned-dispatch/1` profile, unchanged.
    pub fn new(
        journal: Arc<Mutex<Journal>>,
        owners: Arc<Mutex<OwnerStore>>,
        verifications: VerificationStore,
        runtime_root: PathBuf,
        port: Arc<dyn OwnedLaunchPort>,
        config: DispatcherConfig,
    ) -> Result<Self, String> {
        Self::with_kind(
            OwnedProfileKind::SyntheticDispatch,
            journal,
            owners,
            verifications,
            runtime_root,
            port,
            config,
        )
    }

    /// Explicit `iris-owned-real-bounded/1` composition. Admission reads only
    /// `iris-execution/owned-real-policies.json` and the single real pair.
    pub fn new_real_bounded(
        journal: Arc<Mutex<Journal>>,
        owners: Arc<Mutex<OwnerStore>>,
        verifications: VerificationStore,
        runtime_root: PathBuf,
        port: Arc<dyn OwnedLaunchPort>,
        config: DispatcherConfig,
    ) -> Result<Self, String> {
        Self::with_kind(
            OwnedProfileKind::RealBounded,
            journal,
            owners,
            verifications,
            runtime_root,
            port,
            config,
        )
    }

    /// Explicit `iris-owned-real-project/1` composition with the production
    /// watchdog (60 s permission prompt, policy `max_runtime_sec`).
    pub fn new_real_project(
        journal: Arc<Mutex<Journal>>,
        owners: Arc<Mutex<OwnerStore>>,
        verifications: VerificationStore,
        runtime_root: PathBuf,
        port: Arc<dyn OwnedLaunchPort>,
        config: DispatcherConfig,
    ) -> Result<Self, String> {
        Self::new_real_project_with_watch(
            journal,
            owners,
            verifications,
            runtime_root,
            port,
            config,
            RealProjectWatch::default(),
        )
    }

    /// As [`Self::new_real_project`] with explicit (never longer) watchdog
    /// limits, for trusted composition and isolated tests.
    pub fn new_real_project_with_watch(
        journal: Arc<Mutex<Journal>>,
        owners: Arc<Mutex<OwnerStore>>,
        verifications: VerificationStore,
        runtime_root: PathBuf,
        port: Arc<dyn OwnedLaunchPort>,
        config: DispatcherConfig,
        watch: RealProjectWatch,
    ) -> Result<Self, String> {
        if !watch.valid() {
            return Err("OWNED_WATCH_CONFIGURATION".into());
        }
        Self::compose(
            OwnedProfileKind::RealProject,
            journal,
            owners,
            verifications,
            runtime_root,
            port,
            config,
            Some(watch),
        )
    }

    fn with_kind(
        kind: OwnedProfileKind,
        journal: Arc<Mutex<Journal>>,
        owners: Arc<Mutex<OwnerStore>>,
        verifications: VerificationStore,
        runtime_root: PathBuf,
        port: Arc<dyn OwnedLaunchPort>,
        config: DispatcherConfig,
    ) -> Result<Self, String> {
        Self::compose(
            kind,
            journal,
            owners,
            verifications,
            runtime_root,
            port,
            config,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn compose(
        kind: OwnedProfileKind,
        journal: Arc<Mutex<Journal>>,
        owners: Arc<Mutex<OwnerStore>>,
        verifications: VerificationStore,
        runtime_root: PathBuf,
        port: Arc<dyn OwnedLaunchPort>,
        config: DispatcherConfig,
        watch: Option<RealProjectWatch>,
    ) -> Result<Self, String> {
        if (kind == OwnedProfileKind::RealProject) != watch.is_some() {
            return Err("OWNED_WATCH_CONFIGURATION".into());
        }
        config.validate().map_err(|e| e.to_string())?;
        let watch = watch.map(|config| {
            Arc::new(WatchState {
                config,
                entries: Mutex::new(HashMap::new()),
            })
        });
        let runtime_root = runtime_root
            .canonicalize()
            .map_err(|_| "OWNED_RUNTIME_ROOT")?;
        if !runtime_root.is_dir() {
            return Err("OWNED_RUNTIME_ROOT".into());
        }
        let can_dispatch = journal
            .lock()
            .map_err(|_| "OWNED_STORAGE")?
            .supports_owned_dispatch()
            .map_err(|e| e.code)?;
        let verifications = Arc::new(Mutex::new(verifications));
        let dispatcher = if can_dispatch {
            let policy_root = runtime_root.clone();
            let policy_store = verifications.clone();
            let launch_admission: LaunchAdmission = Arc::new(move |binding| {
                let store = policy_store
                    .lock()
                    .map_err(|_| DispatchError::Persistence)?;
                store
                    .check_admission()
                    .map_err(|_| DispatchError::AdmissionRejected)?;
                let pin = store
                    .policy_pin(binding)
                    .map_err(|_| DispatchError::Persistence)?;
                // Take current time/source after every potentially contended
                // store wait. An admission read from before that wait cannot
                // authorize a policy that has since expired or been disabled.
                let raw = current_policy(
                    &policy_root,
                    kind,
                    binding,
                    now_ms().map_err(|_| DispatchError::Unavailable)?,
                )
                .map_err(|_| DispatchError::AdmissionRejected)?;
                let policy =
                    PinnedPolicy::parse(&raw, now_ms().map_err(|_| DispatchError::Unavailable)?)
                        .map_err(|_| DispatchError::AdmissionRejected)?;
                store
                    .check_admission()
                    .map_err(|_| DispatchError::AdmissionRejected)?;
                if pin["binding"] != json!(binding) || pin["spec_hash"] != policy.spec_hash() {
                    return Err(DispatchError::AdmissionRejected);
                }
                Ok(())
            });
            let mut hooks = DispatcherHooks {
                admission: Some(launch_admission),
                ..Default::default()
            };
            let cell: Arc<OnceLock<Weak<OwnedDispatcher>>> = Arc::new(OnceLock::new());
            match kind {
                OwnedProfileKind::SyntheticDispatch => {}
                OwnedProfileKind::RealBounded => {
                    // A real project grant never runs under the real bounded
                    // profile (and never existed before it).
                    let check: GrantCheck = Arc::new(|grant| !grant.folder_diff());
                    hooks.grant_check = Some(check);
                }
                OwnedProfileKind::RealProject => {
                    let check: GrantCheck = Arc::new(|grant| grant.folder_diff());
                    hooks.grant_check = Some(check);
                    let store = verifications.clone();
                    // The tools of the policy pinned for this binding; the
                    // admission above already matched it to the source.
                    let tools: LaunchTools = Arc::new(move |binding| {
                        let pin = store
                            .lock()
                            .map_err(|_| DispatchError::Persistence)?
                            .policy_pin(binding)
                            .map_err(|_| DispatchError::AdmissionRejected)?;
                        serde_json::from_value(pin["folder_policy"]["allowed_tools"].clone())
                            .map_err(|_| DispatchError::AdmissionRejected)
                    });
                    hooks.launch_tools = Some(tools);
                    let supervisor: LaunchSupervisor = {
                        let state = watch.clone().ok_or("OWNED_WATCH_CONFIGURATION")?;
                        let port = port.clone();
                        let owners = owners.clone();
                        let verifications = verifications.clone();
                        let cell = cell.clone();
                        Arc::new(move |ticket, session| {
                            let watchdog = Watchdog {
                                state: state.clone(),
                                port: port.clone(),
                                owners: owners.clone(),
                                verifications: verifications.clone(),
                                dispatcher: cell.get().cloned(),
                            };
                            tokio::spawn(watchdog.run(ticket, session));
                        })
                    };
                    hooks.supervisor = Some(supervisor);
                }
            }
            let dispatcher = Arc::new(
                OwnedDispatcher::new_with_hooks(
                    journal.clone(),
                    owners.clone(),
                    runtime_root.clone(),
                    port,
                    config,
                    hooks,
                )
                .map_err(|e| e.to_string())?,
            );
            let _ = cell.set(Arc::downgrade(&dispatcher));
            Some(dispatcher)
        } else {
            // Restored/legacy journals remain inspection-only; constructing a
            // dispatcher would reject them and unnecessarily hide old receipts.
            None
        };
        Ok(Self {
            journal,
            owners,
            verifications,
            dispatcher,
            runtime_root,
            kind,
            admission: Arc::new(Mutex::new(())),
            watch,
        })
    }

    pub fn kind(&self) -> OwnedProfileKind {
        self.kind
    }

    pub fn shares(&self, journal: &Arc<Mutex<Journal>>, owners: &Arc<Mutex<OwnerStore>>) -> bool {
        Arc::ptr_eq(&self.journal, journal) && Arc::ptr_eq(&self.owners, owners)
    }

    pub fn capabilities(&self) -> Value {
        let manifest = read_manifest(&self.runtime_root, self.kind).ok();
        let admitted = self.dispatcher.is_some()
            && self
                .journal
                .lock()
                .is_ok_and(|j| j.supports_owned_dispatch().unwrap_or(false))
            && manifest.as_ref().is_some_and(|m| m.enabled)
            && self
                .verifications
                .lock()
                .is_ok_and(|s| s.check_admission().is_ok());
        capability_document(self.kind, admitted, manifest.as_ref())
    }

    fn start(&self, request_id: &str, binding: &Binding, instance: &str) -> ToolResult {
        let _admission = self.admission.lock().map_err(|_| error("OWNED_STORAGE"))?;
        // Replay is identity-only, even after expiry, disablement or restart.
        // It never repeats policy pin, queue admission, owner intent or launch.
        {
            let journal = self.journal.lock().map_err(|_| error("OWNED_STORAGE"))?;
            match journal.query_request(&binding.principal_id, &binding.workspace_id, request_id) {
                Ok(view) => {
                    let receipt = &view["request_receipt"];
                    if receipt["binding"] != json!(binding) || receipt["operation"] != "start" {
                        return Err(error("REQUEST_CONFLICT"));
                    }
                    return Ok(receipt.clone());
                }
                Err(e) if e.code == "NOT_FOUND" => {}
                Err(e) => return Err((e.code, e.detail)),
            }
        }
        if binding.executor_instance_id != instance {
            return Err(error("STALE_INSTANCE"));
        }
        if !self.kind.admits(binding) {
            return Err(error("OWNED_PROVIDER_UNAVAILABLE"));
        }
        let dispatcher = self
            .dispatcher
            .as_ref()
            .ok_or_else(|| error("OWNED_ADMISSION_DISABLED"))?;
        let at = now_ms()?;
        // pin_policy accepts an expired historical pin, while fresh admission
        // must independently validate the exact currently approved policy.
        let raw = current_policy(&self.runtime_root, self.kind, binding, at)?;
        {
            let mut verifications = self
                .verifications
                .lock()
                .map_err(|_| error("OWNED_STORAGE"))?;
            let mut owners = self.owners.lock().map_err(|_| error("OWNED_STORAGE"))?;
            verifications
                .pin_policy(&raw, &mut owners, at)
                .map_err(|e| (e.code, e.detail))?;
        }
        // The policy may have expired while waiting for storage. Refuse before
        // submitting; no consumed claim or native launch exists at this point.
        PinnedPolicy::parse(&raw, now_ms()?).map_err(|_| error("OWNED_POLICY_EXPIRED"))?;
        dispatcher
            .submit(request_id, binding)
            .map_err(|e| error(e.code()))
    }

    fn execution(&self, binding: &Binding) -> ToolResult {
        if let Some(dispatcher) = &self.dispatcher {
            return dispatcher.query(binding).map_err(|e| error(e.code()));
        }
        let mut execution = {
            let journal = self.journal.lock().map_err(|_| error("OWNED_STORAGE"))?;
            let mut value = journal.query(binding).map_err(|e| (e.code, e.detail))?;
            value["dispatch"] = match journal.query_dispatch(binding) {
                Ok(v) => v,
                Err(e) if e.code == "NOT_FOUND" || e.code == "DISPATCH_SCHEMA_REQUIRED" => {
                    Value::Null
                }
                Err(e) => return Err((e.code, e.detail)),
            };
            value
        };
        execution["owner"] = match self
            .owners
            .lock()
            .map_err(|_| error("OWNED_STORAGE"))?
            .query(binding)
        {
            Ok(value) => value,
            Err(e) if e.code == "NOT_FOUND" => Value::Null,
            Err(e) => return Err((e.code, e.detail)),
        };
        Ok(execution)
    }

    /// Fixed-payload comparison for the real bounded profile only (C10). It
    /// runs inside `observe`, which `call` already places on the blocking
    /// lane, so SQLite and mutex waits never hold the Tokio timer lane.
    ///
    /// The comparison is attempted only when all of these are durable: the
    /// owner's local cessation, the collector's outcome receipt, the folder
    /// collection for those same result bytes and effects, the policy pinned
    /// at admission and the collector grant entry pinned at admission.
    /// Otherwise the verdict stays null with a fixed reason code.
    /// The inputs are the existing verifier contract: the owner's exact ticket
    /// and integrity receipt, the approved result permit of the operator grant
    /// (read without writing the grant trust store) and the pinned policy only. One-time per binding: an existing receipt
    /// is returned as history (also under a restore hold) and is never
    /// re-evaluated; a concurrent or restarted observer gets the same
    /// original receipt from the store's own replay. Restored or
    /// inspection-only compositions never write a new receipt.
    fn real_bounded_verification(&self, binding: &Binding) -> ToolResult {
        let (owner, collection) = {
            let owners = self.owners.lock().map_err(|_| error("OWNED_STORAGE"))?;
            let owner = match owners.query(binding) {
                Ok(value) => value,
                Err(e) if e.code == "NOT_FOUND" => Value::Null,
                Err(e) => return Err((e.code, e.detail)),
            };
            let folder = owners
                .query_folder(binding)
                .map_err(|e| (e.code, e.detail))?;
            (owner, folder["collection"].clone())
        };
        let unavailable = |reason: &str| {
            real_bounded_verification_document(&Value::Null, &collection, Some(reason))
        };
        {
            let store = self
                .verifications
                .lock()
                .map_err(|_| error("OWNED_STORAGE"))?;
            match store.query(binding) {
                Ok(receipt) => {
                    return Ok(real_bounded_verification_document(
                        &receipt,
                        &collection,
                        None,
                    ));
                }
                Err(e) if e.code == "NOT_FOUND" => {}
                Err(e) => return Err((e.code, e.detail)),
            }
        }
        let outcome = &owner["outcome_receipt"];
        let reason = if !self.kind.admits(binding) {
            Some("OWNED_PROVIDER_UNAVAILABLE")
        } else if self.dispatcher.is_none() {
            Some("VERIFY_INSPECTION_ONLY")
        } else if owner.is_null() {
            Some("VERIFY_OWNER_MISSING")
        } else if owner["local_tree_ceased"] != true {
            Some("VERIFY_STOP_UNCONFIRMED")
        } else if outcome.is_null() {
            Some("VERIFY_OUTCOME_MISSING")
        } else if collection.is_null() {
            Some("VERIFY_COLLECTION_MISSING")
        } else if outcome["ticket"] != owner["ticket"]
            || collection["ticket"] != owner["ticket"]
            || collection["result_sha256"] != outcome["artifact_sha256"]
            || collection["outcome_sequence"] != outcome["sequence"]
            || collection["effects"] != outcome["reported_effects"]
        {
            Some("VERIFY_COLLECTION_MISMATCH")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Ok(unavailable(reason));
        }
        {
            let store = self
                .verifications
                .lock()
                .map_err(|_| error("OWNED_STORAGE"))?;
            if let Err(e) = store.check_admission() {
                return Ok(unavailable(&e.code));
            }
            match store.policy_pin(binding) {
                Ok(_) => {}
                Err(e) if e.code == "NOT_FOUND" => {
                    return Ok(unavailable("VERIFY_POLICY_NOT_PINNED"));
                }
                Err(e) => return Err((e.code, e.detail)),
            }
        }
        let ticket: OwnerTicket =
            serde_json::from_value(owner["ticket"].clone()).map_err(|_| error("OWNED_STORAGE"))?;
        let Some(digest) = outcome["artifact_sha256"].as_str() else {
            return Ok(unavailable("VERIFY_OUTCOME_MISSING"));
        };
        if ticket.binding != *binding {
            return Ok(unavailable("VERIFY_COLLECTION_MISMATCH"));
        }
        let at = now_ms()?;
        // The approved result permit of the exact collector grant, read
        // without writing the grant trust store and without depending on
        // other entries' files or folders. It must equal the entry pinned at
        // admission. No lock is held while the operator manifest is read.
        let permit = match GrantLoader::pinned_result_permit(
            &self.runtime_root,
            binding,
            (at / 1000) as i64,
        ) {
            Ok(permit) => permit,
            Err(e) => {
                return Ok(unavailable(match e.code.as_str() {
                    "GRANT_NOT_PINNED" | "GRANT_PIN_MISMATCH" => "VERIFY_GRANT_NOT_PINNED",
                    "GRANT_REVOKED" => "VERIFY_GRANT_REVOKED",
                    "GRANT_EXPIRED" => "VERIFY_GRANT_EXPIRED",
                    _ => "VERIFY_GRANT_UNAVAILABLE",
                }));
            }
        };
        // Same lock order as admission: verifier store, then owner store.
        let mut store = self
            .verifications
            .lock()
            .map_err(|_| error("OWNED_STORAGE"))?;
        let owners = self.owners.lock().map_err(|_| error("OWNED_STORAGE"))?;
        match store.verify(&owners, &permit, &ticket, digest, at) {
            Ok(receipt) => Ok(real_bounded_verification_document(
                &receipt,
                &collection,
                None,
            )),
            Err(e) => Ok(unavailable(&e.code)),
        }
    }

    /// Real project record (shared contract with Iris). Filled under the C10
    /// conditions: the owner's confirmed local stop, the collector's outcome
    /// receipt and the folder-diff collection naming the same ticket, result
    /// SHA256, outcome sequence and effects. Read-only: nothing is evaluated
    /// or written here, so restored and inspection-only compositions read it
    /// the same way.
    fn real_project_verification(&self, binding: &Binding) -> ToolResult {
        let (owner, collection, auto_stop) = {
            let owners = self.owners.lock().map_err(|_| error("OWNED_STORAGE"))?;
            let owner = match owners.query(binding) {
                Ok(value) => value,
                Err(e) if e.code == "NOT_FOUND" => Value::Null,
                Err(e) => return Err((e.code, e.detail)),
            };
            let folder = owners
                .query_folder(binding)
                .map_err(|e| (e.code, e.detail))?;
            let auto_stop = owners
                .query_auto_stop(binding)
                .map_err(|e| (e.code, e.detail))?;
            (owner, folder["collection"].clone(), auto_stop)
        };
        let outcome = &owner["outcome_receipt"];
        let waiting = if !self.kind.admits(binding) {
            Some("OWNED_PROVIDER_UNAVAILABLE")
        } else if owner.is_null() {
            Some("VERIFY_OWNER_MISSING")
        } else if owner["local_tree_ceased"] != true {
            Some("VERIFY_STOP_UNCONFIRMED")
        } else if outcome.is_null() {
            Some("VERIFY_OUTCOME_MISSING")
        } else if collection.is_null() {
            Some("VERIFY_COLLECTION_MISSING")
        } else if collection["collector"] != "folder-diff"
            || outcome["ticket"] != owner["ticket"]
            || collection["ticket"] != owner["ticket"]
            || collection["result_sha256"] != outcome["artifact_sha256"]
            || collection["outcome_sequence"] != outcome["sequence"]
            || collection["effects"] != outcome["reported_effects"]
        {
            Some("VERIFY_COLLECTION_MISMATCH")
        } else {
            None
        };
        Ok(match waiting {
            Some(reason) => real_project_record(Some(reason), None),
            None => real_project_record(auto_stop["reason"].as_str(), Some(&collection)),
        })
    }

    /// Watchdog view for the observation: Core's last permission-prompt
    /// state (null when not observed by this Hub, e.g. after a restart), the
    /// run time since the root started, the limits and any saved auto stop.
    fn watch_document(&self, binding: &Binding, execution: &Value) -> ToolResult {
        let Some(state) = &self.watch else {
            return Ok(Value::Null);
        };
        let entry = state
            .entries
            .lock()
            .map_err(|_| error("OWNED_STORAGE"))?
            .get(&attempt_key(binding))
            .cloned()
            .unwrap_or_default();
        let owner = &execution["owner"];
        let end = if owner["local_tree_ceased"] == true {
            owner["cessation_receipt"]["observed_at_ms"].as_u64()
        } else {
            now_ms().ok()
        };
        let runtime_ms = match (owner["root_started_at_ms"].as_u64(), end) {
            (Some(start), Some(end)) => json!(end.saturating_sub(start)),
            _ => Value::Null,
        };
        let max_runtime_sec = match self
            .verifications
            .lock()
            .map_err(|_| error("OWNED_STORAGE"))?
            .policy_pin(binding)
        {
            Ok(pin) => pin["folder_policy"]["max_runtime_sec"].clone(),
            Err(_) => Value::Null,
        };
        let auto_stop = self
            .owners
            .lock()
            .map_err(|_| error("OWNED_STORAGE"))?
            .query_auto_stop(binding)
            .map_err(|e| (e.code, e.detail))?;
        Ok(json!({"awaiting_approval":entry.awaiting_approval,
            "awaiting_since_ms":entry.awaiting_since_ms,"observed_at_ms":entry.observed_at_ms,
            "awaiting_limit_sec":state.config.awaiting_limit.as_secs_f64(),
            "runtime_ms":runtime_ms,"max_runtime_sec":max_runtime_sec,"auto_stop":auto_stop}))
    }

    fn observe(&self, binding: &Binding, request: Value) -> ToolResult {
        let mut execution = self.execution(binding)?;
        if self.kind == OwnedProfileKind::RealProject {
            execution["watch"] = self.watch_document(binding, &execution)?;
        }
        let verification = match self.kind {
            OwnedProfileKind::RealProject => self.real_project_verification(binding)?,
            OwnedProfileKind::RealBounded => self.real_bounded_verification(binding)?,
            OwnedProfileKind::SyntheticDispatch => match self
                .verifications
                .lock()
                .map_err(|_| error("OWNED_STORAGE"))?
                .query(binding)
            {
                Ok(value) => value,
                Err(e) if e.code == "NOT_FOUND" => Value::Null,
                Err(e) => return Err((e.code, e.detail)),
            },
        };
        Ok(
            json!({"profile":self.kind.profile(),"binding":binding,"request":request,
            "execution":execution,"verification":verification}),
        )
    }

    pub async fn call(
        &self,
        tool: &str,
        args: &Value,
        principal: &str,
        workspace: &str,
        host: &str,
        instance: &str,
    ) -> ToolResult {
        let (binding, request_id) = arguments(self.kind, tool, args)?;
        if let Some(binding) = &binding {
            identity(binding, principal, workspace, host)?;
        }
        let this = self.clone();
        let tool = tool.to_owned();
        let principal = principal.to_owned();
        let workspace = workspace.to_owned();
        let host = host.to_owned();
        let instance = instance.to_owned();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || match tool.as_str() {
            "owned_start" => this.start(
                request_id.as_deref().unwrap(),
                binding.as_ref().unwrap(),
                &instance,
            ),
            "owned_request" => {
                let request = this
                    .journal
                    .lock()
                    .map_err(|_| error("OWNED_STORAGE"))?
                    .query_request(&principal, &workspace, request_id.as_deref().unwrap())
                    .map_err(|e| (e.code, e.detail))?;
                let binding: Binding =
                    serde_json::from_value(request["request_receipt"]["binding"].clone())
                        .map_err(|_| error("OWNED_STORAGE"))?;
                identity(&binding, &principal, &workspace, &host)?;
                this.observe(&binding, request)
            }
            "owned_poll" | "owned_evidence" => this.observe(binding.as_ref().unwrap(), Value::Null),
            "owned_stop" => {
                {
                    let journal = this.journal.lock().map_err(|_| error("OWNED_STORAGE"))?;
                    match journal.query_request(
                        &principal,
                        &workspace,
                        request_id.as_deref().unwrap(),
                    ) {
                        Ok(view) => {
                            let receipt = &view["request_receipt"];
                            if receipt["binding"] != json!(binding.as_ref().unwrap())
                                || receipt["operation"] != "stop"
                            {
                                return Err(error("REQUEST_CONFLICT"));
                            }
                            return Ok(receipt.clone());
                        }
                        Err(e) if e.code == "NOT_FOUND" => {}
                        Err(e) => return Err((e.code, e.detail)),
                    }
                }
                let dispatcher = this
                    .dispatcher
                    .as_ref()
                    .ok_or_else(|| error("OWNED_ADMISSION_DISABLED"))?;
                // All SQLite/mutex admission work runs on the blocking lane.
                // The dispatcher releases its own locks before awaiting stop.
                handle
                    .block_on(
                        dispatcher.request_stop(
                            request_id.as_deref().unwrap(),
                            binding.as_ref().unwrap(),
                        ),
                    )
                    .map_err(|e| error(e.code()))
            }
            _ => Err(error("EXECUTOR_TOOL")),
        })
        .await
        .map_err(|_| error("OWNED_WORKER_FAILED"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> Binding {
        Binding {
            principal_id: "operator".into(),
            workspace_id: "workspace".into(),
            plan_id: "plan".into(),
            plan_revision: 1,
            task_id: "task".into(),
            instruction_revision: 1,
            instruction_hash: "a".repeat(64),
            approval_id: "approval".into(),
            approval_revision: 1,
            scope_hash: "b".repeat(64),
            execution_host_id: "host".into(),
            executor_instance_id: "boot".into(),
            attempt_id: "attempt".into(),
            fence: 1,
            provider: "synthetic".into(),
            profile_id: "synthetic".into(),
        }
    }

    fn manifest() -> Value {
        json!({"schema_version":1,"profile":PROFILE,"enabled":true,"policies":[{
            "schema_version":1,"policy_id":"policy","policy_revision":1,
            "verifier":"strict-json-payload-equality/1","binding":binding(),
            "expires_at_ms":1000,"expected_payload":{"answer":42}
        }]})
    }

    fn root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("iris-execution")).unwrap();
        root
    }

    fn write(root: &Path, raw: &[u8]) {
        std::fs::write(root.join("iris-execution").join("owned-policies.json"), raw).unwrap();
    }

    const SYN: OwnedProfileKind = OwnedProfileKind::SyntheticDispatch;
    const REAL: OwnedProfileKind = OwnedProfileKind::RealBounded;

    fn real_binding() -> Binding {
        let mut b = binding();
        b.provider = REAL_BOUNDED_PROVIDER.into();
        b.profile_id = REAL_BOUNDED_PROFILE_ID.into();
        b
    }

    fn real_manifest() -> Value {
        let mut value = manifest();
        value["profile"] = json!(REAL_BOUNDED_PROFILE);
        value["policies"][0]["binding"] = json!(real_binding());
        value
    }

    fn write_real(root: &Path, raw: &[u8]) {
        std::fs::write(
            root.join("iris-execution").join("owned-real-policies.json"),
            raw,
        )
        .unwrap();
    }

    #[test]
    fn explicit_profile_and_exact_tool_arguments_are_required() {
        assert!(
            arguments(
                SYN,
                "owned_start",
                &json!({"profile":PROFILE,"binding":binding(),"request_id":"request"})
            )
            .is_ok()
        );
        for args in [
            json!({"binding":binding(),"request_id":"request"}),
            json!({"profile":"iris-execution/1","binding":binding(),"request_id":"request"}),
            json!({"profile":PROFILE,"binding":binding(),"request_id":"request","expected_payload":true}),
            json!({"profile":PROFILE,"binding":binding(),"request_id":""}),
        ] {
            assert!(arguments(SYN, "owned_start", &args).is_err());
        }
        assert!(
            arguments(
                SYN,
                "owned_request",
                &json!({"profile":PROFILE,"request_id":"request","binding":binding()})
            )
            .is_err()
        );
        assert!(identity(&binding(), "another", "workspace", "host").is_err());
        assert!(identity(&binding(), "operator", "another", "host").is_err());
        assert!(identity(&binding(), "operator", "workspace", "another").is_err());
    }

    #[test]
    fn fixed_policy_source_rejects_ambiguous_or_injected_authority() {
        let root = root();
        let canonical = root.path().canonicalize().unwrap();
        assert!(read_manifest(&canonical, SYN).is_err());
        let valid = manifest();
        write(&canonical, &serde_json::to_vec(&valid).unwrap());
        assert!(read_manifest(&canonical, SYN).unwrap().enabled);
        let mut disabled = valid.clone();
        disabled["enabled"] = json!(false);
        write(&canonical, &serde_json::to_vec(&disabled).unwrap());
        assert!(!read_manifest(&canonical, SYN).unwrap().enabled);
        let mut unknown = valid.clone();
        unknown["provider"] = json!("synthetic");
        let mut duplicate_policy = valid.clone();
        duplicate_policy["policies"]
            .as_array_mut()
            .unwrap()
            .push(valid["policies"][0].clone());
        let mut real_provider = valid.clone();
        real_provider["policies"][0]["binding"]["provider"] = json!("codex");
        let mut unknown_policy = valid.clone();
        unknown_policy["policies"][0]["command"] = json!("untrusted");
        for value in [unknown, duplicate_policy, real_provider, unknown_policy] {
            write(&canonical, &serde_json::to_vec(&value).unwrap());
            assert!(read_manifest(&canonical, SYN).is_err());
        }
        write(&canonical, br#"{"schema_version":1,"profile":"iris-owned-dispatch/1","enabled":true,"enabled":false,"policies":[]}"#);
        assert!(read_manifest(&canonical, SYN).is_err());
        write(&canonical, &vec![b' '; MAX_BYTES + 1]);
        assert!(read_manifest(&canonical, SYN).is_err());
    }

    #[test]
    fn synthetic_profile_still_rejects_the_real_pair_everywhere() {
        let root = root();
        let canonical = root.path().canonicalize().unwrap();
        let mut claude = manifest();
        claude["policies"][0]["binding"] = json!(real_binding());
        write(&canonical, &serde_json::to_vec(&claude).unwrap());
        assert!(read_manifest(&canonical, SYN).is_err());
        assert_eq!(
            current_policy(&canonical, SYN, &real_binding(), 0)
                .unwrap_err()
                .0,
            "OWNED_PROVIDER_UNAVAILABLE"
        );
        // The real policy source never satisfies the synthetic profile.
        write(&canonical, &serde_json::to_vec(&manifest()).unwrap());
        write_real(&canonical, &serde_json::to_vec(&real_manifest()).unwrap());
        std::fs::remove_file(canonical.join("iris-execution/owned-policies.json")).unwrap();
        assert!(read_manifest(&canonical, SYN).is_err());
        assert!(
            arguments(
                SYN,
                "owned_start",
                &json!({"profile":REAL_BOUNDED_PROFILE,"binding":binding(),"request_id":"r"})
            )
            .is_err()
        );
    }

    #[test]
    fn real_profile_admits_only_the_single_pair_from_its_own_strict_source() {
        let root = root();
        let canonical = root.path().canonicalize().unwrap();
        // The synthetic policy file is not a source for the real profile.
        write(&canonical, &serde_json::to_vec(&manifest()).unwrap());
        assert!(read_manifest(&canonical, REAL).is_err());
        let valid = real_manifest();
        write_real(&canonical, &serde_json::to_vec(&valid).unwrap());
        assert!(read_manifest(&canonical, REAL).unwrap().enabled);
        assert!(current_policy(&canonical, REAL, &real_binding(), 0).is_ok());
        for (provider, profile) in [
            ("codex", REAL_BOUNDED_PROFILE_ID),
            ("synthetic", "synthetic"),
            (REAL_BOUNDED_PROVIDER, "synthetic"),
            (REAL_BOUNDED_PROVIDER, "claude-other"),
            ("codex", "codex"),
        ] {
            let mut b = real_binding();
            b.provider = provider.into();
            b.profile_id = profile.into();
            assert_eq!(
                current_policy(&canonical, REAL, &b, 0).unwrap_err().0,
                "OWNED_PROVIDER_UNAVAILABLE",
                "{provider}/{profile}"
            );
            let mut other = valid.clone();
            other["policies"][0]["binding"] = json!(b);
            write_real(&canonical, &serde_json::to_vec(&other).unwrap());
            assert!(
                read_manifest(&canonical, REAL).is_err(),
                "{provider}/{profile}"
            );
        }
        let mut wrong_profile = valid.clone();
        wrong_profile["profile"] = json!(PROFILE);
        let mut unknown = valid.clone();
        unknown["provider"] = json!(REAL_BOUNDED_PROVIDER);
        let mut duplicate = valid.clone();
        duplicate["policies"]
            .as_array_mut()
            .unwrap()
            .push(valid["policies"][0].clone());
        let mut unknown_policy = valid.clone();
        unknown_policy["policies"][0]["launch_mode"] = json!("bounded-edit");
        for value in [wrong_profile, unknown, duplicate, unknown_policy] {
            write_real(&canonical, &serde_json::to_vec(&value).unwrap());
            assert!(read_manifest(&canonical, REAL).is_err());
        }
        write_real(&canonical, br#"{"schema_version":1,"profile":"iris-owned-real-bounded/1","enabled":true,"enabled":false,"policies":[]}"#);
        assert!(read_manifest(&canonical, REAL).is_err());
        write_real(&canonical, &vec![b' '; MAX_BYTES + 1]);
        assert!(read_manifest(&canonical, REAL).is_err());
        // A hard-linked alias of the fixed source is refused.
        let source = canonical.join("iris-execution/owned-real-policies.json");
        write_real(&canonical, &serde_json::to_vec(&valid).unwrap());
        assert!(read_manifest(&canonical, REAL).is_ok());
        std::fs::hard_link(&source, canonical.join("alias.json")).unwrap();
        assert!(read_manifest(&canonical, REAL).is_err());
        assert!(
            arguments(
                REAL,
                "owned_start",
                &json!({"profile":PROFILE,"binding":real_binding(),"request_id":"r"})
            )
            .is_err()
        );
        assert!(
            arguments(
                REAL,
                "owned_start",
                &json!({"profile":REAL_BOUNDED_PROFILE,"binding":real_binding(),"request_id":"r"})
            )
            .is_ok()
        );
    }

    #[test]
    fn live_provider_capability_requires_real_admission_and_policy() {
        let enabled: Manifest = serde_json::from_value(real_manifest()).unwrap();
        let mut empty_value = real_manifest();
        empty_value["policies"] = json!([]);
        let empty: Manifest = serde_json::from_value(empty_value).unwrap();
        let mut disabled_value = real_manifest();
        disabled_value["enabled"] = json!(false);
        let disabled: Manifest = serde_json::from_value(disabled_value).unwrap();
        let real = capability_document(REAL, true, Some(&enabled));
        assert_eq!(real["live_provider_execution"], true);
        assert_eq!(real["automatic_handoff"], false);
        assert_eq!(real["profile"], REAL_BOUNDED_PROFILE);
        for (admitted, manifest) in [
            (false, Some(&enabled)),
            (true, Some(&empty)),
            (true, Some(&disabled)),
            (true, None),
        ] {
            let value = capability_document(REAL, admitted, manifest);
            assert_eq!(value["live_provider_execution"], false);
            assert_eq!(value["automatic_handoff"], false);
        }
        let synthetic = capability_document(SYN, true, Some(&enabled));
        assert_eq!(
            synthetic,
            json!({"profile":PROFILE,"admission_enabled":true,"query_by_request":true,
                "automatic_handoff":false,"live_provider_execution":false})
        );
    }
}
