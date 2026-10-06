//! Lifetime ownership precedes database open/reset and any Hub registration.
//! Lock files are never unlinked: replacing a locked inode would admit a second
//! owner. Go does not take this new lock, so its live ledger is also refused.
use crate::{application::hub_runtime::RuntimeLedger, config::RuntimePaths, files::safe_fs::Dir};
use std::{fs::File, io, path::Path};

pub(super) struct OwnerLease {
    _directory: Dir,
    _file: File,
}
impl OwnerLease {
    pub(super) fn acquire(directory: &Path, name: &str) -> io::Result<Self> {
        let directory = Dir::open_or_create_private_components(directory)?;
        let file = directory.open_lock(name)?;
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::AddrInUse,
                "selected Hub data already has an active owner",
            ),
            std::fs::TryLockError::Error(error) => error,
        })?;
        Ok(Self {
            _directory: directory,
            _file: file,
        })
    }
    pub(super) fn runtime(paths: &RuntimePaths) -> io::Result<Self> {
        let lease = Self::acquire(paths.root(), "rust-hub-owner.lock")?;
        if RuntimeLedger::open(paths)?
            .read()?
            .is_some_and(|data| crate::process::pid_alive(data.pid))
        {
            // A PID alone cannot establish identity for termination. Refuse to
            // mutate an existing owner's data; never kill it or change its port.
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "selected Hub runtime records a live owner; stop it explicitly before starting a candidate",
            ));
        }
        Ok(lease)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_owner_inode_excludes_until_drop_without_replacing_it() {
        let root = tempfile::tempdir().unwrap();
        let first = OwnerLease::acquire(root.path(), "synthetic-owner.lock").unwrap();
        assert!(
            matches!(OwnerLease::acquire(root.path(), "synthetic-owner.lock"),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse)
        );
        assert!(root.path().join("synthetic-owner.lock").is_file());
        drop(first);
        let _second = OwnerLease::acquire(root.path(), "synthetic-owner.lock").unwrap();
    }
}

#[cfg(all(test, unix))]
mod permission_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn database_owner_preserves_existing_directory_permissions() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("synthetic-cwd");
        std::fs::create_dir(&cwd).unwrap();
        std::fs::set_permissions(&cwd, std::fs::Permissions::from_mode(0o751)).unwrap();
        let _lease = OwnerLease::acquire(&cwd, "synthetic-database-owner.lock").unwrap();
        assert_eq!(
            std::fs::metadata(&cwd).unwrap().permissions().mode() & 0o777,
            0o751
        );
    }
}
