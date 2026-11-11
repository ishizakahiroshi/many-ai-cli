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
    /// Another process holds the server's port (U31).
    foreign: std::sync::atomic::AtomicBool,
    /// Bytes served for the release archive download (U35); others get a model.
    archive: std::sync::Mutex<Option<Vec<u8>>>,
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
            let archive = self.archive.lock().unwrap().clone();
            match archive {
                Some(bytes) if spec.file_name.ends_with(".zip") => {
                    dir.create_new(&spec.file_name, &bytes, 0o600)?
                }
                _ => dir.create_new(&spec.file_name, b"fixture model", 0o600)?,
            }
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
                pid: 0,
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
    fn serves(&self, _: u16, process: &WhisperProcess) -> bool {
        process.done.borrow().is_none() && !self.foreign.load(Ordering::SeqCst)
    }
}
struct Fixture {
    owner: HubTaskOwner,
    manager: Arc<WhisperManager>,
    io: Arc<FakeIo>,
    // Last: fields drop in this order, and on Windows the folder cannot be
    // removed while a field above still holds a file inside it.
    _root: tempfile::TempDir,
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
async fn prepared_runtime_is_copied_on_install_and_changed_files_are_replaced_before_a_start() {
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
    // A copy from another build is replaced before the server starts (U35).
    std::fs::write(bin.join(&payload[0].0), b"runtime from an older build").unwrap();
    std::fs::remove_file(bin.join(&payload[1].0)).unwrap();
    let config = fixture.manager.configuration().unwrap();
    fixture
        .manager
        .ensure(&config, &Cancellation::default())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(bin.join(&payload[0].0)).unwrap(),
        payload[0].1
    );
    assert_eq!(
        std::fs::read(bin.join(&payload[1].0)).unwrap(),
        payload[1].1
    );
    assert_eq!(fixture.io.starts.load(Ordering::SeqCst), 1);
    // While the server runs and holds its files, they are left alone.
    std::fs::write(bin.join(&payload[0].0), b"held by the running server").unwrap();
    fixture
        .manager
        .ensure(&config, &Cancellation::default())
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(bin.join(&payload[0].0)).unwrap(),
        b"held by the running server"
    );
    assert_eq!(fixture.io.starts.load(Ordering::SeqCst), 1);
    // A baked server has no receipt: its version is not claimed.
    let status = fixture.manager.status().unwrap();
    assert_eq!(status.binary_version, "");
    assert_eq!(status.available_version, manifest::RELEASE);
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
#[tokio::test]
async fn audit_u31_a_port_held_by_another_process_is_neither_used_nor_saved() {
    let fixture = Fixture::new(true);
    fixture.manager.install("small").unwrap();
    fixture.owner.drain_effects().await;
    let cfg = fixture.manager.configuration().unwrap();
    let cancel = Cancellation::default();
    fixture.io.foreign.store(true, Ordering::SeqCst);
    let error = fixture.manager.ensure(&cfg, &cancel).await.unwrap_err();
    assert_eq!(error.status, 502);
    assert_eq!(error.code, "whisper_start_failed");
    let saved = fixture.manager.configuration().unwrap();
    assert_eq!(
        (saved.server_url.as_str(), saved.server_port),
        (cfg.server_url.as_str(), cfg.server_port)
    );
    // A server that owns its port is used, and replaced once it no longer does.
    fixture.io.foreign.store(false, Ordering::SeqCst);
    assert!(
        !fixture
            .manager
            .ensure(&cfg, &cancel)
            .await
            .unwrap()
            .server_url
            .is_empty()
    );
    assert_eq!(fixture.io.starts.load(Ordering::SeqCst), 2);
    fixture.io.foreign.store(true, Ordering::SeqCst);
    assert!(fixture.manager.ensure(&cfg, &cancel).await.is_err());
    assert_eq!(fixture.io.starts.load(Ordering::SeqCst), 3);
    fixture.manager.shutdown().await;
    fixture.owner.drain_effects().await;
}
#[cfg(any(windows, target_os = "linux"))]
#[tokio::test]
async fn audit_u31_native_server_check_needs_the_live_process_that_owns_the_port() {
    use crate::process::wrapper_startup::recovery_identity::test_support::Bystander;
    let fixture = Fixture::new(false);
    let io = native::NativeWhisperIo {
        paths: fixture.manager.deps.paths.clone(),
    };
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_running, done) = tokio::sync::watch::channel(None);
    let process = |pid| WhisperProcess {
        cancellation: Cancellation::default(),
        done: done.clone(),
        pid,
    };
    assert!(io.serves(port, &process(std::process::id())));
    let bystander = Bystander::spawn();
    assert!(!io.serves(port, &process(bystander.pid())));
    let (_ended, exited) = tokio::sync::watch::channel(Some(ProcessExit { failed: false }));
    let ended = WhisperProcess {
        cancellation: Cancellation::default(),
        done: exited,
        pid: std::process::id(),
    };
    assert!(!io.serves(port, &ended));
    drop(listener);
    assert!(!io.serves(port, &process(std::process::id())));
}

