use super::*;
use crate::config::Config;
use std::{
    io::Cursor,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Fake {
    calls: AtomicUsize,
    args: Mutex<Vec<String>>,
    available: bool,
    fail: bool,
}
impl IssueIo for Fake {
    fn look_path(&self, _: &str) -> io::Result<String> {
        if self.available {
            Ok("synthetic-gh".into())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "fake absent"))
        }
    }
    fn open_web<'a>(
        &'a self,
        _: &'a str,
        args: Vec<String>,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.args.lock().unwrap() = args;
            if self.fail {
                Err(io::Error::other("synthetic failure"))
            } else {
                Ok(b"opened web preview\n".to_vec())
            }
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    paths: RuntimePaths,
    owner: IssueCli,
    io: Arc<Fake>,
}
fn fixture(environment: Vec<String>, available: bool, fail: bool) -> Fixture {
    let t = tempfile::tempdir().unwrap();
    let run = t.path().join("runtime");
    let installed = t.path().join("installed");
    std::fs::create_dir(&run).unwrap();
    std::fs::create_dir(&installed).unwrap();
    let paths = RuntimePaths::trial(&run, 49376, &installed).unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::defaults(&paths)).unwrap());
    let io = Arc::new(Fake {
        calls: AtomicUsize::new(0),
        args: Mutex::new(vec![]),
        available,
        fail,
    });
    let owner = IssueCli::new(IssueDependencies {
        config,
        paths: paths.clone(),
        environment,
        version: "synthetic".into(),
        platform: "windows".into(),
        arch: "amd64".into(),
        runtime_version: "rustc 1.90.0".into(),
        io: io.clone(),
    });
    Fixture {
        _temp: t,
        paths,
        owner,
        io,
    }
}
fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|v| v.to_string()).collect()
}
#[tokio::test]
async fn dry_run_auto_guard_and_decline_never_delegate_to_external_actor() {
    let f = fixture(vec!["MANY_AI_CLI_AUTO=1".into()], true, false);
    let mut output = vec![];
    assert!(
        f.owner
            .run(
                &args(&["synthetic bug"]),
                &mut Cursor::new(b"y\n"),
                &mut output,
                &mut vec![],
                &Cancellation::default()
            )
            .await
            .is_err()
    );
    f.owner
        .run(
            &args(&["--dry-run", "token=private bug"]),
            &mut Cursor::new(b""),
            &mut output,
            &mut vec![],
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert!(!String::from_utf8(output).unwrap().contains("token=private"));
    assert_eq!(f.io.calls.load(Ordering::SeqCst), 0);
    let f = fixture(vec![], true, false);
    let mut output = vec![];
    f.owner
        .run(
            &args(&["synthetic bug"]),
            &mut Cursor::new(b"n\n"),
            &mut output,
            &mut vec![],
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Issue creation cancelled.")
    );
    assert_eq!(f.io.calls.load(Ordering::SeqCst), 0);
    assert!(!f.paths.root().join("reports").exists());
}
#[tokio::test]
async fn explicit_confirmation_delegates_only_fixed_repo_web_action_with_scrubbed_args() {
    let f = fixture(vec![], true, false);
    let mut output = vec![];
    f.owner
        .run(
            &args(&["token=private bug"]),
            &mut Cursor::new(b"Y\n"),
            &mut output,
            &mut vec![],
            &Cancellation::default(),
        )
        .await
        .unwrap();
    let args = f.io.args.lock().unwrap();
    assert_eq!(
        &args[..5],
        &[
            "issue",
            "create",
            "--repo",
            "ishizakahiroshi/many-ai-cli",
            "--web"
        ]
    );
    assert!(!format!("{args:?}").contains("token=private"));
    assert_eq!(f.io.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn unavailable_gh_prints_fixed_url_failed_gh_saves_private_scrubbed_fallback() {
    for available in [false, true] {
        let f = fixture(vec![], available, true);
        let mut output = vec![];
        let result = f
            .owner
            .run(
                &args(&["token=private bug"]),
                &mut Cursor::new(b"y\n"),
                &mut output,
                &mut vec![],
                &Cancellation::default(),
            )
            .await;
        if available {
            assert!(result.is_err());
            let reports = Dir::open(&f.paths.root().join("reports")).unwrap();
            let entries = reports.entries().unwrap();
            assert_eq!(entries.len(), 1);
            assert!(
                !String::from_utf8(reports.read(&entries[0], 1024 * 1024).unwrap())
                    .unwrap()
                    .contains("token=private")
            );
        } else {
            assert!(result.is_ok());
            assert!(
                String::from_utf8(output)
                    .unwrap()
                    .contains("https://github.com/ishizakahiroshi/many-ai-cli/issues/new?")
            );
            assert!(!f.paths.root().join("reports").exists());
        }
    }
}
#[tokio::test]
async fn source_flag_stop_at_positional_and_symptom_prompt_are_preserved() {
    let f = fixture(vec![], true, false);
    let mut output = vec![];
    let mut prompt = vec![];
    f.owner
        .run(
            &args(&["--dry-run"]),
            &mut Cursor::new(b"synthetic symptom\n"),
            &mut output,
            &mut prompt,
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert!(String::from_utf8(prompt).unwrap().contains("症状を1行"));
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("synthetic symptom")
    );
    assert!(
        f.owner
            .run(
                &args(&["title", "--dry-run"]),
                &mut Cursor::new(b""),
                &mut vec![],
                &mut vec![],
                &Cancellation::default()
            )
            .await
            .is_err()
    );
    assert_eq!(f.io.calls.load(Ordering::SeqCst), 0);
}
