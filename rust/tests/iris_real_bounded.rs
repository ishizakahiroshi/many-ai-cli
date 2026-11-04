//! Real bounded profile collector acceptance with a native synthetic stand-in.
//! No real provider, Hub or network is used: the test executable plays the
//! provider and writes files into the approved folder. Only the trusted local
//! collector may build the result envelope, and only after recorded cessation.
#![cfg(windows)]
use many_ai_cli::{
    hub::{
        iris_execution_folder::{PAYLOAD_ONLY, UNEXPECTED_CHANGE},
        iris_execution_grants::GrantLoader,
        iris_execution_journal::{Binding, Grant, Journal},
        iris_execution_owned::{REAL_BOUNDED_PROFILE_ID, REAL_BOUNDED_PROVIDER},
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
    binding: Binding,
    ticket: OwnerTicket,
    manifest: Value,
    _journal: Journal,
}
impl Fixture {
    fn new() -> Self {
        Self::with_grant(|_, _| {})
    }
    fn with_grant(edit: impl FnOnce(&mut Value, &Path)) -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        std::fs::create_dir(path.join("workspace")).unwrap();
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
            provider: REAL_BOUNDED_PROVIDER.into(),
            profile_id: REAL_BOUNDED_PROFILE_ID.into(),
        };
        let ticket = OwnerTicket {
            binding: binding.clone(),
            owner_id: "synthetic-owner".into(),
            owner_epoch: 1,
        };
        let expires = (now_ms() / 1000) as i64 + 600;
        let mut grant = json!({"binding":binding,"expires_at":expires,"revoked":false,
            "approved_cwd":"workspace","instruction_path":"instruction.txt",
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
        Self {
            root,
            binding,
            ticket,
            manifest,
            _journal: journal,
        }
    }
    fn workspace(&self) -> PathBuf {
        self.root.path().join("workspace")
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
        Arc::new(
            DurableExecutionOwner::adopt(
                self.root.path(),
                &serde_json::to_string(&self.ticket).unwrap(),
                REAL_BOUNDED_PROVIDER,
                REAL_BOUNDED_PROFILE_ID,
            )
            .unwrap(),
        )
    }
    fn process(&self, plan: &Value) -> ProcessPlan {
        let mut env = BTreeMap::new();
        env.insert(PLAN_ENV.into(), Some(plan.to_string().into()));
        env.insert(DONE_ENV.into(), Some(self.done_path().into_os_string()));
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
    let owner = f.adopt();
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
    assert_eq!(folder["before"]["entries"], json!([]));
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
        let f = Fixture::new();
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
