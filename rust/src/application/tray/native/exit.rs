//! Scoped loopback requests for a deliberate application exit. Errors never
//! contain request URLs, credentials or server response bodies.
use super::*;
use crate::application::hub_runtime::{self, Owner};
use serde_json::Value;

pub(super) struct ExitClient {
    pub(super) runtime: hub_runtime::RuntimeData,
    ledger: RuntimeLedger,
    client: reqwest::Client,
    port: u16,
    token: String,
    proof: Option<String>,
}
impl NativeTrayHooks {
    pub(super) fn exit_client(&self) -> io::Result<Option<ExitClient>> {
        let Some(data) = RuntimeLedger::open(&self.paths)?.read()? else {
            return Ok(None);
        };
        if matches!(hub_runtime::owner(&data), Owner::Gone | Owner::Other) {
            return Ok(None);
        }
        let port = u16::try_from(data.port)
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| {
                io::Error::other("Hub の接続先を確認できませんでした。終了していません。")
            })?;
        if self.launch_port().is_some_and(|expected| expected != port)
            || (self.paths.is_trial() && port != self.paths.port())
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "別の Hub は終了できません。保存先とポートを確認してください。",
            ));
        }
        crate::process::listener::require_own_loopback_listener(port)?;
        let config = self.config.snapshot().map_err(io::Error::other)?.config;
        Ok(Some(ExitClient {
            runtime: data,
            ledger: RuntimeLedger::open(&self.paths)?,
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(8))
                .build()
                .map_err(io::Error::other)?,
            port,
            proof: crate::hub::pin::cli_local_proof(&config),
            token: config.token.clone(),
        }))
    }
}
impl ExitClient {
    async fn request(&self, method: reqwest::Method, path: &'static str) -> io::Result<Value> {
        self.request_guarded(method, path, false).await
    }
    async fn request_guarded(
        &self,
        method: reqwest::Method,
        path: &'static str,
        require_empty: bool,
    ) -> io::Result<Value> {
        if self.ledger.read()?.as_ref() != Some(&self.runtime) {
            return Err(io::Error::other(
                "Hub が切り替わったため終了を中止しました。もう一度お試しください。",
            ));
        }
        crate::process::listener::require_own_loopback_listener(self.port)?;
        let reply = self
            .client
            .request(method, format!("http://127.0.0.1:{}{path}", self.port))
            .query(&[("token", self.token.as_str())])
            .headers(hub_runtime::local_proof_headers(self.proof.as_deref()))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(
                "x-many-ai-cli-require-empty",
                if require_empty { "1" } else { "0" },
            )
            .send()
            .await
            .map_err(|_| {
                io::Error::other("Hub との通信に失敗しました。アプリは終了していません。")
            })?;
        if !reply.status().is_success() {
            return Err(io::Error::other(
                "終了処理に失敗しました。アプリを開いたままにしています。もう一度お試しください。",
            ));
        }
        reply.json().await.map_err(|_| {
            io::Error::other("Hub の応答を確認できませんでした。アプリは終了していません。")
        })
    }
    pub(super) async fn active_sessions(&self) -> io::Result<usize> {
        connected_count(&self.request(reqwest::Method::GET, "/api/info").await?)
    }
    pub(super) async fn end_sessions(&self, confirmed: bool) -> io::Result<()> {
        if !confirmed && self.active_sessions().await? > 0 {
            return Err(io::Error::other(
                "新しいAIセッションが開始されました。もう一度「終了」を押して確認してください。",
            ));
        }
        let result = self
            .request_guarded(reqwest::Method::POST, "/api/kill-all", !confirmed)
            .await?;
        require_ok(&result)?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            if self.active_sessions().await? == 0 {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "AIセッションの終了を確認できませんでした。アプリを開いたままにしています。",
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    pub(super) async fn stop_exposure(&self) -> io::Result<()> {
        let status = self
            .request(reqwest::Method::GET, "/api/mobile-connect/tailscale")
            .await?;
        if status.get("probe_ok").and_then(Value::as_bool) != Some(true) {
            return Err(io::Error::other(
                "外部公開の状態を確認できませんでした。アプリを開いたままにしています。",
            ));
        }
        if status.get("state").and_then(Value::as_str) == Some("ready") {
            let result = self
                .request(
                    reqwest::Method::DELETE,
                    "/api/mobile-connect/tailscale/serve",
                )
                .await?;
            if result.get("ok").and_then(Value::as_bool) != Some(true) {
                return Err(io::Error::other(
                    "外部公開を停止できませんでした。アプリを開いたままにしています。",
                ));
            }
        }
        Ok(())
    }
}
fn connected_count(value: &Value) -> io::Result<usize> {
    value
        .get("connected_sessions")
        .and_then(Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(|| {
            io::Error::other("実行中のAIの件数を確認できませんでした。アプリは終了していません。")
        })
}
fn require_ok(value: &Value) -> io::Result<()> {
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(io::Error::other(
            "AIセッションを終了できませんでした。アプリを開いたままにしています。",
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_invalid_session_count_is_never_zero() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"active_sessions":0}),
            serde_json::json!({"connected_sessions":-1}),
            serde_json::json!({"connected_sessions":"0"}),
        ] {
            assert!(connected_count(&value).is_err());
        }
        assert_eq!(
            connected_count(&serde_json::json!({"connected_sessions":0})).unwrap(),
            0
        );
        assert_eq!(
            connected_count(&serde_json::json!({"connected_sessions":3})).unwrap(),
            3
        );
    }
    #[test]
    fn session_stop_requires_explicit_success() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"ok":false}),
            serde_json::json!({"ok":"true"}),
        ] {
            assert!(require_ok(&value).is_err());
        }
        assert!(require_ok(&serde_json::json!({"ok":true})).is_ok());
    }
}
