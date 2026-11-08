use super::*;
use crate::{
    application::hub_runtime::{RuntimeData, RuntimeLedger},
    config::{Config, ConfigStore, Resource},
    profile::store::{HistoryStore, ProviderRegistryStore},
    proto::{provider::Definition, time::Timestamp},
};
use std::{
    io::Cursor,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
struct Fixture {
    _temp: tempfile::TempDir,
    paths: RuntimePaths,
    config: Arc<ConfigStore>,
}
fn fixture() -> Fixture {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("runtime");
    let installed = t.path().join("installed");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&installed).unwrap();
    let paths = RuntimePaths::trial(&root, 49372, &installed).unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::defaults(&paths)).unwrap());
    Fixture {
        _temp: t,
        paths,
        config,
    }
}
#[test]
fn log_clean_real_private_files_preserve_short_answers_and_skip_crash_tail_noise() {
    let f = fixture();
    let input = f.paths.root().join("session.jsonl");
    std::fs::write(&input,concat!("{\"ts\":\"now\",\"type\":\"session_start\",\"session_id\":1,\"provider\":\"codex\",\"pid\":2}\n","{\"type\":\"pty_output\",\"text\":\"\\u001b[31mOK\\u001b[0m\\r\\nOK\\n✳ Thinking esc to interrupt\\nはい\\nNo\\n\"}\n","{\"ts\":\"later\",\"type\":\"user_input\",\"text\":\" next \"}\n","{\"type\":\"pty_output\",\"data_b64\":\"ZG9uZQ==\"}\n","{truncated")).unwrap();
    let output = log_clean(&f.paths, &input, None).unwrap();
    assert_eq!(output, input.with_extension("txt"));
    assert_eq!(
        std::fs::read_to_string(output).unwrap(),
        "[now] session_start #1 codex pid=2\n\n[output]\nOK\nはい\nNo\n\n[later] user_input\n> next\n\n[output]\ndone\n"
    );
    let outside = f._temp.path().join("outside.txt");
    assert!(log_clean(&f.paths, &input, Some(&outside)).is_err());
    assert!(!outside.exists());
}
#[test]
fn converter_go_wire_duplicate_casefold_and_corrupt_records_do_not_discard_valid_output() {
    let input=b"{\"TYPE\":\"pty_output\",\"TEXT\":\"first\\n\",\"text\":null}\nnot-json\n{\"type\":\"pty_output\",\"text\":\"first\\nsecond\"}";
    let mut output = vec![];
    transcript::write_transcript(&mut Cursor::new(input), &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\n[output]\nfirst\nsecond\n"
    );
    assert!(!transcript::is_thinking_noise_line("✓ test passed"));
    assert!(transcript::is_thinking_noise_line("↑111.0k ↓764"));
}
struct StopFixture {
    alive: AtomicBool,
    kills: AtomicUsize,
    graceful: bool,
}
impl stop::StopIo for StopFixture {
    fn alive(&self, _: &RuntimeData) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
    fn request<'a>(
        &'a self,
        port: u16,
        _: &'a str,
        _proof: Option<&'a str>,
        _: &'a crate::process::Cancellation,
    ) -> crate::proto::core::CoreFuture<'a, io::Result<bool>> {
        Box::pin(async move {
            assert_eq!(port, 49372);
            if self.graceful {
                self.alive.store(false, Ordering::SeqCst);
            }
            Ok(self.graceful)
        })
    }
    fn kill(&self, _: &RuntimeData) -> io::Result<bool> {
        self.kills.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}
