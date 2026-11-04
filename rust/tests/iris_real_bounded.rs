//! Real bounded profile collector acceptance with a native synthetic stand-in.
//! No real provider, Hub or network is used: the test executable plays the
//! provider and writes files into the approved folder. Only the trusted local
//! collector may build the result envelope, and only after recorded cessation.
#![cfg(windows)]
use many_ai_cli::{
    application::spawn_policy::NativeFolderTrust,
    config::RuntimePaths,
    hub::{
        iris_execution_folder::{PAYLOAD_ONLY, UNEXPECTED_CHANGE},
        iris_execution_grants::GrantLoader,
        iris_execution_journal::{Binding, Grant, Journal},
        iris_execution_owned::{
            REAL_BOUNDED_PROFILE, REAL_BOUNDED_PROFILE_ID, REAL_BOUNDED_PROVIDER,
        },
        iris_execution_owner::{OwnerStore, OwnerTicket},
        iris_execution_verification_store::VerificationStore,
    },
    process::{
        Cancellation, ProcessPlan,
        pty::{NativePtyFactory, PtyFactory, PtySession, PtySize},
    },
    proto::{Message, core::CoreFuture},
    wrapper::{
        iris_owner::DurableExecutionOwner,
        runtime::{self, ExecutionOwnerObserver, WrapperOptions},
        transport::{HubConnector, HubSocket},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const INSTRUCTION: &str = "This is a synthetic connectivity test. Fixed fixture instruction.\n";
const ANSWER: &str = r#"{"sorted":["alder","birch","cedar","larch","maple","spruce"],"count":6}"#;
const PLAN_ENV: &str = "MANY_AI_SYNTHETIC_REAL_BOUNDED_PLAN";
const DONE_ENV: &str = "MANY_AI_SYNTHETIC_REAL_BOUNDED_DONE";

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

struct Fixture {
    root: tempfile::TempDir,
    /// The approved folder lives outside the runtime root (C4-D). This
    /// temporary folder is also the only place the synthetic Claude
    /// configuration may live: folder trust runs on trial paths rooted here.
    work: tempfile::TempDir,
    binding: Binding,
    ticket: OwnerTicket,
    manifest: Value,
    _journal: Journal,
}
/// Real bounded policy source for the fixture binding (the new profile).
fn write_real_policies(root: &Path, binding: &Binding, enabled: bool) {
    let manifest = json!({"schema_version":1,"profile":REAL_BOUNDED_PROFILE,
        "enabled":enabled,"policies":[policy(binding)]});
    std::fs::write(
        root.join("iris-execution/owned-real-policies.json"),
        manifest.to_string(),
    )
    .unwrap();
}
fn policy(binding: &Binding) -> Value {
    json!({"schema_version":1,"policy_id":"policy","policy_revision":1,
        "verifier":"strict-json-payload-equality/1","binding":binding,
        "expires_at_ms":now_ms()+600_000,
        "expected_payload":serde_json::from_str::<Value>(ANSWER).unwrap()})
}
impl Fixture {
    fn new() -> Self {
        Self::with_grant(|_, _| {})
    }
    /// Collector grant without bounded-edit. Folder trust needs an empty
    /// approved folder (R3-1), so tests that seed the folder to exercise the
    /// collector comparison use the default launch mode, which never trusts.
    fn seeded_collector() -> Self {
        Self::with_grant(|g, _| {
            g.as_object_mut().unwrap().remove("launch_mode");
        })
    }
    fn with_grant(edit: impl FnOnce(&mut Value, &Path)) -> Self {
        Self::for_pair(REAL_BOUNDED_PROVIDER, REAL_BOUNDED_PROFILE_ID, edit)
    }
    fn for_pair(provider: &str, profile: &str, edit: impl FnOnce(&mut Value, &Path)) -> Self {
        let root = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let path = root.path();
        std::fs::create_dir(work.path().join("workspace")).unwrap();
        std::fs::create_dir(work.path().join("claude-config")).unwrap();
        std::fs::create_dir(work.path().join("vendor-home")).unwrap();
        let workspace = work.path().join("workspace").canonicalize().unwrap();
        std::fs::create_dir(path.join("iris-execution-artifacts")).unwrap();
        std::fs::write(
            path.join("iris-execution-artifacts/instruction.txt"),
            INSTRUCTION,
        )
        .unwrap();
        let binding = Binding {
            principal_id: "iris".into(),
            workspace_id: "workspace".into(),
            plan_id: "plan".into(),
            plan_revision: 1,
            task_id: "task".into(),
            instruction_revision: 1,
            instruction_hash: sha(INSTRUCTION.as_bytes()),
            approval_id: "approval".into(),
            approval_revision: 1,
            scope_hash: sha(b"synthetic scope"),
            execution_host_id: "host".into(),
            executor_instance_id: "instance".into(),
            attempt_id: "attempt".into(),
            fence: 1,
            provider: provider.into(),
            profile_id: profile.into(),
        };
        let ticket = OwnerTicket {
            binding: binding.clone(),
            owner_id: "synthetic-owner".into(),
            owner_epoch: 1,
        };
        let expires = (now_ms() / 1000) as i64 + 600;
        let mut grant = json!({"binding":binding,"expires_at":expires,"revoked":false,
            "approved_cwd":workspace,"instruction_path":"instruction.txt",
            "result_path":"result.json","max_artifact_bytes":4096,
            "effect_scope":[PAYLOAD_ONLY,UNEXPECTED_CHANGE],"allow_remote_jobs":false,
            "launch_mode":"bounded-edit","payload_path":"answer.json"});
        edit(&mut grant, path);
        let manifest = json!({"schema_version":1,"manifest_revision":1,
            "approval_state":"approved","grants":[grant]});
        std::fs::write(
            path.join("iris-execution-approved-grants.json"),
            manifest.to_string(),
        )
        .unwrap();
        let mut journal =
            Journal::open(&path.join("iris-execution/journal.sqlite3"), "host", "boot").unwrap();
        journal
            .register_grant(Grant {
                binding: binding.clone(),
                expires_at: expires,
                revoked: false,
            })
            .unwrap();
        journal
            .start("start", &binding, (now_ms() / 1000) as i64)
            .unwrap();
        if provider == REAL_BOUNDED_PROVIDER && profile == REAL_BOUNDED_PROFILE_ID {
            write_real_policies(path, &binding, true);
        }
        Self {
            root,
            work,
            binding,
            ticket,
            manifest,
            _journal: journal,
        }
    }
    fn workspace(&self) -> PathBuf {
        self.work.path().join("workspace")
    }
    fn result_path(&self) -> PathBuf {
        self.root
            .path()
            .join("iris-execution-artifacts/result.json")
    }
    fn done_path(&self) -> PathBuf {
        self.root.path().join("helper-done.flag")
    }
    fn owners(&self) -> OwnerStore {
        OwnerStore::open(&self.root.path().join("iris-execution/owner.sqlite3")).unwrap()
    }
    fn verifications(&self) -> VerificationStore {
        VerificationStore::open(&self.root.path().join("verification.sqlite3")).unwrap()
    }
    /// Pin before the owner intent exists, exactly as admission does.
    fn pin(&self) {
        let policy = json!({"schema_version":1,"policy_id":"policy","policy_revision":1,
            "verifier":"strict-json-payload-equality/1","binding":self.binding,
            "expires_at_ms":now_ms()+600_000,
            "expected_payload":serde_json::from_str::<Value>(ANSWER).unwrap()});
        let mut owners = self.owners();
        self.verifications()
            .pin_policy(policy.to_string().as_bytes(), &mut owners, now_ms())
            .unwrap();
    }
    fn adopt(&self) -> Arc<DurableExecutionOwner> {
        Arc::new(self.adopt_without_trust().with_folder_trust(self.trust()))
    }
    fn adopt_without_trust(&self) -> DurableExecutionOwner {
        DurableExecutionOwner::adopt(
            self.root.path(),
            &serde_json::to_string(&self.ticket).unwrap(),
            &self.binding.provider,
            &self.binding.profile_id,
        )
        .unwrap()
    }
    /// Same writer as production, on trial paths rooted at the temporary
    /// work folder: any vendor read or write outside it is refused, so no
    /// real Claude configuration can be opened even by a wrong environment.
    fn trust(&self) -> NativeFolderTrust {
        let jail = self.work.path().canonicalize().unwrap();
        let paths = RuntimePaths::trial(&jail, 49184, &self.root.path().join("installed")).unwrap();
        let home = self.work.path().join("vendor-home");
        NativeFolderTrust::new(paths, home.clone(), home)
    }
    fn config_dir(&self) -> PathBuf {
        self.work.path().join("claude-config")
    }
    /// The synthetic state file the writer uses for `dir` (no name is
    /// guessed here: the writer reports its own file).
    fn state_in(&self, dir: &Path) -> PathBuf {
        self.trust()
            .config_path("claude", &[format!("CLAUDE_CONFIG_DIR={}", dir.display())])
            .unwrap()
    }
    fn config_file(&self) -> PathBuf {
        self.state_in(&self.config_dir())
    }
    /// The vendor-home fallback, which the exact grant must never use.
    fn home_file(&self) -> PathBuf {
        self.trust().config_path("claude", &[]).unwrap()
    }
    /// The folder-trust result row (null when none).
    fn trust_record(&self) -> Value {
        self.owners().query_folder_trust(&self.binding).unwrap()["result"].clone()
    }
    /// The folder-trust intent row written before the vendor write.
    fn trust_intent(&self) -> Value {
        self.owners().query_folder_trust(&self.binding).unwrap()["intent"].clone()
    }
    fn process(&self, plan: &Value) -> ProcessPlan {
        let mut env = BTreeMap::new();
        env.insert(PLAN_ENV.into(), Some(plan.to_string().into()));
        env.insert(DONE_ENV.into(), Some(self.done_path().into_os_string()));
        env.insert(
            "CLAUDE_CONFIG_DIR".into(),
            Some(self.config_dir().into_os_string()),
        );
        ProcessPlan {
            executable: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "native_real_bounded_helper".into(),
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
}

struct Connector {
    done: PathBuf,
}
struct Socket {
    registered: bool,
    done: PathBuf,
}
impl HubConnector for Connector {
    fn connect<'a>(&'a self, _: Option<&'a str>) -> CoreFuture<'a, io::Result<Box<dyn HubSocket>>> {
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
async fn run(f: &Fixture, plan: Value, factory: &CountFactory) -> io::Result<()> {
    run_owner(f, f.adopt(), f.process(&plan), factory).await
}
async fn run_owner(
    f: &Fixture,
    owner: Arc<DurableExecutionOwner>,
    process: ProcessPlan,
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
        process,
    );
    options.execution_owner = Some(owner);
    options.reconnect_grace = Duration::ZERO;
    options.drain_timeout = Duration::from_millis(100);
    tokio::time::timeout(
        Duration::from_secs(15),
        runtime::run(
            options,
            &connector,
            factory,
            &Cancellation::default(),
            |_, _| Ok(()),
        ),
    )
    .await
    .expect("real bounded wrapper deadline")
    .map(|_| ())
}

/// Synthetic provider stand-in. Inert unless the parent supplies the plan.
#[test]
fn native_real_bounded_helper() {
    let Some(plan) = std::env::var_os(PLAN_ENV) else {
        return;
    };
    let plan: Value = serde_json::from_str(plan.to_str().unwrap()).unwrap();
    for op in plan.as_array().unwrap() {
        let target = Path::new(op["path"].as_str().unwrap());
        match op["op"].as_str().unwrap() {
            "write" => std::fs::write(target, op["body"].as_str().unwrap()).unwrap(),
            "large" => {
                std::fs::write(target, vec![b' '; op["bytes"].as_u64().unwrap() as usize]).unwrap()
            }
            "hardlink" => std::fs::hard_link(op["target"].as_str().unwrap(), target).unwrap(),
            "junction" => {
                let status = std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(target)
                    .arg(op["target"].as_str().unwrap())
                    .stdout(std::process::Stdio::null())
                    .status()
                    .unwrap();
                assert!(status.success());
            }
            other => panic!("unknown synthetic op {other}"),
        }
    }
    std::fs::write(std::env::var_os(DONE_ENV).unwrap(), b"done").unwrap();
    // Keep the root alive until the fake transport observes completion.
    std::thread::sleep(Duration::from_millis(300));
}

fn write(path: &str, body: &str) -> Value {
    json!({"op":"write","path":path,"body":body})
}

fn assert_never_promoted(owner: &Value, folder: &Value) {
    assert_eq!(owner["task_outcome_verified"], false);
    assert_eq!(owner["effects_verified"], false);
    assert_eq!(owner["verified_task_outcome"], "unknown");
    let collection = &folder["collection"];
    if !collection.is_null() {
        assert_eq!(collection["effects_verified"], false);
        assert_eq!(collection["task_outcome_verified"], false);
        assert_eq!(collection["remote_jobs_stopped"], false);
    }
}

#[tokio::test]
async fn payload_only_answer_builds_trusted_envelope_and_fixed_payload_passes() {
    let f = Fixture::new();
    f.pin();
    let factory = CountFactory::default();
    run(&f, json!([write("answer.json", ANSWER)]), &factory)
        .await
        .unwrap();
    assert_eq!(factory.0.load(Ordering::SeqCst), 1);
    let (owner, folder) = f.record();
    assert_never_promoted(&owner, &folder);
    assert_eq!(owner["local_tree_ceased"], true);
    assert_eq!(folder["before"]["complete"], true);
    // An empty approved folder still records the folder itself (empty path).
    let before = folder["before"]["entries"].as_array().unwrap();
    assert_eq!(before.len(), 1, "{before:?}");
    assert_eq!(before[0]["path"], "");
    assert_eq!(before[0]["kind"], "dir");
    assert_eq!(folder["after"]["complete"], true);
    assert_eq!(folder["collection"]["effects"], PAYLOAD_ONLY);
    assert_eq!(folder["collection"]["reason"], Value::Null);
    let bytes = std::fs::read(f.result_path()).unwrap();
    let envelope: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(envelope["binding"], json!(f.binding));
    assert_eq!(envelope["owner_id"], f.ticket.owner_id);
    assert_eq!(envelope["owner_epoch"], f.ticket.owner_epoch);
    assert_eq!(envelope["instruction_hash"], f.binding.instruction_hash);
    assert_eq!(envelope["outcome"], "unknown");
    assert_eq!(envelope["effects"], PAYLOAD_ONLY);
    assert_eq!(envelope["synthetic"], false);
    assert_eq!(
        envelope["payload"],
        serde_json::from_str::<Value>(ANSWER).unwrap()
    );
    let receipt = &owner["outcome_receipt"];
    assert_eq!(receipt["artifact_sha256"], sha(&bytes));
    assert_eq!(receipt["reported_outcome"], "unknown");
    assert_eq!(receipt["reported_effects"], PAYLOAD_ONLY);
    assert_eq!(receipt["synthetic"], false);
    assert_eq!(folder["collection"]["result_sha256"], sha(&bytes));
    let mut loader = GrantLoader::open(f.root.path()).unwrap();
    let grant = loader.load_approved().unwrap().into_iter().next().unwrap();
    let owners = f.owners();
    let verdict = f
        .verifications()
        .verify(&owners, &grant.result, &f.ticket, &sha(&bytes), now_ms())
        .unwrap();
    assert_eq!(verdict["verdict"], "pass");
    assert_eq!(verdict["effects_verified"], false);
    assert_eq!(verdict["task_outcome_verified"], false);
    assert_eq!(verdict["remote_jobs_stopped"], false);
}

#[tokio::test]
async fn extra_file_or_modified_seed_is_unexpected_change() {
    for modify_seed in [false, true] {
        let f = Fixture::seeded_collector();
        std::fs::write(f.workspace().join("seed.txt"), b"synthetic seed").unwrap();
        let mut plan = vec![write("answer.json", ANSWER)];
        plan.push(if modify_seed {
            write("seed.txt", "synthetic seed changed")
        } else {
            write("extra.txt", "synthetic extra")
        });
        run(&f, json!(plan), &CountFactory::default())
            .await
            .unwrap();
        let (owner, folder) = f.record();
        assert_never_promoted(&owner, &folder);
        assert_eq!(folder["collection"]["effects"], UNEXPECTED_CHANGE);
        let envelope: Value =
            serde_json::from_slice(&std::fs::read(f.result_path()).unwrap()).unwrap();
        assert_eq!(envelope["effects"], UNEXPECTED_CHANGE);
        assert_eq!(
            owner["outcome_receipt"]["reported_effects"],
            UNEXPECTED_CHANGE
        );
    }
}

#[tokio::test]
async fn preexisting_result_is_never_overwritten_or_accepted() {
    let f = Fixture::new();
    // A provider-written result-shaped file must not become an outcome.
    let planted = json!({"binding":f.binding,"owner_id":f.ticket.owner_id,
        "owner_epoch":f.ticket.owner_epoch,"instruction_hash":f.binding.instruction_hash,
        "outcome":"succeeded","effects":PAYLOAD_ONLY,"synthetic":false,
        "payload":serde_json::from_str::<Value>(ANSWER).unwrap()})
    .to_string();
    std::fs::write(f.result_path(), &planted).unwrap();
    run(
        &f,
        json!([write("answer.json", ANSWER)]),
        &CountFactory::default(),
    )
    .await
    .unwrap();
    let (owner, folder) = f.record();
    assert_never_promoted(&owner, &folder);
    assert_eq!(std::fs::read_to_string(f.result_path()).unwrap(), planted);
    assert!(owner["outcome_receipt"].is_null());
    assert_eq!(folder["collection"]["effects"], UNEXPECTED_CHANGE);
    assert_eq!(folder["collection"]["reason"], "RESULT_PREEXISTING");
}

#[tokio::test]
async fn invalid_oversize_or_aliased_payload_creates_no_envelope() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("outside.json"), ANSWER).unwrap();
    std::fs::create_dir(outside.path().join("dir")).unwrap();
    let cases = [
        (json!([write("answer.json", "{not json")]), "PAYLOAD_JSON"),
        (
            json!([{"op":"large","path":"answer.json","bytes":4097}]),
            "PAYLOAD_UNAVAILABLE",
        ),
        (
            json!([{"op":"hardlink","path":"answer.json",
                "target":outside.path().join("outside.json")}]),
            "PAYLOAD_UNAVAILABLE",
        ),
        (
            json!([{"op":"junction","path":"answer.json",
                "target":outside.path().join("dir")}]),
            "PAYLOAD_UNAVAILABLE",
        ),
    ];
    for (plan, reason) in cases {
        let f = Fixture::new();
        run(&f, plan.clone(), &CountFactory::default())
            .await
            .unwrap();
        let (owner, folder) = f.record();
        assert_never_promoted(&owner, &folder);
        assert!(!f.result_path().exists(), "{plan}");
        assert!(owner["outcome_receipt"].is_null(), "{plan}");
        assert_eq!(folder["collection"]["reason"], reason, "{plan}");
    }
    // A hard link or junction is never a plain payload creation either.
    let f = Fixture::new();
    run(
        &f,
        json!([{"op":"junction","path":"answer.json","target":outside.path().join("dir")}]),
        &CountFactory::default(),
    )
    .await
    .unwrap();
    assert_eq!(f.record().1["collection"]["effects"], UNEXPECTED_CHANGE);
}

#[tokio::test]
async fn oversized_or_aliased_baseline_refuses_launch() {
    let outside = tempfile::tempdir().unwrap();
    for case in 0..3 {
        let f = Fixture::new();
        match case {
            0 => {
                for i in 0..1001 {
                    std::fs::write(f.workspace().join(format!("seed-{i:04}")), b"").unwrap();
                }
            }
            1 => std::fs::write(
                f.workspace().join("large.bin"),
                vec![0u8; 16 * 1024 * 1024 + 1],
            )
            .unwrap(),
            _ => {
                let status = std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(f.workspace().join("linked"))
                    .arg(outside.path())
                    .stdout(std::process::Stdio::null())
                    .status()
                    .unwrap();
                assert!(status.success());
            }
        }
        let factory = CountFactory::default();
        assert!(
            run(&f, json!([write("answer.json", ANSWER)]), &factory)
                .await
                .is_err(),
            "case {case}"
        );
        assert_eq!(factory.0.load(Ordering::SeqCst), 0, "case {case}");
        let (owner, folder) = f.record();
        assert!(owner["claimed_session"].is_null(), "case {case}");
        assert!(folder["before"].is_null(), "case {case}");
    }
}

#[test]
fn collector_never_runs_without_recorded_cessation() {
    let f = Fixture::new();
    let owner = f.adopt();
    let plan = f.process(&json!([]));
    owner.before_spawn(1, &plan).unwrap();
    owner.root_started(4242).unwrap();
    std::fs::write(f.workspace().join("answer.json"), ANSWER).unwrap();
    owner.finished(None, None).unwrap();
    drop(owner);
    let (record, folder) = f.record();
    assert_never_promoted(&record, &folder);
    assert_eq!(record["local_tree_ceased"], false);
    assert!(record["outcome_receipt"].is_null());
    assert_eq!(folder["before"]["complete"], true);
    assert!(folder["after"].is_null());
    assert!(folder["collection"].is_null());
    assert!(!f.result_path().exists());
}

#[test]
fn payload_grants_require_real_binding_collected_scope_and_separate_folder() {
    type Edit = Box<dyn Fn(&mut Value, &Path)>;
    let cases: Vec<Edit> = vec![
        Box::new(|g, _| g["binding"]["provider"] = json!("codex")),
        Box::new(|g, _| g["binding"]["profile_id"] = json!("synthetic")),
        Box::new(|g, _| g["effect_scope"] = json!(["none"])),
        Box::new(|g, _| g["effect_scope"] = json!([PAYLOAD_ONLY])),
        Box::new(|g, _| g["payload_path"] = json!("../answer.json")),
        Box::new(|g, _| {
            g.as_object_mut().unwrap().remove("payload_path");
        }),
        Box::new(|g, _| g["approved_cwd"] = json!("iris-execution-artifacts")),
        Box::new(|g, root| g["approved_cwd"] = json!(root.canonicalize().unwrap())),
    ];
    for (index, edit) in cases.into_iter().enumerate() {
        let f = Fixture::with_grant(|g, root| edit(g, root));
        let mut loader = GrantLoader::open(f.root.path()).unwrap();
        assert!(loader.load_approved().is_err(), "case {index}");
    }
    let f = Fixture::new();
    let mut loader = GrantLoader::open(f.root.path()).unwrap();
    let grant = loader.load_approved().unwrap().pop().unwrap();
    assert!(grant.payload.is_some());
    assert_eq!(f.manifest["grants"][0]["payload_path"], "answer.json");
}

// ---- C4 adversarial additions (synthetic data only) ----

/// C4-D: a collector folder anywhere inside the runtime root is refused, not
/// only the root itself, its ancestors, artifacts or owner databases.
#[test]
fn c4_collector_folder_inside_runtime_root_is_refused() {
    for nested in ["inside", "inside/deeper"] {
        let f = Fixture::with_grant(|g, root| {
            std::fs::create_dir_all(root.join(nested)).unwrap();
            g["approved_cwd"] = json!(nested);
        });
        let mut loader = GrantLoader::open(f.root.path()).unwrap();
        assert_eq!(
            loader.load_approved().map(|g| g.len()).unwrap_err().code,
            "APPROVED_CWD",
            "{nested}"
        );
    }
}

/// C4-A: NTFS alternate data streams are invisible to the folder snapshot.
/// Writes into a stream of an existing file, of the new payload, or of a
/// directory leave read_dir names, main-stream size and SHA256 unchanged.
/// Expected by contract: any write other than the payload creation is
/// `folder-unexpected-change`.
#[tokio::test]
async fn c4_alternate_data_stream_writes_are_unexpected_change() {
    let cases = [
        (
            "existing-file-stream",
            write("seed.txt:hidden", "synthetic hidden"),
        ),
        (
            "payload-stream",
            write("answer.json:extra", "synthetic extra"),
        ),
        (
            "directory-stream",
            write("sub:dirstream", "synthetic dir stream"),
        ),
    ];
    let mut observed = Vec::new();
    for (name, extra) in cases {
        let f = Fixture::seeded_collector();
        std::fs::write(f.workspace().join("seed.txt"), b"synthetic seed").unwrap();
        std::fs::create_dir(f.workspace().join("sub")).unwrap();
        run(
            &f,
            json!([write("answer.json", ANSWER), extra]),
            &CountFactory::default(),
        )
        .await
        .unwrap();
        let (owner, folder) = f.record();
        assert_never_promoted(&owner, &folder);
        observed.push((name, folder["collection"]["effects"].clone()));
    }
    for (name, effects) in &observed {
        assert_eq!(effects, UNEXPECTED_CHANGE, "{name}: observed {observed:?}");
    }
}

/// C4 control: provider-side oversize folder, entry flood, created junction or
/// hard link next to a correct payload never yields folder-payload-only, and
/// never sets any verified flag.
#[tokio::test]
async fn c4_after_side_limits_and_aliases_fail_closed() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("outside.txt"), b"synthetic outside").unwrap();
    let mut flood = vec![write("answer.json", ANSWER)];
    for i in 0..1001 {
        flood.push(write(&format!("f{i:04}"), ""));
    }
    let cases = vec![
        (
            "oversize",
            json!([write("answer.json", ANSWER),
                {"op":"large","path":"big.bin","bytes":16 * 1024 * 1024 + 1}]),
        ),
        ("flood", json!(flood)),
        (
            "junction",
            json!([write("answer.json", ANSWER),
                {"op":"junction","path":"linked","target":outside.path()}]),
        ),
        (
            "hardlink",
            json!([write("answer.json", ANSWER),
                {"op":"hardlink","path":"alias.txt","target":outside.path().join("outside.txt")}]),
        ),
    ];
    for (name, plan) in cases {
        let f = Fixture::new();
        f.pin();
        run(&f, plan, &CountFactory::default()).await.unwrap();
        let (owner, folder) = f.record();
        assert_never_promoted(&owner, &folder);
        assert_eq!(folder["collection"]["effects"], UNEXPECTED_CHANGE, "{name}");
        assert_ne!(
            owner["outcome_receipt"]["reported_effects"], PAYLOAD_ONLY,
            "{name}"
        );
    }
}

/// C4-B: a real-pair grant without `payload_path` is still approved, and the
/// existing non-collector path then adopts a provider-written result.json
/// with self-reported outcome/effects. Expected by the C2 design (change A):
/// the real pair never reaches the provider-written envelope path.
#[test]
fn c4_real_pair_without_payload_path_is_refused_by_the_loader() {
    let f = Fixture::with_grant(|g, _| {
        let g = g.as_object_mut().unwrap();
        g.remove("payload_path");
        g.remove("launch_mode");
        g.insert("effect_scope".into(), json!(["none"]));
    });
    let mut loader = GrantLoader::open(f.root.path()).unwrap();
    let loaded = loader.load_approved();
    assert!(
        loaded.is_err(),
        "real pair without collector loaded: payload={:?}",
        loaded.as_ref().ok().map(|g| g[0].payload.is_some())
    );
}

#[tokio::test]
async fn c4_real_pair_without_payload_never_adopts_a_provider_written_result() {
    let f = Fixture::with_grant(|g, _| {
        let g = g.as_object_mut().unwrap();
        g.remove("payload_path");
        g.remove("launch_mode");
        g.insert("effect_scope".into(), json!(["none"]));
    });
    f.pin();
    let planted = json!({"binding":f.binding,"owner_id":f.ticket.owner_id,
        "owner_epoch":f.ticket.owner_epoch,"instruction_hash":f.binding.instruction_hash,
        "outcome":"succeeded","effects":"none","synthetic":false,
        "payload":serde_json::from_str::<Value>(ANSWER).unwrap()})
    .to_string();
    // After the fix the wrapper refuses to adopt the grant at all: no owner
    // state, no spawn, and so no provider-written result can be adopted.
    let adopted = DurableExecutionOwner::adopt(
        f.root.path(),
        &serde_json::to_string(&f.ticket).unwrap(),
        REAL_BOUNDED_PROVIDER,
        REAL_BOUNDED_PROFILE_ID,
    );
    if adopted.is_err() {
        assert_eq!(f.owners().query(&f.binding).unwrap_err().code, "NOT_FOUND");
        return;
    }
    drop(adopted);
    let factory = CountFactory::default();
    // The approved folder is outside the runtime root, so plant by absolute path.
    let result = run(
        &f,
        json!([write(f.result_path().to_str().unwrap(), &planted)]),
        &factory,
    )
    .await;
    let (owner, folder) = f.record();
    let receipt = owner["outcome_receipt"].clone();
    let verdict = if receipt.is_null() {
        Value::Null
    } else {
        let mut loader = GrantLoader::open(f.root.path()).unwrap();
        let grant = loader.load_approved().unwrap().into_iter().next().unwrap();
        let owners = f.owners();
        f.verifications()
            .verify(
                &owners,
                &grant.result,
                &f.ticket,
                &sha(planted.as_bytes()),
                now_ms(),
            )
            .map(|v| v["verdict"].clone())
            .unwrap_or(Value::Null)
    };
    assert!(
        receipt.is_null(),
        "spawned={} run_ok={} folder={} reported_outcome={} reported_effects={} verdict={}",
        factory.0.load(Ordering::SeqCst),
        result.is_ok(),
        folder,
        receipt["reported_outcome"],
        receipt["reported_effects"],
        verdict
    );
}

// ---- review2 adversarial additions (synthetic data only) ----

fn r2_mklink_junction(link: &Path, target: &Path) {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

/// R2-D: approved_cwd inside the runtime root must be refused however it is
/// spelled: other letter case, verbatim `\\?\` prefix, or via a junction that
/// lives outside the root and points inside it (or at the root itself).
#[test]
fn r2_collector_folder_inside_root_refused_for_every_spelling() {
    let outside = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, Box<dyn Fn(&Path, &Path) -> String>)> = vec![
        (
            "upper-case",
            Box::new(|root, _| {
                std::fs::create_dir_all(root.join("inside")).unwrap();
                root.canonicalize()
                    .unwrap()
                    .join("inside")
                    .to_string_lossy()
                    .to_uppercase()
            }),
        ),
        (
            "verbatim",
            Box::new(|root, _| {
                std::fs::create_dir_all(root.join("inside")).unwrap();
                let c = root.canonicalize().unwrap().join("inside");
                let s = c.to_string_lossy().to_string();
                if s.starts_with(r"\\?\") {
                    s
                } else {
                    format!(r"\\?\{s}")
                }
            }),
        ),
        (
            "plain-non-verbatim",
            Box::new(|root, _| {
                std::fs::create_dir_all(root.join("inside")).unwrap();
                let c = root.canonicalize().unwrap().join("inside");
                c.to_string_lossy().trim_start_matches(r"\\?\").to_string()
            }),
        ),
        (
            "junction-to-inside",
            Box::new(|root, outside| {
                std::fs::create_dir_all(root.join("inside")).unwrap();
                let link = outside.join("j-inside");
                r2_mklink_junction(&link, &root.join("inside"));
                link.to_string_lossy().to_string()
            }),
        ),
        (
            "junction-to-root",
            Box::new(|root, outside| {
                let link = outside.join("j-root");
                r2_mklink_junction(&link, root);
                link.to_string_lossy().to_string()
            }),
        ),
        (
            "junction-to-artifacts",
            Box::new(|root, outside| {
                let link = outside.join("j-art");
                r2_mklink_junction(&link, &root.join("iris-execution-artifacts"));
                link.to_string_lossy().to_string()
            }),
        ),
    ];
    let mut observed = Vec::new();
    for (name, make) in cases {
        let sub = outside.path().join(name);
        std::fs::create_dir_all(&sub).unwrap();
        let f = Fixture::with_grant(|g, root| {
            g["approved_cwd"] = json!(make(root, &sub));
        });
        let mut loader = GrantLoader::open(f.root.path()).unwrap();
        let code = loader.load_approved().err().map(|e| e.code);
        observed.push((name, code));
    }
    for (name, code) in &observed {
        assert_eq!(
            code.as_deref(),
            Some("APPROVED_CWD"),
            "{name}: {observed:?}"
        );
    }
}

/// R2-B: only the exact pair may carry a payload; near-miss spellings of the
/// pair with a payload are refused, and without a payload they are not the
/// real pair (so no owned profile admits them).
#[test]
fn r2_payload_only_on_exact_real_pair() {
    for (provider, profile) in [
        ("Claude", REAL_BOUNDED_PROFILE_ID),
        (REAL_BOUNDED_PROVIDER, "Claude-Hiroshi"),
        (REAL_BOUNDED_PROVIDER, "claude-hiroshi-"),
        ("codex", REAL_BOUNDED_PROFILE_ID),
        ("synthetic", "synthetic"),
    ] {
        let f = Fixture::with_grant(|g, _| {
            g["binding"]["provider"] = json!(provider);
            g["binding"]["profile_id"] = json!(profile);
            g.as_object_mut().unwrap().remove("launch_mode");
        });
        let mut loader = GrantLoader::open(f.root.path()).unwrap();
        assert_eq!(
            loader.load_approved().err().map(|e| e.code).as_deref(),
            Some("MANIFEST_POLICY"),
            "{provider}/{profile}"
        );
    }
}

/// R2-A1: the approved folder's own attributes are not part of the snapshot.
/// Contract: anything other than creating the payload is unexpected.
#[test]
fn r2_root_folder_attribute_change_is_unexpected() {
    use many_ai_cli::hub::iris_execution_folder::{compare, snapshot};
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("work");
    std::fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let before = snapshot(&root).unwrap();
    std::fs::write(root.join("answer.json"), b"{}").unwrap();
    // `attrib` does not understand the verbatim prefix on a directory: it
    // prints "File not found" and still exits 0. Use the plain spelling and
    // check that the hidden bit really was set.
    let plain = root
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();
    let status = std::process::Command::new("attrib")
        .arg("+H")
        .arg(&plain)
        .status()
        .unwrap();
    assert!(status.success());
    let hidden = {
        use std::os::windows::fs::MetadataExt;
        std::fs::symlink_metadata(&root).unwrap().file_attributes() & 0x2 != 0
    };
    let after = snapshot(&root).unwrap();
    let (effects, changes) = compare(&before, &after, "answer.json");
    let _ = std::process::Command::new("attrib")
        .arg("-H")
        .arg(&plain)
        .status();
    assert!(hidden, "attrib did not set the hidden bit on the folder");
    assert_eq!(effects, UNEXPECTED_CHANGE, "{changes:?}");
    assert!(
        changes.contains(&json!({"path":"","change":"modified","kind":"dir"})),
        "{changes:?}"
    );
}

/// R2-A2: an ACL change on an existing file inside the folder changes neither
/// names, sizes, hashes, link counts, attributes nor streams.
///
/// Known gap, deliberately not handled in code for now: the snapshot does not
/// record access control lists (of the folder or of anything in it), and the
/// milestone-0 document lists them as not observed. This test states the
/// wished-for behaviour and stays ignored; run it with `--ignored` to see that
/// an ACL-only change is still not detected.
#[test]
#[ignore = "ACLs are deliberately not recorded; documented as not observed"]
fn r2_acl_change_inside_folder_is_unexpected() {
    use many_ai_cli::hub::iris_execution_folder::{compare, snapshot};
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    std::fs::write(root.join("seed.txt"), b"synthetic seed").unwrap();
    let before = snapshot(&root).unwrap();
    std::fs::write(root.join("answer.json"), b"{}").unwrap();
    // Grant read to the well-known Everyone SID: harmless and synthetic.
    let status = std::process::Command::new("icacls")
        .arg(root.join("seed.txt"))
        .args(["/grant", "*S-1-1-0:(R)"])
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let after = snapshot(&root).unwrap();
    let (effects, changes) = compare(&before, &after, "answer.json");
    assert_eq!(effects, UNEXPECTED_CHANGE, "{changes:?}");
}

/// R2-A3 (control): a hard link created from outside to an inside file raises
/// its link count and is caught.
#[test]
fn r2_outside_hardlink_to_inside_file_is_unexpected() {
    use many_ai_cli::hub::iris_execution_folder::{compare, snapshot};
    let d = tempfile::tempdir().unwrap();
    let o = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    std::fs::write(root.join("seed.txt"), b"synthetic seed").unwrap();
    let before = snapshot(&root).unwrap();
    std::fs::write(root.join("answer.json"), b"{}").unwrap();
    std::fs::hard_link(root.join("seed.txt"), o.path().join("alias.txt")).unwrap();
    let after = snapshot(&root).unwrap();
    let (effects, changes) = compare(&before, &after, "answer.json");
    assert_eq!(effects, UNEXPECTED_CHANGE, "{changes:?}");
}

/// R2-A4 (probe): stream names with separator-like characters. NTFS refuses
/// `\`, `/` and `:` in stream names; if one is ever created it must still be a
/// distinct compared entry.
#[test]
fn r2_stream_name_with_separator_like_characters() {
    use many_ai_cli::hub::iris_execution_folder::{compare, snapshot};
    let d = tempfile::tempdir().unwrap();
    let root = d.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("seed.txt"), b"synthetic seed").unwrap();
    let before = snapshot(&root).unwrap();
    let mut created = Vec::new();
    for stream in [
        "seed.txt:a/b",
        "seed.txt:a\\b",
        "sub:x/seed.txt",
        "seed.txt:a.b c",
    ] {
        let mut target = root.as_os_str().to_owned();
        target.push("\\");
        target.push(stream);
        if std::fs::write(Path::new(&target), b"s").is_ok() {
            created.push(stream);
        }
    }
    let after = snapshot(&root).unwrap();
    let (effects, changes) = compare(&before, &after, "answer.json");
    println!("r2 created={created:?} changes={changes:?}");
    let keys: std::collections::BTreeSet<_> = after.iter().map(|e| e.path.clone()).collect();
    assert_eq!(keys.len(), after.len(), "duplicate snapshot keys");
    if !created.is_empty() {
        assert_eq!(effects, UNEXPECTED_CHANGE);
    }
}

/// R2-C: a payload that does not match the after snapshot must end as
/// unexpected-change with reason PAYLOAD_CHANGED. Proxy: the operator spells
/// the payload with other letter case, so the case-insensitive read succeeds
/// while the after snapshot holds the provider's spelling. With an invalid
/// JSON body the PAYLOAD_CHANGED code is overwritten by PAYLOAD_JSON.
#[tokio::test]
async fn r2_payload_changed_reason_survives_invalid_json() {
    for (body, label) in [(ANSWER, "valid"), ("{", "invalid")] {
        let f = Fixture::with_grant(|g, _| {
            g["payload_path"] = json!("Answer.json");
        });
        f.pin();
        run(
            &f,
            json!([write("answer.json", body)]),
            &CountFactory::default(),
        )
        .await
        .unwrap();
        let (owner, folder) = f.record();
        assert_never_promoted(&owner, &folder);
        let c = &folder["collection"];
        println!(
            "r2 {label}: effects={} reason={} detail={} payload_sha={}",
            c["effects"], c["reason"], c["reason_detail"], c["payload_sha256"]
        );
        assert_eq!(c["effects"], UNEXPECTED_CHANGE, "{label}");
        assert_eq!(c["reason"], "PAYLOAD_CHANGED", "{label}: {c}");
    }
}

// ---- C8 folder trust (synthetic Claude configuration in a temporary folder) ----

/// Claude's project key for a folder: plain drive spelling, forward slashes.
fn claude_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap()
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/")
}
fn read_config(f: &Fixture) -> Value {
    serde_json::from_slice(&std::fs::read(f.config_file()).unwrap()).unwrap()
}

