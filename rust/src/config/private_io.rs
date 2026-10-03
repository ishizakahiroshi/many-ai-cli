use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        super::private_windows::restrict(path, true)?;
    }
    Ok(())
}
/// Same-directory exclusive temporary file, fsync, atomic replacement, directory sync.
/// A failed write/rename keeps the previous destination intact and removes our temporary file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("destination has no parent"))?;
    ensure_private_dir(parent)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("destination has no filename"))?
        .to_string_lossy();
    let (temporary, mut file) = loop {
        let temporary = parent.join(format!(
            ".{name}.tmp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => break (temporary, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = (|| {
        #[cfg(windows)]
        super::private_windows::restrict(&temporary, false)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(not(windows))]
        fs::rename(&temporary, path)?;
        #[cfg(windows)]
        super::private_windows::replace(&temporary, path)?;
        #[cfg(unix)]
        {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_atomic_replace_and_failure_preserve_data() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("config.yaml");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        let dir = t.path().join("directory");
        fs::create_dir(&dir).unwrap();
        assert!(write_atomic(&dir, b"bad").is_err());
        assert!(dir.is_dir());
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