#[tokio::test]
async fn stop_real_ledger_graceful_and_force_paths_only_call_fake_process_owner() {
    for graceful in [true, false] {
        let f = fixture();
        let ledger = RuntimeLedger::open(&f.paths).unwrap();
        ledger.write(49372, 987654, Timestamp::now()).unwrap();
        let io = Arc::new(StopFixture {
            alive: AtomicBool::new(true),
            kills: AtomicUsize::new(0),
            graceful,
        });
        let logs = Arc::new(Mutex::new(vec![]));
        let capture = logs.clone();
        let command = stop::StopCommand {
            ledger: ledger.clone(),
            config: f.config,
            io: io.clone(),
            log: Arc::new(move |message, _, _| {
                capture.lock().unwrap().push(message.to_owned());
                Ok(())
            }),
        };
        command
            .run(&crate::process::Cancellation::default())
            .await
            .unwrap();
        assert_eq!(io.kills.load(Ordering::SeqCst), usize::from(!graceful));
        assert!(ledger.read().unwrap().is_none());
        assert_eq!(logs.lock().unwrap().len(), if graceful { 1 } else { 2 });
    }
}
#[test]
fn native_trial_refuses_arbitrary_pid_without_sending_a_signal() {
    use stop::StopIo;
    let f = fixture();
    let native = stop::NativeStopIo { paths: f.paths };
    let data = RuntimeData {
        pid: 987654,
        port: 49372,
        ..RuntimeData::default()
    };
    assert_eq!(
        native.kill(&data).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}
/// What the force path finds on the recorded PID (U11).
type Outcome = fn() -> io::Result<bool>;
struct ForceOutcome {
    outcome: Outcome,
    kills: AtomicUsize,
}
impl stop::StopIo for ForceOutcome {
    fn alive(&self, _: &RuntimeData) -> bool {
        true
    }
    fn request<'a>(
        &'a self,
        _: u16,
        _: &'a str,
        _proof: Option<&'a str>,
        _: &'a crate::process::Cancellation,
    ) -> crate::proto::core::CoreFuture<'a, io::Result<bool>> {
        Box::pin(async { Err(io::Error::other("synthetic: nothing listens")) })
    }
    fn kill(&self, _: &RuntimeData) -> io::Result<bool> {
        self.kills.fetch_add(1, Ordering::SeqCst);
        (self.outcome)()
    }
}
#[tokio::test]
async fn audit_u11_force_stop_removes_the_record_only_when_its_hub_is_known_gone() {
    let cases: [(Outcome, bool, bool); 3] = [
        (|| Ok(true), true, true),
        // Another process holds the PID: left running, the stale record goes.
        (|| Ok(false), false, true),
        // Not identified, or not ended: the record stays for the next stop.
        (
            || Err(io::Error::other("synthetic: not identified")),
            false,
            false,
        ),
    ];
    for (outcome, succeeds, removed) in cases {
        let f = fixture();
        let ledger = RuntimeLedger::open(&f.paths).unwrap();
        ledger.write(49372, 987654, Timestamp::now()).unwrap();
        let io = Arc::new(ForceOutcome {
            outcome,
            kills: AtomicUsize::new(0),
        });
        let command = stop::StopCommand {
            ledger: ledger.clone(),
            config: f.config,
            io: io.clone(),
            log: Arc::new(|_, _, _| Ok(())),
        };
        let result = command.run(&crate::process::Cancellation::default()).await;
        assert_eq!(io.kills.load(Ordering::SeqCst), 1);
        assert_eq!(result.is_ok(), succeeds, "{result:?}");
        assert_eq!(ledger.read().unwrap().is_none(), removed);
        if !succeeds && removed {
            assert!(result.unwrap_err().to_string().contains("not running"));
        }
    }
}
#[cfg(any(windows, target_os = "linux"))]
#[test]
fn audit_u11_native_force_stop_ends_only_the_hub_the_record_names() {
    use crate::process::wrapper_startup::recovery_identity::{of_pid, test_support::Bystander};
    use stop::StopIo;
    let home = tempfile::tempdir().unwrap();
    let native = stop::NativeStopIo {
        paths: RuntimePaths::production(home.path()).unwrap(),
    };
    let mut bystander = Bystander::spawn();
    let pid = bystander.pid();
    let record = |process: &str, started_at: &str| RuntimeData {
        version: 1,
        pid: i64::from(pid),
        port: 49372,
        started_at: started_at.into(),
        process: process.into(),
    };
    // A Hub that ended, whose PID this unrelated process was given later.
    let other = record("synthetic:another-process", "2026-01-01T00:00:00Z");
    // The same, recorded before identities were kept: written before this
    // process existed.
    let older = record("", "2001-01-01T00:00:00Z");
    for stale in [&other, &older] {
        assert!(!native.alive(stale));
        assert!(!native.kill(stale).unwrap());
        assert!(
            bystander.running(),
            "a process that reuses the PID must keep running"
        );
    }
    let hub = record(&of_pid(pid).unwrap().unwrap(), "2026-01-01T00:00:00Z");
    assert!(native.alive(&hub));
    assert!(native.kill(&hub).unwrap());
    assert!(bystander.ends_within(std::time::Duration::from_secs(10)));
}
#[test]
fn provider_cli_actual_history_reset_and_recover_keep_revision_receipts() {
    let f = fixture();
    let store = Arc::new(ProviderRegistryStore::new(&f.paths, f.config).unwrap());
    let strings = |args: &[&str]| {
        args.iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
    };
    assert!(provider_command(&store, &strings(&["reset", "--distributed", "claude"])).is_err());
    let output = provider_command(
        &store,
        &strings(&[
            "reset",
            "--distributed",
            "claude",
            "--expected-revision",
            "",
        ]),
    )
    .unwrap();
    assert!(output.starts_with("reset\t"));
    let first = store.history.current("claude").unwrap();
    assert!(
        provider_command(
            &store,
            &strings(&[
                "reset",
                "--distributed",
                "claude",
                "--expected-revision",
                ""
            ])
        )
        .is_err()
    );
    let dir = Dir::open(&f.paths.resource(Resource::ProviderOverrides))
        .unwrap()
        .child_dir("claude", false)
        .unwrap();
    dir.replace("HEAD", b"missing-revision", 0o600).unwrap();
    let candidates = provider_command(&store, &strings(&["recover", "claude", "--list"])).unwrap();
    assert!(candidates.contains(&first.revision));
    assert!(candidates.ends_with("candidate\tbase\n"));
    let recovered =
        provider_command(&store, &strings(&["recover", "claude", &first.revision])).unwrap();
    assert!(recovered.starts_with("recovered\t"));
    assert_eq!(store.history.current("claude").unwrap().reason, "restore");
}
#[test]
fn history_backup_cas_tamper_and_recovery_are_canonical_and_fail_before_pointer_changes() {
    let f = fixture();
    let store = HistoryStore::new(&f.paths);
    let baseline = crate::profile::registry::embedded_definitions()
        .unwrap()
        .remove(0);
    let first = store
        .save_override(
            "claude",
            Definition {
                id: "claude".into(),
                enabled: Some(false),
                ..Default::default()
            },
            &baseline,
            "",
            "edit",
        )
        .unwrap();
    let second = store.reset("claude", &first.revision).unwrap();
    let backup = store.verify_backup("claude", &first.revision).unwrap();
    assert_eq!(backup.revision, first.revision);
    assert_eq!(store.list_backups("claude").unwrap().len(), 1);
    assert!(
        store
            .restore_backup("claude", "../HEAD", &second.revision)
            .is_err()
    );
    assert!(
        store
            .restore_backup("claude", &first.revision, "stale")
            .is_err()
    );
    assert_eq!(store.current("claude").unwrap().revision, second.revision);
    assert!(store.recover_head("claude", "").is_err());
    let dir = Dir::open(&f.paths.resource(Resource::ProviderOverrides))
        .unwrap()
        .child_dir("claude", false)
        .unwrap();
    let revisions = dir.child_dir("revisions", false).unwrap();
    revisions
        .create_new("zz-corrupt.json", b"corrupt", 0o600)
        .unwrap();
    dir.replace("HEAD", b"missing-revision", 0o600).unwrap();
    assert!(store.last_verified_revision("claude").unwrap().is_some());
    let restored = store.recover_head("claude", &first.revision).unwrap();
    assert_eq!(restored.parent_revision, "");
    assert!(restored.payload == first.payload);
    let quarantine = Dir::open(&f.paths.resource(Resource::ProviderBackups))
        .unwrap()
        .child_dir("quarantine", false)
        .unwrap();
    assert_eq!(quarantine.entries().unwrap().len(), 1);
    assert_eq!(
        quarantine
            .read(&quarantine.entries().unwrap()[0], 1024)
            .unwrap(),
        b"missing-revision"
    );
}

