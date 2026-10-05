//! Hidden usage-relay command. Only numeric metadata leaves this process.
mod input;
mod parser;
#[cfg(test)]
mod tests;
use super::session_usage::UsageRequest;
use crate::{
    config::RuntimePaths,
    proto::{core::CoreFuture, time::Timestamp},
};
use serde_json::Value;
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
};
pub type RelayWarning<'a> = dyn Fn(&'static str) + Send + Sync + 'a;
#[derive(Debug)]
pub struct RelayError(pub String);
impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for RelayError {}
/// Token intentionally has no Debug or public accessor.
pub struct RelayOptions {
    pub provider: String,
    pub hub: String,
    pub session: i64,
    token: String,
}
fn integer(raw: &str) -> Option<i64> {
    let (negative, raw) = if let Some(v) = raw.strip_prefix('-') {
        (true, v)
    } else {
        (false, raw.strip_prefix('+').unwrap_or(raw))
    };
    if raw.is_empty() {
        return None;
    }
    let (radix, digits, prefix) = if raw.starts_with("0x") || raw.starts_with("0X") {
        (16, &raw[2..], true)
    } else if raw.starts_with("0b") || raw.starts_with("0B") {
        (2, &raw[2..], true)
    } else if raw.starts_with("0o") || raw.starts_with("0O") {
        (8, &raw[2..], true)
    } else if raw.len() > 1 && raw.starts_with('0') {
        (8, &raw[1..], true)
    } else {
        (10, raw, false)
    };
    let mut previous = prefix;
    for (i, c) in digits.chars().enumerate() {
        if c == '_' {
            if !previous || i + 1 == digits.len() {
                return None;
            }
            previous = false;
        } else if c.is_digit(radix) {
            previous = true;
        } else {
            return None;
        }
    }
    if !previous {
        return None;
    }
    let value = u64::from_str_radix(&digits.replace('_', ""), radix).ok()?;
    if negative {
        if value == 1u64 << 63 {
            Some(i64::MIN)
        } else {
            i64::try_from(value).ok().map(|v| -v)
        }
    } else {
        i64::try_from(value).ok()
    }
}
pub fn parse(
    args: &[String],
    environment: &[String],
    warning: &RelayWarning<'_>,
) -> Result<RelayOptions, RelayError> {
    let mut options = RelayOptions {
        provider: String::new(),
        hub: String::new(),
        session: 0,
        token: String::new(),
    };
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" || arg == "-" || !arg.starts_with('-') {
            break;
        }
        index += 1;
        let flag = arg.strip_prefix("--").unwrap_or(&arg[1..]);
        if flag.is_empty() || flag.starts_with('-') {
            return Err(RelayError(format!("bad flag syntax: {arg}")));
        }
        let (name, inline) = flag
            .split_once('=')
            .map_or((flag, None), |(k, v)| (k, Some(v)));
        if matches!(name, "h" | "help") {
            return Err(RelayError("flag: help requested".into()));
        }
        if !matches!(name, "provider" | "hub" | "token" | "session") {
            return Err(RelayError(format!(
                "flag provided but not defined: -{name}"
            )));
        }
        let value = match inline {
            Some(v) => v,
            None => {
                let value = args
                    .get(index)
                    .ok_or_else(|| RelayError(format!("flag needs an argument: -{name}")))?;
                index += 1;
                value
            }
        };
        match name {
            "provider" => options.provider = value.into(),
            "hub" => options.hub = value.into(),
            "token" => options.token = value.into(),
            "session" => {
                options.session = integer(value).ok_or_else(|| {
                    RelayError(format!(
                        "invalid value {} for flag -session: parse error",
                        crate::proto::go_quote::quote(value)
                    ))
                })?
            }
            _ => unreachable!(),
        }
    }
    let environment_token = environment
        .iter()
        .rev()
        .find_map(|v| {
            v.split_once('=')
                .filter(|(k, _)| *k == "MANY_AI_CLI_HUB_TOKEN")
                .map(|(_, v)| v)
        })
        .unwrap_or("");
    if !environment_token.is_empty() {
        options.token = environment_token.into();
    } else if !options.token.is_empty() {
        warning("deprecated token argument");
    }
    if !matches!(options.provider.as_str(), "claude" | "codex") {
        return Err(RelayError(format!(
            "usage-relay: --provider must be claude or codex (got {})",
            crate::proto::go_quote::quote(&options.provider)
        )));
    }
    Ok(options)
}
pub trait RelayIo: Send + Sync {
    fn now(&self) -> Timestamp;
    fn rollout(&self, path: &str) -> io::Result<Box<dyn BufRead + Send>>;
    fn post<'a>(
        &'a self,
        hub: &'a str,
        token: &'a str,
        payload: &'a Value,
    ) -> CoreFuture<'a, io::Result<()>>;
}
pub struct NativeRelayIo {
    trial: Option<RuntimePaths>,
}
impl NativeRelayIo {
    pub fn new(trial: Option<RuntimePaths>) -> Self {
        Self { trial }
    }
    pub fn validate_target(&self, hub: &str) -> io::Result<()> {
        validate_hub_url(hub)?;
        if let Some(paths) = &self.trial {
            let url = url::Url::parse(hub).map_err(io::Error::other)?;
            if url.port_or_known_default() != Some(paths.port()) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial relay Hub port mismatch",
                ));
            }
        }
        Ok(())
    }
}
pub fn validate_hub_url(raw: &str) -> io::Result<()> {
    if raw.trim().is_empty() || raw.bytes().any(|b| b < 32 || b == 127) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty Hub URL"));
    }
    let url = url::Url::parse(raw)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid Hub URL"))?;
    // net/url preserves host spelling; WHATWG parsing canonicalizes localhost
    // and abbreviated IPs, which would widen Go's explicit allowlist.
    let authority = raw
        .split_once("://")
        .filter(|(scheme, _)| {
            scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
        })
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid Hub URL"))?;
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split_once(']').map(|(host, _)| host).unwrap_or("")
    } else {
        host_port
            .rsplit_once(':')
            .map_or(host_port, |(host, _)| host)
    };
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(host, "127.0.0.1" | "localhost" | "::1")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Hub URL must be loopback HTTP or HTTPS",
        ));
    }
    Ok(())
}
impl RelayIo for NativeRelayIo {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
    fn rollout(&self, path: &str) -> io::Result<Box<dyn BufRead + Send>> {
        if let Some(paths) = &self.trial {
            crate::profile::subscriptions::check_path(paths, Path::new(path))?;
        }
        Ok(Box::new(BufReader::with_capacity(
            64 * 1024,
            std::fs::File::open(path)?,
        )))
    }
    fn post<'a>(
        &'a self,
        hub: &'a str,
        token: &'a str,
        payload: &'a Value,
    ) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            self.validate_target(hub)?;
            let bytes = crate::proto::provider::to_go_json(payload).map_err(io::Error::other)?;
            let client = reqwest::Client::builder()
                .no_proxy()
                // Intentional trial/auth boundary: Go follows redirects, but
                // a relay must not carry trial data to a second Hub or origin.
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(io::Error::other)?;
            let mut request = client
                .post(format!("{hub}/api/session-usage"))
                .header("Content-Type", "application/json")
                .body(bytes);
            if !token.is_empty() {
                request = request.bearer_auth(token);
            }
            let response = request
                .send()
                .await
                .map_err(|_| io::Error::other("Hub usage POST failed"))?;
            if response.status() != reqwest::StatusCode::OK {
                return Err(io::Error::other("Hub usage POST rejected"));
            }
            Ok(())
        })
    }
}
fn payload(request: UsageRequest) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(request)?;
    if let Some(fields) = value.as_object_mut() {
        fields.retain(|key, v| {
            matches!(key.as_str(), "provider" | "session_id" | "cost_from_relay")
                || !(v == &Value::Bool(false)
                    || v.as_str() == Some("")
                    || v.as_i64() == Some(0)
                    || v.as_f64() == Some(0.0))
        });
    }
    Ok(value)
}
pub async fn run_with_io(
    args: &[String],
    environment: &[String],
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    io: &dyn RelayIo,
    warning: &RelayWarning<'_>,
) -> Result<(), RelayError> {
    let options = parse(args, environment, warning)?;
    let mut raw = vec![];
    if stdin.read_to_end(&mut raw).is_err() {
        warning("stdin read failed");
        return Ok(());
    }
    let mut request = if options.provider == "claude" {
        match parser::claude(&raw, io.now(), io.now()) {
            Ok((status, request)) => {
                let _ = stdout.write_all(status.as_bytes());
                request
            }
            Err(_) => {
                warning("Claude JSON parse failed");
                return Ok(());
            }
        }
    } else {
        let stop: input::Stop = match crate::proto::wire::decode(&raw) {
            Ok(value) => value,
            Err(_) => {
                warning("Codex JSON parse failed");
                return Ok(());
            }
        };
        if stop.transcript_path.is_empty() {
            warning("Codex transcript path absent");
            return Ok(());
        }
        let mut rollout = match io.rollout(&stop.transcript_path) {
            Ok(value) => value,
            Err(_) => {
                warning("Codex rollout scan failed");
                return Ok(());
            }
        };
        let scan = match parser::scan(&mut *rollout) {
            Ok(value) => value,
            Err(_) => {
                warning("Codex rollout scan failed");
                return Ok(());
            }
        };
        parser::codex_payload(stop, scan, io.now())
    };
    if options.hub.is_empty() || options.token.is_empty() || options.session <= 0 {
        return Ok(());
    }
    request.session_id = options.session;
    let value = match payload(request) {
        Ok(value) => value,
        Err(_) => {
            warning("usage marshal failed");
            return Ok(());
        }
    };
    if io.post(&options.hub, &options.token, &value).await.is_err() {
        warning("usage POST failed");
    }
    Ok(())
}
/// Invoke before loading main configuration, exactly like fixed Go's hidden command.
pub async fn run_native(
    args: &[String],
    environment: &[String],
    trial: Option<RuntimePaths>,
) -> Result<(), RelayError> {
    let io = NativeRelayIo::new(trial);
    let warning = |operation| eprintln!("usage-relay: {operation}");
    let options = parse(args, environment, &|_| {})?;
    if !options.hub.is_empty() && io.validate_target(&options.hub).is_err() {
        warning("Hub URL rejected");
        return Ok(());
    }
    run_with_io(
        args,
        environment,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &io,
        &warning,
    )
    .await
}
