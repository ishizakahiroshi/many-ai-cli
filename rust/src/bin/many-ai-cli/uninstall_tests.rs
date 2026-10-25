//! The binary's actual uninstall composition with synthetic Hub and platform IO.
//! No product process, network request or native deletion is executed here.
use super::*;
use many_ai_cli::{
    application::platform_cli::{Locations, PlatformCli, PlatformIo},
    config::{Resource, RuntimePaths},
    proto::core::CoreFuture,
};
use std::{
    io::{self, BufRead, Read, Write},
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

struct State {
    events: Mutex<Vec<&'static str>>,
    running: AtomicBool,
}
struct SyntheticHub {
    state: Arc<State>,
    config: Arc<ConfigStore>,
}
impl Drop for SyntheticHub {
    fn drop(&mut self) {
        self.state.events.lock().unwrap().push("context-drop");
    }
}
impl UninstallHub for SyntheticHub {
    fn running_port(&self) -> CoreFuture<'_, io::Result<Option<u16>>> {
        Box::pin(async move {
            self.config.snapshot().unwrap();
            self.state.events.lock().unwrap().push("lookup");
            Ok(self.state.running.load(Ordering::SeqCst).then_some(49352))
        })
    }
    fn stop<'a>(&'a self, _: &'a Cancellation) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            self.state.events.lock().unwrap().push("stop");
            assert!(self.state.running.swap(false, Ordering::SeqCst));
            Ok(())
        })
    }
}
struct RecordedPlatform {
    state: Arc<State>,
    config: Weak<ConfigStore>,
}
impl PlatformIo for RecordedPlatform {
    fn prepare_directory(&self, _: &Path) -> io::Result<()> {
        panic!("uninstall must not prepare directories")
    }
    fn write(&self, _: &Path, _: &[u8]) -> io::Result<()> {
        panic!("uninstall must not write setup files")
    }
    fn make_executable(&self, _: &Path) -> io::Result<()> {
        panic!("uninstall must not change executable permissions")
    }
    fn exists(&self, _: &Path) -> bool {
        panic!("uninstall must not perform setup discovery")
    }
    fn remove_file(&self, _: &Path) -> io::Result<()> {
        assert!(self.config.upgrade().is_none());
        self.state.events.lock().unwrap().push("remove-shortcut");
        Ok(())
    }
    fn remove_data(&self, _: &Path) -> io::Result<bool> {
        assert!(!self.state.running.load(Ordering::SeqCst));
        assert!(
            self.config.upgrade().is_none(),
            "context must release the actual config owner first"
        );
        self.state.events.lock().unwrap().push("remove-data");
        Ok(true)
    }
    fn shortcut<'a>(
        &'a self,
        _: &'a Path,
        _: &'a Path,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<()>> {
        panic!("uninstall must not create shortcuts")
    }
    fn purge<'a>(&'a self, _: &'a Path, _: &'a Cancellation) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            self.state.events.lock().unwrap().push("purge");
            Ok(())
        })
    }
}
struct PromptWriter {
    state: Arc<State>,
    bytes: Vec<u8>,
    recorded: bool,
}
impl Write for PromptWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        assert!(String::from_utf8_lossy(&self.bytes).contains("続行しますか? [y/N]: "));
        assert!(!self.recorded, "confirmation is prompted once");
        self.recorded = true;
        self.state.events.lock().unwrap().push("prompt");
        Ok(())
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    owner: PlatformCli,
    context: Option<SyntheticHub>,
    state: Arc<State>,
    out: PromptWriter,
    config_path: std::path::PathBuf,
    config_bytes: Vec<u8>,
    sentinel: std::path::PathBuf,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49352, installed.path()).unwrap();
    let config = Arc::new(
        ConfigStore::load_or_create(paths.clone(), || Ok("synthetic-uninstall-token".into()))
            .unwrap(),
    );
    let config_path = paths.resource(Resource::Config);
    let config_bytes = std::fs::read(&config_path).unwrap();
    let sentinel = root.path().join("preserved-sentinel.txt");
    std::fs::write(&sentinel, b"synthetic data must be preserved\n").unwrap();
    let state = Arc::new(State {
        events: Mutex::new(Vec::new()),
        running: AtomicBool::new(true),
    });
    let owner = PlatformCli {
        paths,
        locations: Locations {
            config: root.path().to_path_buf(),
            data: root.path().join("data"),
            desktop: root.path().join("desktop"),
            startup: Some(root.path().join("startup")),
            executable: root.path().join("synthetic-executable"),
        },
        io: Arc::new(RecordedPlatform {
            state: state.clone(),
            config: Arc::downgrade(&config),
        }),
    };
    Fixture {
        _root: root,
        _installed: installed,
        owner,
        context: Some(SyntheticHub {
            state: state.clone(),
            config,
        }),
        out: PromptWriter {
            state: state.clone(),
            bytes: Vec::new(),
            recorded: false,
        },
        state,
        config_path,
        config_bytes,
        sentinel,
    }
}
fn assert_preserved(f: &Fixture) {
    assert_eq!(std::fs::read(&f.config_path).unwrap(), f.config_bytes);
    assert_eq!(
        std::fs::read(&f.sentinel).unwrap(),
        b"synthetic data must be preserved\n"
    );
}

