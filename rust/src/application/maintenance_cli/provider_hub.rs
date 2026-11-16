//! How `many-ai-cli provider ...` reaches a running Hub. The registry API is the
//! one writer when a Hub runs (it validates, backs up, reloads and tells every
//! open dashboard); the CLI writes `providers.d` itself only when none runs.
//! The token is read here and goes into a Bearer header only: it is never
//! printed, logged, put in a URL or kept in an error message.
use crate::config::{Config, RuntimePaths};
use std::{io, time::Duration};

const REPLY_LIMIT: usize = 4 * 1024 * 1024;

pub struct HubReply {
    pub status: u16,
    pub body: Vec<u8>,
}
impl HubReply {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
    /// The Hub's own reason for a refusal: `detail`, else `error`, else the status.
    pub fn reason(&self) -> String {
        let value = self.json();
        let text = value["detail"]
            .as_str()
            .or_else(|| value["error"].as_str())
            .unwrap_or("");
        let mut reason = if text.is_empty() {
            format!("HTTP {}", self.status)
        } else {
            format!("{text} (HTTP {})", self.status)
        };
        if let Some(diagnostics) = value["diagnostics"].as_array() {
            for diagnostic in diagnostics {
                let field = diagnostic["field"].as_str().unwrap_or("");
                let message = diagnostic["message"].as_str().unwrap_or("");
                if !message.is_empty() {
                    reason.push_str(&format!("\n  {field}: {message}"));
                }
            }
        }
        reason
    }
}

#[allow(async_fn_in_trait)]
pub trait HubApi {
    async fn call(&self, method: &str, path: &str, body: Option<Vec<u8>>) -> io::Result<HubReply>;
}

/// No Debug: it holds the Hub token.
pub struct NativeHub {
    port: u16,
    token: String,
    proof: Option<String>,
    client: reqwest::Client,
}
impl NativeHub {
    pub fn new(port: u16, token: String, proof: Option<String>) -> io::Result<Self> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| io::Error::other("could not prepare the Hub client"))?;
        Ok(Self {
            port,
            token,
            proof,
            client,
        })
    }
}
impl HubApi for NativeHub {
    async fn call(&self, method: &str, path: &str, body: Option<Vec<u8>>) -> io::Result<HubReply> {
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid HTTP method"))?;
        let mut bearer = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.token))
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid Hub token"))?;
        bearer.set_sensitive(true);
        let mut request = self
            .client
            .request(method, format!("http://127.0.0.1:{}{path}", self.port))
            .headers(crate::application::hub_runtime::local_proof_headers(
                self.proof.as_deref(),
            ))
            .header(reqwest::header::AUTHORIZATION, bearer)
            .header("Origin", format!("http://127.0.0.1:{}", self.port))
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(body) = body {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        // reqwest's error text carries the URL; keep only the kind.
        let mut response = request.send().await.map_err(|error| {
            if error.is_connect() {
                io::Error::new(io::ErrorKind::ConnectionRefused, "the Hub is not reachable")
            } else if error.is_timeout() {
                io::Error::new(io::ErrorKind::TimedOut, "the Hub did not answer in time")
            } else {
                io::Error::other("the Hub request failed")
            }
        })?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| io::Error::other("the Hub reply could not be read"))?
        {
            if bytes.len() + chunk.len() > REPLY_LIMIT {
                return Err(io::Error::other("the Hub reply is too large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(HubReply {
            status,
            body: bytes,
        })
    }
}

fn environment_value<'a>(environment: &'a [String], name: &str) -> Option<&'a str> {
    environment.iter().rev().find_map(|entry| {
        let (key, value) = entry.split_once('=')?;
        let same = if cfg!(windows) {
            key.eq_ignore_ascii_case(name)
        } else {
            key == name
        };
        same.then_some(value)
    })
}

