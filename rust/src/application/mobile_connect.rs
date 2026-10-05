//! Authenticated mobile connection metadata and contained local service probes.
pub mod native;
#[cfg(test)]
mod tests;
use crate::{
    config::{ConfigError, ConfigStore},
    launcher::ExportIdentity,
    process::Cancellation,
    proto::{
        core::CoreFuture,
        unicode::{simple_fold_key, simple_lower},
        wire::{Field, GoWire, Schema},
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    Tailscale,
    ServiceControl,
    Systemctl,
    Pgrep,
    Sshd,
}
#[derive(Default)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}
pub trait MobileIo: Send + Sync {
    fn available(&self, program: Program) -> bool;
    /// Each command has a five-second deadline and follows request/Hub
    /// cancellation. No authentication token is passed to the process.
    fn run<'a>(
        &'a self,
        program: Program,
        args: &'a [&'a str],
        combined: bool,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, CommandOutput>;
    fn windows(&self) -> bool;
}
pub type Warning = dyn Fn(&'static str) + Send + Sync;
pub struct MobileResponse {
    pub value: Value,
    pub no_store: bool,
}
#[derive(Clone)]
pub struct MobileIdentity {
    pub export: ExportIdentity,
    pub host_label: String,
}
impl MobileIdentity {
    pub fn from_export(export: ExportIdentity, environment: &[String]) -> Self {
        let host_label = environment
            .iter()
            .rev()
            .find_map(|entry| {
                entry
                    .split_once('=')
                    .filter(|(key, _)| {
                        if cfg!(windows) {
                            key.eq_ignore_ascii_case("MANY_AI_CLI_HOST_LABEL")
                        } else {
                            *key == "MANY_AI_CLI_HOST_LABEL"
                        }
                    })
                    .map(|(_, v)| v.to_owned())
            })
            .unwrap_or_default();
        Self { export, host_label }
    }
    fn lan_ip(&self) -> String {
        if !self.host_label.is_empty() {
            return self.host_label.clone();
        }
        if let Some(ip) = self.export.ssh_connection.split_whitespace().nth(2) {
            return ip.to_owned();
        }
        self.export
            .ipv4
            .iter()
            .find(|ip| !ip.is_loopback() && !ip.is_link_local())
            .map(ToString::to_string)
            .unwrap_or_default()
    }
    fn hosts(&self) -> Vec<String> {
        reachable_hosts(&[
            &self.export.public_host,
            &self.lan_ip(),
            &self.export.hostname,
        ])
    }
}
pub fn trim_space(value: &str) -> &str {
    value.trim_matches(|c| {
        matches!(
            c,
            '\t' | '\n'
                | '\u{b}'
                | '\u{c}'
                | '\r'
                | ' '
                | '\u{85}'
                | '\u{a0}'
                | '\u{1680}'
                | '\u{2000}'
                ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
        )
    })
}
pub fn normalize_host(value: &str) -> &str {
    let value = trim_space(value);
    value.strip_suffix('.').unwrap_or(value)
}
pub fn reachable_hosts(values: &[&str]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .filter_map(|v| {
            let host = normalize_host(v);
            (!host.is_empty() && seen.insert(simple_lower(host))).then(|| host.to_owned())
        })
        .collect()
}
pub fn query_escape(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char)
            }
            b' ' => result.push('+'),
            _ => {
                use std::fmt::Write;
                write!(result, "%{byte:02X}").expect("string write");
            }
        }
    }
    result
}
pub fn https_url(dns: &str, token: &str) -> String {
    if dns.is_empty() {
        String::new()
    } else {
        format!("https://{dns}/?token={}", query_escape(token))
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Status {
    #[serde(rename = "BackendState")]
    backend: String,
    #[serde(rename = "Self")]
    own: OwnStatus,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct OwnStatus {
    #[serde(rename = "DNSName")]
    dns: String,
    #[serde(rename = "Online")]
    online: bool,
}
impl GoWire for Status {
    const GO_TYPE: &'static str = "MobileTailscaleStatus";
    const SCHEMAS: &'static [Schema] = &[
        Schema {
            name: "MobileTailscaleStatus",
            fields: &[
                Field {
                    name: "BackendState",
                    kind: "string",
                },
                Field {
                    name: "Self",
                    kind: "MobileTailscaleSelf",
                },
            ],
        },
        Schema {
            name: "MobileTailscaleSelf",
            fields: &[
                Field {
                    name: "DNSName",
                    kind: "string",
                },
                Field {
                    name: "Online",
                    kind: "bool",
                },
            ],
        },
    ];
}
#[derive(Default)]
struct TailState {
    state: &'static str,
    dns: String,
    online: bool,
    admin: String,
}
pub struct MobileConnect {
    config: Arc<ConfigStore>,
    io: Arc<dyn MobileIo>,
    identity: MobileIdentity,
    port: u16,
    warning: Arc<Warning>,
}
impl MobileConnect {
    pub fn new(
        config: Arc<ConfigStore>,
        io: Arc<dyn MobileIo>,
        identity: MobileIdentity,
        port: u16,
        warning: Arc<Warning>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            io,
            identity,
            port,
            warning,
        })
    }
    async fn command(&self, args: &[&str], cancel: &Cancellation) -> CommandOutput {
        self.io.run(Program::Tailscale, args, false, cancel).await
    }
    async fn probe(&self, cancel: &Cancellation) -> TailState {
        let mut st = TailState {
            state: "not_installed",
            ..Default::default()
        };
        if !self.io.available(Program::Tailscale) {
            return st;
        }
        let out = self.command(&["status", "--json"], cancel).await;
        if !out.success {
            return st;
        }
        let Ok(status) = crate::proto::wire::decode::<Status>(out.stdout.as_bytes()) else {
            return st;
        };
        st.dns = normalize_host(&status.own.dns).into();
        st.online = status.own.online;
        if simple_fold_key(&status.backend) != simple_fold_key("Running") {
            st.state = "not_logged_in";
            return st;
        }
        let out = self.command(&["serve", "status"], cancel).await;
        st.state = if out.success && out.stdout.contains(&format!("127.0.0.1:{}", self.port)) {
            "ready"
        } else {
            "serve_inactive"
        };
        st
    }
    pub async fn metadata(&self, cancel: &Cancellation) -> Result<MobileResponse, ConfigError> {
        let sshd = self.sshd(cancel).await;
        let token = self.config.snapshot()?.config.token;
        let ip = self.identity.lan_ip();
        let user = if self.identity.export.username.is_empty() {
            &self.identity.export.user
        } else {
            &self.identity.export.username
        };
        let forward = format!(
            "http://127.0.0.1:{}/?token={}",
            self.port,
            query_escape(&token)
        );
        Ok(MobileResponse {
            no_store: true,
            value: json!({"lan_ip":ip,"ssh_user":user,"ssh_port":22,"hub_port":self.port,"ssh_command":format!("ssh -L {}:127.0.0.1:{} {}@{}",self.port,self.port,user,ip),"ssh_url":format!("ssh://{user}@{ip}:22"),"hub_url":forward,"ssh_forward_url":forward,"host_candidates":self.identity.hosts(),"sshd_state":sshd,"sshd_installed":matches!(sshd,"stopped"|"running"),"sshd_running":sshd=="running","wireguard":{"url_template":format!("http://<wg-server-ip>:{}/?token={}",self.port,query_escape(&token)),"url_template_https":format!("https://<wg-server-ip>/?token={}",query_escape(&token)),"note":"WireGuard サーバ IP は接続後にユーザーが指定する前提（Hub は WG 設定を持たない）。生IP は secure context 外のため通知/音声/PWA が制限される。HTTPS 化を推奨。"}}),
        })
    }
    async fn sshd(&self, cancel: &Cancellation) -> &'static str {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        if self.io.windows() {
            let out = self
                .sshd_command(
                    Program::ServiceControl,
                    &["query", "sshd"],
                    true,
                    cancel,
                    deadline,
                )
                .await;
            return classify_windows_sshd(&out.stdout, out.success);
        }
        if self.io.available(Program::Systemctl) {
            for unit in ["sshd", "ssh"] {
                let out = self
                    .sshd_command(
                        Program::Systemctl,
                        &["is-active", unit],
                        false,
                        cancel,
                        deadline,
                    )
                    .await;
                match trim_space(&out.stdout) {
                    "active" => return "running",
                    "inactive" | "failed" | "deactivating" => return "stopped",
                    _ => {}
                }
            }
        }
        if self.io.available(Program::Pgrep)
            && self
                .sshd_command(Program::Pgrep, &["-x", "sshd"], false, cancel, deadline)
                .await
                .success
        {
            return "running";
        }
        if self.io.available(Program::Sshd) {
            "stopped"
        } else {
            "not_installed"
        }
    }
    async fn sshd_command(
        &self,
        program: Program,
        args: &[&str],
        combined: bool,
        cancel: &Cancellation,
        deadline: tokio::time::Instant,
    ) -> CommandOutput {
        if deadline <= tokio::time::Instant::now() || cancel.is_cancelled() {
            return CommandOutput::default();
        }
        tokio::time::timeout_at(deadline, self.io.run(program, args, combined, cancel))
            .await
            .unwrap_or_default()
    }
    pub async fn status(&self, cancel: &Cancellation) -> Result<MobileResponse, ConfigError> {
        let st = self.probe(cancel).await;
        let mut value = json!({"state":st.state,"dns_name":st.dns,"online":st.online,"hub_port":self.port,"serve_command":format!("tailscale serve --bg {}",self.port),"serve_off_command":"tailscale serve --https=443 off"});
        if !st.admin.is_empty() {
            value["admin_url"] = st.admin.into();
        }
        if st.state == "ready" {
            value["https_url"] = https_url(&st.dns, &self.config.snapshot()?.config.token).into();
        }
        Ok(MobileResponse {
            value,
            no_store: true,
        })
    }
    pub async fn enable(&self, cancel: &Cancellation) -> Result<MobileResponse, ConfigError> {
        let mut st = self.probe(cancel).await;
        if !matches!(st.state, "not_installed" | "not_logged_in") {
            let port = self.port.to_string();
            let out = self.command(&["serve", "--bg", &port], cancel).await;
            st.state = if out.success {
                "ready"
            } else {
                st.admin = admin_url(&out.stderr);
                if !st.admin.is_empty() || simple_lower(&out.stderr).contains("not enabled") {
                    "serve_disabled_on_tailnet"
                } else {
                    "serve_inactive"
                }
            };
        }
        let mut value =
            json!({"state":st.state,"dns_name":st.dns,"hub_port":self.port,"ok":st.state=="ready"});
        let no_store = st.state == "ready";
        if !st.admin.is_empty() {
            value["admin_url"] = st.admin.into();
        }
        if no_store {
            match self.add_allowed_host(&st.dns) {
                Ok(added) => value["allowed_host_added"] = added.into(),
                Err(_) => {
                    (self.warning)("mobile_allowed_host_persist");
                    value["allowed_host_added"] = false.into();
                    value["allowed_host_hint"] = st.dns.clone().into();
                }
            }
            value["https_url"] = https_url(&st.dns, &self.config.snapshot()?.config.token).into();
        }
        Ok(MobileResponse { value, no_store })
    }
    fn add_allowed_host(&self, host: &str) -> Result<bool, ConfigError> {
        let host = normalize_host(host);
        if host.is_empty() {
            return Ok(false);
        }
        loop {
            let mut snapshot = self.config.snapshot()?;
            if snapshot
                .config
                .hub
                .allowed_hosts
                .iter()
                .any(|h| simple_fold_key(normalize_host(h)) == simple_fold_key(host))
            {
                return Ok(false);
            }
            snapshot.config.hub.allowed_hosts.push(host.into());
            match self
                .config
                .publish_then_persist_legacy(snapshot.revision, snapshot.config)
            {
                Err(ConfigError::Conflict { .. }) => continue,
                Err(error) => return Err(error),
                Ok(_) => return Ok(true),
            }
        }
    }
    pub async fn disable(&self, cancel: &Cancellation) -> MobileResponse {
        let ok = self.io.available(Program::Tailscale)
            && self
                .command(&["serve", "--https=443", "off"], cancel)
                .await
                .success;
        if !ok {
            (self.warning)("mobile_tailscale_off");
        }
        MobileResponse {
            value: json!({"ok":ok}),
            no_store: false,
        }
    }
}
pub fn classify_windows_sshd(text: &str, success: bool) -> &'static str {
    let lower = simple_lower(text);
    if lower.contains("does not exist") || lower.contains("1060") {
        "not_installed"
    } else if !success && text.is_empty() {
        "unknown"
    } else if lower.contains("state") {
        if lower.contains("running") {
            "running"
        } else {
            "stopped"
        }
    } else {
        "unknown"
    }
}
pub fn admin_url(stderr: &str) -> String {
    let prefix = "https://login.tailscale.com/";
    stderr
        .match_indices(prefix)
        .find_map(|(at, _)| {
            let value = stderr[at..]
                .split(['\t', '\n', '\u{c}', '\r', ' '])
                .next()
                .unwrap_or_default();
            (value.len() > prefix.len()).then_some(value)
        })
        .unwrap_or_default()
        .into()
}
