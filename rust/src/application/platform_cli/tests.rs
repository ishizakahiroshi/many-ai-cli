use super::*;
use std::sync::Mutex;
#[derive(Default)]
struct Fake {
    events: Mutex<Vec<String>>,
}
impl PlatformIo for Fake {
    fn make_executable(&self, path: &Path) -> io::Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("chmod:{}", path.display()));
        Ok(())
    }
    fn prepare_directory(&self, p: &Path) -> io::Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("mkdir:{}", p.display()));
        Ok(())
    }
    fn write(&self, p: &Path, b: &[u8]) -> io::Result<()> {
        self.events.lock().unwrap().push(format!(
            "write:{}:{}",
            p.display(),
            String::from_utf8_lossy(b)
        ));
        Ok(())
    }
    fn exists(&self, p: &Path) -> bool {
        p.ends_with("Many AI Hub Start.lnk")
    }
    fn remove_file(&self, p: &Path) -> io::Result<()> {
        self.events
            .lock()
            .unwrap()
            .push(format!("remove:{}", p.display()));
        Ok(())
    }
    fn remove_data(&self, p: &Path) -> io::Result<bool> {
        self.events
            .lock()
            .unwrap()
            .push(format!("tree:{}", p.display()));
        Ok(true)
    }
    fn shortcut<'a>(
        &'a self,
        p: &'a Path,
        _: &'a Path,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push(format!("shortcut:{}", p.display()));
            Ok(())
        })
    }
    fn purge<'a>(&'a self, p: &'a Path, _: &'a Cancellation) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push(format!("purge:{}", p.display()));
            Ok(())
        })
    }
}
fn fixture() -> (tempfile::TempDir, PlatformCli, Arc<Fake>) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let io = Arc::new(Fake::default());
    let paths =
        RuntimePaths::trial(root, 49351, &root.join("../installed-platform-fixture")).unwrap();
    let owner = PlatformCli {
        paths,
        locations: Locations {
            config: root.join(".many-ai-cli"),
            data: root.join("ManyAICLI"),
            desktop: root.join("Desktop"),
            startup: Some(root.join("Startup")),
            executable: root.join("many-ai-cli.exe"),
        },
        io: io.clone(),
    };
    (temp, owner, io)
}
#[tokio::test]
async fn setup_writes_source_commands_current_links_before_legacy_cleanup() {
    let (_temp, owner, io) = fixture();
    let mut out = vec![];
    owner
        .setup_for(TargetPlatform::Windows, &mut out, &Cancellation::default())
        .await
        .unwrap();
    let events = io.events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.contains("call \"") && e.contains("serve --open\r\npause\r\n"))
    );
    let last_link = events
        .iter()
        .rposition(|e| e.starts_with("shortcut:"))
        .unwrap();
    let first_remove = events
        .iter()
        .position(|e| e.starts_with("remove:"))
        .unwrap();
    assert!(last_link < first_remove);
    assert_eq!(
        events.iter().filter(|e| e.starts_with("shortcut:")).count(),
        2
    );
    assert!(
        !events
            .iter()
            .any(|e| e.starts_with("remove:") && e.ends_with("Many AI Hub Start.lnk"))
    );
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("自動では消しません")
    );
}
#[tokio::test]
async fn uninstall_decline_has_no_effect_and_yes_orders_cleanup_before_purge() {
    let (_temp, owner, io) = fixture();
    let mut out = vec![];
    owner
        .uninstall(true, &mut &b"no\n"[..], &mut out, &Cancellation::default())
        .await
        .unwrap();
    assert!(io.events.lock().unwrap().is_empty());
    owner
        .uninstall(true, &mut &b"YES\n"[..], &mut out, &Cancellation::default())
        .await
        .unwrap();
    let events = io.events.lock().unwrap();
    assert!(events[0].starts_with("tree:"));
    assert_eq!(
        events.iter().filter(|e| e.starts_with("remove:")).count(),
        2
    );
    assert!(events.last().unwrap().starts_with("purge:"));
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("localStorage.clear()")
    );
}
#[test]
fn batch_and_powershell_values_are_not_interpreted_as_code() {
    assert_eq!(native::quote("a'$()b").unwrap(), "'a''$()b'");
    assert!(native::quote("bad\r\n").is_err());
    assert!(cmd_contents(Path::new("C:/agent%HOME%.exe"), "stop").is_err());
    assert!(cmd_contents(Path::new("C:/agent\".exe"), "stop").is_err());
}