// All archive fixtures are inert bytes: no runtime, model, or provider executes.
fn synthetic_whisper_archive(kind: &str, entries: &[(&str, &[u8])]) -> std::fs::File {
    use std::io::{Seek, Write};
    let file = tempfile::tempfile().unwrap();
    let mut file = match kind {
        "zip" => {
            let mut archive = zip::ZipWriter::new(file);
            for (name, bytes) in entries {
                archive
                    .start_file(
                        *name,
                        zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Stored),
                    )
                    .unwrap();
                archive.write_all(bytes).unwrap();
            }
            archive.finish().unwrap()
        }
        "tar.gz" => {
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            for (name, bytes) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o700);
                header.set_cksum();
                archive.append_data(&mut header, *name, *bytes).unwrap();
            }
            archive.into_inner().unwrap().finish().unwrap()
        }
        _ => panic!("unsupported fixture format"),
    };
    file.rewind().unwrap();
    file
}

fn synthetic_whisper_manifest(kind: &'static str) -> manifest::Binary {
    manifest::Binary {
        archive: kind,
        keep: &["whisper-server.exe", "first.dll", "second.dll"],
        ..manifest::WINDOWS
    }
}

const SYNTHETIC_WHISPER_FILES: &[(&str, &[u8])] = &[
    ("release/whisper-server.exe", b"synthetic server"),
    ("release/first.dll", b"synthetic first dependency"),
    ("release/second.dll", b"synthetic second dependency"),
];

#[test]
fn archive_missing_or_duplicate_members_never_publish_partial_installations() {
    for kind in ["zip", "tar.gz"] {
        for entries in [
            SYNTHETIC_WHISPER_FILES[..2].to_vec(),
            [
                SYNTHETIC_WHISPER_FILES,
                &[("other/whisper-server.exe", SYNTHETIC_WHISPER_FILES[0].1)],
            ]
            .concat(),
        ] {
            let root = tempfile::tempdir().unwrap();
            let directory = Arc::new(Dir::open(root.path()).unwrap());
            directory
                .create_new("unrelated", b"preserve", 0o600)
                .unwrap();
            assert!(
                archive::extract(
                    synthetic_whisper_archive(kind, &entries),
                    directory.clone(),
                    &synthetic_whisper_manifest(kind),
                    &Cancellation::default(),
                )
                .is_err(),
                "{kind}",
            );
            assert_eq!(directory.entries().unwrap(), ["unrelated"], "{kind}");
            assert_eq!(directory.read("unrelated", 32).unwrap(), b"preserve");
        }
    }
}

#[test]
fn archive_integrity_errors_never_publish_partial_installations() {
    use std::io::{Read, Seek, Write};
    for kind in ["zip", "tar.gz"] {
        let mut file = synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES);
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        let position = if kind == "zip" {
            let payload = b"synthetic second dependency";
            bytes
                .windows(payload.len())
                .position(|window| window == payload)
                .unwrap()
        } else {
            // The gzip CRC is after tar's end-of-archive blocks. Reading only tar
            // entries would miss this failure after all selected files copied.
            bytes.len() - 8
        };
        bytes[position] ^= 1;
        file.rewind().unwrap();
        file.write_all(&bytes).unwrap();
        file.rewind().unwrap();
        let root = tempfile::tempdir().unwrap();
        let directory = Arc::new(Dir::open(root.path()).unwrap());
        assert!(
            archive::extract(
                file,
                directory.clone(),
                &synthetic_whisper_manifest(kind),
                &Cancellation::default(),
            )
            .is_err(),
            "{kind}",
        );
        assert!(directory.entries().unwrap().is_empty(), "{kind}");
    }
}

