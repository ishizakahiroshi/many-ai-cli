//! review2 adversarial checks for the opt-in Hub composition. Synthetic data
//! only; no Hub, provider, process or network is started.
use many_ai_cli::{
    application::iris_execution_launch::OwnedLaunchPort,
    hub::{
        iris_execution::{IrisExecutionHttp, RealBoundedComposition},
        iris_execution_owned::{
            REAL_BOUNDED_PROFILE, REAL_BOUNDED_PROFILE_ID, REAL_BOUNDED_PROVIDER,
        },
        iris_execution_owner::{OwnerStore, OwnerTicket},
        iris_execution_runtime::BridgeError,
    },
    proto::core::{
        CoreFuture, HttpWaitCancellation, SessionBinding, SpawnWaitOutcome, WrappedSpawnSpec,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct NeverPort;
impl OwnedLaunchPort for NeverPort {
    fn launch_registered<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        panic!("never")
    }
    fn is_current(&self, _: SessionBinding) -> bool {
        panic!("never")
    }
    fn bind_owner(&self, _: &OwnerTicket, _: SessionBinding) -> Result<Value, BridgeError> {
        panic!("never")
    }
    fn stop_owner<'a>(
        &'a self,
        _: &'a OwnerTicket,
        _: &'a str,
    ) -> CoreFuture<'a, Result<Value, BridgeError>> {
        panic!("never")
    }
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn binding(host: &str, provider: &str, profile_id: &str, instruction_hash: &str) -> Value {
    json!({"principal_id":"iris","workspace_id":"workspace","plan_id":"plan","plan_revision":1,
        "task_id":"task","instruction_revision":1,"instruction_hash":instruction_hash,
        "approval_id":"approval","approval_revision":1,"scope_hash":"b".repeat(64),
        "execution_host_id":host,"executor_instance_id":"boot","attempt_id":"attempt",
        "fence":1,"provider":provider,"profile_id":profile_id})
}

fn identity_root(host: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("iris-execution");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("identity.json"),
        json!({"principal_id":"iris","workspace_id":"workspace","execution_host_id":host})
            .to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("credential"), "c".repeat(64)).unwrap();
    root
}

fn real_manifest(host: &str) -> Value {
    json!({"schema_version":1,"profile":REAL_BOUNDED_PROFILE,"enabled":true,"policies":[{
        "schema_version":1,"policy_id":"policy","policy_revision":1,
        "verifier":"strict-json-payload-equality/1",
        "binding":binding(host, REAL_BOUNDED_PROVIDER, REAL_BOUNDED_PROFILE_ID, &"a".repeat(64)),
        "expires_at_ms":4_000_000_000_000u64,"expected_payload":{"answer":42}
    }]})
}

/// R2-H1 (decided: the CLI rule stays, Iris follows it): the CLI identity and
/// binding rule has no `.` in a host. A dotted host makes `from_local` fail, as
/// it always did; Iris no longer accepts such a value in its configuration
/// either, so the two sides cannot disagree about it.
#[test]
fn r2_dotted_host_identity_is_refused_by_the_cli() {
    for host in ["lab-pc.local", ".", "host."] {
        let root = identity_root(host);
        let result = IrisExecutionHttp::from_local(root.path(), 48765);
        assert!(result.is_err(), "{host:?}: dotted host was accepted");
    }
    let root = identity_root("lab-pc-local");
    assert!(
        IrisExecutionHttp::from_local(root.path(), 48765)
            .unwrap()
            .is_some()
    );
}

/// R2-H2 (decided: intended, safer change): with no owned-real-policies.json, a
/// grant manifest that holds the real pair without payload_path is refused by
/// the grant loader with MANIFEST_POLICY. `from_local` therefore errors and the
/// Hub does not start on that runtime root; before 78c14fac such a manifest
/// loaded as inspection-only. Nothing about the real pair is opened by default.
#[test]
fn r2_default_hub_with_real_pair_grant_without_payload() {
    let root = identity_root("synthetic-host");
    let path = root.path();
    std::fs::create_dir(path.join("iris-execution-artifacts")).unwrap();
    let instruction = b"synthetic instruction\n";
    std::fs::write(
        path.join("iris-execution-artifacts/instruction.txt"),
        instruction,
    )
    .unwrap();
    std::fs::create_dir(path.join("workspace")).unwrap();
    let grant = json!({"binding":binding("synthetic-host", REAL_BOUNDED_PROVIDER,
            REAL_BOUNDED_PROFILE_ID, &sha(instruction)),
        "expires_at":4_000_000_000i64,"revoked":false,"approved_cwd":"workspace",
        "instruction_path":"instruction.txt","result_path":"result.json",
        "max_artifact_bytes":4096,"effect_scope":["none"],"allow_remote_jobs":false});
    std::fs::write(
        path.join("iris-execution-approved-grants.json"),
        json!({"schema_version":1,"manifest_revision":1,"approval_state":"approved",
            "grants":[grant]})
        .to_string(),
    )
    .unwrap();
    assert!(
        !path
            .join("iris-execution/owned-real-policies.json")
            .exists()
    );
    let result = IrisExecutionHttp::from_local(path, 48765);
    assert_eq!(
        result.as_ref().err().map(String::as_str),
        Some("MANIFEST_POLICY"),
        "{:?}",
        result.as_ref().map(|owner| owner.is_some())
    );
    // No start path opened: nothing was created for a verifier or a dispatch.
    assert!(!path.join("iris-execution/verification.sqlite3").exists());
}

