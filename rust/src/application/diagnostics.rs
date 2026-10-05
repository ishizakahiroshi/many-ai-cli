//! Shared, non-mutating doctor actor. All process/environment authority is explicit.
pub mod bug_report;
mod local;
mod native;
pub mod report;
mod supplemental;
use crate::{
    application::subscriptions::cli::SubscriptionCli,
    config::{Config, ConfigStore, RuntimePaths},
    process::Cancellation,
    proto::core::{CoreFuture, SessionError},
};
pub use native::NativeDiagnosticIo;
use serde::Serialize;
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
#[derive(Clone, Serialize, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub level: String,
    pub message: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fix: String,
}
impl Check {
    pub(super) fn new(name: &str, level: &str, message: impl Into<String>, fix: &str) -> Self {
        Self {
            name: name.into(),
            level: level.into(),
            message: message.into(),
            fix: fix.into(),
        }
    }
}
#[derive(Clone, Serialize, Debug)]
pub struct DoctorReport {
    #[serde(rename = "Checks")]
    pub checks: Vec<Check>,
}
#[derive(Debug)]
pub struct ProbeOutput {
    pub bytes: Vec<u8>,
    pub success: bool,
}
pub trait DiagnosticIo: Send + Sync {
    fn look_path(&self, name: &str) -> io::Result<String>;
    fn command<'a>(
        &'a self,
        executable: &'a str,
        args: Vec<String>,
        cwd: &'a Path,
        timeout: Duration,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<ProbeOutput>>;
    fn http_status<'a>(
        &'a self,
        url: &'a str,
        timeout: Duration,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<u16>>;
    fn pid_alive(&self, pid: u32) -> bool;
}
pub struct DiagnosticsDependencies {
    pub config: Arc<ConfigStore>,
    pub paths: RuntimePaths,
    pub cwd: PathBuf,
    pub home: PathBuf,
    pub environment: Vec<String>,
    pub platform: String,
    pub io: Arc<dyn DiagnosticIo>,
    pub subscription_cli: Arc<dyn SubscriptionCli>,
    pub nvidia_key_configured: Arc<dyn Fn() -> io::Result<bool> + Send + Sync>,
}
pub struct Diagnostics {
    deps: DiagnosticsDependencies,
}
impl Diagnostics {
    pub fn new(deps: DiagnosticsDependencies) -> Arc<Self> {
        Arc::new(Self { deps })
    }
    pub async fn run(&self, cancel: &Cancellation) -> Result<DoctorReport, SessionError> {
        if cancel.is_cancelled() {
            return Err(SessionError::Cancelled);
        }
        let cfg = self
            .deps
            .config
            .snapshot()
            .map_err(|_| {
                SessionError::InvalidRequest("diagnostic configuration unavailable".into())
            })?
            .config;
        let mut checks = vec![
            self.providers(cancel).await,
            local::port(&cfg),
            local::token(self, &cfg),
            local::acl(self),
            self.ollama(&cfg, cancel).await,
            self.whisper(&cfg, cancel).await,
            self.nvidia(&cfg),
            self.tailscale(&cfg, cancel).await,
            local::logs(self, &cfg),
            local::session_log(self, &cfg),
            local::handoff(self, &cfg),
        ];
        checks.extend(supplemental::residue(self, &cfg, cancel).await);
        checks.extend(self.command_code(cancel).await);
        checks.extend(supplemental::subscriptions(self, &cfg, cancel).await);
        checks.extend(supplemental::custom(self, &cfg));
        if cancel.is_cancelled() {
            return Err(SessionError::Cancelled);
        }
        for check in &mut checks {
            check.message = report::redact(&check.message);
            check.fix = report::redact(&check.fix);
        }
        Ok(DoctorReport { checks })
    }
    async fn output(
        &self,
        name: &str,
        args: &[&str],
        timeout: Duration,
        cancel: &Cancellation,
    ) -> io::Result<ProbeOutput> {
        let exe = self.deps.io.look_path(name)?;
        self.deps
            .io
            .command(
                &exe,
                args.iter().map(|a| a.to_string()).collect(),
                &self.deps.cwd,
                timeout,
                cancel,
            )
            .await
    }
    async fn providers(&self, cancel: &Cancellation) -> Check {
        let mut found = vec![];
        for name in crate::proto::provider::BUILTIN_PROVIDER_IDS {
            let Ok(exe) = self.deps.io.look_path(name) else {
                continue;
            };
            let output = self
                .deps
                .io
                .command(
                    &exe,
                    vec!["--version".into()],
                    &self.deps.cwd,
                    Duration::from_secs(3),
                    cancel,
                )
                .await;
            let line = output
                .ok()
                .filter(|o| o.success)
                .map(|o| first_line(&String::from_utf8_lossy(&o.bytes)))
                .unwrap_or_default();
            found.push(if line.is_empty() {
                name.to_string()
            } else {
                format!("{name} ({line})")
            });
        }
        if found.is_empty() {
            Check::new(
                "provider",
                "FAIL",
                "対応する AI CLI が PATH に見つかりません",
                "Claude Code または Codex CLI をインストールし、PATH を確認してください",
            )
        } else {
            Check::new("provider", "OK", format!("検出: {}", found.join(", ")), "")
        }
    }
    async fn ollama(&self, cfg: &Config, cancel: &Cancellation) -> Check {
        let base = crate::config::effective_ollama_base_url(&cfg.ollama.base_url);
        let url = format!("{}/api/tags", base.trim_end_matches('/'));
        if url::Url::parse(&url).is_err() {
            return Check::new(
                "Ollama",
                "WARN",
                "base_url が不正",
                "ollama.base_url を確認してください",
            );
        }
        match self
            .deps
            .io
            .http_status(&url, Duration::from_secs(2), cancel)
            .await
        {
            Err(_) => Check::new(
                "Ollama",
                "WARN",
                format!("Ollama に接続できません: {base}"),
                "Ollama を起動するか ollama.base_url を確認してください",
            ),
            Ok(code) if code / 100 != 2 => Check::new(
                "Ollama",
                "WARN",
                format!("Ollama が HTTP {code} {}", http_reason(code)),
                "Ollama の起動状態と base_url を確認してください",
            ),
            Ok(_) => Check::new("Ollama", "OK", "Ollama に接続できます", ""),
        }
    }
    async fn whisper(&self, cfg: &Config, cancel: &Cancellation) -> Check {
        let url = cfg.voice.whisper.server_url.trim();
        if url.is_empty() {
            return Check::new(
                "Whisper",
                "WARN",
                "Whisper サーバーは未設定です",
                "音声入力を使う場合は Settings で Whisper を設定してください",
            );
        }
        if url::Url::parse(url).is_err() {
            return Check::new(
                "Whisper",
                "WARN",
                "server_url が不正",
                "voice.whisper.server_url を確認してください",
            );
        }
        match self
            .deps
            .io
            .http_status(url, Duration::from_secs(2), cancel)
            .await
        {
            Err(_) => Check::new(
                "Whisper",
                "WARN",
                "Whisper に接続できません",
                "Whisper runtime を起動するか server_url を確認してください",
            ),
            Ok(_) => Check::new("Whisper", "OK", "Whisper endpoint に接続できます", ""),
        }
    }
    fn nvidia(&self, cfg: &Config) -> Check {
        if !cfg.nvidia_nim.enabled {
            return Check::new(
                "NVIDIA NIM",
                "OK",
                "DISABLED: NVIDIA NIM route is turned off",
                "",
            );
        }
        match (self.deps.nvidia_key_configured)() {
            Err(_) => Check::new(
                "NVIDIA NIM",
                "WARN",
                "API key status is unavailable; details are hidden",
                "Check the Hub settings directory permissions",
            ),
            Ok(false) => Check::new(
                "NVIDIA NIM",
                "WARN",
                "NOT CONFIGURED: no NVIDIA API key is configured",
                "Configure a key in Hub Settings → NVIDIA NIM",
            ),
            Ok(true) => Check::new(
                "NVIDIA NIM",
                "WARN",
                "Configured; API connectivity was not checked by Doctor",
                "Use Test connection in Hub Settings to check NVIDIA /v1/models",
            ),
        }
    }
    async fn tailscale(&self, cfg: &Config, cancel: &Cancellation) -> Check {
        let Ok(exe) = self.deps.io.look_path("tailscale") else {
            return Check::new(
                "Tailscale",
                "WARN",
                "tailscale コマンドは見つかりません",
                "モバイル接続に Tailscale を使う場合はインストールしてください",
            );
        };
        let out = match self
            .deps
            .io
            .command(
                &exe,
                vec!["status".into(), "--json".into()],
                &self.deps.cwd,
                Duration::from_secs(2),
                cancel,
            )
            .await
        {
            Ok(o) if o.success => o.bytes,
            _ => {
                return Check::new(
                    "Tailscale",
                    "WARN",
                    "tailscale は検出しましたが接続状態を確認できません",
                    "tailscale login を実行して接続状態を確認してください",
                );
            }
        };
        let value = serde_json::from_slice::<serde_json::Value>(&out).unwrap_or_default();
        if !value["BackendState"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("Running"))
        {
            return Check::new(
                "Tailscale",
                "WARN",
                "Tailscale は未接続です",
                "tailscale up を実行してから再試行してください",
            );
        };
        let host = value["Self"]["DNSName"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_end_matches('.');
        if !host.is_empty() && !cfg.hub.allowed_hosts.is_empty() {
            if cfg
                .hub
                .allowed_hosts
                .iter()
                .any(|s| s.trim().trim_end_matches('.').eq_ignore_ascii_case(host))
            {
                return Check::new(
                    "Tailscale",
                    "OK",
                    "Tailscale は接続済みで allowed_hosts に登録済みです",
                    "",
                );
            }
            return Check::new(
                "Tailscale",
                "WARN",
                "Tailscale は接続済みですが、この端末名は allowed_hosts に未登録です",
                "Hub UI の Expose から公開を有効にするか hub.allowed_hosts を確認してください",
            );
        };
        Check::new("Tailscale", "OK", "Tailscale は接続済みです", "")
    }
    async fn command_code(&self, cancel: &Cancellation) -> Vec<Check> {
        if self.deps.io.look_path("command-code").is_err() {
            return vec![];
        }
        let out = self
            .output(
                "command-code",
                &["status", "--json"],
                Duration::from_secs(3),
                cancel,
            )
            .await
            .ok();
        let mut checks = vec![match out
            .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.bytes).ok())
        {
            None => Check::new(
                "Command Code",
                "WARN",
                "command-code の状態を読めませんでした",
                "command-code status --json を実行して出力を確認してください",
            ),
            Some(v) => {
                let version = v["version"].as_str().unwrap_or("");
                if v["authenticated"].as_bool() == Some(true) {
                    Check::new(
                        "Command Code",
                        "OK",
                        format!("command-code {version} はログイン済みです"),
                        "",
                    )
                } else {
                    Check::new(
                        "Command Code",
                        "WARN",
                        format!("command-code {version} は未ログインです"),
                        "command-code login を実行してください",
                    )
                }
            }
        }];
        if let Ok(out) = self
            .output("node", &["--version"], Duration::from_secs(3), cancel)
            .await
            && out.success
        {
            let text = first_line(&String::from_utf8_lossy(&out.bytes));
            if text
                .trim_start_matches('v')
                .split('.')
                .next()
                .and_then(|s| s.parse::<i64>().ok())
                .is_some_and(|v| v < 22)
            {
                checks.push(Check::new(
                    "Command Code",
                    "WARN",
                    format!("Node.js {text} は Command Code の要件（22 以上）を満たしません"),
                    "Node.js 22 以上へ更新してください",
                ))
            }
        }
        checks
    }
}
fn first_line(raw: &str) -> String {
    raw.trim().split('\n').next().unwrap_or_default().into()
}
fn http_reason(code: u16) -> &'static str {
    reqwest::StatusCode::from_u16(code)
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or("")
}
#[cfg(test)]
mod tests;
