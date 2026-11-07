//! Lifetime ownership precedes database open/reset and any Hub registration.
//! Lock files are never unlinked: replacing a locked inode would admit a second
//! owner. Go does not take this new lock, so its live ledger is also refused.
use crate::{
    application::hub_runtime::{self, Owner, RuntimeLedger},
    config::RuntimePaths,
    files::safe_fs::Dir,
};
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
        if RuntimeLedger::open(paths)?.read()?.is_some_and(|data| {
            matches!(
                hub_runtime::owner(&data),
                Owner::Recorded | Owner::Unverified
            )
        }) {
            // Refuse to mutate a live owner's data; never kill it or change its
            // port. A process that was given the PID after that owner ended
            // does not count (U11); where processes cannot be identified, a
            // live PID still does.
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

#[cfg(all(test, any(windows, target_os = "linux")))]
mod reused_pid_tests {
    use super::*;
    use crate::{
        process::wrapper_startup::recovery_identity::test_support::Bystander,
        proto::time::{Timestamp, format_rfc3339_nano},
    };

    fn record(paths: &RuntimePaths, json: &str) {
        Dir::open_or_create_private_components(paths.root())
            .unwrap()
            .replace(hub_runtime::FILE, json.as_bytes(), 0o600)
            .unwrap();
    }
    fn refused(paths: &RuntimePaths) -> bool {
        match OwnerLease::runtime(paths) {
            Ok(_) => false,
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => true,
            Err(error) => panic!("unexpected: {error}"),
        }
    }

    #[test]
    fn audit_u11_a_pid_given_to_another_process_does_not_lock_out_startup() {
        let home = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::production(home.path()).unwrap();
        let bystander = Bystander::spawn();
        let pid = bystander.pid();
        // The recorded Hub's identity is not the one now on its PID.
        record(
            &paths,
            &format!(
                r#"{{"version":1,"pid":{pid},"port":49372,"started_at":"2026-01-01T00:00:00Z","process":"synthetic:another-process"}}"#
            ),
        );
        assert!(!refused(&paths));
        // A record from before identities were kept, written before the
        // process now on its PID existed.
        record(
            &paths,
            &format!(
                r#"{{"version":1,"pid":{pid},"port":49372,"started_at":"2001-01-01T00:00:00Z"}}"#
            ),
        );
        assert!(!refused(&paths));
        // The process a record names is still refused, in both forms.
        let now = format_rfc3339_nano(Timestamp::now()).unwrap();
        record(
            &paths,
            &format!(r#"{{"version":1,"pid":{pid},"port":49372,"started_at":"{now}"}}"#),
        );
        assert!(refused(&paths));
        RuntimeLedger::open(&paths)
            .unwrap()
            .write(49372, i64::from(pid), Timestamp::now())
            .unwrap();
        assert!(refused(&paths));
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