/// R2-H3: a rejected composition must not leave new state behind. Outside a
/// Tokio runtime the dispatcher cannot be built; that is settled before the
/// verification store is opened, so no `verification.sqlite3` is created and
/// the launch port factory is never called.
#[test]
fn r2_rejected_composition_leaves_no_new_store() {
    let root = identity_root("synthetic-host");
    std::fs::write(
        root.path().join("iris-execution/owned-real-policies.json"),
        real_manifest("synthetic-host").to_string(),
    )
    .unwrap();
    let owner = IrisExecutionHttp::from_local(root.path(), 48765)
        .unwrap()
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let (owner, outcome) =
        owner.compose_real_bounded(root.path(), move |_: Arc<Mutex<OwnerStore>>| {
            counted.fetch_add(1, Ordering::SeqCst);
            Arc::new(NeverPort) as Arc<dyn OwnedLaunchPort>
        });
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the launch port was built");
    assert_eq!(
        outcome,
        RealBoundedComposition::Rejected("REAL_BOUNDED_RUNTIME")
    );
    assert!(owner.capabilities().get("owned_dispatch").is_none());
    assert!(
        !root
            .path()
            .join("iris-execution/verification.sqlite3")
            .exists(),
        "rejected composition left verification.sqlite3"
    );
}

/// R2-5: the other process condition. A journal that cannot admit owned
/// dispatch (here: restore hold set in its meta row) rejects the composition
/// before any file is created, even inside a Tokio runtime.
#[tokio::test]
async fn r2_journal_without_owned_dispatch_is_rejected_before_any_file() {
    let root = identity_root("synthetic-host");
    std::fs::write(
        root.path().join("iris-execution/owned-real-policies.json"),
        real_manifest("synthetic-host").to_string(),
    )
    .unwrap();
    let owner = IrisExecutionHttp::from_local(root.path(), 48765)
        .unwrap()
        .unwrap();
    rusqlite::Connection::open(root.path().join("iris-execution/journal.sqlite3"))
        .unwrap()
        .execute("UPDATE meta SET restore_hold=1 WHERE id=1", [])
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let (owner, outcome) =
        owner.compose_real_bounded(root.path(), move |_: Arc<Mutex<OwnerStore>>| {
            counted.fetch_add(1, Ordering::SeqCst);
            Arc::new(NeverPort) as Arc<dyn OwnedLaunchPort>
        });
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the launch port was built");
    assert_eq!(
        outcome,
        RealBoundedComposition::Rejected("REAL_BOUNDED_JOURNAL")
    );
    assert!(owner.capabilities().get("owned_dispatch").is_none());
    assert!(
        !root
            .path()
            .join("iris-execution/verification.sqlite3")
            .exists()
    );
}

/// R2-H4 (control): a symlink/junction at the policy path is a rejected
/// request, never a silent absence or an attachment.
#[cfg(windows)]
#[tokio::test]
async fn r2_policy_source_junction_is_rejected() {
    let root = identity_root("synthetic-host");
    let other = tempfile::tempdir().unwrap();
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(
            root.path()
                .join("iris-execution")
                .join("owned-real-policies.json"),
        )
        .arg(other.path())
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let owner = IrisExecutionHttp::from_local(root.path(), 48765)
        .unwrap()
        .unwrap();
    let (owner, outcome) = owner.compose_real_bounded(root.path(), |_: Arc<Mutex<OwnerStore>>| {
        Arc::new(NeverPort) as Arc<dyn OwnedLaunchPort>
    });
    assert!(
        matches!(outcome, RealBoundedComposition::Rejected(_)),
        "{outcome:?}"
    );
    assert!(owner.capabilities().get("owned_dispatch").is_none());
    let _ = Path::new("");
}