/// A Hub this CLI may talk to, or `None` when none runs. The session's own
/// `MANY_AI_CLI_HUB_PORT` / `MANY_AI_CLI_HUB_TOKEN` come first (an AI that runs
/// inside a Hub session); otherwise the port and token in the many-ai-cli
/// config, but only for a listener this OS user holds. A Hub that answers but
/// refuses the credentials is an error, not "no Hub": writing `providers.d`
/// behind a running Hub would leave it stale.
pub async fn discover(
    environment: &[String],
    config: &Config,
    paths: &RuntimePaths,
) -> io::Result<Option<NativeHub>> {
    let from_session = environment_value(environment, "MANY_AI_CLI_HUB_PORT")
        .and_then(|port| port.trim().parse::<u16>().ok())
        .filter(|port| *port > 0)
        .zip(
            environment_value(environment, "MANY_AI_CLI_HUB_TOKEN")
                .map(str::trim)
                .filter(|token| !token.is_empty()),
        );
    let proof = crate::hub::pin::cli_local_proof(config);
    let hub = if let Some((port, token)) = from_session {
        if paths.is_trial() && port != paths.port() {
            return Ok(None);
        }
        NativeHub::new(port, token.to_owned(), proof)?
    } else {
        let Ok(port) = u16::try_from(config.hub.port) else {
            return Ok(None);
        };
        if port == 0
            || config.token.is_empty()
            || (paths.is_trial() && port != paths.port())
            || !crate::process::listener::loopback_listener_is_own(port).unwrap_or(false)
        {
            return Ok(None);
        }
        NativeHub::new(port, config.token.clone(), proof)?
    };
    match hub.call("GET", "/api/providers", None).await {
        Ok(reply) if reply.status == 200 => Ok(Some(hub)),
        Ok(reply) => Err(io::Error::other(format!(
            "a Hub is running but refused this request ({}); not changing providers.d behind it. Restart the Hub or run from a Hub session",
            reply.reason()
        ))),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn trial() -> (tempfile::TempDir, RuntimePaths) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("runtime");
        std::fs::create_dir(&root).unwrap();
        let paths = RuntimePaths::trial(&root, 49771, &temp.path().join("installed")).unwrap();
        (temp, paths)
    }
    /// One canned reply per accepted connection; returns what each request was.
    async fn fake_hub(replies: Vec<&'static str>) -> (u16, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let mut seen = vec![];
            for payload in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 8192];
                let n = socket.read(&mut request).await.unwrap();
                seen.push(String::from_utf8_lossy(&request[..n]).into_owned());
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                            payload.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            seen
        });
        (port, task)
    }

    #[test]
    fn session_environment_wins_and_is_read_case_insensitively_on_windows() {
        let environment = vec![
            "MANY_AI_CLI_HUB_PORT=49000".to_owned(),
            "MANY_AI_CLI_HUB_TOKEN=synthetic-token".to_owned(),
        ];
        assert_eq!(
            environment_value(&environment, "MANY_AI_CLI_HUB_PORT"),
            Some("49000")
        );
        assert_eq!(environment_value(&environment, "MANY_AI_CLI_NOPE"), None);
    }
    #[tokio::test]
    async fn bearer_token_origin_and_probe_reach_a_loopback_hub_only_in_headers() {
        let (_temp, mut paths) = trial();
        let (port, task) = fake_hub(vec![r#"{"revision":"r1","providers":[]}"#]).await;
        paths =
            RuntimePaths::trial(paths.root(), port, &paths.root().join("../installed")).unwrap();
        let config = Config::default();
        let environment = vec![
            format!("MANY_AI_CLI_HUB_PORT={port}"),
            "MANY_AI_CLI_HUB_TOKEN=synthetic-token".to_owned(),
        ];
        let hub = discover(&environment, &config, &paths).await.unwrap();
        assert!(hub.is_some());
        let seen = task.await.unwrap();
        let request = seen[0].to_ascii_lowercase();
        assert!(request.starts_with("get /api/providers "));
        assert!(request.contains("authorization: bearer synthetic-token"));
        assert!(request.contains(&format!("origin: http://127.0.0.1:{port}")));
        assert!(
            !request.starts_with("get /api/providers?"),
            "the token must not be in the URL"
        );
    }
    #[tokio::test]
    async fn refused_credentials_are_an_error_and_a_dead_port_means_no_hub() {
        let (_temp, paths) = trial();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let paths =
            RuntimePaths::trial(paths.root(), port, &paths.root().join("../installed")).unwrap();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            let body = r#"{"ok":false,"error":"unauthorized","detail":"bad token"}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let environment = vec![
            format!("MANY_AI_CLI_HUB_PORT={port}"),
            "MANY_AI_CLI_HUB_TOKEN=synthetic-token".to_owned(),
        ];
        let error = discover(&environment, &Config::default(), &paths)
            .await
            .err()
            .expect("a refused probe is an error");
        peer.await.unwrap();
        let message = error.to_string();
        assert!(message.contains("bad token") && message.contains("401"));
        assert!(!message.contains("synthetic-token"));
        // Nothing listens on the port any more: that is "no Hub", not an error.
        assert!(
            discover(&environment, &Config::default(), &paths)
                .await
                .unwrap()
                .is_none()
        );
    }
    #[tokio::test]
    async fn another_trial_port_is_never_contacted() {
        let (_temp, paths) = trial();
        let environment = vec![
            "MANY_AI_CLI_HUB_PORT=49999".to_owned(),
            "MANY_AI_CLI_HUB_TOKEN=synthetic-token".to_owned(),
        ];
        assert!(
            discover(&environment, &Config::default(), &paths)
                .await
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn reason_prefers_the_hub_detail_and_lists_diagnostics() {
        let reply = HubReply {
            status: 422,
            body: br#"{"valid":false,"diagnostics":[{"field":"launch.executable","message":"executable is required"}]}"#.to_vec(),
        };
        let reason = reply.reason();
        assert!(reason.starts_with("HTTP 422"));
        assert!(reason.contains("launch.executable: executable is required"));
        let reply = HubReply {
            status: 409,
            body: br#"{"ok":false,"error":"provider_id_exists","detail":"provider id already exists"}"#.to_vec(),
        };
        assert_eq!(reply.reason(), "provider id already exists (HTTP 409)");
    }
}
