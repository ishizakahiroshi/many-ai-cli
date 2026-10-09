use super::*;
use crate::{
    application::hub_runtime::RuntimeLedger,
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
    fn alive(&self, _: i64) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
    fn request<'a>(
        &'a self,
        port: u16,
        _: &'a str,
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
    fn kill(&self, _: i64) -> io::Result<()> {
        self.kills.fetch_add(1, Ordering::SeqCst);
        Ok(())
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
    assert_eq!(
        native.kill(987654).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
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
    fn alive(&self, _: i64) -> bool {
        true
    }
    fn request<'a>(
        &'a self,
        _: u16,
        _: &'a str,
        _: &'a crate::process::Cancellation,
    ) -> crate::proto::core::CoreFuture<'a, io::Result<bool>> {
        Box::pin(async move {
            if self.replacement {
                self.ledger.write(49372, 987655, Timestamp::now())?;
            }
            Ok(true)
        })
    }
    fn kill(&self, _: i64) -> io::Result<()> {
        self.kills.fetch_add(1, Ordering::SeqCst);
        Ok(())
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
