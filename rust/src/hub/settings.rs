use super::http::{Request, Response, decode_json};
use crate::{
    config::{self, ConfigStore, LogConfig, Resource, RuntimePaths},
    proto::wire::{Field, GoWire, Schema},
};
use serde::Deserialize;
use serde_json::json;

macro_rules! request_type {
    ($name:ident {$($field:ident:$ty:ty => $kind:literal),*$(,)?})=>{
        #[derive(Default,Deserialize)] #[serde(default)] pub(crate) struct $name {$(pub $field:$ty),*}
        impl GoWire for $name {const GO_TYPE:&'static str=stringify!($name); const SCHEMAS:&'static[Schema]=&[Schema{name:stringify!($name),fields:&[$(Field{name:stringify!($field),kind:$kind}),*]}];}
    }
}
request_type!(TerminalColor { terminal_color:String=>"string" });
request_type!(HandoffMode { intent_mode:String=>"string" });
request_type!(Reconnect { wrapper_reconnect_grace_sec:i64=>"int" });
request_type!(Input { deferred_enter_ms:i64=>"int" });
request_type!(IdleTimeout { idle_timeout_min:i64=>"int" });
request_type!(Orchestration { board_notify_mode:String=>"string",spawn_confirm_mode:String=>"string",spawn_confirm_providers:Option<Vec<String>> =>"[]string",child_timeout_seconds:i64=>"int",timeout_respawn:bool=>"bool",max_children_per_parent:Option<i64> =>"*int" });
impl GoWire for LogConfig {
    const GO_TYPE: &'static str = "ServiceLogConfig";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "ServiceLogConfig",
        fields: &[
            Field {
                name: "enabled",
                kind: "bool",
            },
            Field {
                name: "session_enabled",
                kind: "bool",
            },
            Field {
                name: "legacy_logs_notice_shown",
                kind: "bool",
            },
            Field {
                name: "max_size_mb",
                kind: "int",
            },
            Field {
                name: "max_backups",
                kind: "int",
            },
            Field {
                name: "compress",
                kind: "bool",
            },
            Field {
                name: "session_retention_days",
                kind: "int",
            },
            Field {
                name: "session_max_size_mb",
                kind: "int",
            },
            Field {
                name: "attachment_retention_days",
                kind: "int",
            },
            Field {
                name: "attachment_max_total_mb",
                kind: "int",
            },
        ],
    }];
}
pub const PATHS: &[&str] = &[
    "/api/notify-config",
    "/api/idle-timeout",
    "/api/log-config",
    "/api/terminal-color",
    "/api/handoff-intent-mode",
    "/api/reconnect-grace",
    "/api/input-config",
    "/api/orchestration-config",
];
/// Caller has applied the full guard. These six Go handlers publish their
/// field mutations before Save, so a failed save remains visible in memory.
/// The explicit shared legacy entry retains revision/trial-path protections.
pub fn handle(request: &Request, store: &ConfigStore, paths: &RuntimePaths) -> Response {
    handle_with_published(request, store, paths, &|_| {})
}
pub fn handle_with_published(
    request: &Request,
    store: &ConfigStore,
    paths: &RuntimePaths,
    published: &dyn Fn(&crate::config::Config),
) -> Response {
    let mut snap = match store.snapshot() {
        Ok(s) => s,
        Err(_) => return Response::error(500, "internal", "configuration unavailable"),
    };
    if request.method == "GET" {
        let c = &snap.config;
        let value = match request.path.as_str() {
            "/api/notify-config" => {
                let mut value = json!({});
                if let Some(backends) = c
                    .notify
                    .backends
                    .as_ref()
                    .filter(|values| !values.is_empty())
                {
                    value["backends"] = backends
                        .iter()
                        .map(|backend| {
                            let mut value = json!({"type":backend.r#type,"url":backend.url});
                            if !backend.topic.is_empty() {
                                value["topic"] = backend.topic.clone().into();
                            }
                            value
                        })
                        .collect::<Vec<_>>()
                        .into();
                }
                if let Some(events) = c.notify.events.as_ref().filter(|values| !values.is_empty()) {
                    value["events"] = json!(events);
                }
                if c.notify.include_body {
                    value["include_body"] = true.into();
                }
                value
            }
            "/api/idle-timeout" => json!({"idle_timeout_min":c.hub.idle_timeout_min}),
            "/api/log-config" => {
                let mut v = serde_json::to_value(&c.log).expect("log config");
                v["log_dir"] = c.hub.log_dir.clone().into();
                v["attach_dir"] = paths
                    .resource(Resource::Attachments)
                    .to_string_lossy()
                    .into_owned()
                    .into();
                v
            }
            "/api/terminal-color" => {
                json!({"terminal_color":config::normalize_terminal_color(&c.hub.terminal_color)})
            }
            "/api/handoff-intent-mode" => {
                json!({"intent_mode":config::normalize_handoff_intent_mode(&c.handoff.intent_mode)})
            }
            "/api/reconnect-grace" => {
                json!({"wrapper_reconnect_grace_sec":c.hub.wrapper_reconnect_grace_sec})
            }
            "/api/input-config" => json!({"deferred_enter_ms":c.input.deferred_enter_ms}),
            "/api/orchestration-config" => {
                json!({"board_notify_mode":config::effective_board_notify_mode(&c.orchestration.board_notify_mode),"spawn_confirm_mode":config::effective_spawn_confirm_mode(&c.orchestration.spawn_confirm_mode),"spawn_confirm_providers":if c.orchestration.spawn_confirm_providers.is_empty(){None}else{Some(&c.orchestration.spawn_confirm_providers)},"child_timeout_seconds":c.orchestration.child_timeout_seconds,"timeout_respawn":c.orchestration.timeout_respawn,"max_children_per_parent":c.orchestration.max_children_per_parent})
            }
            _ => return Response::error(404, "not_found", "not found"),
        };
        return Response::json(200, &value);
    }
    macro_rules! body {
        ($ty:ty) => {
            match decode_json::<$ty>(request) {
                Ok(v) => v,
                Err(e) => return e,
            }
        };
    }
    let result = match request.path.as_str() {
        "/api/notify-config" => {
            let body = body!(crate::config::NotifyConfig);
            for backend in body.backends.as_deref().unwrap_or_default() {
                if let Err(error) = crate::notify::validate_backend(backend) {
                    return Response::error(400, "invalid_backend", error);
                }
            }
            snap.config.notify = body;
            json!({"ok":true})
        }
        "/api/idle-timeout" => {
            let body = body!(IdleTimeout);
            snap.config.hub.idle_timeout_min = body.idle_timeout_min.clamp(0, 1440);
            json!({"ok":true})
        }
        "/api/log-config" => {
            let mut b = body!(LogConfig);
            b.max_size_mb = b.max_size_mb.clamp(1, 1000);
            b.max_backups = b.max_backups.clamp(0, 100);
            b.session_retention_days = b.session_retention_days.clamp(0, 365);
            b.session_max_size_mb = b.session_max_size_mb.clamp(0, 10000);
            b.attachment_retention_days = b.attachment_retention_days.clamp(0, 365);
            b.attachment_max_total_mb = b.attachment_max_total_mb.clamp(0, 100000);
            b.legacy_logs_notice_shown = snap.config.log.legacy_logs_notice_shown;
            snap.config.log = b;
            json!({"ok":true})
        }
        "/api/terminal-color" => {
            let b = body!(TerminalColor);
            let mode = config::normalize_terminal_color(&b.terminal_color);
            snap.config.hub.terminal_color = mode.into();
            json!({"ok":true,"terminal_color":mode})
        }
        "/api/handoff-intent-mode" => {
            let b = body!(HandoffMode);
            let mode = config::normalize_handoff_intent_mode(&b.intent_mode);
            snap.config.handoff.intent_mode = mode.into();
            json!({"ok":true,"intent_mode":mode})
        }
        "/api/reconnect-grace" => {
            let b = body!(Reconnect);
            snap.config.hub.wrapper_reconnect_grace_sec =
                b.wrapper_reconnect_grace_sec.clamp(0, 86400);
            json!({"ok":true})
        }
        "/api/input-config" => {
            let b = body!(Input);
            if !(0..=10000).contains(&b.deferred_enter_ms) {
                return Response::error(
                    400,
                    "invalid_deferred_enter_ms",
                    "deferred_enter_ms must be between 0 and 10000",
                );
            }
            snap.config.input.deferred_enter_ms = b.deferred_enter_ms;
            json!({"ok":true})
        }
        "/api/orchestration-config" => {
            let b = body!(Orchestration);
            if !matches!(
                b.board_notify_mode.as_str(),
                "soft-notify" | "queue-until-idle" | "interrupt"
            ) || !matches!(b.spawn_confirm_mode.as_str(), "on" | "off" | "providers")
            {
                return Response::error(
                    400,
                    "invalid_board_notify_mode",
                    "board_notify_mode must be soft-notify, queue-until-idle, or interrupt",
                );
            }
            if !(60..=86400).contains(&b.child_timeout_seconds) {
                return Response::error(
                    400,
                    "invalid_child_timeout",
                    "child_timeout_seconds must be between 60 and 86400",
                );
            }
            if b.max_children_per_parent
                .is_some_and(|v| !(1..=256).contains(&v))
            {
                return Response::error(
                    400,
                    "invalid_child_limit",
                    "max_children_per_parent must be between 1 and 256",
                );
            }
            let c = &mut snap.config.orchestration;
            if let Some(n) = b.max_children_per_parent {
                c.max_children_per_parent = n;
            }
            c.board_notify_mode = b.board_notify_mode;
            c.spawn_confirm_mode = b.spawn_confirm_mode;
            c.spawn_confirm_providers = b.spawn_confirm_providers.unwrap_or_default();
            c.child_timeout_seconds = b.child_timeout_seconds;
            c.timeout_respawn = b.timeout_respawn;
            json!({"ok":true})
        }
        _ => return Response::error(404, "not_found", "not found"),
    };
    match store.publish_then_persist_legacy_with(snap.revision, snap.config, published) {
        Ok(_) => Response::json(200, &result),
        Err(error) => Response::error(500, "save_failed", &format!("save failed: {error}")),
    }
}
