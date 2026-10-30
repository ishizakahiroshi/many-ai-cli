//! Start-time retention sweep. The retention settings (`session_retention_days`,
//! `handoff.retention_days`, `attachment_retention_days`) were never read by
//! the Rust Hub, so old session files, handoff records, history rows and
//! attachments piled up. Every step is best effort and bounded by the configured
//! number of days; a value of zero or less disables that step.
use crate::{
    config::{Config, Resource, RuntimePaths},
    files::{FilesService, safe_fs::Dir},
    orchestration::handoff::HandoffStore,
    proto::{core::SessionStorage, time::Timestamp},
};
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// Written once the pre-masking `.log` files have been scrubbed.
const SCRUB_MARKER: &str = ".logs-masked-v1";
/// A file touched this recently may still be open for writing by a wrapper.
const LIVE_GRACE: Duration = Duration::from_secs(5 * 60);

pub(super) struct Sweep {
    pub paths: RuntimePaths,
    pub config: Config,
    pub handoff: HandoffStore,
    pub storage: Option<Arc<dyn SessionStorage>>,
    pub files: Arc<FilesService>,
}

impl Sweep {
    pub(super) fn run(&self, now: SystemTime) {
        let _ = self.handoff.compact_transcripts();
        self.mask_legacy_logs(now);
        self.remove_old_session_files(now);
        self.prune_handoff(now);
        self.prune_history(now);
        let log = &self.config.log;
        let _ = self.files.clean_attachments(
            log.attachment_retention_days,
            (log.attachment_max_total_mb.max(0) as u64).saturating_mul(1024 * 1024),
            now,
        );
    }
    fn cutoff(now: SystemTime, days: i64) -> Option<SystemTime> {
        (days > 0).then(|| now - DAY * days.min(3650) as u32)
    }
    fn sessions_dir(&self) -> Option<Dir> {
        Dir::open(&self.paths.resource(Resource::Logs))
            .and_then(|logs| logs.child_dir("sessions", false))
            .ok()
    }
    fn remove_old_session_files(&self, now: SystemTime) {
        let Some(cutoff) = Self::cutoff(now, self.config.log.session_retention_days) else {
            return;
        };
        let Some(sessions) = self.sessions_dir() else {
            return;
        };
        for name in sessions.entries().unwrap_or_default() {
            if !matches!(
                name.rsplit_once('.').map(|(_, ext)| ext),
                Some("log" | "jsonl" | "txt")
            ) {
                continue;
            }
            let old = sessions
                .metadata(&name)
                .ok()
                .filter(|metadata| metadata.is_file())
                .and_then(|metadata| metadata.modified().ok())
                .is_some_and(|modified| modified < cutoff);
            if old {
                let _ = sessions.remove_file(&name);
            }
        }
    }
    fn prune_handoff(&self, now: SystemTime) {
        let days = self.config.handoff.retention_days_or_default();
        if let Some(cutoff) = Self::cutoff(now, days)
            && let Ok(cutoff) = Timestamp::from_system_time(cutoff)
        {
            let _ = self.handoff.prune_older_than(cutoff);
        }
    }
    fn prune_history(&self, now: SystemTime) {
        if let Some(cutoff) = Self::cutoff(now, self.config.log.session_retention_days)
            && let Ok(cutoff) = Timestamp::from_system_time(cutoff)
            && let Some(storage) = &self.storage
        {
            let _ = storage.prune_older_than(cutoff);
        }
    }
    /// `.log` files written before the raw log was masked may hold a secret in
    /// the clear. Mask them once, whole-file, so a key split across reads and a
    /// multi-line private key are caught too. Newer files are written masked.
    fn mask_legacy_logs(&self, now: SystemTime) {
        let Some(sessions) = self.sessions_dir() else {
            return;
        };
        if sessions.metadata(SCRUB_MARKER).is_ok() {
            return;
        }
        for name in sessions.entries().unwrap_or_default() {
            if !name.ends_with(".log") {
                continue;
            }
            let idle = sessions
                .metadata(&name)
                .ok()
                .filter(|metadata| metadata.is_file())
                .and_then(|metadata| metadata.modified().ok())
                .is_some_and(|modified| {
                    now.duration_since(modified).unwrap_or_default() > LIVE_GRACE
                });
            if !idle {
                continue;
            }
            let mut original = Vec::new();
            if sessions
                .open_file(&name, false)
                .and_then(|mut file| std::io::Read::read_to_end(&mut file, &mut original))
                .is_err()
            {
                continue;
            }
            let masked = crate::storage::mask_secret_bytes(&original);
            if masked != original {
                let _ = sessions.replace(&name, &masked, 0o600);
            }
        }
        let _ = sessions.replace(SCRUB_MARKER, b"", 0o600);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49121, installed.path()).unwrap();
        (root, installed, paths)
    }
    fn sweep(paths: &RuntimePaths, config: Config) -> Sweep {
        Sweep {
            paths: paths.clone(),
            config,
            handoff: HandoffStore::new(paths.clone()),
            storage: None,
            files: Arc::new(FilesService::new(paths.root().to_path_buf(), paths.clone())),
        }
    }
    fn sessions(paths: &RuntimePaths) -> std::path::PathBuf {
        let dir = paths.resource(Resource::Logs).join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn set_age(path: &std::path::Path, age: Duration) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn session_files_older_than_the_retention_days_are_removed() {
        let (_root, _installed, paths) = root();
        let dir = sessions(&paths);
        std::fs::write(dir.join("old.log"), b"x").unwrap();
        std::fs::write(dir.join("old.jsonl"), b"x").unwrap();
        std::fs::write(dir.join("new.log"), b"x").unwrap();
        std::fs::write(dir.join("other.dat"), b"x").unwrap();
        set_age(&dir.join("old.log"), DAY * 8);
        set_age(&dir.join("old.jsonl"), DAY * 8);
        set_age(&dir.join("other.dat"), DAY * 8);
        let mut config = Config::default();
        config.log.session_retention_days = 7;
        sweep(&paths, config).run(SystemTime::now());
        assert!(!dir.join("old.log").exists());
        assert!(!dir.join("old.jsonl").exists());
        assert!(dir.join("new.log").exists());
        assert!(dir.join("other.dat").exists());
    }

    #[test]
    fn zero_retention_days_keeps_everything() {
        let (_root, _installed, paths) = root();
        let dir = sessions(&paths);
        std::fs::write(dir.join("old.log"), b"x").unwrap();
        set_age(&dir.join("old.log"), DAY * 400);
        let mut config = Config::default();
        config.log.session_retention_days = 0;
        sweep(&paths, config).run(SystemTime::now());
        assert!(dir.join("old.log").exists());
    }

    #[test]
    fn legacy_logs_are_masked_once_and_a_live_file_is_left_alone() {
        let (_root, _installed, paths) = root();
        let dir = sessions(&paths);
        let secret = format!("token ghp_{}\nnext\n", "A".repeat(30));
        std::fs::write(dir.join("done.log"), &secret).unwrap();
        std::fs::write(dir.join("live.log"), &secret).unwrap();
        set_age(&dir.join("done.log"), DAY);
        let mut config = Config::default();
        config.log.session_retention_days = 0;
        sweep(&paths, config).run(SystemTime::now());
        let done = std::fs::read_to_string(dir.join("done.log")).unwrap();
        assert!(!done.contains("AAAA"), "{done}");
        assert!(done.contains("token ***"), "{done}");
        assert!(
            std::fs::read_to_string(dir.join("live.log"))
                .unwrap()
                .contains("AAAA")
        );
        assert!(dir.join(SCRUB_MARKER).exists());
    }
}