#[tokio::test]
async fn actual_uninstall_entry_decline_and_eof_preserve_the_running_hub_and_data() {
    for answer in ["N\n", "no\n", "", "\n"] {
        let mut f = fixture();
        uninstall_entry(
            &f.owner,
            f.context.take().unwrap(),
            true,
            &mut answer.as_bytes(),
            &mut f.out,
            &Cancellation::default(),
        )
        .await
        .unwrap();
        assert_eq!(*f.state.events.lock().unwrap(), ["prompt", "context-drop"]);
        assert!(f.state.running.load(Ordering::SeqCst));
        assert!(String::from_utf8_lossy(&f.out.bytes).contains("キャンセルしました"));
        assert_preserved(&f);
    }
}

struct FailedRead;
impl Read for FailedRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::BrokenPipe.into())
    }
}
impl BufRead for FailedRead {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(io::ErrorKind::BrokenPipe.into())
    }
    fn consume(&mut self, _: usize) {}
    fn read_line(&mut self, line: &mut String) -> io::Result<usize> {
        line.push_str("yes");
        Err(io::ErrorKind::BrokenPipe.into())
    }
}
#[tokio::test]
async fn actual_uninstall_entry_failed_consent_read_preserves_the_running_hub_and_data() {
    let mut f = fixture();
    let error = uninstall_entry(
        &f.owner,
        f.context.take().unwrap(),
        true,
        &mut FailedRead,
        &mut f.out,
        &Cancellation::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(*f.state.events.lock().unwrap(), ["prompt", "context-drop"]);
    assert!(f.state.running.load(Ordering::SeqCst));
    assert_preserved(&f);
}

#[tokio::test]
async fn actual_uninstall_entry_yes_stops_and_releases_config_before_ordered_fake_removal() {
    for purge in [false, true] {
        let mut f = fixture();
        uninstall_entry(
            &f.owner,
            f.context.take().unwrap(),
            purge,
            &mut &b"yes\n"[..],
            &mut f.out,
            &Cancellation::default(),
        )
        .await
        .unwrap();
        let mut expected = vec![
            "prompt",
            "lookup",
            "stop",
            "context-drop",
            "remove-data",
            "remove-shortcut",
            "remove-shortcut",
        ];
        if purge {
            expected.push("purge");
        }
        assert_eq!(*f.state.events.lock().unwrap(), expected);
        assert!(!f.state.running.load(Ordering::SeqCst));
        assert!(String::from_utf8_lossy(&f.out.bytes).contains("アンインストール完了"));
        // Platform deletion is a recorder, so even the positive test cannot
        // remove temporary data or invoke a native uninstall/reaper.
        assert_preserved(&f);
    }
}
