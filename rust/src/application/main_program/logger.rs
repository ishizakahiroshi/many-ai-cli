use crate::{config::LogConfig, files::safe_fs::Dir, logging::RollingLog, storage::mask_secrets};
use std::{io, path::Path, sync::Arc};
pub struct HubLogger {
    file: RollingLog,
    config: LogConfig,
    debug: bool,
}
impl HubLogger {
    pub fn new(directory: &Path, config: LogConfig, debug: bool) -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            file: RollingLog::new(Arc::new(Dir::open_or_create_private(directory)?), "hub.log")?,
            config,
            debug,
        }))
    }
    pub fn write(&self, level: &str, message: &str, detail: &str) {
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
        if self.config.enabled && self.file.write(&self.config, line.as_bytes()).is_err() {
            eprintln!("Hub diagnostic log write unavailable");
        }
        eprint!("{line}");
    }
    pub fn close(&self) -> io::Result<()> {
        self.file.close()
    }
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
}