struct HeldStop {
    ledger: RuntimeLedger,
    replacement: bool,
    kills: AtomicUsize,
}
impl stop::StopIo for HeldStop {
    fn alive(&self, _: &RuntimeData) -> bool {
        true
    }
    fn request<'a>(
        &'a self,
        _: u16,
        _: &'a str,
        _proof: Option<&'a str>,
        _: &'a crate::process::Cancellation,
    ) -> crate::proto::core::CoreFuture<'a, io::Result<bool>> {
        Box::pin(async move {
            if self.replacement {
                self.ledger.write(49372, 987655, Timestamp::now())?;
            }
            Ok(true)
        })
    }
    fn kill(&self, _: &RuntimeData) -> io::Result<bool> {
        self.kills.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}
#[tokio::test(start_paused = true)]
async fn graceful_deadline_forces_only_original_ledger_incarnation() {
    for replacement in [false, true] {
        let f = fixture();
        let ledger = RuntimeLedger::open(&f.paths).unwrap();
        ledger.write(49372, 987654, Timestamp::now()).unwrap();
        let owner = Arc::new(HeldStop {
            ledger: ledger.clone(),
            replacement,
            kills: AtomicUsize::new(0),
        });
        let command = stop::StopCommand {
            ledger: ledger.clone(),
            config: f.config,
            io: owner.clone(),
            log: Arc::new(|_, _, _| Ok(())),
        };
        let start = tokio::time::Instant::now();
        let result = command.run(&crate::process::Cancellation::default()).await;
        assert!(
            tokio::time::Instant::now().duration_since(start) >= std::time::Duration::from_secs(5)
        );
        assert_eq!(
            owner.kills.load(Ordering::SeqCst),
            usize::from(!replacement)
        );
        if replacement {
            assert!(result.is_err());
            assert_eq!(ledger.read().unwrap().unwrap().pid, 987655);
        } else {
            assert!(result.is_ok());
            assert!(ledger.read().unwrap().is_none());
        }
    }
}

#[cfg(windows)]
#[test]
fn log_clean_trial_accepts_native_and_plain_windows_root_spellings_and_denies_parent_traversal() {
    let f = fixture();
    let native = f.paths.root().join("native.jsonl");
    std::fs::write(&native, b"{\"type\":\"pty_output\",\"text\":\"OK\"}").unwrap();
    let plain = PathBuf::from(
        native
            .to_string_lossy()
            .strip_prefix(r"\\?\")
            .unwrap_or(&native.to_string_lossy())
            .to_string(),
    );
    let output = log_clean(&f.paths, &plain, None).unwrap();
    assert_eq!(std::fs::read_to_string(output).unwrap(), "\n[output]\nOK\n");
    assert!(
        log_clean(
            &f.paths,
            &native,
            Some(&f.paths.root().join("../escaped.txt"))
        )
        .is_err()
    );
}