#[test]
fn archive_cancellation_never_publishes_even_empty_selected_files() {
    for kind in ["zip", "tar.gz"] {
        for payload in [&b"synthetic server"[..], &b""[..]] {
            let root = tempfile::tempdir().unwrap();
            let directory = Arc::new(Dir::open(root.path()).unwrap());
            let cancellation = Cancellation::default();
            cancellation.cancel();
            let binary = manifest::Binary {
                keep: &["whisper-server.exe"],
                ..synthetic_whisper_manifest(kind)
            };
            let result = archive::extract(
                synthetic_whisper_archive(kind, &[("whisper-server.exe", payload)]),
                directory.clone(),
                &binary,
                &cancellation,
            );
            assert!(result.is_err(), "{kind}");
            assert!(directory.entries().unwrap().is_empty(), "{kind}");
        }
    }
}

#[test]
fn archive_publication_preserves_existing_executable_and_rolls_back_new_files() {
    for kind in ["zip", "tar.gz"] {
        let existing = "whisper-server.exe";
        let root = tempfile::tempdir().unwrap();
        let directory = Arc::new(Dir::open(root.path()).unwrap());
        directory
            .create_new(existing, b"existing bytes", 0o600)
            .unwrap();
        assert!(
            archive::extract(
                synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES),
                directory.clone(),
                &synthetic_whisper_manifest(kind),
                &Cancellation::default(),
            )
            .is_err(),
            "{kind} {existing}",
        );
        assert_eq!(
            directory.entries().unwrap(),
            [existing],
            "{kind} {existing}"
        );
        assert_eq!(directory.read(existing, 64).unwrap(), b"existing bytes");
        let root = tempfile::tempdir().unwrap();
        let directory = Arc::new(Dir::open(root.path()).unwrap());
        let existing = directory.child_dir("second.dll", true).unwrap();
        existing
            .create_new("preserve", b"existing child", 0o600)
            .unwrap();
        assert!(
            archive::extract(
                synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES),
                directory.clone(),
                &synthetic_whisper_manifest(kind),
                &Cancellation::default(),
            )
            .is_err(),
            "{kind}",
        );
        assert_eq!(directory.entries().unwrap(), ["second.dll"], "{kind}");
        assert_eq!(existing.read("preserve", 64).unwrap(), b"existing child");
    }
}

#[test]
fn audit_u35_an_update_replaces_the_server_and_dependencies_or_keeps_them_all() {
    for kind in ["zip", "tar.gz"] {
        let root = tempfile::tempdir().unwrap();
        let directory = Arc::new(Dir::open(root.path()).unwrap());
        for name in ["whisper-server.exe", "first.dll", "second.dll"] {
            directory.create_new(name, b"old release", 0o600).unwrap();
        }
        // A broken archive changes nothing.
        assert!(
            archive::extract_replacing(
                synthetic_whisper_archive(kind, &SYNTHETIC_WHISPER_FILES[..2]),
                directory.clone(),
                &synthetic_whisper_manifest(kind),
                &Cancellation::default(),
            )
            .is_err()
        );
        let mut names = directory.entries().unwrap();
        names.sort();
        assert_eq!(
            names,
            ["first.dll", "second.dll", "whisper-server.exe"],
            "{kind}"
        );
        for name in &names {
            assert_eq!(directory.read(name, 64).unwrap(), b"old release", "{kind}");
        }
        archive::extract_replacing(
            synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES),
            directory.clone(),
            &synthetic_whisper_manifest(kind),
            &Cancellation::default(),
        )
        .unwrap();
        let mut names = directory.entries().unwrap();
        names.sort();
        assert_eq!(
            names,
            ["first.dll", "second.dll", "whisper-server.exe"],
            "{kind}"
        );
        assert_eq!(
            directory.read("whisper-server.exe", 64).unwrap(),
            b"synthetic server"
        );
        assert_eq!(
            directory.read("second.dll", 64).unwrap(),
            b"synthetic second dependency"
        );
    }
}

