use super::*;
use crate::{config::Config, hub::task_owner::HubTaskOwner};
use std::sync::atomic::{AtomicUsize, Ordering};
#[derive(Default)]
struct FakeIo {
    downloads: AtomicUsize,
    starts: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    gate: std::sync::atomic::AtomicBool,
}
impl WhisperIo for FakeIo {
    fn download<'a>(
        &'a self,
        spec: Download,
        dir: Arc<Dir>,
        progress: Progress,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<(), WhisperError>> {
        Box::pin(async move {
            self.downloads.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            if self.gate.load(Ordering::SeqCst) {
                tokio::select! {_=cancel.cancelled()=>return Err(WhisperError::new(499,"cancelled","fixture cancelled")),_=self.release.notified()=>{}}
            }
            dir.create_new(&spec.file_name, b"fixture model", 0o600)?;
            progress(13, Some(13));
            Ok(())
        })
    }
    fn room(&self, _: &Dir, _: u64) -> Result<(), WhisperError> {
        Ok(())
    }
    fn start<'a>(
        &'a self,
        plan: ProcessPlan,
        _: Arc<Dir>,
        tasks: HubTaskHandle,
    ) -> CoreFuture<'a, Result<WhisperProcess, WhisperError>> {
        Box::pin(async move {
            assert_eq!(plan.timeout, Duration::ZERO);
            assert!(plan.args.iter().any(|arg| arg == "127.0.0.1"));
            self.starts.fetch_add(1, Ordering::SeqCst);
            let cancel = Cancellation::default();
            let held = cancel.clone();
            let (sender, done) = tokio::sync::watch::channel(None);
            let permit = tasks.effect_permit().unwrap();
            drop(permit.start(async move {
                held.cancelled().await;
                let _ = sender.send(Some(ProcessExit { failed: false }));
            }));
            Ok(WhisperProcess {
                cancellation: cancel,
                done,
            })
        })
    }
    fn ready<'a>(
        &'a self,
        _: u16,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, Result<(), WhisperError>> {
        Box::pin(async { Ok(()) })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    owner: HubTaskOwner,
    manager: Arc<WhisperManager>,
    io: Arc<FakeIo>,
}
impl Fixture {
    fn new(baked: bool) -> Self {
        Self::with_runtime_payload(baked, Vec::new())
    }
    fn with_runtime_payload(baked: bool, runtime_payload: Vec<(String, Vec<u8>)>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime");
        let installed = root.path().join("installed");
        std::fs::create_dir(&runtime).unwrap();
        std::fs::create_dir(&installed).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49998, &installed).unwrap();
        let config = Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap());
        let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
        let io = Arc::new(FakeIo::default());
        let mut environment = Vec::new();
        if baked {
            let path = runtime.join("whisper-server.exe");
            std::fs::write(&path, b"fixture server never executed").unwrap();
            environment.push(format!("MANY_AI_CLI_WHISPER_SERVER={}", path.display()));
        }
        let manager = WhisperManager::new(WhisperDependencies {
            paths,
            config,
            tasks: owner.handle(),
            environment,
            platform: "windows".into(),
            arch: "amd64".into(),
            io: io.clone(),
            warning: Arc::new(|_, _| {}),
            runtime_payload,
        })
        .unwrap();
        Self {
            _root: root,
            owner,
            manager,
            io,
        }
    }
}
#[tokio::test]
async fn prepared_runtime_is_copied_on_install_and_ensure_without_replacing_existing_files() {
    let payload: Vec<_> = crate::asset_contract::WINDOWS_RUNTIME_NAMES
        .iter()
        .map(|name| {
            (
                (*name).to_owned(),
                format!("synthetic runtime {name}").into_bytes(),
            )
        })
        .collect();
    let fixture = Fixture::with_runtime_payload(true, payload.clone());
    fixture.manager.install("small").unwrap();
    fixture.owner.drain_effects().await;
    assert!(fixture.manager.status().unwrap().installed);
    let bin = fixture.manager.base().join("bin");
    for (name, bytes) in &payload {
        assert_eq!(std::fs::read(bin.join(name)).unwrap(), *bytes);
    }
    std::fs::write(bin.join(&payload[0].0), b"existing runtime preserved").unwrap();
    std::fs::remove_file(bin.join(&payload[1].0)).unwrap();
    let config = fixture.manager.configuration().unwrap();
    fixture
        .manager
        .ensure(&config, &Cancellation::default())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(bin.join(&payload[0].0)).unwrap(),
        b"existing runtime preserved"
    );
    assert_eq!(
        std::fs::read(bin.join(&payload[1].0)).unwrap(),
        payload[1].1
    );
    assert_eq!(fixture.io.starts.load(Ordering::SeqCst), 1);
    fixture.manager.shutdown().await;
    fixture.owner.drain_effects().await;
}
#[tokio::test]
async fn install_owned_singleflight_publishes_config_then_start_reuses_same_process_and_uninstall_resets()
 {
    let fixture = Fixture::new(true);
    fixture.io.gate.store(true, Ordering::SeqCst);
    fixture.manager.install("bad-model-falls-back").unwrap();
    fixture.io.entered.notified().await;
    fixture.manager.install("tiny-q5_1").unwrap();
    assert_eq!(fixture.io.downloads.load(Ordering::SeqCst), 1);
    assert!(fixture.manager.status().unwrap().install.installing);
    fixture.io.release.notify_one();
    fixture.owner.drain_effects().await;
    let status = fixture.manager.status().unwrap();
    assert!(status.installed && status.managed);
    assert_eq!(status.model, "small");
    assert_eq!(status.install.phase, "done");
    let cfg = fixture.manager.configuration().unwrap();
    let cancel = Cancellation::default();
    let first = fixture.manager.ensure(&cfg, &cancel).await.unwrap();
    let second = fixture.manager.ensure(&cfg, &cancel).await.unwrap();
    assert_eq!(first.server_url, second.server_url);
    assert_eq!(fixture.io.starts.load(Ordering::SeqCst), 1);
    fixture.manager.uninstall().await.unwrap();
    fixture.owner.drain_effects().await;
    let cfg = fixture.manager.configuration().unwrap();
    assert!(!cfg.managed);
    assert_eq!(cfg.server_port, 0);
    assert!(cfg.server_url.is_empty());
    assert!(!fixture.manager.base().exists());
}
#[tokio::test]
async fn shutdown_cancels_install_and_joins_owned_producer() {
    let fixture = Fixture::new(true);
    fixture.io.gate.store(true, Ordering::SeqCst);
    fixture.manager.install("small").unwrap();
    fixture.io.entered.notified().await;
    assert_eq!(fixture.manager.uninstall().await.unwrap_err().status, 409);
    fixture.manager.shutdown().await;
    fixture.owner.drain_effects().await;
    assert!(!fixture.manager.status().unwrap().install.installing);
    assert!(fixture.manager.install("small").is_err());
}
#[tokio::test]
async fn unmanaged_ensure_never_runs_process_or_download_and_trial_native_is_closed() {
    let fixture = Fixture::new(false);
    let cfg = fixture.manager.configuration().unwrap();
    assert!(!cfg.managed);
    let actual = fixture
        .manager
        .ensure(&cfg, &Cancellation::default())
        .await
        .unwrap();
    assert_eq!(actual.server_url, cfg.server_url);
    assert_eq!(fixture.io.starts.load(Ordering::SeqCst), 0);
    let io = native::NativeWhisperIo {
        paths: fixture.manager.deps.paths.clone(),
    };
    let directory = Arc::new(fixture.manager.root.child_dir("test", true).unwrap());
    let result = io
        .download(
            Download {
                url: "https://example.invalid/file".into(),
                file_name: "file".into(),
                sha256: String::new(),
                maximum_bytes: 64,
            },
            directory,
            Arc::new(|_, _| {}),
            &Cancellation::default(),
        )
        .await;
    assert!(result.is_err());
}
#[test]
fn exact_manifest_and_hash_contract() {
    assert_eq!(manifest::WINDOWS.size_bytes, 4093849);
    assert_eq!(manifest::WINDOWS.sha256.len(), 64);
    assert!(manifest::binary("linux", "amd64").is_none());
    assert_eq!(manifest::model("").unwrap().id, "small");
    assert!(native::verify_hash(
        b"abc",
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    ));
    assert!(!native::verify_hash(
        b"abd",
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    ));
    let public = serde_json::to_value(manifest::MODELS).unwrap();
    assert!(public[0].get("url").is_none());
    assert_eq!(public[0]["hash_checked"], true);
}
#[test]
fn selected_tar_extraction_flattens_regular_files_and_checks_missing_required_files() {
    let root = tempfile::tempdir().unwrap();
    let dir = Arc::new(Dir::open_or_create_private(root.path()).unwrap());
    let archive_file = dir.open_write_or_create("input.tar.gz", 0o600).unwrap();
    let encoder = flate2::write::GzEncoder::new(archive_file, flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(4);
    header.set_mode(0o700);
    header.set_cksum();
    archive
        .append_data(&mut header, "release/bin/whisper-server", &b"fake"[..])
        .unwrap();
    archive.into_inner().unwrap().finish().unwrap();
    let binary = manifest::Binary {
        version: "test",
        url: "",
        sha256: "",
        size_bytes: 0,
        archive: "tar.gz",
        server_names: &["whisper-server"],
        keep: &["whisper-server"],
        runtime: "test",
    };
    let output = Arc::new(dir.child_dir("out", true).unwrap());
    archive::extract(
        dir.open_file("input.tar.gz", false).unwrap(),
        output.clone(),
        &binary,
        &Cancellation::default(),
    )
    .unwrap();
    assert!(output.open_file("whisper-server", false).is_ok());
    let missing = manifest::Binary {
        keep: &["missing"],
        ..binary
    };
    assert!(
        archive::extract(
            dir.open_file("input.tar.gz", false).unwrap(),
            output,
            &missing,
            &Cancellation::default()
        )
        .is_err()
    );
}

#[test]
fn zip_extraction_rejects_traversal_duplicates_and_size_bombs_before_writing() {
    use std::io::Write;
    for names in [
        vec!["../whisper-server.exe"],
        vec!["a/whisper-server.exe", "b/whisper-server.exe"],
    ] {
        let root = tempfile::tempdir().unwrap();
        let directory = Arc::new(Dir::open_or_create_private(root.path()).unwrap());
        let file = directory
            .open_write_or_create("fixture.zip", 0o600)
            .unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for name in names {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"synthetic").unwrap();
        }
        zip.finish().unwrap();
        let binary = manifest::Binary {
            keep: &["whisper-server.exe"],
            ..manifest::WINDOWS
        };
        assert!(
            archive::extract(
                directory.open_file("fixture.zip", false).unwrap(),
                Arc::new(directory.child_dir("out", true).unwrap()),
                &binary,
                &Cancellation::default()
            )
            .is_err()
        );
        assert!(
            !root
                .path()
                .parent()
                .unwrap()
                .join("whisper-server.exe")
                .exists()
        );
    }
    let root = tempfile::tempdir().unwrap();
    let directory = Dir::open_or_create_private(root.path()).unwrap();
    let mut input = &b"x"[..];
    let mut total = 0;
    // Do not allocate/decompress the advertised payload.
    assert!(super::archive::test_copy_bound(&mut input, &directory, &mut total).is_err());
    assert!(directory.open_file("bomb", false).is_err());
}
#[tokio::test]
async fn native_readiness_observes_owned_loopback_and_cancel_without_starting_provider() {
    let fixture = Fixture::new(false);
    let io = native::NativeWhisperIo {
        paths: fixture.manager.deps.paths.clone(),
    };
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    io.ready(port, &Cancellation::default()).await.unwrap();
    drop(listener);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(io.ready(port, &cancel).await.is_err());
}
