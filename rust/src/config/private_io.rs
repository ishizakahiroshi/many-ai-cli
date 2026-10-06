/// Protected ACLs supplied at Windows object creation, before any payload write.
#[cfg(windows)]
pub use super::private_windows::PrivateSecurity;
use std::{fs, io, path::Path};

/// Private append-only file under a capability-walked parent. Each write uses
/// the kernel append offset; existing log bodies are never read and rewritten.
pub fn open_append(path: &Path) -> io::Result<fs::File> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("append destination has no parent"))?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| io::Error::other("append destination has no UTF-8 filename"))?;
    crate::files::safe_fs::Dir::open_or_create_private_components(parent)?.open_append(name)
}
pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    crate::files::safe_fs::Dir::open_or_create_private(path).map(|_| ())
}
/// Same-directory exclusive temporary file, fsync, atomic replacement, directory sync.
/// A failed write/rename keeps the previous destination intact and removes our temporary file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("destination has no parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("destination has no UTF-8 filename"))?;
    crate::files::safe_fs::Dir::open_or_create_private_components(parent)?
        .replace(name, bytes, 0o600)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_existing_parent_and_explicit_repair_remains_available() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("existing");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o750)).unwrap();
        let path = directory.join("synthetic.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o750
        );
        ensure_private_dir(&directory).unwrap();
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
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
