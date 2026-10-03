//! The local argv, SSH join and remote shell are separate parsing boundaries.
use super::{Profile, profile::space_or_control};
use crate::config::RuntimePaths;
use std::sync::OnceLock;

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn ssh_target(p: &Profile) -> String {
    if p.user.is_empty() {
        p.host.clone()
    } else {
        format!("{}@{}", p.user, p.host)
    }
}
pub fn ssh_base_args(p: &Profile) -> Vec<String> {
    let mut args = [
        "-o",
        "BatchMode=yes",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "ServerAliveInterval=30",
    ]
    .map(String::from)
    .to_vec();
    if p.ssh_port > 0 {
        args.extend(["-p".into(), p.ssh_port.to_string()]);
    }
    if !p.identity_file.is_empty() {
        args.extend(["-i".into(), p.identity_file.clone()]);
    }
    args
}
/// Trial mode must explicitly name the candidate executable and remote root.
/// A local runtime root is not assumed to have the same meaning on another host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTrial {
    pub root: String,
    pub binary: String,
    pub(crate) connection_id: Option<String>,
}
impl RemoteTrial {
    pub fn new(root: impl Into<String>, binary: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            binary: binary.into(),
            connection_id: None,
        }
    }
    pub fn command_identity(&self) -> String {
        match &self.connection_id {
            Some(id) => format!("{} [many-ai-connection={id}]", self.binary),
            None => self.binary.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.root.starts_with('/')
            || self.root == "/"
            || self.root.chars().any(|c| c == '\0')
            || self.root.split('/').any(|c| c == "..")
        {
            return Err("remote trial root must be an explicit absolute non-root path without parent traversal".into());
        }
        if !self.binary.starts_with('/') || self.binary.chars().any(|c| c == '\0') {
            return Err("remote trial executable must be an explicit absolute path".into());
        }
        Ok(())
    }
}
pub fn validate_remote_scope(
    paths: &RuntimePaths,
    scope: Option<&RemoteTrial>,
) -> Result<(), String> {
    if paths.is_trial() && scope.is_none() {
        return Err(
            "trial connection requires an explicit remote candidate root and executable".into(),
        );
    }
    if !paths.is_trial() && scope.is_some() {
        return Err("remote trial scope requires local trial mode".into());
    }
    if let Some(scope) = scope {
        scope.validate()?;
    }
    Ok(())
}
pub fn remote_command_prefix(p: &Profile, port: i64, scope: Option<&RemoteTrial>) -> String {
    match scope {
        Some(scope) => format!(
            "{} --trial-root {} --trial-port {port}",
            shell_quote(&scope.binary),
            shell_quote(&scope.root)
        ),
        None => shell_quote(p.binary()),
    }
}
pub fn ssh_serve_script(p: &Profile, port: i64, scope: Option<&RemoteTrial>) -> String {
    let command = if let Some(scope) = scope.filter(|scope| scope.connection_id.is_some()) {
        format!(
            "MANY_AI_CLI_HOST_LABEL={} exec -a {} {} serve --port {port}",
            shell_quote(&p.host),
            shell_quote(&scope.command_identity()),
            remote_command_prefix(p, port, Some(scope))
        )
    } else {
        format!(
            "exec env MANY_AI_CLI_HOST_LABEL={} {} serve --port {port}",
            shell_quote(&p.host),
            remote_command_prefix(p, port, scope)
        )
    };
    if p.cwd.is_empty() {
        command
    } else {
        format!("cd {} && {command}", shell_quote(&p.cwd))
    }
}
pub fn ssh_serve_args(p: &Profile, port: i64, scope: Option<&RemoteTrial>) -> Vec<String> {
    let mut args = ssh_base_args(p);
    args.extend([
        "-t".into(),
        "-L".into(),
        format!("127.0.0.1:{port}:127.0.0.1:{port}"),
        ssh_target(p),
    ]);
    // OpenSSH joins these elements with spaces. Quote the whole script as one
    // shell argument as required by the migration supplement (Go omitted this).
    args.extend([
        "--".into(),
        "bash".into(),
        "-ilc".into(),
        shell_quote(&ssh_serve_script(p, port, scope)),
    ]);
    args
}
pub fn ssh_import_args(p: &Profile, scope: Option<&RemoteTrial>, trial_port: u16) -> Vec<String> {
    let mut args = ssh_base_args(p);
    args.push(ssh_target(p));
    let script = format!(
        "{} profile-export --json",
        remote_command_prefix(p, i64::from(trial_port), scope)
    );
    args.extend([
        "--".into(),
        "bash".into(),
        "-lc".into(),
        shell_quote(&script),
    ]);
    args
}
pub fn ssh_tunnel_args(p: &Profile) -> Vec<String> {
    let mut args = ssh_base_args(p);
    args.extend([
        "-N".into(),
        "-L".into(),
        format!("127.0.0.1:{}:127.0.0.1:{}", p.hub_port, p.hub_port),
        ssh_target(p),
    ]);
    args
}
pub fn ssh_token_args(p: &Profile) -> Vec<String> {
    let mut args = ssh_base_args(p);
    // This already IS the complete remote shell script. OpenSSH's join leaves
    // its bytes unchanged; another bash layer would change login-shell semantics.
    args.extend([ssh_target(p), "--".into(), p.token_command.clone()]);
    args
}
/// POSIX ERE quoting matching regexp.QuoteMeta, not Rust regex's wider escaping.
pub fn quote_ere(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if "\\.+*?()|[]{}^$".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}
pub fn cleanup_pattern(p: &Profile, port: i64, scope: Option<&RemoteTrial>) -> String {
    // Anchor the complete argv-shaped suffix: a port prefix must never kill
    // another port, and a candidate root must never select a Go/default instance.
    let prefix = match scope {
        Some(scope) => format!(
            "{} --trial-root {} --trial-port {port}",
            scope.command_identity(),
            scope.root
        ),
        None => p.binary().to_owned(),
    };
    format!("^{} serve --port {port}$", quote_ere(&prefix))
}
pub fn ssh_cleanup_args(p: &Profile, port: i64, scope: Option<&RemoteTrial>) -> Vec<String> {
    let mut args = ssh_base_args(p);
    args.extend([
        ssh_target(p),
        "--".into(),
        "pkill".into(),
        "-f".into(),
        shell_quote(&cleanup_pattern(p, port, scope)),
    ]);
    args
}
pub fn wsl_serve_args(p: &Profile, port: i64, scope: Option<&RemoteTrial>) -> Vec<String> {
    let mut args = Vec::new();
    if !p.distro.is_empty() {
        args.extend(["-d".into(), p.distro.clone()]);
    }
    args.extend([
        "--cd".into(),
        if p.cwd.is_empty() {
            "~".into()
        } else {
            p.cwd.clone()
        },
    ]);
    let identity = scope
        .filter(|scope| scope.connection_id.is_some())
        .map(|scope| format!("-a {} ", shell_quote(&scope.command_identity())))
        .unwrap_or_default();
    let script = format!(
        "export MANY_AI_CLI_WSL_LAUNCHER=1; exec {identity}{} serve --port {port}",
        remote_command_prefix(p, port, scope)
    );
    // WSL passes argv directly: do not apply SSH's extra shell-join quotation.
    args.extend(["--".into(), "bash".into(), "-ilc".into(), script]);
    args
}
pub fn wsl_cleanup_args(p: &Profile, port: i64, scope: Option<&RemoteTrial>) -> Vec<String> {
    let mut args = Vec::new();
    if !p.distro.is_empty() {
        args.extend(["-d".into(), p.distro.clone()]);
    }
    args.extend([
        "--".into(),
        "pkill".into(),
        "-f".into(),
        cleanup_pattern(p, port, scope),
    ]);
    args
}
/// net/url.QueryEscape's RFC3986 unreserved set (form_urlencoded escapes '~').
pub fn query_escape(value: &str) -> String {
    let mut out = String::new();
    use std::fmt::Write;
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            b => write!(out, "%{b:02X}").unwrap(),
        }
    }
    out
}
pub fn tunnel_hub_url(port: i64, token: &str, host: &str) -> String {
    format!(
        "http://127.0.0.1:{port}/?token={}&via=ssh&host_label={}&env_kind=remote-tunnel",
        query_escape(token),
        query_escape(host)
    )
}
pub fn normalize_hub_token(token: &str) -> Result<&str, String> {
    if token.is_empty() {
        return Err("empty token".into());
    }
    if token.chars().any(space_or_control) {
        return Err("token must not contain whitespace or control characters".into());
    }
    Ok(token)
}
pub fn find_hub_url(line: &[u8]) -> Option<String> {
    static RE: OnceLock<regex::bytes::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::bytes::Regex::new(r"http://127\.0\.0\.1:[0-9]+/\?token=[0-9a-fA-F]+").unwrap()
    })
    .find(line)
    .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
}
/// Mirrors ScanLines: remove one trailing CR, preserve bytes, emit CRLF, 1MiB cap.
#[derive(Default)]
pub struct UrlScanner {
    buffer: Vec<u8>,
    stopped: bool,
}
impl UrlScanner {
    pub fn feed(&mut self, bytes: &[u8], eof: bool) -> Vec<(Vec<u8>, Option<String>)> {
        if self.stopped {
            return vec![];
        }
        let mut result = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                result.push(self.line());
            } else {
                self.buffer.push(byte);
                if self.buffer.len() >= 1024 * 1024 {
                    self.buffer.clear();
                    self.stopped = true;
                    break;
                }
            }
        }
        if eof && !self.stopped && !self.buffer.is_empty() {
            result.push(self.line());
        }
        result
    }
    fn line(&mut self) -> (Vec<u8>, Option<String>) {
        let mut line = std::mem::take(&mut self.buffer);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let url = find_hub_url(&line);
        line.extend_from_slice(b"\r\n");
        (line, url)
    }
}
