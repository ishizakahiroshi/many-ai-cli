//! `iris-owned-real-project/1` acceptance (Many Iris C9 C2). No real provider,
//! Hub, Iris or network is started. The Hub-side tests use a stand-in launch
//! port; the wrapper-side tests (Windows) run the test executable itself as a
//! synthetic provider that edits a copied work folder. Every value is
//! synthetic.
use many_ai_cli::{
    application::{
        iris_execution_dispatcher::DispatcherConfig,
        iris_execution_launch::{OwnedLaunchPort, prepare_spec, prepare_spec_with_tools},
    },
    hub::{
        iris_execution::{IrisExecutionHttp, RealBoundedComposition},
        iris_execution_folder::{
            NO_CHANGE, PAYLOAD_ONLY, SCOPED_CHANGE, UNEXPECTED_CHANGE, baseline_digest, snapshot,
        },
        iris_execution_grants::GrantLoader,
        iris_execution_journal::{Binding, Journal},
        iris_execution_owned::{
            OwnedProfile, REAL_BOUNDED_PROFILE, REAL_BOUNDED_PROFILE_ID, REAL_BOUNDED_PROVIDER,
            REAL_PROJECT_PROFILE, RealProjectWatch, auto_stop_request_id,
        },
        iris_execution_owner::{
            OwnerStore, OwnerTicket, STOPPED_AWAITING_APPROVAL, STOPPED_RUNTIME_LIMIT,
        },
        iris_execution_runtime::BridgeError,
        iris_execution_verification_store::VerificationStore,
    },
    proto::core::{
        CoreFuture, HttpWaitCancellation, LiveSessionId, SessionBinding, SessionIncarnation,
        SpawnWaitOutcome, WrappedSpawnSpec, WrapperConnectionId,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const INSTRUCTION: &str = "Synthetic real project instruction. Fixed fixture text.\n";
/// Case A's tool value (shared contract, 2026-11-04 update). Configuration
/// data only: nothing in the product embeds it.
const TOOL_A: &str = "Bash(python -B -m pytest -p no:cacheprovider --basetemp=.pytest-tmp:*)";
const RECORD_KEYS: [&str; 11] = [
    "kind",
    "reason",
    "folder_effects",
    "folder_reason",
    "change_count",
    "changes_sha256",
    "check",
    "unobserved",
    "task_outcome_verified",
    "effects_verified",
    "remote_jobs_stopped",
];

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn binding(task: &str, instance: &str) -> Binding {
    Binding {
        principal_id: "iris".into(),
        workspace_id: "workspace".into(),
        plan_id: "plan".into(),
        plan_revision: 1,
        task_id: task.into(),
        instruction_revision: 1,
        instruction_hash: sha(INSTRUCTION.as_bytes()),
        approval_id: "approval".into(),
        approval_revision: 1,
        scope_hash: sha(b"synthetic scope"),
        execution_host_id: "host".into(),
        executor_instance_id: instance.into(),
        attempt_id: format!("attempt-{task}"),
        fence: 1,
        provider: REAL_BOUNDED_PROVIDER.into(),
        profile_id: REAL_BOUNDED_PROFILE_ID.into(),
    }
}

fn grant(b: &Binding, workspace: &Path) -> Value {
    json!({"binding":b,"expires_at":(now_ms()/1000) as i64 + 600,"revoked":false,
        "approved_cwd":workspace.canonicalize().unwrap(),"instruction_path":"instruction.txt",
        "result_path":format!("{}-result.json",b.task_id),"max_artifact_bytes":65536,
        "effect_scope":[NO_CHANGE,SCOPED_CHANGE,UNEXPECTED_CHANGE],"allow_remote_jobs":false,
        "launch_mode":"bounded-edit","collector":"folder-diff"})
}

fn folder_policy(
    b: &Binding,
    workspace: &Path,
    allowed: &[&str],
    tools: &[&str],
    max: u64,
) -> Value {
    let entries = snapshot(&workspace.canonicalize().unwrap()).unwrap();
    json!({"schema_version":1,"policy_id":"policy","policy_revision":1,
        "verifier":"folder-change-record/1","binding":b,"expires_at_ms":now_ms()+600_000,
        "baseline_sha256":baseline_digest(&entries).unwrap(),
        "baseline_entry_count":entries.len(),"allowed_paths":allowed,"allowed_tools":tools,
        "max_runtime_sec":max})
}

fn write_json(path: &Path, value: &Value) {
    std::fs::write(path, value.to_string()).unwrap();
}

/// The verification field is exactly the shared `real_project_record`.
fn assert_record(v: &Value) {
    let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    let mut expected: Vec<_> = RECORD_KEYS.iter().map(|k| k.to_string()).collect();
    expected.sort();
    assert_eq!(keys, expected, "{v}");
    assert!(v.get("verdict").is_none());
    assert_eq!(v["kind"], "real_project_record");
    assert_eq!(v["check"], json!({"status":"not-run-by-collector"}));
    assert_eq!(
        v["unobserved"],
        json!(["outside-approved-cwd", "network", "remote-jobs", "acl"])
    );
    for flag in [
        "task_outcome_verified",
        "effects_verified",
        "remote_jobs_stopped",
    ] {
        assert_eq!(v[flag], false, "{flag}");
    }
}

fn assert_waiting(v: &Value, reason: &str) {
    assert_record(v);
    assert_eq!(v["reason"], reason, "{v}");
    for field in [
        "folder_effects",
        "folder_reason",
        "change_count",
        "changes_sha256",
    ] {
        assert_eq!(v[field], Value::Null, "{field}");
    }
}

async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("deadline: {what}"));
}

// ---------------------------------------------------------------------------
// Hub side: a stand-in port that claims and binds the root like a wrapper.

#[derive(Default)]
struct PortState {
    launches: AtomicUsize,
    awaiting: AtomicBool,
    specs: Mutex<Vec<WrappedSpawnSpec>>,
    stops: Mutex<Vec<(String, String)>>,
    sessions: Mutex<Vec<SessionBinding>>,
}
struct FakePort {
    owners: Arc<Mutex<OwnerStore>>,
    state: Arc<PortState>,
}
impl OwnedLaunchPort for FakePort {
    fn launch_registered<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            let ticket: OwnerTicket =
                serde_json::from_str(&spec.registration_metadata.iris_owner_context).unwrap();
            let index = self.state.launches.fetch_add(1, Ordering::SeqCst) as i64 + 1;
            let session = SessionBinding {
                session: LiveSessionId(100 + index),
                incarnation: SessionIncarnation(1),
                wrapper: WrapperConnectionId(200 + index as u64),
            };
            {
                let mut owners = self.owners.lock().unwrap();
                owners
                    .claim_spawn(&ticket, session.session.0, now_ms())
                    .unwrap();
                owners
                    .bind_root(&ticket, 9000 + index as u32, now_ms())
                    .unwrap();
            }
            self.state.specs.lock().unwrap().push(spec);
            self.state.sessions.lock().unwrap().push(session);
            SpawnWaitOutcome::Registered(session)
        })
    }
    fn is_current(&self, binding: SessionBinding) -> bool {
        self.state.sessions.lock().unwrap().contains(&binding)
    }
    fn bind_owner(&self, _: &OwnerTicket, _: SessionBinding) -> Result<Value, BridgeError> {
        Ok(json!({"bound":true,"owned_execution_ceased":false}))
    }
    fn stop_owner<'a>(
        &'a self,
        ticket: &'a OwnerTicket,
        id: &'a str,
    ) -> CoreFuture<'a, Result<Value, BridgeError>> {
        Box::pin(async move {
            self.state
                .stops
                .lock()
                .unwrap()
                .push((ticket.binding.attempt_id.clone(), id.to_string()));
            Ok(json!({"stop_request_id":id,"owned_execution_ceased":false}))
        })
    }
    fn awaiting_approval(&self, binding: SessionBinding) -> Option<bool> {
        self.is_current(binding)
            .then(|| self.state.awaiting.load(Ordering::SeqCst))
    }
}