#[tokio::test]
async fn audit_u35_install_replaces_a_managed_server_without_a_current_receipt() {
    use std::io::Read;
    let fixture = Fixture::new(false);
    let mut bytes = Vec::new();
    synthetic_whisper_archive(
        "zip",
        &manifest::WINDOWS
            .keep
            .iter()
            .map(|name| (*name, b"new release".as_slice()))
            .collect::<Vec<_>>(),
    )
    .read_to_end(&mut bytes)
    .unwrap();
    *fixture.io.archive.lock().unwrap() = Some(bytes);
    // An installation from an earlier version: files, no receipt.
    let bin = fixture.manager.base().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in manifest::WINDOWS.keep {
        std::fs::write(bin.join(name), b"old release").unwrap();
    }
    assert_eq!(fixture.manager.status().unwrap().binary_version, "");
    fixture.manager.install("small").unwrap();
    fixture.owner.drain_effects().await;
    let status = fixture.manager.status().unwrap();
    assert_eq!(status.install.error, "");
    assert_eq!(status.binary_version, manifest::RELEASE);
    for name in manifest::WINDOWS.keep {
        assert_eq!(
            std::fs::read(bin.join(name)).unwrap(),
            b"new release",
            "{name}"
        );
    }
    // Release archive and model: one download each.
    assert_eq!(fixture.io.downloads.load(Ordering::SeqCst), 2);
    // With a current receipt nothing is downloaded again.
    fixture.manager.install("small").unwrap();
    fixture.owner.drain_effects().await;
    assert_eq!(fixture.io.downloads.load(Ordering::SeqCst), 2);
    fixture.manager.shutdown().await;
    fixture.owner.drain_effects().await;
}

#[test]
fn archive_success_publishes_complete_files_without_staging_residue() {
    for kind in ["zip", "tar.gz"] {
        let root = tempfile::tempdir().unwrap();
        let directory = Arc::new(Dir::open(root.path()).unwrap());
        archive::extract(
            synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES),
            directory.clone(),
            &synthetic_whisper_manifest(kind),
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(
            directory.entries().unwrap(),
            ["first.dll", "second.dll", "whisper-server.exe"],
            "{kind}",
        );
        for (name, bytes) in SYNTHETIC_WHISPER_FILES {
            let leaf = name.rsplit('/').next().unwrap();
            assert_eq!(directory.read(leaf, 128).unwrap(), *bytes, "{kind} {leaf}");
        }
    }
}

#[cfg(unix)]
#[test]
fn archive_publication_stays_with_the_held_directory_after_rename() {
    for kind in ["zip", "tar.gz"] {
        let root = tempfile::tempdir().unwrap();
        let parent = Dir::open(root.path()).unwrap();
        let directory = Arc::new(parent.child_dir("out", true).unwrap());
        std::fs::rename(root.path().join("out"), root.path().join("moved")).unwrap();
        let replacement = parent.child_dir("out", true).unwrap();
        replacement
            .create_new("preserve", b"replacement", 0o600)
            .unwrap();
        archive::extract(
            synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES),
            directory,
            &synthetic_whisper_manifest(kind),
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(replacement.entries().unwrap(), ["preserve"]);
        let moved = parent.child_dir("moved", false).unwrap();
        assert_eq!(
            moved.entries().unwrap(),
            ["first.dll", "second.dll", "whisper-server.exe"],
            "{kind}",
        );
    }
}

#[test]
fn archive_truncation_never_publishes_partial_installations() {
    for kind in ["zip", "tar.gz"] {
        let file = synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES);
        file.set_len(file.metadata().unwrap().len() - 8).unwrap();
        let root = tempfile::tempdir().unwrap();
        let directory = Arc::new(Dir::open(root.path()).unwrap());
        assert!(
            archive::extract(
                file,
                directory.clone(),
                &synthetic_whisper_manifest(kind),
                &Cancellation::default(),
            )
            .is_err(),
            "{kind}",
        );
        assert!(directory.entries().unwrap().is_empty(), "{kind}");
    }
}

#[test]
fn archive_recovers_missing_executable_by_replacing_old_dependencies() {
    // Fixed Go extraction overwrites DLLs when reinstalling a missing server.
    // An existing dependency must no longer turn this recovery into a collision.
    for kind in ["zip", "tar.gz"] {
        for existing_names in [&["second.dll"][..], &["first.dll", "second.dll"][..]] {
            let root = tempfile::tempdir().unwrap();
            let directory = Arc::new(Dir::open(root.path()).unwrap());
            for name in existing_names {
                directory
                    .create_new(name, b"old dependency", 0o600)
                    .unwrap();
            }
            directory
                .create_new("unrelated", b"preserve", 0o600)
                .unwrap();
            archive::extract(
                synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES),
                directory.clone(),
                &synthetic_whisper_manifest(kind),
                &Cancellation::default(),
            )
            .unwrap();
            assert_eq!(
                directory.entries().unwrap(),
                ["first.dll", "second.dll", "unrelated", "whisper-server.exe"],
                "{kind}",
            );
            for (name, bytes) in SYNTHETIC_WHISPER_FILES {
                let leaf = name.rsplit('/').next().unwrap();
                assert_eq!(directory.read(leaf, 128).unwrap(), *bytes, "{kind} {leaf}");
            }
            assert_eq!(directory.read("unrelated", 32).unwrap(), b"preserve");
        }
    }
}

