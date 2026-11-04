//! Hub composition of the explicit real bounded profile. No Hub, provider,
//! process or network is started: the launch port is a stand-in that must
//! never be invoked. Only operator files under a temporary runtime root select
//! whether the profile is attached; every value here is synthetic.
use many_ai_cli::{
    application::iris_execution_launch::OwnedLaunchPort,
    hub::{
        iris_execution::{IrisExecutionHttp, RealBoundedComposition},
        iris_execution_owned::{
            PROFILE, REAL_BOUNDED_PROFILE, REAL_BOUNDED_PROFILE_ID, REAL_BOUNDED_PROVIDER,
        },
        iris_execution_owner::{OwnerStore, OwnerTicket},
        iris_execution_runtime::BridgeError,
    },
    proto::core::{
        CoreFuture, HttpWaitCancellation, SessionBinding, SpawnWaitOutcome, WrappedSpawnSpec,
    },
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

const MARKER: &str = "synthetic-file-content-marker";

/// Launching is out of scope for composition. Any call is a test failure.
struct NeverPort;
impl OwnedLaunchPort for NeverPort {
    fn launch_registered<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        panic!("composition must never launch")
    }
    fn is_current(&self, _: SessionBinding) -> bool {
        panic!("composition must never query a session")
    }
    fn bind_owner(&self, _: &OwnerTicket, _: SessionBinding) -> Result<Value, BridgeError> {
        panic!("composition must never bind an owner")
    }
    fn stop_owner<'a>(
        &'a self,
        _: &'a OwnerTicket,
        _: &'a str,
    ) -> CoreFuture<'a, Result<Value, BridgeError>> {
        panic!("composition must never stop an owner")
    }
}

fn binding(provider: &str, profile_id: &str) -> Value {
    json!({"principal_id":"iris","workspace_id":"workspace","plan_id":"plan","plan_revision":1,
        "task_id":"task","instruction_revision":1,"instruction_hash":"a".repeat(64),
        "approval_id":"approval","approval_revision":1,"scope_hash":"b".repeat(64),
        "execution_host_id":"synthetic-host","executor_instance_id":"boot","attempt_id":"attempt",
        "fence":1,"provider":provider,"profile_id":profile_id})
}

fn real_manifest() -> Value {
    json!({"schema_version":1,"profile":REAL_BOUNDED_PROFILE,"enabled":true,"policies":[{
        "schema_version":1,"policy_id":"policy","policy_revision":1,
        "verifier":"strict-json-payload-equality/1",
        "binding":binding(REAL_BOUNDED_PROVIDER, REAL_BOUNDED_PROFILE_ID),
        "expires_at_ms":4_000_000_000_000u64,"expected_payload":{"answer":42}
    }]})
}

/// Identity and credential only, exactly what enables the inspection owner.
fn runtime_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("iris-execution");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("identity.json"),
        json!({"principal_id":"iris","workspace_id":"workspace",
            "execution_host_id":"synthetic-host"})
        .to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("credential"), "c".repeat(64)).unwrap();
    root
}

fn write_real(root: &Path, raw: &[u8]) {
    std::fs::write(
        root.join("iris-execution").join("owned-real-policies.json"),
        raw,
    )
    .unwrap();
}

fn compose(root: &Path, calls: &Arc<AtomicUsize>) -> (IrisExecutionHttp, RealBoundedComposition) {
    let owner = IrisExecutionHttp::from_local(root, 48765)
        .unwrap()
        .expect("identity selects the inspection owner");
    let calls = calls.clone();
    owner.compose_real_bounded(root, move |_owners: Arc<Mutex<OwnerStore>>| {
        calls.fetch_add(1, Ordering::SeqCst);
        Arc::new(NeverPort) as Arc<dyn OwnedLaunchPort>
    })
}

fn legacy_is_unchanged(capabilities: &Value) {
    assert_eq!(capabilities["contract_version"], "iris-execution/1");
    assert_eq!(capabilities["execution_host_id"], "synthetic-host");
    for field in [
        "execution_ready",
        "live_provider_execution",
        "automatic_handoff",
        "synthetic",
    ] {
        assert_eq!(capabilities[field], false, "{field}");
    }
}

#[tokio::test]
async fn missing_policy_file_keeps_the_current_inspection_only_owner() {
    let root = runtime_root();
    let before = IrisExecutionHttp::from_local(root.path(), 48765)
        .unwrap()
        .unwrap()
        .capabilities();
    // The synthetic profile's own source must not opt the Hub into anything.
    let mut synthetic = real_manifest();
    synthetic["profile"] = json!(PROFILE);
    synthetic["policies"][0]["binding"] = binding("synthetic", "synthetic");
    std::fs::write(
        root.path().join("iris-execution/owned-policies.json"),
        synthetic.to_string(),
    )
    .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (owner, outcome) = compose(root.path(), &calls);
    assert_eq!(outcome, RealBoundedComposition::NotRequested);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let after = owner.capabilities();
    assert!(after.get("owned_dispatch").is_none(), "{after}");
    legacy_is_unchanged(&after);
    let strip = |mut v: Value| {
        v.as_object_mut().unwrap().remove("executor_instance_id");
        v
    };
    assert_eq!(strip(before), strip(after));
    assert!(
        !root
            .path()
            .join("iris-execution/verification.sqlite3")
            .exists()
    );
}