struct HubFixture {
    root: tempfile::TempDir,
    _work: tempfile::TempDir,
    bindings: Vec<Binding>,
    journal: Arc<Mutex<Journal>>,
    owners: Arc<Mutex<OwnerStore>>,
    state: Arc<PortState>,
}
impl HubFixture {
    fn new(count: usize, tools: &[&str], max_runtime_sec: u64) -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let path = root.path();
        std::fs::create_dir_all(path.join("iris-execution")).unwrap();
        std::fs::create_dir_all(path.join("iris-execution-artifacts")).unwrap();
        std::fs::write(
            path.join("iris-execution-artifacts/instruction.txt"),
            INSTRUCTION,
        )
        .unwrap();
        let mut grants = Vec::new();
        let mut policies = Vec::new();
        let mut bindings = Vec::new();
        for i in 0..count {
            let b = binding(&format!("task-{i}"), "boot");
            let workspace = work.path().join(format!("case-{i}"));
            std::fs::create_dir_all(workspace.join("src")).unwrap();
            std::fs::write(workspace.join("src/a.py"), b"print(1)\n").unwrap();
            grants.push(grant(&b, &workspace));
            policies.push(folder_policy(
                &b,
                &workspace,
                &["src"],
                tools,
                max_runtime_sec,
            ));
            bindings.push(b);
        }
        write_json(
            &path.join("iris-execution-approved-grants.json"),
            &json!({"schema_version":1,"manifest_revision":1,"approval_state":"approved",
                "grants":grants}),
        );
        write_json(
            &path.join("iris-execution/owned-real-project-policies.json"),
            &json!({"schema_version":1,"profile":REAL_PROJECT_PROFILE,"enabled":true,
                "policies":policies}),
        );
        let journal = Arc::new(Mutex::new(
            Journal::open(&path.join("iris-execution/journal.sqlite3"), "host", "boot").unwrap(),
        ));
        let owners = Arc::new(Mutex::new(
            OwnerStore::open(&path.join("iris-execution/owner.sqlite3")).unwrap(),
        ));
        Self {
            root,
            _work: work,
            bindings,
            journal,
            owners,
            state: Arc::new(PortState::default()),
        }
    }
    fn config() -> DispatcherConfig {
        DispatcherConfig {
            queue_capacity: 8,
            registration_timeout: Duration::from_secs(5),
            root_timeout: Duration::from_secs(5),
            poll_interval: Duration::from_millis(5),
            stop_timeout: Duration::from_millis(500),
        }
    }
    fn port(&self) -> Arc<FakePort> {
        Arc::new(FakePort {
            owners: self.owners.clone(),
            state: self.state.clone(),
        })
    }
    fn verifications(&self) -> VerificationStore {
        VerificationStore::open(&self.root.path().join("iris-execution/verification.sqlite3"))
            .unwrap()
    }
    fn profile(&self, watch: RealProjectWatch) -> OwnedProfile {
        OwnedProfile::new_real_project_with_watch(
            self.journal.clone(),
            self.owners.clone(),
            self.verifications(),
            self.root.path().to_path_buf(),
            self.port(),
            Self::config(),
            watch,
        )
        .unwrap()
    }
}

