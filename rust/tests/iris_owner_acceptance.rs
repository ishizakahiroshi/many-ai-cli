//! Independent owner acceptance: native local fixtures, no Hub or provider.
//! A fixture exit or caller boolean is never treated as cessation authority.
use many_ai_cli::process::{
    ProcessPlan,
    pty::{NativePtyFactory, PtyFactory, PtySession, PtySize},
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const ROLE: &str = "IRIS_OWNER_ACCEPTANCE_ROLE";
const FIXTURE_ROOT: &str = "IRIS_OWNER_ACCEPTANCE_ROOT";

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn fixture_command(role: &str, root: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "owner_fixture_helper",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .env(ROLE, role)
        .env(FIXTURE_ROOT, root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }
    command
}

/// Runs only in an explicitly owned child. The deadline also prevents a test
/// failure from leaving an indefinitely running synthetic fixture.
#[test]
fn owner_fixture_helper() {
    let Some(role) = std::env::var_os(ROLE) else {
        return;
    };
    let root = PathBuf::from(std::env::var_os(FIXTURE_ROOT).expect("synthetic fixture root"));
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    if role == "parent" || role == "exit-parent" {
        let mut child = fixture_command("descendant", &root).spawn().unwrap();
        std::fs::write(root.join("descendant.pid"), child.id().to_string()).unwrap();
        if role == "exit-parent" {
            return;
        }
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
    } else if role == "descendant" || role == "unrelated" {
        let heartbeat = if role == "unrelated" {
            "unrelated.heartbeat"
        } else {
            "descendant.heartbeat"
        };
        while std::time::Instant::now() < deadline {
            std::fs::write(root.join(heartbeat), now_ms().to_string()).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
    } else {
        panic!("unexpected synthetic fixture role");
    }
}

struct OwnedFixture(Child);
impl Drop for OwnedFixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn plan(root: &Path, role: &str) -> ProcessPlan {
    // Remove inherited values inside the child ProcessPlan. Never alter the
    // test process environment or print any inherited environment value.
    let mut env: BTreeMap<OsString, Option<OsString>> =
        std::env::vars_os().map(|(name, _)| (name, None)).collect();
    env.insert(ROLE.into(), Some(role.into()));
    env.insert(FIXTURE_ROOT.into(), Some(root.as_os_str().into()));
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        env.insert("SystemRoot".into(), Some(system_root));
    }
    ProcessPlan {
        executable: std::env::current_exe().unwrap(),
        args: [
            "--exact",
            "owner_fixture_helper",
            "--nocapture",
            "--test-threads=1",
        ]
        .into_iter()
        .map(Into::into)
        .collect(),
        cwd: root.into(),
        env,
        stdin: vec![],
        timeout: Duration::ZERO,
        output_cap: 65536,
        pipe_drain_timeout: Duration::from_secs(2),
    }
}

async fn await_file(path: &Path, limit: Duration) -> Vec<u8> {
    let end = tokio::time::Instant::now() + limit;
    loop {
        if let Ok(value) = std::fs::read(path) {
            if !value.is_empty() {
                return value;
            }
        }
        assert!(
            tokio::time::Instant::now() < end,
            "owned fixture did not signal readiness"
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

#[cfg(windows)]
#[tokio::test]
async fn native_job_proof_stops_real_descendant_without_touching_unrelated_process() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut unrelated = OwnedFixture(fixture_command("unrelated", root).spawn().unwrap());
    await_file(&root.join("unrelated.heartbeat"), Duration::from_secs(5)).await;
    let session = NativePtyFactory
        .spawn(&plan(root, "parent"), PtySize::default())
        .unwrap();
    let descendant = await_file(&root.join("descendant.pid"), Duration::from_secs(5)).await;
    let descendant: u32 = std::str::from_utf8(&descendant).unwrap().parse().unwrap();
    await_file(&root.join("descendant.heartbeat"), Duration::from_secs(5)).await;
    let requested = now_ms();
    let proof = tokio::time::timeout(
        Duration::from_secs(8),
        session.terminate_checked(Duration::from_secs(6)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(proof.scope(), "windows-owned-job");
    assert_eq!(proof.root_pid(), session.pid());
    assert!(proof.root_waited());
    assert_eq!(proof.active_processes(), 0);
    assert!(proof.checked_at_unix_ms() >= requested);
    assert!(
        !many_ai_cli::process::pid_alive(i64::from(descendant)),
        "native proof left owned descendant alive"
    );
    let stopped = std::fs::read(root.join("descendant.heartbeat")).unwrap();
    let unrelated_before =
        await_file(&root.join("unrelated.heartbeat"), Duration::from_secs(1)).await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(
        stopped,
        std::fs::read(root.join("descendant.heartbeat")).unwrap()
    );
    assert_ne!(
        unrelated_before,
        await_file(&root.join("unrelated.heartbeat"), Duration::from_secs(1)).await
    );
    assert!(unrelated.0.try_wait().unwrap().is_none());
    drop(session);
}

#[cfg(windows)]
#[tokio::test]
async fn native_proof_remains_queryable_after_root_wait_and_reaper_completion() {
    let dir = tempfile::tempdir().unwrap();
    let session = NativePtyFactory
        .spawn(&plan(dir.path(), "exit-parent"), PtySize::default())
        .unwrap();
    await_file(&dir.path().join("descendant.pid"), Duration::from_secs(5)).await;
    tokio::time::timeout(Duration::from_secs(8), session.wait())
        .await
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    let proof = tokio::time::timeout(
        Duration::from_secs(8),
        session.terminate_checked(Duration::from_secs(6)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(proof.root_pid(), session.pid());
    assert!(proof.root_waited());
    assert_eq!(proof.active_processes(), 0);
}

struct UncheckedSession;
impl PtySession for UncheckedSession {
    fn pid(&self) -> u32 {
        0
    }
    fn read<'a>(
        &'a self,
        _: &'a mut [u8],
    ) -> many_ai_cli::proto::core::CoreFuture<'a, io::Result<usize>> {
        Box::pin(async { Ok(0) })
    }
    fn write<'a>(
        &'a self,
        bytes: &'a [u8],
    ) -> many_ai_cli::proto::core::CoreFuture<'a, io::Result<usize>> {
        Box::pin(async move { Ok(bytes.len()) })
    }
    fn resize(&self, _: PtySize) -> io::Result<()> {
        Ok(())
    }
    fn close(&self) {}
    fn wait(
        &self,
    ) -> many_ai_cli::proto::core::CoreFuture<'_, io::Result<many_ai_cli::process::pty::PtyExit>>
    {
        Box::pin(async {
            Ok(many_ai_cli::process::pty::PtyExit {
                code: 0,
                signal: String::new(),
                forced: false,
            })
        })
    }
}

#[tokio::test]
async fn exit_zero_on_unchecked_session_never_creates_native_cessation_authority() {
    let session = UncheckedSession;
    assert_eq!(session.wait().await.unwrap().code, 0);
    assert_eq!(
        session
            .terminate_checked(Duration::from_millis(50))
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );
}

// OwnerStore is supplied by the composition owner before these acceptance
// cases are compiled. It is a trusted local Rust boundary, not token auth.
#[cfg(windows)]
fn owner_binding() -> many_ai_cli::hub::iris_execution_journal::Binding {
    use sha2::{Digest, Sha256};
    many_ai_cli::hub::iris_execution_journal::Binding {
        principal_id: "synthetic-principal".into(),
        workspace_id: "synthetic-workspace".into(),
        plan_id: "synthetic-plan".into(),
        plan_revision: 1,
        task_id: "synthetic-task".into(),
        instruction_revision: 1,
        instruction_hash: Sha256::digest(b"synthetic instruction")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        approval_id: "synthetic-approval".into(),
        approval_revision: 1,
        scope_hash: Sha256::digest(b"synthetic scope")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        execution_host_id: "synthetic-host".into(),
        executor_instance_id: "synthetic-instance".into(),
        attempt_id: "synthetic-attempt".into(),
        fence: 1,
        provider: "synthetic".into(),
        profile_id: "synthetic".into(),
    }
}

#[cfg(windows)]
#[tokio::test]
async fn owner_native_receipt_persists_and_rejects_wrong_owner_epoch_fence_and_stop() {
    use many_ai_cli::hub::iris_execution_owner::OwnerStore;
    let dir = tempfile::tempdir().unwrap();
    let b = owner_binding();
    let path = dir.path().join("owner.sqlite3");
    let mut store = OwnerStore::open(&path).unwrap();
    let started = now_ms();
    let ticket = store
        .begin_owner(&b, "synthetic-owner", 1, started)
        .unwrap();
    store.claim_spawn(&ticket, 1001, now_ms()).unwrap();
    assert!(
        store.claim_spawn(&ticket, 1001, now_ms()).is_err(),
        "same owner/session claimed spawn twice"
    );
    assert!(
        store.claim_spawn(&ticket, 1002, now_ms()).is_err(),
        "one owner ticket claimed another spawn"
    );
    let session = NativePtyFactory
        .spawn(&plan(dir.path(), "parent"), PtySize::default())
        .unwrap();
    store.bind_root(&ticket, session.pid(), now_ms()).unwrap();
    await_file(
        &dir.path().join("descendant.heartbeat"),
        Duration::from_secs(5),
    )
    .await;
    store
        .request_stop(&ticket, 1, "synthetic-stop", now_ms())
        .unwrap();
    assert!(
        store
            .request_stop(&ticket, 1, "different-stop", now_ms())
            .is_err()
    );
    let proof = tokio::time::timeout(
        Duration::from_secs(8),
        session.terminate_checked(Duration::from_secs(6)),
    )
    .await
    .unwrap()
    .unwrap();
    let mut wrong = ticket.clone();
    wrong.owner_id = "wrong-owner".into();
    assert!(store.append_cessation(&wrong, 2, &proof).is_err());
    let mut wrong = ticket.clone();
    wrong.owner_epoch += 1;
    assert!(store.append_cessation(&wrong, 2, &proof).is_err());
    let mut wrong = ticket.clone();
    wrong.binding.fence += 1;
    assert!(store.append_cessation(&wrong, 2, &proof).is_err());
    // A valid native proof from a different owned process must not satisfy
    // this ticket. The test cannot construct or forge LocalCessation fields.
    let other = tempfile::tempdir().unwrap();
    let other_session = NativePtyFactory
        .spawn(&plan(other.path(), "parent"), PtySize::default())
        .unwrap();
    await_file(
        &other.path().join("descendant.heartbeat"),
        Duration::from_secs(5),
    )
    .await;
    let other_proof = tokio::time::timeout(
        Duration::from_secs(8),
        other_session.terminate_checked(Duration::from_secs(6)),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(store.append_cessation(&ticket, 2, &other_proof).is_err());
    drop(other_session);
    let receipt = store.append_cessation(&ticket, 2, &proof).unwrap();
    assert_eq!(receipt["local_tree_ceased"], true);
    assert_eq!(receipt["owned_execution_ceased"], false);
    assert_eq!(receipt["remote_jobs_stopped"], false);
    assert_eq!(receipt["task_outcome_verified"], false);
    let saved = store.query(&b).unwrap();
    drop(store);
    drop(session);
    let reopened = OwnerStore::open(&path).unwrap();
    assert_eq!(reopened.query(&b).unwrap(), saved);
}

#[cfg(windows)]
fn fixture_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(windows)]
fn approved_manifest(
    root: &Path,
    b: &many_ai_cli::hub::iris_execution_journal::Binding,
    result_path: &str,
) {
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    std::fs::create_dir_all(root.join("iris-execution-artifacts")).unwrap();
    std::fs::write(
        root.join("iris-execution-artifacts/instruction.txt"),
        b"synthetic instruction",
    )
    .unwrap();
    let manifest = serde_json::json!({"schema_version":1,"manifest_revision":1,"approval_state":"approved","grants":[{"binding":b,"expires_at":10000000000i64,"revoked":false,"approved_cwd":"workspace","instruction_path":"instruction.txt","result_path":result_path,"max_artifact_bytes":65536,"effect_scope":["none"],"allow_remote_jobs":false}]});
    std::fs::write(
        root.join("iris-execution-approved-grants.json"),
        manifest.to_string(),
    )
    .unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn operator_permitted_artifact_tamper_and_binding_rejected_without_claiming_task_success() {
    use many_ai_cli::hub::{iris_execution_grants::GrantLoader, iris_execution_owner::OwnerStore};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let b = owner_binding();
    approved_manifest(root, &b, "result.json");
    let mut loader = GrantLoader::open(root).unwrap();
    let mut grants = loader.load_approved().unwrap();
    assert_eq!(grants.len(), 1);
    let permit = grants.remove(0).result;
    let mut store = OwnerStore::open(&root.join("owner.sqlite3")).unwrap();
    let ticket = store
        .begin_owner(&b, "synthetic-owner", 1, now_ms())
        .unwrap();
    store.claim_spawn(&ticket, 2001, now_ms()).unwrap();
    let session = NativePtyFactory
        .spawn(&plan(root, "exit-parent"), PtySize::default())
        .unwrap();
    store.bind_root(&ticket, session.pid(), now_ms()).unwrap();
    tokio::time::timeout(Duration::from_secs(8), session.wait())
        .await
        .unwrap()
        .unwrap();
    let proof = tokio::time::timeout(
        Duration::from_secs(8),
        session.terminate_checked(Duration::from_secs(6)),
    )
    .await
    .unwrap()
    .unwrap();
    let exit = store.append_cessation(&ticket, 1, &proof).unwrap();
    assert_eq!(exit["kind"], "local_exit");
    assert_eq!(exit["owned_execution_ceased"], false);
    let artifact = root.join("iris-execution-artifacts/result.json");
    let body=serde_json::json!({"binding":b,"owner_id":"synthetic-owner","owner_epoch":1,"instruction_hash":b.instruction_hash,"outcome":"succeeded","effects":"none","synthetic":true}).to_string();
    let expected = fixture_hash(body.as_bytes());
    std::fs::write(&artifact, &body).unwrap();
    permit.verify(Some(&expected)).unwrap();
    std::fs::write(&artifact, b"tampered synthetic artifact").unwrap();
    assert!(
        store
            .append_outcome(&ticket, 2, &permit, &expected)
            .is_err()
    );
    assert!(store.query(&b).unwrap()["outcome_receipt"].is_null());
    let mut wrong: serde_json::Value = serde_json::from_str(&body).unwrap();
    wrong["instruction_hash"] = serde_json::json!(fixture_hash(b"other instruction"));
    let wrong = wrong.to_string();
    std::fs::write(&artifact, &wrong).unwrap();
    assert!(
        store
            .append_outcome(&ticket, 2, &permit, &fixture_hash(wrong.as_bytes()))
            .is_err()
    );
    std::fs::write(&artifact, &body).unwrap();
    let receipt = store
        .append_outcome(&ticket, 2, &permit, &expected)
        .unwrap();
    assert_eq!(receipt["artifact_sha256"], expected);
    assert_eq!(receipt["reported_outcome"], "succeeded");
    assert_eq!(receipt["task_outcome_verified"], false);
    assert_eq!(receipt["effects_verified"], false);
    assert_eq!(store.query(&b).unwrap()["verified_task_outcome"], "unknown");
    std::fs::hard_link(&artifact, root.join("iris-execution-artifacts/alias.json")).unwrap();
    assert!(
        permit.verify(None).is_err(),
        "artifact hardlink alias was accepted"
    );
}

#[cfg(windows)]
#[test]
fn operator_manifest_cannot_grant_artifact_path_outside_fixed_root() {
    use many_ai_cli::hub::iris_execution_grants::GrantLoader;
    let dir = tempfile::tempdir().unwrap();
    let b = owner_binding();
    approved_manifest(dir.path(), &b, "../outside.json");
    let mut loader = GrantLoader::open(dir.path()).unwrap();
    assert!(loader.load_approved().is_err());
}
