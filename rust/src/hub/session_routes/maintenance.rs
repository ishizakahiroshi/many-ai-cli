//! Fixed purge_handlers.go log maintenance, sharing canonical core and repository.
use super::*;
use crate::{
    config::{ConfigStore, RuntimePaths},
    files::safe_fs::Dir,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

impl SessionHttp {
    pub fn handle_log_maintenance_authenticated(
        &self,
        request: &Request,
        config: &ConfigStore,
        paths: &RuntimePaths,
    ) -> Response {
        let mut snapshot = match config.snapshot() {
            Ok(value) => value,
            Err(_) => return Response::error(500, "internal", "configuration unavailable"),
        };
        let log_dir = PathBuf::from(&snapshot.config.hub.log_dir);
        if paths.is_trial() && log_dir != paths.resource(crate::config::Resource::Logs) {
            return Response::error(403, "forbidden", "log directory is outside trial root");
        }
        match request.path.as_str() {
            "/api/logs/legacy-notice" if request.method == "GET" => {
                let exists = Dir::open(&log_dir)
                    .and_then(|dir| dir.child_dir("sessions", false))
                    .ok()
                    .is_some_and(|dir| {
                        dir.entries().unwrap_or_default().iter().any(|name| {
                            (name.ends_with(".log") || name.ends_with(".jsonl"))
                                && dir.metadata(name).is_ok_and(|metadata| metadata.is_file())
                        })
                    });
                Response::json(
                    200,
                    &json!({"show": !snapshot.config.log.legacy_logs_notice_shown && !snapshot.config.log.session_enabled && exists}),
                )
            }
            "/api/logs/legacy-notice" => {
                snapshot.config.log.legacy_logs_notice_shown = true;
                if let Some(enable) = optional_enable_logging(request) {
                    snapshot.config.log.session_enabled = enable;
                }
                let enabled = snapshot.config.log.session_enabled;
                match config.publish_then_persist_legacy_with(
                    snapshot.revision,
                    snapshot.config,
                    |_| self.journal.set_enabled(enabled),
                ) {
                    Ok(_) => Response::json(200, &json!({"ok":true})),
                    Err(_) => Response::error(500, "save_failed", "save failed"),
                }
            }
            "/api/logs/purge" => {
                let _guard = self
                    .log_maintenance
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let ids = self.core.registered_session_ids();
                let mut active = BTreeSet::new();
                for id in &ids {
                    if let Some(details) = self.core.details(*id) {
                        for path in [&details.snapshot.log_path, &details.snapshot.jsonl_path] {
                            if !path.is_empty() {
                                active.insert(log_base(Path::new(path)));
                            }
                        }
                    }
                }
                let mut session_files = 0;
                let mut spawn_files = 0;
                if let Ok(dir) = Dir::open(&log_dir) {
                    if let Ok(sessions) = dir.child_dir("sessions", false) {
                        for name in sessions.entries().unwrap_or_default() {
                            if !matches!(
                                Path::new(&name)
                                    .extension()
                                    .and_then(|value| value.to_str()),
                                Some("log" | "jsonl" | "txt")
                            ) {
                                continue;
                            }
                            if active.contains(&log_base(&log_dir.join("sessions").join(&name))) {
                                continue;
                            }
                            if sessions
                                .metadata(&name)
                                .is_ok_and(|metadata| metadata.is_file())
                                && sessions.remove_file(&name).is_ok()
                            {
                                session_files += 1;
                            }
                        }
                    }
                    if let Ok(spawn) = dir.child_dir("spawn", false) {
                        for name in spawn.entries().unwrap_or_default() {
                            if spawn
                                .metadata(&name)
                                .is_ok_and(|metadata| metadata.is_file())
                                && spawn.remove_file(&name).is_ok()
                            {
                                spawn_files += 1;
                            }
                        }
                    }
                }
                drop(_guard);
                let store_sessions = self
                    .storage()
                    .and_then(|store| store.reset_history(&ids).ok())
                    .map_or(0, |result| result.sessions);
                Response::json(
                    200,
                    &json!({"ok":true,"session_files":session_files,"spawn_files":spawn_files,"store_sessions":store_sessions}),
                )
            }
            _ => Response::error(404, "not_found", "not found"),
        }
    }
}
fn log_base(path: &Path) -> PathBuf {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return path.into();
    };
    match name.rsplit_once('.') {
        Some((base, _)) => path.parent().unwrap_or(Path::new("")).join(base),
        None => path.into(),
    }
}
fn optional_enable_logging(request: &Request) -> Option<bool> {
    // Decoder validates a complete first JSON value before decoding fields;
    // wrong field types retain any earlier successfully decoded bool pointer.
    let members = crate::proto::wire::decode_http_go_members(&request.body).ok()??;
    let mut enabled = None;
    for (name, value) in members {
        if crate::proto::unicode::simple_lower(&name) == "enable_logging" {
            if value == b"null" {
                enabled = None;
            } else if let Ok(value) = serde_json::from_slice::<bool>(&value) {
                enabled = Some(value);
            }
        }
    }
    enabled
}