#[test]
fn archive_failed_recovery_restores_old_dependencies() {
    for kind in ["zip", "tar.gz"] {
        let root = tempfile::tempdir().unwrap();
        let directory = Arc::new(Dir::open(root.path()).unwrap());
        directory
            .create_new("first.dll", b"old dependency", 0o600)
            .unwrap();
        let blocker = directory.child_dir("second.dll", true).unwrap();
        blocker
            .create_new("preserve", b"existing child", 0o600)
            .unwrap();
        assert!(
            archive::extract(
                synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES),
                directory.clone(),
                &synthetic_whisper_manifest(kind),
                &Cancellation::default(),
            )
            .is_err(),
            "{kind}"
        );
        assert_eq!(
            directory.entries().unwrap(),
            ["first.dll", "second.dll"],
            "{kind}"
        );
        assert_eq!(directory.read("first.dll", 64).unwrap(), b"old dependency");
        assert_eq!(blocker.read("preserve", 64).unwrap(), b"existing child");
    }
}

#[test]
fn archive_cancellation_during_copy_removes_partial_staging_and_preserves_old_files() {
    struct CancelDuringRead {
        source: std::io::Cursor<Vec<u8>>,
        directory: Arc<Dir>,
        cancellation: Cancellation,
        reads: usize,
    }
    impl std::io::Read for CancelDuringRead {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.reads += 1;
            if self.reads == 2 {
                // The first chunk is already written, so this is beyond the
                // entry guard and strictly inside an unfinished copy.
                let staged: Vec<_> = self
                    .directory
                    .entries()
                    .unwrap()
                    .into_iter()
                    .filter(|name| name.starts_with(".whisper-extract-"))
                    .collect();
                assert_eq!(staged.len(), 1);
                let written = self.directory.metadata(&staged[0]).unwrap().len();
                assert!(written > 0 && written < self.source.get_ref().len() as u64);
                assert!(
                    self.directory
                        .open_file("whisper-server.exe", false)
                        .is_err()
                );
                self.cancellation.cancel();
            }
            std::io::Read::read(&mut self.source, buffer)
        }
    }
    let root = tempfile::tempdir().unwrap();
    let directory = Arc::new(Dir::open(root.path()).unwrap());
    directory
        .create_new("first.dll", b"old dependency", 0o600)
        .unwrap();
    directory
        .create_new("unrelated", b"preserve", 0o600)
        .unwrap();
    let cancellation = Cancellation::default();
    let size = 3 * 64 * 1024;
    let mut reader = CancelDuringRead {
        source: std::io::Cursor::new(vec![b'x'; size]),
        directory: directory.clone(),
        cancellation: cancellation.clone(),
        reads: 0,
    };
    let error =
        archive::test_copy_with_reader(&mut reader, directory.clone(), size as u64, &cancellation)
            .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert_eq!(error.status, 499);
    assert_eq!(reader.reads, 2);
    assert_eq!(directory.entries().unwrap(), ["first.dll", "unrelated"]);
    assert_eq!(directory.read("first.dll", 64).unwrap(), b"old dependency");
    assert_eq!(directory.read("unrelated", 32).unwrap(), b"preserve");
}

