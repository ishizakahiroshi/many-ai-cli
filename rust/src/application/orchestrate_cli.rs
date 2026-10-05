//! Source orchestration CLI: credentials stay in the explicit header-only actor.
pub use super::issue_cli::flags;
mod native;
use crate::{
    application::diagnostics::report,
    config::RuntimePaths,
    process::Cancellation,
    proto::{
        core::CoreFuture,
        wire::{Field, GoWire, Schema},
    },
};
pub use native::NativeOrchestrateIo;
use serde_json::{Map, Value, json};
use std::{io, path::PathBuf, sync::Arc, time::Duration};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    Timeout,
    Cancelled,
    Transport,
}
pub struct HttpReply {
    pub status: u16,
    pub body: Vec<u8>,
}
pub trait OrchestrateIo: Send + Sync {
    fn exchange<'a>(
        &'a self,
        request: HttpRequest<'a>,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<HttpReply, RequestError>>;
}
pub struct HttpRequest<'a> {
    pub port: u16,
    pub path: String,
    pub token: &'a str,
    pub body: Option<Value>,
    pub timeout: Duration,
}
pub struct OrchestrateCli {
    pub paths: RuntimePaths,
    pub cwd: PathBuf,
    pub environment: Vec<String>,
    pub io: Arc<dyn OrchestrateIo>,
}
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct ChildReply {
    ok: bool,
    session_id: i64,
    board_path: String,
    cwd: String,
    relay: Option<RelayStatus>,
    relays: Option<Vec<RelayItem>>,
    error: String,
    detail: String,
}
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct RelayItem {
    relay: RelayStatus,
}
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct RelayStatus {
    orchestration_id: String,
    mode: String,
    state: String,
    completed_cs: i64,
    round: i64,
    max_rounds: i64,
    implementation_session_id: i64,
    worktree_path: String,
    branch: String,
}
const fn f(name: &'static str, kind: &'static str) -> Field {
    Field { name, kind }
}
impl GoWire for ChildReply {
    const GO_TYPE: &'static str = "ChildAPIResponse";
    const SCHEMAS: &'static [Schema] = &[
        Schema {
            name: "ChildAPIResponse",
            fields: &[
                f("ok", "bool"),
                f("session_id", "int"),
                f("board_path", "string"),
                f("cwd", "string"),
                f("relay", "*RelayStatus"),
                f("relays", "[]RelayItem"),
                f("error", "string"),
                f("detail", "string"),
            ],
        },
        Schema {
            name: "RelayItem",
            fields: &[f("relay", "RelayStatus")],
        },
        Schema {
            name: "RelayStatus",
            fields: &[
                f("orchestration_id", "string"),
                f("mode", "string"),
                f("state", "string"),
                f("completed_cs", "int"),
                f("round", "int"),
                f("max_rounds", "int"),
                f("implementation_session_id", "int"),
                f("worktree_path", "string"),
                f("branch", "string"),
            ],
        },
    ];
}
fn environment<'a>(entries: &'a [String], key: &str) -> &'a str {
    entries
        .iter()
        .rev()
        .find_map(|entry| {
            entry
                .split_once('=')
                .filter(|(name, _)| {
                    if cfg!(windows) {
                        name.eq_ignore_ascii_case(key)
                    } else {
                        *name == key
                    }
                })
                .map(|(_, value)| value)
        })
        .unwrap_or("")
}
fn error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
fn string_fields(flags: &flags::Flags, mapping: &[(&str, &str)], body: &mut Map<String, Value>) {
    for (flag, field) in mapping {
        let value = flags.value(flag);
        if !value.is_empty() {
            body.insert((*field).into(), value.into());
        }
    }
}
fn parse_role(raw: &str) -> io::Result<(String, String, String)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(error("provider is required"));
    }
    let (rest, effort) = raw
        .split_once('@')
        .map_or((raw, ""), |(rest, effort)| (rest, effort.trim()));
    if raw.contains('@') && effort.is_empty() {
        return Err(error("effort after \"@\" is empty"));
    }
    let (provider, model) = rest.split_once('/').unwrap_or((rest, ""));
    let (provider, model) = (provider.trim(), model.trim());
    if provider.is_empty() {
        return Err(error("provider is required"));
    }
    if provider.contains(' ') || model.contains(' ') || effort.contains(' ') {
        return Err(error("provider/model@effort must not contain spaces"));
    }
    Ok((provider.into(), model.into(), effort.into()))
}
fn timeout_value(raw: &str) -> Duration {
    let raw = raw.trim().strip_prefix('+').unwrap_or(raw.trim());
    let mut text = raw;
    let mut total = 0u128;
    while !text.is_empty() {
        let end = text
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b'.')
            .count();
        if end == 0 {
            return Duration::from_secs(300);
        }
        let number = &text[..end];
        text = &text[end..];
        let unit = if text.starts_with("ns") {
            ("ns", 1)
        } else if text.starts_with("us") {
            ("us", 1000)
        } else if text.starts_with("µs") {
            ("µs", 1000)
        } else if text.starts_with("μs") {
            ("μs", 1000)
        } else if text.starts_with("ms") {
            ("ms", 1_000_000)
        } else if text.starts_with('s') {
            ("s", 1_000_000_000)
        } else if text.starts_with('m') {
            ("m", 60_000_000_000)
        } else if text.starts_with('h') {
            ("h", 3_600_000_000_000)
        } else {
            return Duration::from_secs(300);
        };
        text = &text[unit.0.len()..];
        let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
        if whole.is_empty() && fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Duration::from_secs(300);
        }
        let whole = if whole.is_empty() {
            0
        } else {
            match whole.parse::<u128>() {
                Ok(number) => number,
                Err(_) => return Duration::from_secs(300),
            }
        };
        let fraction = &fraction[..fraction.len().min(18)];
        let numerator = fraction.parse::<u128>().unwrap_or(0);
        let divisor = 10u128.pow(fraction.len() as u32);
        let Some(amount) = whole.checked_mul(unit.1).and_then(|amount| {
            numerator
                .checked_mul(unit.1)
                .and_then(|part| amount.checked_add(part / divisor))
        }) else {
            return Duration::from_secs(300);
        };
        let Some(next) = total.checked_add(amount) else {
            return Duration::from_secs(300);
        };
        total = next;
    }
    if total == 0 || total > i64::MAX as u128 {
        Duration::from_secs(300)
    } else {
        Duration::from_nanos(total as u64)
    }
}
impl OrchestrateCli {
    pub async fn run(&self, args: &[String], cancel: &Cancellation) -> io::Result<String> {
        let command = args
            .first()
            .ok_or_else(|| error("orchestrate <spawn|send|relay>"))?;
        let rest = &args[1..];
        let mut warning = String::new();
        let (body, suffix, role, cwd_requested, filter, status_kind) = match command.as_str() {
            "spawn" => {
                let flags = flags::parse(
                    rest,
                    &[
                        ("role", flags::Kind::String),
                        ("provider", flags::Kind::String),
                        ("model", flags::Kind::String),
                        ("cwd", flags::Kind::String),
                        ("same-tree", flags::Kind::Bool),
                        ("force", flags::Kind::Bool),
                        ("effort", flags::Kind::String),
                        ("execution-mode", flags::Kind::String),
                        ("permission", flags::Kind::String),
                    ],
                )
                .map_err(error)?;
                if flags.help {
                    return Ok(String::new());
                }
                let prompt = flags
                    .positional
                    .first()
                    .ok_or_else(|| error("orchestrate spawn --role <role> \"<prompt>\""))?;
                if flags.value("role").is_empty() {
                    return Err(error("orchestrate spawn: --role is required"));
                }
                let mut body = Map::from_iter([
                    ("role".into(), flags.value("role").into()),
                    ("initial_prompt".into(), prompt.clone().into()),
                ]);
                string_fields(
                    &flags,
                    &[
                        ("provider", "provider"),
                        ("model", "model"),
                        ("cwd", "cwd"),
                        ("effort", "effort"),
                        ("execution-mode", "execution_mode"),
                        ("permission", "permission_preset"),
                    ],
                    &mut body,
                );
                if flags.boolean("same-tree") {
                    body.insert("same_tree".into(), true.into());
                    warning="warning: same-tree mode: the child edits your working tree directly; do not edit the repository in parallel\n".into();
                }
                if flags.boolean("force") {
                    body.insert("force".into(), true.into());
                }
                (
                    Some(Value::Object(body)),
                    "spawn-child",
                    flags.value("role").to_owned(),
                    flags.value("cwd").to_owned(),
                    String::new(),
                    "spawn",
                )
            }
            "send" => {
                let flags = flags::parse(rest, &[("role", flags::Kind::String)]).map_err(error)?;
                if flags.help {
                    return Ok(String::new());
                }
                let text = flags
                    .positional
                    .first()
                    .ok_or_else(|| error("orchestrate send --role <role> \"<text>\""))?;
                if flags.value("role").is_empty() {
                    return Err(error("orchestrate send: --role is required"));
                }
                (
                    Some(json!({"role":flags.value("role"),"text":text})),
                    "send-child",
                    flags.value("role").to_owned(),
                    String::new(),
                    String::new(),
                    "send",
                )
            }
            "relay" => self.relay_plan(rest, &mut warning)?,
            _ => {
                return Err(error(format!(
                    "orchestrate: unknown subcommand {command:?} (want spawn|send|relay)"
                )));
            }
        };
        let session=environment(&self.environment,"MANY_AI_CLI_SESSION_ID").parse::<i64>().ok().filter(|id|*id>0).ok_or_else(||error(format!("orchestrate {command}: this session is not an orchestration session (missing/invalid MANY_AI_CLI_SESSION_ID)")))?;
        let raw_port = environment(&self.environment, "MANY_AI_CLI_HUB_PORT");
        if raw_port.is_empty() {
            return Err(error(format!(
                "orchestrate {command}: MANY_AI_CLI_HUB_PORT is not set"
            )));
        }
        let port = raw_port
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| error("invalid orchestration Hub port"))?;
        if self.paths.is_trial() && port != self.paths.port() {
            return Err(error("trial orchestration Hub port refused"));
        }
        let token = environment(&self.environment, "MANY_AI_CLI_HUB_TOKEN");
        if token.is_empty() {
            return Err(error(format!(
                "orchestrate {command}: MANY_AI_CLI_HUB_TOKEN is not set"
            )));
        }
        let timeout = if body.is_none() {
            Duration::from_secs(30)
        } else {
            timeout_value(environment(&self.environment, "MANY_AI_CLI_SPAWN_TIMEOUT"))
        };
        let post = body.is_some();
        let reply=self.io.exchange(HttpRequest{port,path:format!("/api/sessions/{session}/{suffix}"),token,body,timeout},cancel).await.map_err(|error|match error{RequestError::Timeout if !post=>io::Error::new(io::ErrorKind::TimedOut,"orchestration HTTP get timed out"),RequestError::Timeout=>io::Error::new(io::ErrorKind::TimedOut,"spawn confirmation pending in Hub: user has not yet decided in browser. DO NOT retry spawn (duplicate request will be rejected). The child will start once approved, and its session ID will be delivered via orchestration notification. Once running, instruct it via `orchestrate send`"),RequestError::Cancelled=>io::Error::new(io::ErrorKind::Interrupted,"orchestration request cancelled"),RequestError::Transport=>io::Error::other("orchestration HTTP request failed")})?;
        let result = crate::proto::decode_wire::<ChildReply>(&reply.body)
            .map_err(|_| error(format!("parse response (status {})", reply.status)))?;
        if reply.status != 200 || !result.ok {
            let detail = if result.detail.is_empty() {
                result.error.clone()
            } else {
                result.detail
            };
            let detail = report::redact(&detail.replace(token, "<REDACTED_SECRET>"));
            return Err(error(if result.error == "spawn_refused" {
                format!("spawn refused by the user: {detail}")
            } else {
                format!("hub returned {}: {detail}", reply.status)
            }));
        }
        let text = match status_kind {
            "spawn" => {
                let cwd = if !cwd_requested.trim().is_empty() && result.cwd != cwd_requested {
                    format!("{} (requested={cwd_requested})", result.cwd)
                } else {
                    result.cwd
                };
                format!(
                    "spawned child session #{} role={role} board={} cwd={cwd}\n",
                    result.session_id, result.board_path
                )
            }
            "send" => format!(
                "sent instruction to child session #{} role={role} board={}\n",
                result.session_id, result.board_path
            ),
            "relay-start" => {
                let relay = result
                    .relay
                    .ok_or_else(|| error("hub returned no relay status"))?;
                format!(
                    "relay started orchestration={} mode={} branch={} worktree={} implementation=#{} max_rounds={}\n",
                    relay.orchestration_id,
                    relay.mode,
                    relay.branch,
                    relay.worktree_path,
                    relay.implementation_session_id,
                    relay.max_rounds
                )
            }
            "relay-stop" => {
                let relay = result
                    .relay
                    .ok_or_else(|| error("hub returned no relay status"))?;
                format!(
                    "relay stopped orchestration={} state={} completed_cs={} round={}/{} branch={}\n",
                    relay.orchestration_id,
                    relay.state,
                    relay.completed_cs,
                    relay.round,
                    relay.max_rounds,
                    relay.branch
                )
            }
            _ => {
                let mut output = String::new();
                for item in result.relays.unwrap_or_default() {
                    let relay = item.relay;
                    if !filter.is_empty() && relay.orchestration_id != filter {
                        continue;
                    }
                    output.push_str(&format!(
                        "relay orchestration={} state={} completed_cs={} round={}/{} branch={}\n",
                        relay.orchestration_id,
                        relay.state,
                        relay.completed_cs,
                        relay.round,
                        relay.max_rounds,
                        relay.branch
                    ));
                }
                if output.is_empty() {
                    if !filter.is_empty() {
                        return Err(error(format!(
                            "orchestrate relay status: relay {filter:?} not found"
                        )));
                    }
                    output = "no relays\n".into();
                }
                output
            }
        };
        Ok(report::redact(
            &format!("{warning}{text}").replace(token, "<REDACTED_SECRET>"),
        ))
    }
    fn relay_plan(&self, args: &[String], warning: &mut String) -> io::Result<RelayPlan> {
        let action = args.first().map(String::as_str).unwrap_or("");
        if matches!(action, "status" | "stop") {
            let flags = flags::parse(&args[1..], &[("id", flags::Kind::String)]).map_err(error)?;
            if flags.help {
                return Err(error("relay help"));
            }
            if !flags.positional.is_empty() {
                return Err(error(format!(
                    "orchestrate relay {action} [--id <orchestration-id>]"
                )));
            }
            let id = flags.value("id").trim().to_owned();
            return Ok((
                if action == "stop" {
                    Some(if id.is_empty() {
                        json!({})
                    } else {
                        json!({"orchestration_id":id})
                    })
                } else {
                    None
                },
                if action == "stop" {
                    "relay-stop"
                } else {
                    "relay"
                },
                String::new(),
                String::new(),
                id,
                if action == "stop" {
                    "relay-stop"
                } else {
                    "relay-status"
                },
            ));
        }
        if !action.is_empty() && !action.starts_with('-') {
            return Err(error(format!(
                "orchestrate relay: unknown action {action:?} (want status|stop, or pass --plan to start)"
            )));
        }
        let flags = flags::parse(
            args,
            &[
                ("plan", flags::Kind::String),
                ("max-rounds", flags::Kind::Int),
                ("impl", flags::Kind::String),
                ("review", flags::Kind::String),
                ("strong", flags::Kind::String),
                ("escalate-after", flags::Kind::Int),
                ("same-tree", flags::Kind::Bool),
                ("extra-impl", flags::Kind::String),
                ("extra-review", flags::Kind::String),
                ("execution-mode", flags::Kind::String),
                ("permission", flags::Kind::String),
            ],
        )
        .map_err(error)?;
        let plan = flags.value("plan").trim();
        if plan.is_empty() {
            return Err(error("orchestrate relay: --plan is required"));
        }
        let plan = std::path::absolute(self.cwd.join(plan))?;
        let mut roles = Map::new();
        for (flag, role) in [
            ("impl", "implementation"),
            ("review", "review"),
            ("strong", "implementation-strong"),
        ] {
            if flags.value(flag).trim().is_empty() {
                continue;
            }
            let (provider, model, effort) = parse_role(flags.value(flag))
                .map_err(|err| error(format!("orchestrate relay --{flag}: {err}")))?;
            let mut value = Map::from_iter([("provider".into(), provider.into())]);
            for (key, value_text) in [
                ("model", model.as_str()),
                ("effort", effort.as_str()),
                ("execution_mode", flags.value("execution-mode")),
                ("permission_preset", flags.value("permission")),
            ] {
                if !value_text.is_empty() {
                    value.insert(key.into(), value_text.into());
                }
            }
            roles.insert(role.into(), Value::Object(value));
        }
        let mut body=Map::from_iter([("plan_path".into(),plan.to_string_lossy().into_owned().into()),("mode".into(),if flags.boolean("same-tree"){*warning="warning: same-tree mode: the relay children edit your working tree directly; do not edit the repository in parallel\n".into();"same-tree"}else{"worktree"}.into()),("acknowledge_child_full_bypass".into(),true.into())]);
        for key in ["max-rounds", "escalate-after"] {
            let value = flags.integer(key);
            if value != 0 {
                body.insert(key.replace('-', "_"), value.into());
            }
        }
        if !roles.is_empty() {
            body.insert("roles".into(), roles.into());
        }
        let mut extra = Map::new();
        for (flag, role) in [("extra-impl", "implementation"), ("extra-review", "review")] {
            if !flags.value(flag).trim().is_empty() {
                extra.insert(role.into(), flags.value(flag).into());
            }
        }
        if !extra.is_empty() {
            body.insert("extra".into(), extra.into());
        }
        Ok((
            Some(body.into()),
            "relay",
            String::new(),
            String::new(),
            String::new(),
            "relay-start",
        ))
    }
}
type RelayPlan = (
    Option<Value>,
    &'static str,
    String,
    String,
    String,
    &'static str,
);
#[cfg(test)]
mod tests;
