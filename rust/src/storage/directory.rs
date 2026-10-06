//! All filesystem mutations below the SQLite directory use a held capability.
use super::*;
pub(super) const DATABASE: &str = "any-ai-cli.db";
pub(super) const WAL: &str = "any-ai-cli.db-wal";
pub(super) const SHM: &str = "any-ai-cli.db-shm";
pub(super) const RESET_MARKER: &str = "any-ai-cli.db.reset-pending";

pub(super) fn open(path: &Path) -> StorageResult<Dir> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| {
                error(
                    StorageErrorKind::Open,
                    "database working directory is unavailable",
                )
            })?
            .join(path)
    };
    Dir::open_or_create_private_components(&path).map_err(|_| {
        error(
            StorageErrorKind::Open,
            "database directory could not be held privately",
        )
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    #[test]
    fn reset_recovery_uses_held_directory_after_path_replacement() {
        let top = tempfile::tempdir().unwrap();
        let original = top.path().join("original");
        let moved = top.path().join("moved");
        let outside = top.path().join("outside");
        std::fs::create_dir(&original).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let held = open(&original).unwrap();
        for name in [DATABASE, WAL, SHM, RESET_MARKER] {
            held.create_new(name, b"owned synthetic", 0o600).unwrap();
            std::fs::write(outside.join(name), b"outside synthetic").unwrap();
        }
        std::fs::rename(&original, &moved).unwrap();
        symlink(&outside, &original).unwrap();
        schema::apply_pending_reset(&held).unwrap();
        for name in [DATABASE, WAL, SHM, RESET_MARKER] {
            assert!(!moved.join(name).exists());
            assert_eq!(
                std::fs::read(outside.join(name)).unwrap(),
                b"outside synthetic"
            );
        }
        // File creation is also anchored if replacement occurs between holding
        // the directory and the actual openat/create operation.
        held.create_new(DATABASE, b"new owned synthetic", 0o600)
            .unwrap();
        assert_eq!(
            std::fs::read(moved.join(DATABASE)).unwrap(),
            b"new owned synthetic"
        );
        assert_eq!(
            std::fs::read(outside.join(DATABASE)).unwrap(),
            b"outside synthetic"
        );
    }
}

#[cfg(all(test, unix))]
mod permission_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn database_parent_preserves_existing_working_directory_permissions() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("synthetic-cwd");
        std::fs::create_dir(&cwd).unwrap();
        std::fs::set_permissions(&cwd, std::fs::Permissions::from_mode(0o751)).unwrap();
        let held = open(&cwd).unwrap();
        assert_eq!(
            held.own_metadata().unwrap().permissions().mode() & 0o777,
            0o751
        );
    }
}
