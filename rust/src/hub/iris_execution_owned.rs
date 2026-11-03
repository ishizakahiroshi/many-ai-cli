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
type ToolResult = Result<Value, (String, String)>;

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

fn read_manifest(root: &Path) -> Result<Manifest, (String, String)> {
    // secure_open rejects path aliases/reparse points and validates the final
    // opened handle. Only this fixed operator source can select a policy.
    let relative = Path::new("iris-execution").join("owned-policies.json");
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
    if manifest.schema_version != 1 || manifest.profile != PROFILE || manifest.policies.len() > 128
    {
        return Err(error("OWNED_POLICY_SOURCE"));
    }
    let mut seen = HashSet::new();
    for document in &manifest.policies {
        let raw = serde_json::to_vec(document).map_err(|_| error("OWNED_POLICY_SOURCE"))?;
        // Historical policies may remain queryable after their expiry. Only
        // the selected new-admission policy is parsed against current time.
        let policy = PinnedPolicy::parse(&raw, 0).map_err(|_| error("OWNED_POLICY_SOURCE"))?;
        if policy.binding().provider != "synthetic"
            || policy.binding().profile_id != "synthetic"
            || !seen.insert(
                serde_json::to_vec(policy.binding()).map_err(|_| error("OWNED_POLICY_SOURCE"))?,
            )
        {
            return Err(error("OWNED_POLICY_SOURCE"));
        }
    }
    Ok(manifest)
}

fn current_policy(root: &Path, binding: &Binding, at: u64) -> Result<Vec<u8>, (String, String)> {
    if binding.provider != "synthetic" || binding.profile_id != "synthetic" {
        return Err(error("OWNED_PROVIDER_UNAVAILABLE"));
    }
    let manifest = read_manifest(root)?;
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
    tool: &str,
    args: &Value,
) -> Result<(Option<Binding>, Option<String>), (String, String)> {
    let object = args.as_object().ok_or_else(|| error("INVALID_ARGUMENT"))?;
    if args["profile"] != PROFILE {
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
    pub fn new(
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
            admission: Arc::new(Mutex::new(())),
        })
    }

    pub fn shares(&self, journal: &Arc<Mutex<Journal>>, owners: &Arc<Mutex<OwnerStore>>) -> bool {
        Arc::ptr_eq(&self.journal, journal) && Arc::ptr_eq(&self.owners, owners)
    }

    pub fn capabilities(&self) -> Value {
        let admitted = self.dispatcher.is_some()
            && self
                .journal
                .lock()
                .is_ok_and(|j| j.supports_owned_dispatch().unwrap_or(false))
            && read_manifest(&self.runtime_root).is_ok_and(|m| m.enabled)
            && self
                .verifications
                .lock()
                .is_ok_and(|s| s.check_admission().is_ok());
        json!({"profile":PROFILE,"admission_enabled":admitted,"query_by_request":true,
            "automatic_handoff":false,"live_provider_execution":false})
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
        if binding.provider != "synthetic" || binding.profile_id != "synthetic" {
            return Err(error("OWNED_PROVIDER_UNAVAILABLE"));
        }
        let dispatcher = self
            .dispatcher
            .as_ref()
            .ok_or_else(|| error("OWNED_ADMISSION_DISABLED"))?;
        let at = now_ms()?;
        // pin_policy accepts an expired historical pin, while fresh admission
        // must independently validate the exact currently approved policy.
        let raw = current_policy(&self.runtime_root, binding, at)?;
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
            json!({"profile":PROFILE,"binding":binding,"request":request,
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
        let (binding, request_id) = arguments(tool, args)?;
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

    #[test]
    fn explicit_profile_and_exact_tool_arguments_are_required() {
        assert!(
            arguments(
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
            assert!(arguments("owned_start", &args).is_err());
        }
        assert!(
            arguments(
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
        assert!(read_manifest(&canonical).is_err());
        let valid = manifest();
        write(&canonical, &serde_json::to_vec(&valid).unwrap());
        assert!(read_manifest(&canonical).unwrap().enabled);
        let mut disabled = valid.clone();
        disabled["enabled"] = json!(false);
        write(&canonical, &serde_json::to_vec(&disabled).unwrap());
        assert!(!read_manifest(&canonical).unwrap().enabled);
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
            assert!(read_manifest(&canonical).is_err());
        }
        write(&canonical, br#"{"schema_version":1,"profile":"iris-owned-dispatch/1","enabled":true,"enabled":false,"policies":[]}"#);
        assert!(read_manifest(&canonical).is_err());
        write(&canonical, &vec![b' '; MAX_BYTES + 1]);
        assert!(read_manifest(&canonical).is_err());
    }
}
