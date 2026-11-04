//! Explicit, opt-in owned dispatch for isolated fake-port acceptance.
//! Native evidence stays native: neither payload equality nor local cessation
//! grants task success, global effects proof, or automatic handoff authority.
use super::{
    iris_execution_journal::{Binding, Journal},
    iris_execution_mcp::{MAX_BYTES, parse_json},
    iris_execution_owner::{OwnerStore, check_binding, secure_open},
    iris_execution_verification_store::VerificationStore,
    iris_execution_verifier::PinnedPolicy,
};
use crate::application::{
    iris_execution_dispatcher::{
        DispatchError, DispatcherConfig, LaunchAdmission, OwnedDispatcher,
    },
    iris_execution_launch::OwnedLaunchPort,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

pub const PROFILE: &str = "iris-owned-dispatch/1";
/// Separate opt-in version for one bounded real-provider acceptance. It never
/// widens the synthetic profile above and has its own fixed policy source.
pub const REAL_BOUNDED_PROFILE: &str = "iris-owned-real-bounded/1";
/// The single provider/subscription pair the real bounded profile admits. This
/// pair is the only place that names it; every gate calls real_bounded_binding.
pub const REAL_BOUNDED_PROVIDER: &str = "claude";
pub const REAL_BOUNDED_PROFILE_ID: &str = "claude-hiroshi";
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
}
impl OwnedProfileKind {
    pub fn profile(self) -> &'static str {
        match self {
            Self::SyntheticDispatch => PROFILE,
            Self::RealBounded => REAL_BOUNDED_PROFILE,
        }
    }
    fn policy_file(self) -> &'static str {
        match self {
            Self::SyntheticDispatch => "owned-policies.json",
            Self::RealBounded => "owned-real-policies.json",
        }
    }
    fn admits(self, binding: &Binding) -> bool {
        match self {
            Self::SyntheticDispatch => {
                binding.provider == "synthetic" && binding.profile_id == "synthetic"
            }
            Self::RealBounded => real_bounded_binding(binding),
        }
    }
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
        OwnedProfileKind::RealBounded => {
            admitted && manifest.is_some_and(|m| m.enabled && !m.policies.is_empty())
        }
    };
    json!({"profile":kind.profile(),"admission_enabled":admitted,"query_by_request":true,
        "automatic_handoff":false,"live_provider_execution":live})
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

    fn with_kind(
        kind: OwnedProfileKind,
        journal: Arc<Mutex<Journal>>,
        owners: Arc<Mutex<OwnerStore>>,
        verifications: VerificationStore,
        runtime_root: PathBuf,
        port: Arc<dyn OwnedLaunchPort>,
        config: DispatcherConfig,
    ) -> Result<Self, String> {
        config.validate().map_err(|e| e.to_string())?;
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
            Some(Arc::new(
                OwnedDispatcher::new_with_admission(
                    journal.clone(),
                    owners.clone(),
                    runtime_root.clone(),
                    port,
                    config,
                    Some(launch_admission),
                )
                .map_err(|e| e.to_string())?,
            ))
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

    fn observe(&self, binding: &Binding, request: Value) -> ToolResult {
        let execution = self.execution(binding)?;
        let verification = match self
            .verifications
            .lock()
            .map_err(|_| error("OWNED_STORAGE"))?
            .query(binding)
        {
            Ok(value) => value,
            Err(e) if e.code == "NOT_FOUND" => Value::Null,
            Err(e) => return Err((e.code, e.detail)),
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