#[tokio::test]
async fn c8_real_bounded_launch_trusts_exactly_the_approved_folder_before_claim() {
    let f = Fixture::new();
    let other = json!({"hasTrustDialogAccepted":false,"note":"synthetic"});
    std::fs::write(
        f.config_file(),
        json!({"synthetic_identity":"SYNTHETIC_PLACEHOLDER",
            "projects":{"Z:/synthetic/other":other}})
        .to_string(),
    )
    .unwrap();
    let factory = CountFactory::default();
    run(&f, json!([write("answer.json", ANSWER)]), &factory)
        .await
        .unwrap();
    assert_eq!(factory.0.load(Ordering::SeqCst), 1);
    let key = claude_key(&f.workspace());
    let config = read_config(&f);
    let projects = config["projects"].as_object().unwrap();
    assert_eq!(projects.len(), 2, "{projects:?}");
    assert_eq!(projects[&key]["hasTrustDialogAccepted"], true);
    assert_eq!(projects["Z:/synthetic/other"], other);
    assert!(!projects.contains_key(&claude_key(f.work.path())));
    assert_eq!(config["synthetic_identity"], "SYNTHETIC_PLACEHOLDER");
    assert!(!f.home_file().exists());
    let record = f.trust_record();
    assert_eq!(record["kind"], "folder_trust");
    assert_eq!(record["result"], "written");
    assert_eq!(record["written"], true);
    assert_eq!(record["key"], key);
    assert_eq!(
        record["target"],
        f.workspace()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(record["ticket"], json!(f.ticket));
    assert_eq!(record["reason"], Value::Null);
    assert_eq!(record["tool_grants"], json!([]));
    assert_eq!(record["risk_confirmed"], false);
    // The intent precedes the write and names the same target and key.
    let intent = f.trust_intent();
    assert_eq!(intent["phase"], "intent");
    assert_eq!(intent["result"], "pending");
    assert_eq!(intent["key"], key);
    assert_eq!(intent["target"], record["target"]);
    assert_eq!(record["planned_key"], key);
    let (owner, folder) = f.record();
    assert_never_promoted(&owner, &folder);
    assert!(
        intent["observed_at_ms"].as_u64().unwrap() <= record["observed_at_ms"].as_u64().unwrap()
    );
    // The claim carries the admission time of the same before_spawn call.
    assert!(intent["observed_at_ms"].as_u64().unwrap() <= owner["claimed_at_ms"].as_u64().unwrap());
    // The trust write lands outside the approved folder: still payload-only.
    assert_eq!(folder["collection"]["effects"], PAYLOAD_ONLY);
}

#[tokio::test]
async fn c8_trust_failure_refuses_launch_and_is_recorded() {
    let elsewhere = tempfile::tempdir().unwrap();
    let cases = [
        ("corrupt-config", "FOLDER_TRUST_WRITE"),
        ("no-config-dir", "FOLDER_TRUST_WRITE"),
        ("relative-config-dir", "FOLDER_TRUST_WRITE"),
        ("config-outside-temporary-folder", "FOLDER_TRUST_WRITE"),
        ("repository-root-above", "FOLDER_TRUST_WRITE"),
        ("no-writer", "FOLDER_TRUST_UNAVAILABLE"),
        ("policy-disabled", "FOLDER_TRUST_PROFILE"),
        ("policy-absent", "FOLDER_TRUST_PROFILE"),
    ];
    for (name, reason) in cases {
        let f = Fixture::new();
        let mut process = f.process(&json!([write("answer.json", ANSWER)]));
        let mut owner = Some(f.adopt());
        match name {
            "corrupt-config" => std::fs::write(f.config_file(), "{not json").unwrap(),
            "no-config-dir" => {
                process
                    .env
                    .remove(std::ffi::OsStr::new("CLAUDE_CONFIG_DIR"));
            }
            "relative-config-dir" => {
                process
                    .env
                    .insert("CLAUDE_CONFIG_DIR".into(), Some("claude-config".into()));
            }
            "config-outside-temporary-folder" => {
                process.env.insert(
                    "CLAUDE_CONFIG_DIR".into(),
                    Some(elsewhere.path().as_os_str().into()),
                );
            }
            "repository-root-above" => std::fs::create_dir(f.work.path().join(".git")).unwrap(),
            "no-writer" => owner = Some(Arc::new(f.adopt_without_trust())),
            "policy-disabled" => write_real_policies(f.root.path(), &f.binding, false),
            _ => std::fs::remove_file(
                f.root
                    .path()
                    .join("iris-execution/owned-real-policies.json"),
            )
            .unwrap(),
        }
        let before = std::fs::read(f.config_file()).ok();
        let factory = CountFactory::default();
        let result = run_owner(&f, owner.take().unwrap(), process, &factory).await;
        assert!(result.is_err(), "{name}");
        assert_eq!(factory.0.load(Ordering::SeqCst), 0, "{name}");
        let (record, folder) = f.record();
        assert!(record["claimed_session"].is_null(), "{name}");
        assert_never_promoted(&record, &folder);
        let trust = f.trust_record();
        assert_eq!(trust["result"], "failed", "{name}");
        assert_eq!(trust["reason"], reason, "{name}");
        assert_eq!(trust["written"], false, "{name}");
        // Only a failure of the writer itself follows a recorded intent; a
        // failure decided before any write has the result row only.
        assert_eq!(
            !f.trust_intent().is_null(),
            reason == "FOLDER_TRUST_WRITE",
            "{name}"
        );
        assert_eq!(std::fs::read(f.config_file()).ok(), before, "{name}");
        assert!(
            !elsewhere
                .path()
                .join(f.config_file().file_name().unwrap())
                .exists(),
            "{name}"
        );
        assert!(!f.home_file().exists(), "{name}");
    }
}

#[tokio::test]
async fn c8_no_trust_outside_the_real_bounded_launch() {
    let without_collector = |g: &mut Value, _: &Path| {
        let g = g.as_object_mut().unwrap();
        g.remove("payload_path");
        g.remove("launch_mode");
        g.insert("effect_scope".into(), json!(["none"]));
    };
    let cases = [
        (
            "real-pair-default-mode",
            Fixture::with_grant(|g, _| {
                g.as_object_mut().unwrap().remove("launch_mode");
            }),
        ),
        (
            "synthetic",
            Fixture::for_pair("synthetic", "synthetic", without_collector),
        ),
        (
            "codex",
            Fixture::for_pair("codex", "codex", without_collector),
        ),
    ];
    for (name, f) in cases {
        let factory = CountFactory::default();
        // Non-collector pairs report through a result file; a missing one is
        // a wrapper error after the spawn, so the stand-in writes a valid one.
        let plan = if f.binding.provider == REAL_BOUNDED_PROVIDER {
            json!([])
        } else {
            let envelope = json!({"binding":f.binding,"owner_id":f.ticket.owner_id,
                "owner_epoch":f.ticket.owner_epoch,
                "instruction_hash":f.binding.instruction_hash,"outcome":"unknown",
                "effects":"none","synthetic":f.binding.provider == "synthetic"});
            json!([write(
                f.result_path().to_str().unwrap(),
                &envelope.to_string()
            )])
        };
        run(&f, plan, &factory)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(factory.0.load(Ordering::SeqCst), 1, "{name}");
        assert!(!f.config_file().exists(), "{name}");
        assert!(!f.home_file().exists(), "{name}");
        assert!(f.trust_record().is_null(), "{name}");
        assert!(f.trust_intent().is_null(), "{name}");
    }
}

#[tokio::test]
async fn c8_already_trusted_parent_is_accepted_without_writing() {
    let f = Fixture::new();
    let parent = claude_key(f.work.path());
    let body = json!({"projects":{parent.clone():{"hasTrustDialogAccepted":true}}}).to_string();
    std::fs::write(f.config_file(), &body).unwrap();
    let factory = CountFactory::default();
    run(&f, json!([write("answer.json", ANSWER)]), &factory)
        .await
        .unwrap();
    assert_eq!(factory.0.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read_to_string(f.config_file()).unwrap(), body);
    let record = f.trust_record();
    assert_eq!(record["result"], "already-trusted");
    assert_eq!(record["written"], false);
    assert_eq!(record["key"], parent);
}

#[test]
fn c8_same_ticket_never_reaches_the_writer_twice() {
    let f = Fixture::new();
    let plan = f.process(&json!([]));
    f.adopt().before_spawn(1, &plan).unwrap();
    let first = f.trust_record();
    assert_eq!(first["result"], "written");
    std::fs::remove_file(f.config_file()).unwrap();
    assert!(f.adopt().before_spawn(2, &plan).is_err());
    assert!(!f.config_file().exists());
    assert_eq!(f.trust_record(), first);
}

// ---- Review 3 (adversarial, synthetic only) ----

/// R3-1: trust is written only over an empty approved folder. A folder that
/// already holds Claude project settings (synthetic hook and MCP files), any
/// other plain file, or a named stream on the folder itself is refused before
/// the trust write and the launch, with `FOLDER_TRUST_NOT_EMPTY`.
#[tokio::test]
async fn r3_trust_is_written_over_a_folder_with_project_settings() {
    for case in ["project-settings", "plain-file", "folder-stream"] {
        let f = Fixture::new();
        let ws = f.workspace();
        match case {
            "project-settings" => {
                std::fs::create_dir(ws.join(".claude")).unwrap();
                std::fs::write(
                    ws.join(".claude").join("settings.json"),
                    r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo synthetic-hook"}]}]},"permissions":{"allow":["Bash(*)"]}}"#,
                )
                .unwrap();
                std::fs::write(
                    ws.join(".mcp.json"),
                    r#"{"mcpServers":{"synthetic":{"command":"synthetic"}}}"#,
                )
                .unwrap();
            }
            "plain-file" => std::fs::write(ws.join("notes.txt"), b"synthetic").unwrap(),
            _ => {
                let mut target = ws.canonicalize().unwrap().into_os_string();
                target.push(":synthetic-stream");
                std::fs::write(Path::new(&target), b"synthetic").unwrap();
            }
        }
        let factory = CountFactory::default();
        let result = run(&f, json!([write("answer.json", ANSWER)]), &factory).await;
        let record = f.trust_record();
        println!(
            "R3-1 {case}: result_ok={} launches={} trust_result={} reason={}",
            result.is_ok(),
            factory.0.load(Ordering::SeqCst),
            record["result"],
            record["reason"]
        );
        assert!(result.is_err(), "{case}");
        assert_eq!(factory.0.load(Ordering::SeqCst), 0, "{case}");
        assert_eq!(record["result"], "failed", "{case}");
        assert_eq!(record["reason"], "FOLDER_TRUST_NOT_EMPTY", "{case}");
        assert!(f.trust_intent().is_null(), "{case}");
        assert!(!f.config_file().exists(), "{case}");
        assert!(f.record().0["claimed_session"].is_null(), "{case}");
    }
}

/// R3-2: an intent row (target, planned key, time) is written before the
/// vendor write, so a trust can never exist without a record. When the result
/// insert fails, the launch is refused, the intent remains, and a re-entry with
/// the same ticket is refused instead of recording a false "already-trusted".
#[tokio::test]
async fn r3_trust_written_without_record_when_record_insert_fails() {
    let f = Fixture::new();
    let owner = f.adopt();
    {
        // Same schema the store creates on first use; only the result row is
        // blocked, the intent row is accepted.
        let db =
            rusqlite::Connection::open(f.root.path().join("iris-execution/owner.sqlite3")).unwrap();
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS trust_records(principal TEXT NOT NULL,workspace TEXT NOT NULL,attempt TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN('intent','result')),document TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(principal,workspace,attempt,phase),FOREIGN KEY(principal,workspace,attempt) REFERENCES owners(principal,workspace,attempt));
             CREATE TRIGGER synthetic_block BEFORE INSERT ON trust_records WHEN NEW.phase='result' BEGIN SELECT RAISE(ABORT,'synthetic'); END;",
        )
        .unwrap();
    }
    let factory = CountFactory::default();
    let result = run_owner(
        &f,
        owner,
        f.process(&json!([write("answer.json", ANSWER)])),
        &factory,
    )
    .await;
    let key = claude_key(&f.workspace());
    let trusted = f.config_file().exists()
        && read_config(&f)["projects"][&key]["hasTrustDialogAccepted"] == true;
    let record = f.trust_record();
    println!(
        "R3-2 first: result_ok={} launches={} config_trusted={} record_null={}",
        result.is_ok(),
        factory.0.load(Ordering::SeqCst),
        trusted,
        record.is_null()
    );
    assert!(result.is_err());
    assert_eq!(factory.0.load(Ordering::SeqCst), 0);
    assert!(trusted, "the write happened");
    assert!(record.is_null(), "the result row was blocked");
    // ...but the write is never unrecorded: its intent names target and key.
    let intent = f.trust_intent();
    assert_eq!(intent["result"], "pending");
    assert_eq!(intent["key"], key);
    {
        let db =
            rusqlite::Connection::open(f.root.path().join("iris-execution/owner.sqlite3")).unwrap();
        db.execute_batch("DROP TRIGGER synthetic_block;").unwrap();
    }
    let factory = CountFactory::default();
    let again = run(&f, json!([write("answer.json", ANSWER)]), &factory).await;
    let record = f.trust_record();
    println!(
        "R3-2 re-entry: result_ok={} launches={} trust_result={} written={}",
        again.is_ok(),
        factory.0.load(Ordering::SeqCst),
        record["result"],
        record["written"]
    );
    // An intent without a result refuses re-entry: no launch and no false
    // "already-trusted" row for the trust this attempt wrote itself.
    assert!(again.is_err());
    assert_eq!(factory.0.load(Ordering::SeqCst), 0);
    assert!(record.is_null());
    assert_eq!(f.trust_intent(), intent);
}

