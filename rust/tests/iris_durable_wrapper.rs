//! Actual durable owner, concurrent Hub journal, native synthetic wrapper only.
#![cfg(windows)]
use many_ai_cli::{
    hub::{
        iris_execution_journal::{Binding, Grant, Journal},
        iris_execution_owner::{OwnerStore, OwnerTicket},
    },
    process::{
        Cancellation, ProcessPlan,
        pty::{NativePtyFactory, PtyFactory, PtySession, PtySize},
    },
    proto::{Message, core::CoreFuture},
    wrapper::{
        iris_owner::DurableExecutionOwner,
        runtime::{self, WrapperOptions},
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
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
struct Fixture {
    root: tempfile::TempDir,
    binding: Binding,
    ticket: OwnerTicket,
    manifest: Value,
    journal: Option<Journal>,
}
impl Fixture {
    fn new(with_main: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("workspace")).unwrap();
        std::fs::create_dir(root.path().join("iris-execution-artifacts")).unwrap();
        std::fs::write(
            root.path().join("iris-execution-artifacts/instruction.txt"),
            b"fixed synthetic durable instruction",
        )
        .unwrap();
        let binding = Binding {
            principal_id: "iris".into(),
            workspace_id: "workspace".into(),
            plan_id: "plan".into(),
            plan_revision: 1,
            task_id: "task".into(),
            instruction_revision: 1,
            instruction_hash: sha(b"fixed synthetic durable instruction"),
            approval_id: "approval".into(),
            approval_revision: 1,
            scope_hash: sha(b"synthetic scope"),
            execution_host_id: "host".into(),
            executor_instance_id: "instance".into(),
            attempt_id: "attempt".into(),
            fence: 1,
            provider: "synthetic".into(),
            profile_id: "synthetic".into(),
        };
        let ticket = OwnerTicket {
            binding: binding.clone(),
            owner_id: "synthetic-owner".into(),
            owner_epoch: 1,
        };
        let manifest = json!({"schema_version":1,"manifest_revision":1,"approval_state":"approved","grants":[{"binding":binding,"expires_at":now()+600,"revoked":false,"approved_cwd":"workspace","instruction_path":"instruction.txt","result_path":"result.json","max_artifact_bytes":65536,"effect_scope":["none"],"allow_remote_jobs":false}]});
        std::fs::write(
            root.path().join("iris-execution-approved-grants.json"),
            manifest.to_string(),
        )
        .unwrap();
        let journal = if with_main {
            let mut j = Journal::open(
                &root.path().join("iris-execution/journal.sqlite3"),
                "host",
                "boot",
            )
            .unwrap();
            j.register_grant(Grant {
                binding: binding.clone(),
                expires_at: manifest["grants"][0]["expires_at"].as_i64().unwrap(),
                revoked: false,
            })
            .unwrap();
            j.start("start", &binding, now()).unwrap();
            Some(j)
        } else {
            None
        };
        Self {
            root,
            binding,
            ticket,
            manifest,
            journal,
        }
    }
    fn adopt(&self) -> Arc<DurableExecutionOwner> {
        Arc::new(
            DurableExecutionOwner::adopt(
                self.root.path(),
                &serde_json::to_string(&self.ticket).unwrap(),
                "synthetic",
                "synthetic",
            )
            .unwrap(),
        )
    }
    fn result_path(&self) -> PathBuf {
        self.root
            .path()
            .join("iris-execution-artifacts/result.json")
    }
    fn process(&self) -> ProcessPlan {
        let body = json!({"binding":self.binding,"owner_id":self.ticket.owner_id,"owner_epoch":self.ticket.owner_epoch,"instruction_hash":self.binding.instruction_hash,"outcome":"succeeded","effects":"none","synthetic":true}).to_string();
        let mut env = BTreeMap::new();
        env.insert(
            "MANY_AI_SYNTHETIC_DURABLE_RESULT_PATH".into(),
            Some(self.result_path().into_os_string()),
        );
        env.insert(
            "MANY_AI_SYNTHETIC_DURABLE_RESULT_BODY".into(),
            Some(body.into()),
        );
        ProcessPlan {
            executable: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "native_durable_helper".into(),
                "--nocapture".into(),
            ],
            cwd: self.root.path().join("workspace"),
            env,
            stdin: vec![],
            timeout: Duration::ZERO,
            output_cap: 1024,
            pipe_drain_timeout: Duration::from_millis(100),
        }
    }
    fn write_manifest(&self, manifest: &Value) {
        std::fs::write(
            self.root.path().join("iris-execution-approved-grants.json"),
            manifest.to_string(),
        )
        .unwrap();
    }
}
struct Connector {
    path: PathBuf,
    disconnected: Arc<AtomicBool>,
    frames: Arc<Mutex<Vec<String>>>,
}
struct Socket {
    registered: bool,
    path: PathBuf,
    disconnected: Arc<AtomicBool>,
    frames: Arc<Mutex<Vec<String>>>,
}
impl HubConnector for Connector {
    fn connect<'a>(&'a self, _: Option<&'a str>) -> CoreFuture<'a, io::Result<Box<dyn HubSocket>>> {
        Box::pin(async move {
            Ok(Box::new(Socket {
                registered: false,
                path: self.path.clone(),
                disconnected: self.disconnected.clone(),
                frames: self.frames.clone(),
            }) as Box<dyn HubSocket>)
        })
    }
    fn probe(&self) -> CoreFuture<'_, bool> {
        Box::pin(async { false })
    }
}
impl HubSocket for Socket {
    fn send<'a>(&'a mut self, frame: &'a Message) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            self.frames.lock().unwrap().push(frame.r#type.clone());
            Ok(())
        })
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
            while !self.path.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            self.disconnected.store(true, Ordering::SeqCst);
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "synthetic transport loss after artifact write",
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
async fn run(
    f: &Fixture,
    owner: Arc<DurableExecutionOwner>,
    factory: &CountFactory,
    disconnected: Arc<AtomicBool>,
) -> io::Result<runtime::WrapperResult> {
    let connector = Connector {
        path: f.result_path(),
        disconnected,
        frames: Arc::new(Mutex::new(vec![])),
    };
    let mut options = WrapperOptions::new(
        Message {
            provider: "synthetic".into(),
            token: "synthetic-only".into(),
            ..Default::default()
        },
        f.process(),
    );
    options.execution_owner = Some(owner);
    options.reconnect_grace = Duration::ZERO;
    options.drain_timeout = Duration::from_millis(100);
    tokio::time::timeout(
        Duration::from_secs(12),
        runtime::run(
            options,
            &connector,
            factory,
            &Cancellation::default(),
            |_, _| Ok(()),
        ),
    )
    .await
    .expect("durable wrapper deadline")
}
#[test]
fn native_durable_helper() {
    if let Some(path) = std::env::var_os("MANY_AI_SYNTHETIC_DURABLE_RESULT_PATH") {
        let body = std::env::var_os("MANY_AI_SYNTHETIC_DURABLE_RESULT_BODY").unwrap();
        std::fs::write(Path::new(&path), body.to_str().unwrap()).unwrap();
        // Keep the root alive until the fake transport observes the file and
        // disconnects; output integrity and local cessation remain independent.
        std::thread::sleep(Duration::from_millis(300));
    }
}
#[tokio::test]
async fn durable_native_owner_survives_socket_loss_and_replay_cannot_spawn() {
    let f = Fixture::new(true);
    let owner = f.adopt();
    let factory = CountFactory::default();
    let disconnected = Arc::new(AtomicBool::new(false));
    run(&f, owner.clone(), &factory, disconnected.clone())
        .await
        .unwrap();
    assert!(disconnected.load(Ordering::SeqCst));
    assert_eq!(factory.0.load(Ordering::SeqCst), 1);
    // The wrapper inspected the concurrently owned Journal without recovery.
    assert_eq!(
        f.journal.as_ref().unwrap().query(&f.binding).unwrap()["execution_state"],
        "accepted"
    );
    drop(owner);
    let store = OwnerStore::open(&f.root.path().join("iris-execution/owner.sqlite3")).unwrap();
    let saved = store.query(&f.binding).unwrap();
    assert_eq!(saved["local_tree_ceased"], true);
    assert_eq!(saved["cessation_receipt"]["kind"], "local_exit");
    assert_eq!(saved["cessation_receipt"]["scope"], "windows-owned-job");
    let native_exit = &saved["cessation_receipt"]["process_exit"];
    assert_eq!(
        native_exit["verification_scope"],
        "native-exit-observation-only"
    );
    assert_eq!(native_exit["signal"], "");
    if native_exit["forced"] == true {
        assert_eq!(native_exit["code"], 1);
    } else {
        assert_eq!(native_exit["code"], 0);
    }
    assert!(saved["root_pid"].as_u64().unwrap() > 0);
    assert_eq!(saved["outcome_receipt"]["reported_outcome"], "succeeded");
    assert_eq!(
        saved["outcome_receipt"]["verification_scope"],
        "artifact-integrity-only"
    );
    assert_eq!(
        saved["outcome_receipt"]["artifact_sha256"],
        sha(&std::fs::read(f.result_path()).unwrap())
    );
    assert_eq!(saved["outcome_receipt"]["ticket"], json!(f.ticket));
    assert_eq!(saved["task_outcome_verified"], false);
    assert_eq!(saved["verified_task_outcome"], "unknown");
    assert_eq!(saved["effects_verified"], false);
    drop(store);
    let second = f.adopt();
    let replay = CountFactory::default();
    assert!(
        run(&f, second, &replay, Arc::new(AtomicBool::new(false)))
            .await
            .is_err()
    );
    assert_eq!(replay.0.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn missing_or_revoked_main_journal_rejects_before_native_factory() {
    for with_main in [false, true] {
        let mut f = Fixture::new(with_main);
        let owner = f.adopt();
        if let Some(j) = f.journal.as_mut() {
            j.register_grant(Grant {
                binding: f.binding.clone(),
                expires_at: f.manifest["grants"][0]["expires_at"].as_i64().unwrap(),
                revoked: true,
            })
            .unwrap();
        }
        let factory = CountFactory::default();
        assert!(
            run(&f, owner, &factory, Arc::new(AtomicBool::new(false)))
                .await
                .is_err()
        );
        assert_eq!(factory.0.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn operator_manifest_changed_after_adoption_rejects_before_native_factory() {
    let f = Fixture::new(true);
    let owner = f.adopt();
    let mut changed = f.manifest.clone();
    changed["manifest_revision"] = json!(2);
    changed["grants"][0]["approved_cwd"] = json!("iris-execution-artifacts");
    f.write_manifest(&changed);
    let factory = CountFactory::default();
    assert!(
        run(&f, owner, &factory, Arc::new(AtomicBool::new(false)))
            .await
            .is_err()
    );
    assert_eq!(factory.0.load(Ordering::SeqCst), 0);
}
