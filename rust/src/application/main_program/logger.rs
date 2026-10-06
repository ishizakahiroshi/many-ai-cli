use crate::{config::LogConfig, files::safe_fs::Dir, logging::RollingLog, storage::mask_secrets};
use std::{
    io::{self, Write},
    path::Path,
    sync::Arc,
};
pub struct HubLogger {
    file: Option<RollingLog>,
    config: LogConfig,
    debug: bool,
}
impl HubLogger {
    pub fn new(directory: &Path, config: LogConfig, debug: bool) -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            // Fixed Go falls back to stderr when safe file logging is not
            // available. Never follow an alias or replace an obstructing file.
            file: Dir::open_or_create_private_components(directory)
                .ok()
                .and_then(|dir| RollingLog::new(Arc::new(dir), "hub.log").ok()),
            config,
            debug,
        }))
    }
    pub fn write(&self, level: &str, message: &str, detail: &str) {
        self.write_with_stderr(level, message, detail, &mut io::stderr().lock());
    }
    fn write_with_stderr(&self, level: &str, message: &str, detail: &str, stderr: &mut impl Write) {
        if level == "DEBUG" && !self.debug {
            return;
        }
        // TextHandler-compatible semantic fields. Diagnostic values stay on one
        // line and are masked before either output sink receives them.
        let message = crate::proto::go_quote::quote(&mask_secrets(message));
        let detail = crate::proto::go_quote::quote(&mask_secrets(detail));
        let line = format!(
            "time={} level={level} msg={message} detail={detail}\n",
            chrono::Local::now().to_rfc3339()
        );
        if self.config.enabled
            && self
                .file
                .as_ref()
                .is_some_and(|file| file.write(&self.config, line.as_bytes()).is_err())
        {
            write_stderr(stderr, b"Hub diagnostic log write unavailable\n");
        }
        write_stderr(stderr, line.as_bytes());
    }
    pub fn close(&self) -> io::Result<()> {
        self.file.as_ref().map_or(Ok(()), RollingLog::close)
    }
}
pub(super) fn write_stderr(stderr: &mut impl Write, bytes: &[u8]) {
    // The auto-started Unix Hub may outlive the terminal that supplied stderr.
    // Go's slog/fmt diagnostics ignore these write failures as well.
    let _ = stderr.write_all(bytes);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistent_debug_filter_and_enabled_gate_use_real_private_logs() {
        let root = tempfile::tempdir().unwrap();
        let config = LogConfig {
            enabled: true,
            ..Default::default()
        };
        let logger = HubLogger::new(root.path(), config.clone(), false).unwrap();
        logger.write("DEBUG", "hidden", "private");
        logger.write("INFO", "started", "loopback");
        logger.close().unwrap();
        let first = std::fs::read_to_string(root.path().join("hub.log")).unwrap();
        assert!(!first.contains("hidden"));
        assert!(first.contains("started"));
        let logger = HubLogger::new(root.path(), config, true).unwrap();
        logger.write("DEBUG", "visible", "loopback");
        logger.close().unwrap();
        assert!(
            std::fs::read_to_string(root.path().join("hub.log"))
                .unwrap()
                .contains("visible")
        );
    }
    #[test]
    fn obstructed_log_directory_uses_stderr_without_overwriting_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("logs");
        std::fs::write(&path, b"synthetic obstruction").unwrap();
        let logger = HubLogger::new(&path, LogConfig::default(), false).unwrap();
        assert!(logger.file.is_none());
        logger.write("INFO", "synthetic fallback", "");
        logger.close().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"synthetic obstruction");
    }
    struct FailedStderr {
        attempts: usize,
    }
    impl Write for FailedStderr {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            self.attempts += 1;
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn closed_stderr_does_not_panic_or_suppress_persistent_diagnostics() {
        let root = tempfile::tempdir().unwrap();
        let logger = HubLogger::new(
            root.path(),
            LogConfig {
                enabled: true,
                ..Default::default()
            },
            false,
        )
        .unwrap();
        let mut stderr = FailedStderr { attempts: 0 };
        logger.write_with_stderr("DEBUG", "hidden", "", &mut stderr);
        assert_eq!(stderr.attempts, 0);
        logger.write_with_stderr("INFO", "survives terminal close", "synthetic", &mut stderr);
        logger.close().unwrap();
        assert_eq!(stderr.attempts, 1);
        let contents = std::fs::read_to_string(root.path().join("hub.log")).unwrap();
        assert!(contents.contains("survives terminal close"));
        assert!(!contents.contains("hidden"));
    }
    #[test]
    fn failed_file_and_stderr_sinks_do_not_panic() {
        let root = tempfile::tempdir().unwrap();
        let logger = HubLogger::new(
            root.path(),
            LogConfig {
                enabled: true,
                ..Default::default()
            },
            false,
        )
        .unwrap();
        std::fs::create_dir(root.path().join("hub.log")).unwrap();
        let mut stderr = FailedStderr { attempts: 0 };
        logger.write_with_stderr("INFO", "synthetic sink failure", "", &mut stderr);
        assert_eq!(stderr.attempts, 2);
        assert!(root.path().join("hub.log").is_dir());
    }
    #[test]
    fn bootstrap_diagnostics_ignore_broken_pipe_and_preserve_healthy_output() {
        let messages: [&[u8]; 2] = [
            b"Hub binary changed; automatic restart deferred\n",
            b"Hub stale restart did not finish; retaining current endpoint\n",
        ];
        let mut failed = FailedStderr { attempts: 0 };
        let mut healthy = Vec::new();
        for message in messages {
            write_stderr(&mut failed, message);
            write_stderr(&mut healthy, message);
        }
        assert_eq!(failed.attempts, 2);
        assert_eq!(healthy, messages.concat());
    }
}

#[cfg(all(test, unix))]
mod directory_permission_tests {
    use super::*;
    use crate::config::{Resource, RuntimePaths};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn hub_logs_preserve_existing_cwd_and_configured_directory_permissions() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("synthetic-cwd");
        let configured = root.path().join("configured-logs");
        for directory in [&cwd, &configured] {
            std::fs::create_dir(directory).unwrap();
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o751)).unwrap();
        }
        for setting in [Path::new(""), configured.as_path()] {
            let paths = RuntimePaths::production(&root.path().join("synthetic-home"))
                .unwrap()
                .with_log_dir_at(setting, &cwd)
                .unwrap();
            let directory = paths.resource(Resource::Logs);
            let config = LogConfig {
                enabled: true,
                ..Default::default()
            };
            let logger = HubLogger::new(&directory, config, false).unwrap();
            logger.write("INFO", "synthetic permission test", "");
            logger.close().unwrap();
            assert_eq!(
                std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o751
            );
            assert!(directory.join("hub.log").is_file());
        }
    }
}