async fn call(profile: &OwnedProfile, tool: &str, args: Value) -> Result<Value, (String, String)> {
    profile
        .call(tool, &args, "iris", "workspace", "host", "boot")
        .await
}
fn start_args(b: &Binding, request: &str) -> Value {
    json!({"profile":REAL_PROJECT_PROFILE,"binding":b,"request_id":request})
}
fn fast_watch() -> RealProjectWatch {
    RealProjectWatch {
        awaiting_limit: Duration::from_millis(200),
        poll_interval: Duration::from_millis(20),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn awaiting_approval_for_the_limit_is_stopped_with_its_reason() {
    let f = HubFixture::new(1, &[TOOL_A], 1800);
    f.state.awaiting.store(true, Ordering::SeqCst);
    let profile = f.profile(fast_watch());
    let b = f.bindings[0].clone();
    call(&profile, "owned_start", start_args(&b, "start-0"))
        .await
        .unwrap();
    until("auto stop", || f.state.stops.lock().unwrap().len() == 1).await;
    // The launch carried exactly the pinned policy's tools and acceptEdits.
    let spec = f.state.specs.lock().unwrap()[0].clone();
    assert_eq!(spec.permission_mode, "acceptEdits");
    assert_eq!(spec.grants.allowed_tools(), [TOOL_A.to_string()]);
    assert!(!spec.risk_confirmed);
    let stops = f.state.stops.lock().unwrap().clone();
    assert_eq!(stops, [(b.attempt_id.clone(), auto_stop_request_id(&b))]);
    let row = f.owners.lock().unwrap().query_auto_stop(&b).unwrap();
    assert_eq!(row["reason"], STOPPED_AWAITING_APPROVAL);
    assert_eq!(row["task_outcome"], "unknown");
    println!(
        "C2 awaiting: stops={} reason={} awaiting_since_ms_set={}",
        stops.len(),
        row["reason"],
        row["awaiting_since_ms"].is_u64()
    );
    let journal = f.journal.lock().unwrap().query(&b).unwrap();
    assert_eq!(journal["stop_request_id"], auto_stop_request_id(&b));
    // Not confirmed: only the owner's local cessation would confirm it.
    let observed = call(
        &profile,
        "owned_poll",
        json!({"profile":REAL_PROJECT_PROFILE,"binding":b}),
    )
    .await
    .unwrap();
    assert_waiting(&observed["verification"], "VERIFY_STOP_UNCONFIRMED");
    let watch = &observed["execution"]["watch"];
    assert_eq!(watch["awaiting_approval"], true, "{watch}");
    assert_eq!(watch["auto_stop"]["reason"], STOPPED_AWAITING_APPROVAL);
    assert_eq!(watch["max_runtime_sec"], 1800);
    assert!(watch["runtime_ms"].is_u64());
    // No retry: one launch, one stop, even after more watchdog steps.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(f.state.launches.load(Ordering::SeqCst), 1);
    assert_eq!(f.state.stops.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runtime_limit_is_stopped_with_its_reason_and_short_prompts_are_not() {
    let f = HubFixture::new(1, &[], 1);
    let profile = f.profile(fast_watch());
    let b = f.bindings[0].clone();
    call(&profile, "owned_start", start_args(&b, "start-0"))
        .await
        .unwrap();
    // A prompt shorter than the limit resets without a stop.
    f.state.awaiting.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    f.state.awaiting.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(f.state.stops.lock().unwrap().is_empty());
    let spec = f.state.specs.lock().unwrap()[0].clone();
    assert!(spec.grants.allowed_tools().is_empty());
    until("runtime stop", || f.state.stops.lock().unwrap().len() == 1).await;
    let row = f.owners.lock().unwrap().query_auto_stop(&b).unwrap();
    assert_eq!(row["reason"], STOPPED_RUNTIME_LIMIT);
    assert!(row["runtime_ms"].as_u64().unwrap() >= 1000, "{row}");
    println!(
        "C2 runtime: stops={} reason={} runtime_ms={}",
        f.state.stops.lock().unwrap().len(),
        row["reason"],
        row["runtime_ms"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_cases_run_in_parallel_once_each_without_resend() {
    let f = HubFixture::new(3, &[], 1800);
    let profile = f.profile(RealProjectWatch::default());
    let mut receipts = Vec::new();
    for (i, b) in f.bindings.iter().enumerate() {
        receipts.push(
            call(
                &profile,
                "owned_start",
                start_args(b, &format!("start-{i}")),
            )
            .await
            .unwrap(),
        );
    }
    until("three running", || {
        f.bindings
            .iter()
            .all(|b| f.journal.lock().unwrap().query(b).unwrap()["execution_state"] == "running")
    })
    .await;
    assert_eq!(f.state.launches.load(Ordering::SeqCst), 3);
    // Same request replays its original receipt; nothing is launched again.
    for (i, b) in f.bindings.iter().enumerate() {
        let replay = call(
            &profile,
            "owned_start",
            start_args(b, &format!("start-{i}")),
        )
        .await
        .unwrap();
        assert_eq!(replay, receipts[i]);
        // A new request for the same attempt never re-dispatches.
        assert!(
            call(
                &profile,
                "owned_start",
                start_args(b, &format!("again-{i}"))
            )
            .await
            .is_err()
        );
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(f.state.launches.load(Ordering::SeqCst), 3);
    assert!(f.state.stops.lock().unwrap().is_empty());
    let attempts: std::collections::BTreeSet<_> = f
        .state
        .specs
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.cwd.clone())
        .collect();
    assert_eq!(attempts.len(), 3, "each case in its own folder");
    println!(
        "C2 parallel: launches={} running=3 stops=0",
        f.state.launches.load(Ordering::SeqCst)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_real_profile_launches_only_its_own_collector_grants() {
    // A folder-diff grant under the real bounded profile is refused.
    let f = HubFixture::new(1, &[], 1800);
    let b = f.bindings[0].clone();
    write_json(
        &f.root
            .path()
            .join("iris-execution/owned-real-policies.json"),
        &json!({"schema_version":1,"profile":REAL_BOUNDED_PROFILE,"enabled":true,"policies":[{
            "schema_version":1,"policy_id":"policy","policy_revision":1,
            "verifier":"strict-json-payload-equality/1","binding":b,
            "expires_at_ms":now_ms()+600_000,"expected_payload":{"answer":1}}]}),
    );
    let bounded = OwnedProfile::new_real_bounded(
        f.journal.clone(),
        f.owners.clone(),
        f.verifications(),
        f.root.path().to_path_buf(),
        f.port(),
        HubFixture::config(),
    )
    .unwrap();
    let error = bounded
        .call(
            "owned_start",
            &json!({"profile":REAL_BOUNDED_PROFILE,"binding":b,"request_id":"start-0"}),
            "iris",
            "workspace",
            "host",
            "boot",
        )
        .await
        .unwrap_err();
    assert_eq!(error.0, "DISPATCH_GRANT_REJECTED");
    // A payload grant under the real project profile is refused.
    let g = HubFixture::new(1, &[], 1800);
    let b = g.bindings[0].clone();
    let manifest_path = g.root.path().join("iris-execution-approved-grants.json");
    let mut manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let entry = &mut manifest["grants"][0];
    entry.as_object_mut().unwrap().remove("collector");
    entry["payload_path"] = json!("answer.json");
    entry["effect_scope"] = json!([PAYLOAD_ONLY, UNEXPECTED_CHANGE]);
    write_json(&manifest_path, &manifest);
    let profile = g.profile(fast_watch());
    let error = call(&profile, "owned_start", start_args(&b, "start-0"))
        .await
        .unwrap_err();
    assert_eq!(error.0, "DISPATCH_GRANT_REJECTED");
    assert_eq!(f.state.launches.load(Ordering::SeqCst), 0);
    assert_eq!(g.state.launches.load(Ordering::SeqCst), 0);
}

#[test]
fn tools_ride_only_on_folder_diff_grants_through_the_existing_route() {
    let f = HubFixture::new(1, &[TOOL_A], 1800);
    let mut loader = GrantLoader::open(f.root.path()).unwrap();
    let grant = loader.load_approved().unwrap().pop().unwrap();
    assert!(grant.folder_diff());
    let ticket = OwnerTicket {
        binding: f.bindings[0].clone(),
        owner_id: "owner".into(),
        owner_epoch: 1,
    };
    let spec = prepare_spec_with_tools(&grant, &ticket, &[TOOL_A.to_string()]).unwrap();
    assert_eq!(spec.permission_mode, "acceptEdits");
    assert_eq!(spec.grants.allowed_tools(), [TOOL_A.to_string()]);
    assert!(!spec.grants.grant_folder_trust());
    assert!(
        prepare_spec(&grant, &ticket)
            .unwrap()
            .grants
            .allowed_tools()
            .is_empty()
    );
    for tools in [
        vec!["--dangerously-skip-permissions".to_string()],
        vec![" Read".to_string()],
        vec!["Read".to_string(), "Read".to_string()],
        vec!["Bash(rm -rf /;x)".to_string()],
    ] {
        assert!(
            prepare_spec_with_tools(&grant, &ticket, &tools).is_err(),
            "{tools:?}"
        );
    }
}

#[test]
fn folder_diff_grant_rules() {
    let make = |edit: &dyn Fn(&mut Value, &Path, &Path)| {
        let f = HubFixture::new(2, &[], 1800);
        let path = f.root.path().join("iris-execution-approved-grants.json");
        let mut manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        edit(&mut manifest, f.root.path(), f._work.path());
        write_json(&path, &manifest);
        GrantLoader::open(f.root.path())
            .unwrap()
            .load_approved()
            .err()
            .map(|e| e.code)
    };
    assert_eq!(make(&|_, _, _| {}), None);
    type Edit = Box<dyn Fn(&mut Value, &Path, &Path)>;
    let cases: Vec<(&str, Edit, &str)> = vec![
        (
            "unknown collector",
            Box::new(|m, _, _| m["grants"][0]["collector"] = json!("folder")),
            "MANIFEST_POLICY",
        ),
        (
            "with payload",
            Box::new(|m, _, _| m["grants"][0]["payload_path"] = json!("answer.json")),
            "MANIFEST_POLICY",
        ),
        (
            "default launch",
            Box::new(|m, _, _| {
                m["grants"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("launch_mode");
            }),
            "MANIFEST_POLICY",
        ),
        (
            "synthetic pair",
            Box::new(|m, _, _| {
                m["grants"][0]["binding"]["provider"] = json!("synthetic");
                m["grants"][0]["binding"]["profile_id"] = json!("synthetic");
            }),
            "MANIFEST_POLICY",
        ),
        (
            "other profile",
            Box::new(|m, _, _| m["grants"][0]["binding"]["profile_id"] = json!("claude-other")),
            "MANIFEST_POLICY",
        ),
        (
            "payload scope",
            Box::new(|m, _, _| {
                m["grants"][0]["effect_scope"] = json!([PAYLOAD_ONLY, UNEXPECTED_CHANGE])
            }),
            "INVALID_PERMIT",
        ),
        (
            "same folder",
            Box::new(|m, _, _| {
                m["grants"][1]["approved_cwd"] = m["grants"][0]["approved_cwd"].clone()
            }),
            "MANIFEST_POLICY",
        ),
        (
            "nested folder",
            Box::new(|m, _, w| {
                let inner = w.join("case-0").join("src");
                m["grants"][1]["approved_cwd"] = json!(inner.canonicalize().unwrap());
            }),
            "MANIFEST_POLICY",
        ),
        (
            "inside runtime root",
            Box::new(|m, r, _| {
                std::fs::create_dir_all(r.join("inside")).unwrap();
                m["grants"][0]["approved_cwd"] = json!(r.join("inside").canonicalize().unwrap());
            }),
            "APPROVED_CWD",
        ),
        (
            "repository above",
            Box::new(|_, _, w| std::fs::create_dir(w.join(".git")).unwrap()),
            "APPROVED_CWD",
        ),
        (
            "repository inside",
            Box::new(|_, _, w| std::fs::write(w.join("case-0/.git"), b"gitdir: x").unwrap()),
            "APPROVED_CWD",
        ),
    ];
    for (name, edit, code) in cases {
        assert_eq!(make(&*edit), Some(code.to_string()), "{name}");
    }
}

// ---------------------------------------------------------------------------
// Hub composition: exactly one real profile source may select the pair.

struct NeverPort;
impl OwnedLaunchPort for NeverPort {
    fn launch_registered<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        panic!("never launches")
    }
    fn is_current(&self, _: SessionBinding) -> bool {
        panic!("never queries a session")
    }
    fn bind_owner(&self, _: &OwnerTicket, _: SessionBinding) -> Result<Value, BridgeError> {
        panic!("never binds")
    }
    fn stop_owner<'a>(
        &'a self,
        _: &'a OwnerTicket,
        _: &'a str,
    ) -> CoreFuture<'a, Result<Value, BridgeError>> {
        panic!("never stops")
    }
}

fn identity_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("iris-execution");
    std::fs::create_dir(&dir).unwrap();
    write_json(
        &dir.join("identity.json"),
        &json!({"principal_id":"iris","workspace_id":"workspace","execution_host_id":"host"}),
    );
    std::fs::write(dir.join("credential"), "c".repeat(64)).unwrap();
    root
}
fn project_manifest() -> Value {
    json!({"schema_version":1,"profile":REAL_PROJECT_PROFILE,"enabled":true,"policies":[{
        "schema_version":1,"policy_id":"policy","policy_revision":1,
        "verifier":"folder-change-record/1","binding":binding("task","boot"),
        "expires_at_ms":4_000_000_000_000u64,"baseline_sha256":"c".repeat(64),
        "baseline_entry_count":1,"allowed_paths":["."],"allowed_tools":[]}]})
}
fn bounded_manifest() -> Value {
    json!({"schema_version":1,"profile":REAL_BOUNDED_PROFILE,"enabled":true,"policies":[{
        "schema_version":1,"policy_id":"policy","policy_revision":1,
        "verifier":"strict-json-payload-equality/1","binding":binding("task","boot"),
        "expires_at_ms":4_000_000_000_000u64,"expected_payload":{"answer":42}}]})
}
type Compose = fn(IrisExecutionHttp, &Path) -> (IrisExecutionHttp, RealBoundedComposition);
fn never(_: Arc<Mutex<OwnerStore>>) -> Arc<dyn OwnedLaunchPort> {
    Arc::new(NeverPort)
}

#[tokio::test]
async fn composition_attaches_one_real_profile_and_refuses_both_sources() {
    let profiles: Compose = |o, r| o.compose_real_profiles(r, never);
    let project: Compose = |o, r| o.compose_real_project(r, never);
    let bounded: Compose = |o, r| o.compose_real_bounded(r, never);
    let setup = |files: &[(&str, Value)]| {
        let root = identity_root();
        for (name, value) in files {
            write_json(&root.path().join("iris-execution").join(name), value);
        }
        root
    };
    let owner = |root: &tempfile::TempDir| {
        IrisExecutionHttp::from_local(root.path(), 48765)
            .unwrap()
            .unwrap()
    };
    // Project source alone attaches the project profile.
    let root = setup(&[("owned-real-project-policies.json", project_manifest())]);
    let (attached, outcome) = profiles(owner(&root), root.path());
    assert_eq!(outcome, RealBoundedComposition::Attached);
    let owned = &attached.capabilities()["owned_dispatch"];
    assert_eq!(owned["profile"], REAL_PROJECT_PROFILE);
    assert_eq!(owned["live_provider_execution"], true);
    assert_eq!(owned["automatic_handoff"], false);
    // The real bounded source alone keeps attaching the real bounded profile.
    let root = setup(&[("owned-real-policies.json", bounded_manifest())]);
    let (attached, outcome) = profiles(owner(&root), root.path());
    assert_eq!(outcome, RealBoundedComposition::Attached);
    assert_eq!(
        attached.capabilities()["owned_dispatch"]["profile"],
        REAL_BOUNDED_PROFILE
    );
    // Both sources: neither is attached, by any entry point, and no
    // verification store is created.
    for compose in [profiles, project, bounded] {
        let root = setup(&[
            ("owned-real-project-policies.json", project_manifest()),
            ("owned-real-policies.json", bounded_manifest()),
        ]);
        let (owner, outcome) = compose(owner(&root), root.path());
        assert_eq!(
            outcome,
            RealBoundedComposition::Rejected("REAL_PROFILE_SOURCE_CONFLICT")
        );
        assert!(owner.capabilities().get("owned_dispatch").is_none());
        assert!(
            !root
                .path()
                .join("iris-execution/verification.sqlite3")
                .exists()
        );
    }
    // Each source accepts only its own verifier shape.
    let mut strict_in_project = bounded_manifest();
    strict_in_project["profile"] = json!(REAL_PROJECT_PROFILE);
    let root = setup(&[("owned-real-project-policies.json", strict_in_project)]);
    assert_eq!(
        profiles(owner(&root), root.path()).1,
        RealBoundedComposition::Rejected("REAL_PROJECT_POLICY_SOURCE")
    );
    let mut folder_in_bounded = project_manifest();
    folder_in_bounded["profile"] = json!(REAL_BOUNDED_PROFILE);
    let root = setup(&[("owned-real-policies.json", folder_in_bounded)]);
    assert_eq!(
        profiles(owner(&root), root.path()).1,
        RealBoundedComposition::Rejected("REAL_BOUNDED_POLICY_SOURCE")
    );
}

// ---------------------------------------------------------------------------
// Wrapper side (Windows): the test executable plays the provider.

#[cfg(windows)]
mod native {
    use super::*;
    use many_ai_cli::{
        application::spawn_policy::NativeFolderTrust,
        config::RuntimePaths,
        process::{
            Cancellation, ProcessPlan,
            pty::{NativePtyFactory, PtyFactory, PtySession, PtySize},
        },
        proto::Message,
        wrapper::{
            iris_owner::DurableExecutionOwner,
            runtime::{self, WrapperOptions},
            transport::{HubConnector, HubSocket},
        },
    };
    use std::{collections::BTreeMap, io};

    const PLAN_ENV: &str = "MANY_AI_SYNTHETIC_REAL_PROJECT_PLAN";
    const DONE_ENV: &str = "MANY_AI_SYNTHETIC_REAL_PROJECT_DONE";

    struct Native {
        root: tempfile::TempDir,
        work: tempfile::TempDir,
        binding: Binding,
        ticket: OwnerTicket,
        tools: Vec<String>,
        policy: Value,
        journal: Arc<Mutex<Journal>>,
    }
    impl Native {
        fn new(seed: impl FnOnce(&Path), allowed: &[&str], tools: &[&str]) -> Self {
            Self::build(seed, allowed, tools, true)
        }
        fn build(seed: impl FnOnce(&Path), allowed: &[&str], tools: &[&str], pin: bool) -> Self {
            let root = tempfile::tempdir().unwrap();
            let work = tempfile::tempdir().unwrap();
            let path = root.path();
            for dir in ["workspace", "claude-config", "vendor-home"] {
                std::fs::create_dir(work.path().join(dir)).unwrap();
            }
            let workspace = work.path().join("workspace");
            seed(&workspace);
            std::fs::create_dir_all(path.join("iris-execution")).unwrap();
            std::fs::create_dir_all(path.join("iris-execution-artifacts")).unwrap();
            std::fs::write(
                path.join("iris-execution-artifacts/instruction.txt"),
                INSTRUCTION,
            )
            .unwrap();
            let b = binding("task", "instance");
            write_json(
                &path.join("iris-execution-approved-grants.json"),
                &json!({"schema_version":1,"manifest_revision":1,"approval_state":"approved",
                    "grants":[grant(&b, &workspace)]}),
            );
            let mut journal =
                Journal::open(&path.join("iris-execution/journal.sqlite3"), "host", "boot")
                    .unwrap();
            let expires = (now_ms() / 1000) as i64 + 600;
            journal
                .register_grant(many_ai_cli::hub::iris_execution_journal::Grant {
                    binding: b.clone(),
                    expires_at: expires,
                    revoked: false,
                })
                .unwrap();
            journal
                .start("start", &b, (now_ms() / 1000) as i64)
                .unwrap();
            let policy = folder_policy(&b, &workspace, allowed, tools, 1800);
            let fixture = Self {
                root,
                work,
                ticket: OwnerTicket {
                    binding: b.clone(),
                    owner_id: "synthetic-owner".into(),
                    owner_epoch: 1,
                },
                binding: b,
                tools: tools.iter().map(|t| t.to_string()).collect(),
                policy,
                journal: Arc::new(Mutex::new(journal)),
            };
            fixture.write_policy(&fixture.policy, true);
            if pin {
                fixture.pin(&fixture.policy);
            }
            fixture
        }
        fn write_policy(&self, policy: &Value, enabled: bool) {
            write_json(
                &self
                    .root
                    .path()
                    .join("iris-execution/owned-real-project-policies.json"),
                &json!({"schema_version":1,"profile":REAL_PROJECT_PROFILE,
                    "enabled":enabled,"policies":[policy]}),
            );
        }
        /// Exactly what admission does: the current policy bytes are pinned
        /// before the owner intent exists.
        fn pin(&self, policy: &Value) {
            let mut owners = self.owners();
            self.verifications()
                .pin_policy(policy.to_string().as_bytes(), &mut owners, now_ms())
                .unwrap();
        }
        fn verifications(&self) -> VerificationStore {
            VerificationStore::open(&self.root.path().join("iris-execution/verification.sqlite3"))
                .unwrap()
        }
        fn owners(&self) -> OwnerStore {
            OwnerStore::open(&self.root.path().join("iris-execution/owner.sqlite3")).unwrap()
        }
        fn workspace(&self) -> PathBuf {
            self.work.path().join("workspace")
        }
        fn artifact(&self, name: &str) -> PathBuf {
            self.root.path().join("iris-execution-artifacts").join(name)
        }
        fn done_path(&self) -> PathBuf {
            self.root.path().join("helper-done.flag")
        }
        fn trust(&self) -> NativeFolderTrust {
            let jail = self.work.path().canonicalize().unwrap();
            let paths =
                RuntimePaths::trial(&jail, 49185, &self.root.path().join("installed")).unwrap();
            let home = self.work.path().join("vendor-home");
            NativeFolderTrust::new(paths, home.clone(), home)
        }
        fn adopt(&self) -> io::Result<DurableExecutionOwner> {
            DurableExecutionOwner::adopt(
                self.root.path(),
                &serde_json::to_string(&self.ticket).unwrap(),
                &self.binding.provider,
                &self.binding.profile_id,
            )
        }
        fn owner(&self) -> Arc<DurableExecutionOwner> {
            Arc::new(
                self.adopt()
                    .unwrap()
                    .with_folder_trust(self.trust())
                    .with_launch_tools(self.tools.clone()),
            )
        }
        fn process(&self, plan: &Value) -> ProcessPlan {
            let mut env = BTreeMap::new();
            env.insert(PLAN_ENV.into(), Some(plan.to_string().into()));
            env.insert(DONE_ENV.into(), Some(self.done_path().into_os_string()));
            env.insert(
                "CLAUDE_CONFIG_DIR".into(),
                Some(self.work.path().join("claude-config").into_os_string()),
            );
            ProcessPlan {
                executable: std::env::current_exe().unwrap(),
                args: vec![
                    "--exact".into(),
                    "native::native_real_project_helper".into(),
                    "--nocapture".into(),
                ],
                cwd: self.workspace(),
                env,
                stdin: vec![],
                timeout: Duration::ZERO,
                output_cap: 1024,
                pipe_drain_timeout: Duration::from_millis(100),
            }
        }
        fn record(&self) -> (Value, Value) {
            let owners = self.owners();
            (
                owners.query(&self.binding).unwrap(),
                owners.query_folder(&self.binding).unwrap(),
            )
        }
        /// The Hub-side real project profile over this runtime root.
        fn profile(&self) -> OwnedProfile {
            OwnedProfile::new_real_project(
                self.journal.clone(),
                Arc::new(Mutex::new(self.owners())),
                self.verifications(),
                self.root.path().to_path_buf(),
                Arc::new(NeverPort),
                DispatcherConfig::default(),
            )
            .unwrap()
        }
        async fn verification(&self) -> Value {
            self.profile()
                .call(
                    "owned_evidence",
                    &json!({"profile":REAL_PROJECT_PROFILE,"binding":self.binding}),
                    "iris",
                    "workspace",
                    "host",
                    "instance",
                )
                .await
                .unwrap()["verification"]
                .clone()
        }
    }

    struct Connector {
        done: PathBuf,
    }
    struct Socket {
        registered: bool,
        done: PathBuf,
    }
    impl HubConnector for Connector {
        fn connect<'a>(
            &'a self,
            _: Option<&'a str>,
        ) -> CoreFuture<'a, io::Result<Box<dyn HubSocket>>> {
            Box::pin(async move {
                Ok(Box::new(Socket {
                    registered: false,
                    done: self.done.clone(),
                }) as Box<dyn HubSocket>)
            })
        }
        fn probe(&self) -> CoreFuture<'_, bool> {
            Box::pin(async { false })
        }
    }
    impl HubSocket for Socket {
        fn send<'a>(&'a mut self, _: &'a Message) -> CoreFuture<'a, io::Result<()>> {
            Box::pin(async { Ok(()) })
        }
        fn receive(&mut self) -> CoreFuture<'_, io::Result<Message>> {
            Box::pin(async move {
                if !self.registered {
                    self.registered = true;
                    return Ok(Message {
                        r#type: "registered".into(),
                        session_id: 1,
                        cols: 80,
                        rows: 24,
                        ..Default::default()
                    });
                }
                while !self.done.exists() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "synthetic transport loss after helper finished",
                ))
            })
        }
    }
    #[derive(Default)]
    struct CountFactory(AtomicUsize);
    impl PtyFactory for CountFactory {
        fn spawn(&self, plan: &ProcessPlan, size: PtySize) -> io::Result<Arc<dyn PtySession>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            NativePtyFactory.spawn(plan, size)
        }
    }
    async fn run_owner(
        f: &Native,
        owner: Arc<DurableExecutionOwner>,
        plan: Value,
        factory: &CountFactory,
    ) -> io::Result<()> {
        let connector = Connector {
            done: f.done_path(),
        };
        let mut options = WrapperOptions::new(
            Message {
                provider: "synthetic".into(),
                token: "synthetic-only".into(),
                ..Default::default()
            },
            f.process(&plan),
        );
        options.execution_owner = Some(owner);
        options.reconnect_grace = Duration::ZERO;
        options.drain_timeout = Duration::from_millis(100);
        tokio::time::timeout(
            Duration::from_secs(20),
            runtime::run(
                options,
                &connector,
                factory,
                &Cancellation::default(),
                |_, _| Ok(()),
            ),
        )
        .await
        .expect("real project wrapper deadline")
        .map(|_| ())
    }
    async fn run(f: &Native, plan: Value) -> (io::Result<()>, usize) {
        let factory = CountFactory::default();
        let result = run_owner(f, f.owner(), plan, &factory).await;
        (result, factory.0.load(Ordering::SeqCst))
    }

    /// Synthetic provider stand-in. Inert unless the parent supplies a plan.
    #[test]
    fn native_real_project_helper() {
        let Some(plan) = std::env::var_os(PLAN_ENV) else {
            return;
        };
        let plan: Value = serde_json::from_str(plan.to_str().unwrap()).unwrap();
        for op in plan.as_array().unwrap() {
            let target = Path::new(op["path"].as_str().unwrap_or("."));
            match op["op"].as_str().unwrap() {
                "write" => std::fs::write(target, op["body"].as_str().unwrap()).unwrap(),
                "mkdir" => std::fs::create_dir_all(target).unwrap(),
                "remove" => std::fs::remove_file(target).unwrap(),
                "rmdir" => std::fs::remove_dir_all(target).unwrap(),
                "junction" => {
                    // mklink reads a `/` in the link path as a switch.
                    let link = op["path"].as_str().unwrap().replace('/', "\\");
                    let status = std::process::Command::new("cmd")
                        .args(["/C", "mklink", "/J"])
                        .arg(link)
                        .arg(op["target"].as_str().unwrap())
                        .stdout(std::process::Stdio::null())
                        .status()
                        .unwrap();
                    assert!(status.success());
                }
                "sleep" => std::thread::sleep(Duration::from_millis(op["ms"].as_u64().unwrap())),
                other => panic!("unknown synthetic op {other}"),
            }
        }
        std::fs::write(std::env::var_os(DONE_ENV).unwrap(), b"done").unwrap();
        std::thread::sleep(Duration::from_millis(300));
    }
    fn write(path: &str, body: &str) -> Value {
        json!({"op":"write","path":path,"body":body})
    }
    fn mkdir(path: &str) -> Value {
        json!({"op":"mkdir","path":path})
    }
    fn rmdir(path: &str) -> Value {
        json!({"op":"rmdir","path":path})
    }
    fn seed_project(dir: &Path) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join(".pytest-tmp")).unwrap();
        std::fs::write(dir.join("src/a.py"), b"print(1)\n").unwrap();
        std::fs::write(dir.join("src/old.py"), b"x\n").unwrap();
        std::fs::write(dir.join("README.md"), b"readme\n").unwrap();
    }

    #[tokio::test]
    async fn scoped_edit_records_every_change_and_fills_the_record() {
        let f = Native::new(seed_project, &["src", ".pytest-tmp/"], &[TOOL_A]);
        let plan = json!([
            write("src/a.py", "print(2)\n"),
            write("src/b.py", "new\n"),
            {"op":"remove","path":"src/old.py"},
            write(".pytest-tmp/log.txt", "log\n"),
        ]);
        let (result, launches) = run(&f, plan).await;
        result.unwrap();
        assert_eq!(launches, 1);
        let (owner, folder) = f.record();
        assert_eq!(owner["local_tree_ceased"], true);
        assert_eq!(owner["verified_task_outcome"], "unknown");
        let collection = &folder["collection"];
        assert_eq!(collection["collector"], "folder-diff");
        assert_eq!(collection["effects"], SCOPED_CHANGE, "{collection}");
        assert_eq!(collection["reason"], Value::Null);
        assert_eq!(collection["change_count"], 4);
        // The full change list artifact, hashed in the collection.
        let bytes = std::fs::read(f.artifact("task-result.json.changes.json")).unwrap();
        assert_eq!(collection["changes_sha256"], sha(&bytes));
        let list: Value = serde_json::from_slice(&bytes).unwrap();
        let changes = list["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 4);
        assert!(
            changes.contains(&json!({"path":"src/a.py","change":"modified","kind":"file",
            "before_size":9,"before_sha256":sha(b"print(1)\n"),
            "after_size":9,"after_sha256":sha(b"print(2)\n")}))
        );
        assert!(changes.contains(
            &json!({"path":"src/old.py","change":"removed","kind":"file",
            "before_size":2,"before_sha256":sha(b"x\n"),"after_size":null,"after_sha256":null})
        ));
        assert!(
            changes
                .iter()
                .any(|c| c["path"] == ".pytest-tmp/log.txt" && c["change"] == "added")
        );
        // The envelope is built by the collector, never by the provider.
        let result = std::fs::read(f.artifact("task-result.json")).unwrap();
        let envelope: Value = serde_json::from_slice(&result).unwrap();
        assert_eq!(envelope["outcome"], "unknown");
        assert_eq!(envelope["synthetic"], false);
        assert_eq!(envelope["effects"], SCOPED_CHANGE);
        assert_eq!(envelope["payload"]["change_count"], 4);
        assert_eq!(envelope["payload"]["changes_sha256"], sha(&bytes));
        assert_eq!(owner["outcome_receipt"]["artifact_sha256"], sha(&result));
        assert_eq!(owner["outcome_receipt"]["reported_effects"], SCOPED_CHANGE);
        // Trust was written exactly over the pinned baseline, before the claim.
        let trust = f.owners().query_folder_trust(&f.binding).unwrap();
        assert_eq!(trust["result"]["result"], "written", "{trust}");
        assert_eq!(trust["result"]["tool_grants"], json!([]));
        let v = f.verification().await;
        assert_record(&v);
        assert_eq!(v["reason"], Value::Null);
        assert_eq!(v["folder_effects"], SCOPED_CHANGE);
        assert_eq!(v["folder_reason"], Value::Null);
        assert_eq!(v["change_count"], 4);
        assert_eq!(v["changes_sha256"], sha(&bytes));
        println!(
            "C2 scoped: launches={launches} effects={} change_count={} record_reason={}",
            v["folder_effects"], v["change_count"], v["reason"]
        );
    }

    #[tokio::test]
    async fn untouched_folder_is_no_change() {
        let f = Native::new(seed_project, &["src"], &[]);
        let (result, launches) = run(&f, json!([])).await;
        result.unwrap();
        assert_eq!(launches, 1);
        let v = f.verification().await;
        assert_record(&v);
        assert_eq!(v["folder_effects"], NO_CHANGE);
        assert_eq!(v["change_count"], 0);
    }

    #[tokio::test]
    async fn out_of_scope_protected_stream_and_folder_changes_are_unexpected() {
        let cases = [
            (
                "out of scope",
                json!([write("README.md", "changed\n")]),
                "OUT_OF_SCOPE",
            ),
            (
                ".claude",
                json!([mkdir(".claude"), write(".claude/settings.json", "{}")]),
                "PROTECTED_NAME",
            ),
            (
                ".mcp.json",
                json!([write(".mcp.json", "{}")]),
                "PROTECTED_NAME",
            ),
            ("stream", json!([write("src/a.py:hidden", "x")]), "STREAM"),
        ];
        let mut counted = 0;
        for (name, plan, code) in cases {
            let f = Native::new(seed_project, &["src"], &[]);
            let (result, launches) = run(&f, plan).await;
            result.unwrap();
            assert_eq!(launches, 1, "{name}");
            let (owner, folder) = f.record();
            assert_eq!(folder["collection"]["effects"], UNEXPECTED_CHANGE, "{name}");
            assert_eq!(folder["collection"]["reason"], code, "{name}");
            assert_eq!(
                owner["outcome_receipt"]["reported_effects"], UNEXPECTED_CHANGE,
                "{name}"
            );
            let v = f.verification().await;
            assert_record(&v);
            assert_eq!(v["folder_effects"], UNEXPECTED_CHANGE, "{name}");
            assert_eq!(v["folder_reason"], code, "{name}");
            counted += 1;
        }
        // A new plain folder under an allowed path was `NOT_PLAIN_FILE` before
        // the 2026-11-04 contract change; it is now a scoped change.
        let f = Native::new(seed_project, &["src"], &[]);
        let (result, launches) = run(&f, json!([mkdir("src/new")])).await;
        result.unwrap();
        assert_eq!(launches, 1);
        let v = f.verification().await;
        assert_record(&v);
        assert_eq!(v["folder_effects"], SCOPED_CHANGE, "{v}");
        assert_eq!(v["folder_reason"], Value::Null);
        assert_eq!(v["change_count"], 1);
        println!("C2 unexpected cases={counted} in-scope new folder=scoped");
    }

    /// The case A pytest pattern: `--basetemp=.pytest-tmp` creates several
    /// nested folders with files and removes earlier ones. With `.pytest-tmp/`
    /// allowed, every plain folder change is scoped and listed as `kind: dir`.
    #[tokio::test]
    async fn folders_under_allowed_paths_are_scoped_and_listed() {
        let seed = |w: &Path| {
            seed_project(w);
            std::fs::create_dir_all(w.join(".pytest-tmp/old/deep")).unwrap();
            std::fs::write(w.join(".pytest-tmp/old/deep/f.txt"), b"old\n").unwrap();
        };
        let f = Native::new(seed, &["src", ".pytest-tmp/"], &[TOOL_A]);
        let plan = json!([
            mkdir(".pytest-tmp/basetemp/test_a0/sub"),
            write(".pytest-tmp/basetemp/test_a0/sub/out.txt", "out\n"),
            // Made and removed during the run: not a difference at all.
            mkdir(".pytest-tmp/basetemp/gone/inner"),
            rmdir(".pytest-tmp/basetemp/gone"),
            rmdir(".pytest-tmp/old"),
            mkdir("src/new"),
        ]);
        let (result, launches) = run(&f, plan).await;
        result.unwrap();
        assert_eq!(launches, 1);
        let (owner, folder) = f.record();
        let collection = &folder["collection"];
        assert_eq!(collection["effects"], SCOPED_CHANGE, "{collection}");
        assert_eq!(collection["reason"], Value::Null);
        assert_eq!(owner["outcome_receipt"]["reported_effects"], SCOPED_CHANGE);
        let bytes = std::fs::read(f.artifact("task-result.json.changes.json")).unwrap();
        assert_eq!(collection["changes_sha256"], sha(&bytes));
        let list: Value = serde_json::from_slice(&bytes).unwrap();
        let changes = list["changes"].as_array().unwrap();
        let dir = |path: &str, change: &str| {
            json!({"path":path,"change":change,"kind":"dir","before_size":
                if change == "removed" { json!(0) } else { Value::Null },
                "before_sha256":null,
                "after_size": if change == "added" { json!(0) } else { Value::Null },
                "after_sha256":null})
        };
        for expected in [
            dir(".pytest-tmp/basetemp", "added"),
            dir(".pytest-tmp/basetemp/test_a0", "added"),
            dir(".pytest-tmp/basetemp/test_a0/sub", "added"),
            dir(".pytest-tmp/old", "removed"),
            dir(".pytest-tmp/old/deep", "removed"),
            dir("src/new", "added"),
        ] {
            assert!(changes.contains(&expected), "{expected} in {changes:?}");
        }
        assert!(
            changes.contains(&json!({"path":".pytest-tmp/old/deep/f.txt",
            "change":"removed","kind":"file","before_size":4,"before_sha256":sha(b"old\n"),
            "after_size":null,"after_sha256":null}))
        );
        assert!(
            changes
                .iter()
                .any(|c| c["path"] == ".pytest-tmp/basetemp/test_a0/sub/out.txt"
                    && c["change"] == "added"
                    && c["kind"] == "file")
        );
        assert!(
            !changes
                .iter()
                .any(|c| c["path"].as_str().unwrap().contains("gone"))
        );
        assert_eq!(changes.len(), 8, "{changes:?}");
        assert_eq!(collection["change_count"], 8);
        let v = f.verification().await;
        assert_record(&v);
        assert_eq!(v["folder_effects"], SCOPED_CHANGE);
        assert_eq!(v["folder_reason"], Value::Null);
        assert_eq!(v["change_count"], 8);
        println!(
            "C2 folders scoped: effects={} change_count={}",
            v["folder_effects"], v["change_count"]
        );
    }

    /// A plain folder added or removed outside every allowed path stays
    /// unexpected, with the scope reason.
    #[tokio::test]
    async fn folders_outside_allowed_paths_stay_unexpected() {
        let cases = [
            ("added outside", json!([mkdir("docs/new")])),
            ("removed outside", json!([rmdir(".pytest-tmp")])),
            ("sibling prefix", json!([mkdir("src2")])),
        ];
        let mut counted = 0;
        for (name, plan) in cases {
            let f = Native::new(seed_project, &["src"], &[]);
            let (result, launches) = run(&f, plan).await;
            result.unwrap();
            assert_eq!(launches, 1, "{name}");
            let v = f.verification().await;
            assert_record(&v);
            assert_eq!(v["folder_effects"], UNEXPECTED_CHANGE, "{name}");
            assert_eq!(v["folder_reason"], "OUT_OF_SCOPE", "{name}");
            counted += 1;
        }
        println!("C2 folders outside unexpected={counted}");
    }

    /// A junction is a link, never a plain folder, even under an allowed path.
    #[tokio::test]
    async fn junction_under_allowed_path_stays_unexpected() {
        let mut counted = 0;
        for path in ["src/j", ".pytest-tmp/j"] {
            let f = Native::new(seed_project, &["src", ".pytest-tmp/"], &[TOOL_A]);
            let outside = f.work.path().join("outside");
            std::fs::create_dir_all(&outside).unwrap();
            let plan = json!([{"op":"junction","path":path,"target":outside}]);
            let (result, launches) = run(&f, plan).await;
            result.unwrap();
            assert_eq!(launches, 1, "{path}");
            let (_, folder) = f.record();
            assert_eq!(folder["collection"]["effects"], UNEXPECTED_CHANGE, "{path}");
            // The junction really exists and is listed as a link.
            let bytes = std::fs::read(f.artifact("task-result.json.changes.json")).unwrap();
            let list: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                list["changes"],
                json!([{"path":path,"change":"added","kind":"link","before_size":null,
                    "before_sha256":null,"after_size":0,"after_sha256":null}]),
                "{path}"
            );
            let v = f.verification().await;
            assert_record(&v);
            assert_eq!(v["folder_effects"], UNEXPECTED_CHANGE, "{path}");
            assert_eq!(v["folder_reason"], "LINK", "{path}");
            counted += 1;
        }
        println!("C2 junction unexpected={counted}");
    }

    #[tokio::test]
    async fn a_folder_not_equal_to_the_pinned_baseline_never_launches() {
        type Edit = Box<dyn Fn(&Path)>;
        let edits: Vec<(&str, Edit)> = vec![
            (
                "extra file",
                Box::new(|w| std::fs::write(w.join("extra.txt"), b"x").unwrap()),
            ),
            (
                "changed file",
                Box::new(|w| std::fs::write(w.join("src/a.py"), b"print(9)\n").unwrap()),
            ),
            (
                "hard link",
                Box::new(|w| {
                    std::fs::hard_link(w.join("src/a.py"), w.join("src/alias.py")).unwrap()
                }),
            ),
            (
                "stream",
                Box::new(|w| std::fs::write(w.join("src/a.py:hidden"), b"x").unwrap()),
            ),
            (
                ".claude",
                Box::new(|w| std::fs::create_dir(w.join(".claude")).unwrap()),
            ),
        ];
        let mut refused = 0;
        for (name, edit) in edits {
            let f = Native::new(seed_project, &["src"], &[]);
            edit(&f.workspace());
            let (result, launches) = run(&f, json!([])).await;
            assert!(result.is_err(), "{name}");
            assert_eq!(launches, 0, "{name}");
            let owner = f.owners().query(&f.binding).unwrap();
            assert_eq!(owner["claimed_session"], Value::Null, "{name}");
            let trust = f.owners().query_folder_trust(&f.binding).unwrap();
            assert_eq!(trust["intent"], Value::Null, "{name}: no trust write");
            refused += 1;
        }
        // Pinned over such a folder, it is refused as well.
        let f = Native::new(
            |w| {
                seed_project(w);
                std::fs::write(w.join("src/a.py:hidden"), b"x").unwrap();
            },
            &["src"],
            &[],
        );
        let (result, launches) = run(&f, json!([])).await;
        assert!(result.is_err() && launches == 0);
        refused += 1;
        println!("C2 baseline refused={refused}");
    }

    #[tokio::test]
    async fn tools_policy_and_pin_must_match_at_the_exec_boundary() {
        // Tools other than the pinned policy's, or none declared.
        let f = Native::new(seed_project, &["src"], &[TOOL_A]);
        let factory = CountFactory::default();
        let fewer = Arc::new(
            f.adopt()
                .unwrap()
                .with_folder_trust(f.trust())
                .with_launch_tools(vec![]),
        );
        assert!(run_owner(&f, fewer, json!([]), &factory).await.is_err());
        let more = Arc::new(
            f.adopt()
                .unwrap()
                .with_folder_trust(f.trust())
                .with_launch_tools(vec![TOOL_A.into(), "Read".into()]),
        );
        assert!(run_owner(&f, more, json!([]), &factory).await.is_err());
        let undeclared = Arc::new(f.adopt().unwrap().with_folder_trust(f.trust()));
        assert!(
            run_owner(&f, undeclared, json!([]), &factory)
                .await
                .is_err()
        );
        assert_eq!(factory.0.load(Ordering::SeqCst), 0);
        // No pin, a changed current policy, or a disabled source.
        let unpinned = Native::build(seed_project, &["src"], &[], false);
        assert_eq!(run(&unpinned, json!([])).await.1, 0);
        let changed = Native::new(seed_project, &["src"], &[]);
        let mut other = changed.policy.clone();
        other["max_runtime_sec"] = json!(1700);
        changed.write_policy(&other, true);
        assert_eq!(run(&changed, json!([])).await.1, 0);
        let disabled = Native::new(seed_project, &["src"], &[]);
        disabled.write_policy(&disabled.policy, false);
        assert_eq!(run(&disabled, json!([])).await.1, 0);
        // A repository above the folder refuses adoption itself.
        let above = Native::new(seed_project, &["src"], &[]);
        std::fs::create_dir(above.work.path().join(".git")).unwrap();
        assert!(above.adopt().is_err());
    }

    #[tokio::test]
    async fn watchdog_reason_fills_the_record_after_the_confirmed_stop() {
        let f = Native::new(seed_project, &["src"], &[]);
        let factory = CountFactory::default();
        let owner = f.owner();
        let b = f.binding.clone();
        let ticket = f.ticket.clone();
        let path = f.root.path().join("iris-execution/owner.sqlite3");
        let recorder = tokio::spawn(async move {
            loop {
                let store = OwnerStore::open(&path).unwrap();
                if store.query(&b).is_ok_and(|r| r["root_pid"].is_u64()) {
                    let mut store = store;
                    let document = json!({"kind":"auto_stop","ticket":ticket,
                        "reason":STOPPED_AWAITING_APPROVAL,
                        "stop_request_id":auto_stop_request_id(&b),
                        "requested_at_ms":now_ms(),"awaiting_since_ms":now_ms(),
                        "runtime_ms":1,"awaiting_limit_ms":60000,"task_outcome":"unknown",
                        "effects_verified":false,"task_outcome_verified":false,
                        "remote_jobs_stopped":false});
                    return store.record_auto_stop(&ticket, &document).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        // Before the stop is confirmed the record waits.
        run_owner(&f, owner, json!([{"op":"sleep","ms":1500}]), &factory)
            .await
            .unwrap();
        recorder.await.unwrap();
        let v = f.verification().await;
        assert_record(&v);
        assert_eq!(v["reason"], STOPPED_AWAITING_APPROVAL);
        assert_eq!(v["folder_effects"], NO_CHANGE);
    }

    #[tokio::test]
    async fn the_record_waits_without_an_owner() {
        let f = Native::new(seed_project, &["src"], &[]);
        assert_waiting(&f.verification().await, "VERIFY_OWNER_MISSING");
    }
}