#[test]
fn archive_cancellation_between_renames_rolls_back_new_and_replaced_dependencies() {
    for kind in ["zip", "tar.gz"] {
        for recover in [false, true] {
            for stop_after in ["first.dll", "second.dll"] {
                let root = tempfile::tempdir().unwrap();
                let directory = Arc::new(Dir::open(root.path()).unwrap());
                directory
                    .create_new("unrelated", b"preserve", 0o600)
                    .unwrap();
                if recover {
                    for name in ["first.dll", "second.dll"] {
                        directory
                            .create_new(name, b"old dependency", 0o600)
                            .unwrap();
                    }
                }
                let cancellation = Cancellation::default();
                let cancel_after_rename = cancellation.clone();
                let observed = Arc::new(AtomicUsize::new(0));
                let observed_in_callback = observed.clone();
                let held = directory.clone();
                let error = archive::test_extract_after_publication(
                    synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES),
                    directory.clone(),
                    &synthetic_whisper_manifest(kind),
                    &cancellation,
                    move |name| {
                        observed_in_callback.fetch_add(1, Ordering::SeqCst);
                        assert!(held.open_file("whisper-server.exe", false).is_err());
                        let (_, expected) = SYNTHETIC_WHISPER_FILES
                            .iter()
                            .find(|(path, _)| path.rsplit('/').next() == Some(name))
                            .unwrap();
                        assert_eq!(held.read(name, 128).unwrap(), *expected);
                        if name == stop_after {
                            cancel_after_rename.cancel();
                        }
                    },
                )
                .unwrap_err();
                assert_eq!(error.code, "cancelled", "{kind} {recover} {stop_after}");
                assert_eq!(error.status, 499);
                assert_eq!(
                    observed.load(Ordering::SeqCst),
                    if stop_after == "first.dll" { 1 } else { 2 }
                );
                assert!(cancellation.is_cancelled());
                if recover {
                    assert_eq!(
                        directory.entries().unwrap(),
                        ["first.dll", "second.dll", "unrelated"]
                    );
                    for name in ["first.dll", "second.dll"] {
                        assert_eq!(directory.read(name, 64).unwrap(), b"old dependency");
                    }
                } else {
                    assert_eq!(directory.entries().unwrap(), ["unrelated"]);
                }
                assert_eq!(directory.read("unrelated", 32).unwrap(), b"preserve");
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn archive_recovery_rejects_symlink_dependencies_without_touching_target() {
    for kind in ["zip", "tar.gz"] {
        let root = tempfile::tempdir().unwrap();
        let parent = Dir::open(root.path()).unwrap();
        let directory = Arc::new(parent.child_dir("out", true).unwrap());
        parent
            .create_new("outside", b"outside bytes", 0o600)
            .unwrap();
        directory
            .create_new("first.dll", b"old dependency", 0o600)
            .unwrap();
        std::os::unix::fs::symlink("../outside", root.path().join("out/second.dll")).unwrap();
        assert!(
            archive::extract(
                synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES),
                directory.clone(),
                &synthetic_whisper_manifest(kind),
                &Cancellation::default(),
            )
            .is_err(),
            "{kind}"
        );
        assert_eq!(directory.entries().unwrap(), ["first.dll", "second.dll"]);
        assert_eq!(directory.read("first.dll", 64).unwrap(), b"old dependency");
        assert_eq!(parent.read("outside", 64).unwrap(), b"outside bytes");
        assert_eq!(
            std::fs::read_link(root.path().join("out/second.dll")).unwrap(),
            PathBuf::from("../outside")
        );
    }
}

#[test]
fn archive_obstructed_rollback_retains_the_old_backup() {
    for kind in ["zip", "tar.gz"] {
        let root = tempfile::tempdir().unwrap();
        let directory = Arc::new(Dir::open(root.path()).unwrap());
        directory
            .create_new("first.dll", b"old dependency", 0o600)
            .unwrap();
        let cancellation = Cancellation::default();
        let cancel_after_rename = cancellation.clone();
        let held = directory.clone();
        let error = archive::test_extract_after_publication(
            synthetic_whisper_archive(kind, SYNTHETIC_WHISPER_FILES),
            directory.clone(),
            &synthetic_whisper_manifest(kind),
            &cancellation,
            move |name| {
                assert_eq!(name, "first.dll");
                assert_eq!(held.read(name, 128).unwrap(), SYNTHETIC_WHISPER_FILES[1].1);
                // Simulate another writer obstructing rollback after publication.
                // Restoration must never delete the only retained old bytes.
                held.remove_file(name).unwrap();
                held.child_dir(name, true).unwrap();
                cancel_after_rename.cancel();
            },
        )
        .unwrap_err();
        assert_eq!(error.code, "cancelled", "{kind}");
        let entries = directory.entries().unwrap();
        let backups: Vec<_> = entries
            .iter()
            .filter(|name| name.starts_with(".whisper-backup-"))
            .collect();
        assert_eq!(backups.len(), 1, "{kind}");
        assert_eq!(directory.read(backups[0], 64).unwrap(), b"old dependency");
        assert_eq!(entries.len(), 2, "{kind}");
        assert!(entries.iter().any(|name| name == "first.dll"));
        assert!(
            directory
                .child_dir("first.dll", false)
                .unwrap()
                .entries()
                .unwrap()
                .is_empty()
        );
    }
}