#[tokio::test]
async fn native_trial_refuses_host_shortcuts_and_binary_deletion_before_execution() {
    let (_temp, owner, _) = fixture();
    let native = native::NativePlatformIo {
        actor: crate::application::diagnostics::NativeDiagnosticIo {
            paths: owner.paths.clone(),
            cwd: owner.paths.root().into(),
            environment: vec![],
        },
    };
    let cancel = Cancellation::default();
    assert_eq!(
        native
            .shortcut(
                &owner.locations.desktop.join(SHORTCUT),
                &owner.locations.executable,
                &cancel
            )
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    if cfg!(windows) {
        assert_eq!(
            native
                .purge(&owner.locations.executable, &cancel)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    assert!(!owner.locations.executable.exists());
}

#[tokio::test]
async fn unix_setup_artifacts_have_source_targets_and_explicit_executable_modes() {
    for target in [TargetPlatform::Linux, TargetPlatform::MacOS] {
        let (_temp, owner, io) = fixture();
        let mut out = vec![];
        owner
            .setup_for(target, &mut out, &Cancellation::default())
            .await
            .unwrap();
        let events = io.events.lock().unwrap();
        assert!(
            !events
                .iter()
                .any(|event| event.starts_with("shortcut:") || event.starts_with("remove:"))
        );
        let count = if target == TargetPlatform::Linux {
            4
        } else {
            2
        };
        assert_eq!(
            events
                .iter()
                .filter(|event| event.starts_with("chmod:"))
                .count(),
            count
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.starts_with("write:"))
                .count(),
            count
        );
        assert!(
            events
                .iter()
                .any(|event| event.starts_with("write:") && event.contains("serve --open"))
        );
        assert!(
            events
                .iter()
                .any(|event| event.starts_with("write:") && event.contains(" stop\n"))
        );
        if target == TargetPlatform::Linux {
            assert!(
                events
                    .iter()
                    .any(|event| event.contains("Terminal=true\nCategories=Development;\n"))
            );
        } else {
            assert!(events.iter().any(|event| event.contains("#!/bin/sh\n")));
        }
    }
    assert!(
        unix_contents(
            TargetPlatform::Linux,
            Path::new("/opt/100%/many-ai-cli"),
            "Many AI Hub Start",
            "serve --open"
        )
        .unwrap()
        .contains("100%%")
    );
    assert!(
        unix_contents(
            TargetPlatform::MacOS,
            Path::new("/opt/a'$()/many-ai-cli"),
            "Many AI Hub Start",
            "serve --open"
        )
        .unwrap()
        .contains("'\\''")
    );
}

#[test]
fn native_uninstall_removes_real_tree_preserves_link_target_and_reports_missing() {
    let (_temp, owner, _) = fixture();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("retain.txt"), b"retained actor file").unwrap();
    std::fs::create_dir_all(&owner.locations.config).unwrap();
    std::fs::write(owner.locations.config.join("config.yaml"), b"fixture").unwrap();
    let alias = owner.locations.config.join("subscription-home");
    #[cfg(windows)]
    fixture_junction(outside.path(), &alias).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &alias).unwrap();
    let native = native::NativePlatformIo {
        actor: crate::application::diagnostics::NativeDiagnosticIo {
            paths: owner.paths.clone(),
            cwd: owner.paths.root().into(),
            environment: vec![],
        },
    };
    assert!(native.remove_data(&owner.locations.config).unwrap());
    assert!(!owner.locations.config.exists());
    assert_eq!(
        std::fs::read(outside.path().join("retain.txt")).unwrap(),
        b"retained actor file"
    );
    assert!(!native.remove_data(&owner.locations.config).unwrap());

    // Canonical RuntimePaths and ordinary CLI input refer to the same root.
    let canonical = owner.paths.root().join("canonical-data");
    std::fs::create_dir(&canonical).unwrap();
    std::fs::write(canonical.join("inside.txt"), b"confined").unwrap();
    assert!(native.remove_data(&canonical).unwrap());
    assert_eq!(
        native.remove_data(outside.path()).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    let traversal = owner.locations.config.join("../outside-data");
    assert_eq!(
        native.remove_data(&traversal).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        std::fs::read(outside.path().join("retain.txt")).unwrap(),
        b"retained actor file"
    );
}

#[cfg(windows)]
#[test]
fn native_trial_boundary_accepts_disk_prefix_aliases_without_parent_traversal() {
    use super::native::NativePlatformIo;
    let (_temp, owner, _) = fixture();
    let native = NativePlatformIo {
        actor: crate::application::diagnostics::NativeDiagnosticIo {
            paths: owner.paths.clone(),
            cwd: owner.paths.root().into(),
            environment: vec![],
        },
    };
    let ordinary = owner.locations.config.join("ordinary-data");
    native.prepare_directory(&ordinary).unwrap();
    native
        .write(&ordinary.join("inside.txt"), b"confined")
        .unwrap();
    assert!(native.remove_data(&ordinary).unwrap());
    // A parent component must be rejected even when it would resolve back
    // inside the selected root. Prefix equivalence grants no traversal rights.
    // PathBuf::join normalizes parent components for verbatim Windows roots.
    // Preserve the caller's actual raw spelling instead of testing a path that
    // has already lost the traversal component before reaching the boundary.
    let mut raw = owner.paths.root().as_os_str().to_os_string();
    raw.push("\\child\\..\\forbidden");
    let traversal = PathBuf::from(raw);
    assert!(
        traversal
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    );
    assert_eq!(
        native.prepare_directory(&traversal).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[cfg(unix)]
#[tokio::test]
async fn native_unix_locations_and_purge_need_no_windows_environment_or_powershell() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, owner, _) = fixture();
    let native = native::NativePlatformIo {
        actor: crate::application::diagnostics::NativeDiagnosticIo {
            paths: owner.paths.clone(),
            cwd: owner.paths.root().into(),
            environment: vec![],
        },
    };
    let locations = native
        .locations(
            owner.locations.executable.clone(),
            owner.paths.root().into(),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert!(locations.startup.is_none());
    assert_eq!(locations.config, owner.paths.root().join(".many-ai-cli"));
    let executable = owner.paths.root().join("fixture.command");
    native.write(&executable, b"#!/bin/sh\ntrue\n").unwrap();
    native.make_executable(&executable).unwrap();
    assert_eq!(
        std::fs::metadata(&executable).unwrap().permissions().mode() & 0o777,
        0o755
    );
    native
        .purge(&executable, &Cancellation::default())
        .await
        .unwrap();
    assert!(!executable.exists());
}

#[cfg(windows)]
fn fixture_junction(source: &Path, target: &Path) -> io::Result<()> {
    use std::{
        fs::File,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle},
        },
    };
    use windows_sys::Win32::{
        Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, OPEN_EXISTING,
        },
        System::IO::DeviceIoControl,
    };
    let absolute = std::path::absolute(source)?;
    let display: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    let native = display
        .strip_prefix(verbatim.as_slice())
        .unwrap_or(&display);
    let substitute = "\\??\\"
        .encode_utf16()
        .chain(native.iter().copied())
        .collect::<Vec<_>>();
    let size = 16 + (substitute.len() + display.len() + 2) * 2;
    let mut buffer = vec![0u8; size];
    buffer[..4].copy_from_slice(&0xA0000003u32.to_le_bytes());
    for (offset, value) in [
        (4, size - 8),
        (10, substitute.len() * 2),
        (12, (substitute.len() + 1) * 2),
        (14, display.len() * 2),
    ] {
        buffer[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes());
    }
    for (index, unit) in substitute
        .into_iter()
        .chain(Some(0))
        .chain(display)
        .chain(Some(0))
        .enumerate()
    {
        buffer[16 + index * 2..18 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    std::fs::create_dir(target)?;
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let raw = unsafe {
        CreateFileW(
            target.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_handle(raw) };
    let mut returned = 0;
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x000900A4,
            buffer.as_ptr().cast(),
            buffer.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