#[tokio::test]
async fn valid_policy_file_attaches_only_the_real_bounded_profile() {
    let root = runtime_root();
    write_real(root.path(), real_manifest().to_string().as_bytes());
    let calls = Arc::new(AtomicUsize::new(0));
    let (owner, outcome) = compose(root.path(), &calls);
    assert_eq!(outcome, RealBoundedComposition::Attached);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let capabilities = owner.capabilities();
    legacy_is_unchanged(&capabilities);
    let owned = &capabilities["owned_dispatch"];
    assert_eq!(owned["profile"], REAL_BOUNDED_PROFILE);
    assert_eq!(owned["admission_enabled"], true, "{owned}");
    assert_eq!(owned["live_provider_execution"], true);
    assert_eq!(owned["automatic_handoff"], false);

    let mut disabled = real_manifest();
    disabled["enabled"] = json!(false);
    write_real(root.path(), disabled.to_string().as_bytes());
    let owned = &owner.capabilities()["owned_dispatch"];
    assert_eq!(owned["profile"], REAL_BOUNDED_PROFILE);
    assert_eq!(owned["admission_enabled"], false);
    assert_eq!(owned["live_provider_execution"], false);
}

#[tokio::test]
async fn disabled_but_valid_policy_file_attaches_without_admission() {
    let root = runtime_root();
    let mut disabled = real_manifest();
    disabled["enabled"] = json!(false);
    write_real(root.path(), disabled.to_string().as_bytes());
    let calls = Arc::new(AtomicUsize::new(0));
    let (owner, outcome) = compose(root.path(), &calls);
    assert_eq!(outcome, RealBoundedComposition::Attached);
    let owned = &owner.capabilities()["owned_dispatch"];
    assert_eq!(owned["admission_enabled"], false);
    assert_eq!(owned["live_provider_execution"], false);
}

#[tokio::test]
async fn broken_policy_file_keeps_inspection_only_and_reports_a_code_only() {
    let valid = real_manifest();
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("not-json", format!("{{{MARKER}").into_bytes()),
        ("empty", Vec::new()),
        (
            "duplicate-key",
            format!(r#"{{"schema_version":1,"schema_version":1,"note":"{MARKER}"}}"#).into_bytes(),
        ),
    ];
    let mut unknown = valid.clone();
    unknown[MARKER] = json!(true);
    cases.push(("unknown-field", unknown.to_string().into_bytes()));
    let mut synthetic_profile = valid.clone();
    synthetic_profile["profile"] = json!(PROFILE);
    cases.push((
        "synthetic-profile-string",
        synthetic_profile.to_string().into_bytes(),
    ));
    for (provider, profile_id) in [
        ("codex", REAL_BOUNDED_PROFILE_ID),
        ("synthetic", "synthetic"),
        (REAL_BOUNDED_PROVIDER, "claude-other"),
    ] {
        let mut other = valid.clone();
        other["policies"][0]["binding"] = binding(provider, profile_id);
        cases.push(("other-pair", other.to_string().into_bytes()));
    }
    let mut duplicate = valid.clone();
    duplicate["policies"]
        .as_array_mut()
        .unwrap()
        .push(valid["policies"][0].clone());
    cases.push(("duplicate-policy", duplicate.to_string().into_bytes()));
    cases.push((
        "oversized",
        vec![b' '; many_ai_cli::hub::iris_execution_mcp::MAX_BYTES + 1],
    ));
    for (name, raw) in cases {
        let root = runtime_root();
        write_real(root.path(), &raw);
        let calls = Arc::new(AtomicUsize::new(0));
        let (owner, outcome) = compose(root.path(), &calls);
        let RealBoundedComposition::Rejected(code) = outcome else {
            panic!("{name}: broken source attached or was ignored: {outcome:?}");
        };
        assert!(!code.contains(MARKER), "{name}");
        assert!(
            !code.contains('\\') && !code.contains('/'),
            "{name}: {code}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "{name}");
        let capabilities = owner.capabilities();
        assert!(capabilities.get("owned_dispatch").is_none(), "{name}");
        legacy_is_unchanged(&capabilities);
        assert!(
            !root
                .path()
                .join("iris-execution/verification.sqlite3")
                .exists(),
            "{name}"
        );
    }
}

#[tokio::test]
async fn policy_path_that_is_a_directory_is_rejected_not_ignored() {
    let root = runtime_root();
    std::fs::create_dir(root.path().join("iris-execution/owned-real-policies.json")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (owner, outcome) = compose(root.path(), &calls);
    assert!(matches!(outcome, RealBoundedComposition::Rejected(_)));
    assert!(owner.capabilities().get("owned_dispatch").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn without_identity_the_policy_file_selects_nothing() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("iris-execution")).unwrap();
    write_real(root.path(), real_manifest().to_string().as_bytes());
    assert!(
        IrisExecutionHttp::from_local(root.path(), 48765)
            .unwrap()
            .is_none()
    );
    assert!(
        !root
            .path()
            .join("iris-execution/verification.sqlite3")
            .exists()
    );
}