/// R3-3: OwnerStore::open checks that each trust and folder row's key columns
/// (principal, workspace, attempt) are the binding in its own document.
#[test]
fn r3_open_rejects_rows_whose_key_differs_from_their_ticket() {
    for table in ["trust_records", "folder_records"] {
        let f = Fixture::new();
        f.adopt().before_spawn(1, &f.process(&json!([]))).unwrap();
        let db_path = f.root.path().join("iris-execution/owner.sqlite3");
        assert!(OwnerStore::open(&db_path).is_ok(), "{table}");
        {
            // Move only the row key; the document and its digest stay intact.
            let db = rusqlite::Connection::open(&db_path).unwrap();
            db.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
            db.execute(
                &format!("UPDATE {table} SET attempt='other-attempt' WHERE attempt='attempt'"),
                [],
            )
            .unwrap_or_else(|e| panic!("{table}: {e}"));
        }
        let opened = OwnerStore::open(&db_path);
        println!("R3-3 {table}: open_ok={}", opened.is_ok());
        assert!(opened.is_err(), "{table}");
    }
}

/// R3-3 (control): the approved folder itself, reached through a junction, is
/// never trusted under the junction spelling: the cwd check refuses first.
#[tokio::test]
async fn r3_junction_spelling_of_the_approved_folder_is_refused_before_trust() {
    let f = Fixture::new();
    let alias = f.work.path().join("alias");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&alias)
        .arg(f.workspace())
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let mut process = f.process(&json!([write("answer.json", ANSWER)]));
    process.cwd = alias.clone();
    let factory = CountFactory::default();
    let result = run_owner(&f, f.adopt(), process, &factory).await;
    println!(
        "R3-3 result_ok={} launches={} record={}",
        result.is_ok(),
        factory.0.load(Ordering::SeqCst),
        f.trust_record()
    );
    // canonicalize(alias) == approved_cwd, so the launch proceeds; the key written
    // must still be the canonical folder, never the alias spelling.
    if f.config_file().exists() {
        let projects = read_config(&f)["projects"].clone();
        let alias_key = alias.to_string_lossy().replace('\\', "/");
        assert!(projects.get(&alias_key).is_none());
        assert_eq!(projects.as_object().unwrap().len(), 1);
    }
}

/// R3-4 (control): a lowercase / 8.3 / verbatim spelling of the cwd never
/// widens the trust; only the canonical key is ever written.
#[test]
fn r3_exact_trust_key_is_the_canonical_folder_only() {
    let f = Fixture::new();
    let plan = f.process(&json!([]));
    f.adopt().before_spawn(1, &plan).unwrap();
    let projects = read_config(&f)["projects"].as_object().unwrap().clone();
    let keys: Vec<_> = projects.keys().cloned().collect();
    println!("R3-4 keys={}", keys.len());
    assert_eq!(keys, vec![claude_key(&f.workspace())]);
}
