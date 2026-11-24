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

/// Discover the current user's Hub from an operator terminal. Explicit Hub
/// credentials take precedence over the private config. Session callers and
/// rejected credentials return errors so callers cannot edit `providers.d`
/// behind the Hub.
pub async fn discover(
    environment: &[String],
    config: &Config,
    paths: &RuntimePaths,
) -> io::Result<Option<NativeHub>> {
    discover_with(
        environment,
        config,
        paths,
        crate::process::listener::loopback_listener_is_own,
    )
    .await
}

/// Whether anything accepts a connection on the loopback port. Nothing is sent.
async fn something_listens(port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .is_ok_and(|connected| connected.is_ok())
}

async fn discover_with(
    environment: &[String],
    config: &Config,
    paths: &RuntimePaths,
    listener_is_own: impl Fn(u16) -> io::Result<bool>,
) -> io::Result<Option<NativeHub>> {
    // A provider command launched inside a session cannot use the owner's
    // config or fall back to changing providers.d when the Hub is offline.
    if environment_value(environment, "MANY_AI_CLI_SESSION_ID").is_some()
        || environment_value(environment, crate::wrapper::launch::HUB_TOKEN_ENV)
            .is_some_and(crate::hub::child_token::is_child)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Manage providers from the dashboard or a terminal outside a Hub session",
        ));
    }
    let from_environment = environment_value(environment, "MANY_AI_CLI_HUB_PORT")
        .and_then(|port| port.trim().parse::<u16>().ok())
        .filter(|port| *port > 0)
        .zip(
            environment_value(environment, "MANY_AI_CLI_HUB_TOKEN")
                .map(str::trim)
                .filter(|token| !token.is_empty()),
        );
    let proof = crate::hub::pin::cli_local_proof(config);
    let hub = if let Some((port, token)) = from_environment {
        if paths.is_trial() && port != paths.port() {
            return Ok(None);
        }
        // The token goes only to a listener this OS user holds. A dead port is
        // "no Hub"; a listener someone else holds is an error: returning None
        // would let the CLI write providers.d behind a Hub that may be running.
        if !listener_is_own(port).unwrap_or(false) {
            if something_listens(port).await {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "127.0.0.1:{port} is not held by this OS user's Hub; the token was not sent and providers.d was not changed"
                    ),
                ));
            }
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
            || !listener_is_own(port).unwrap_or(false)
        {
            return Ok(None);
        }
        NativeHub::new(port, config.token.clone(), proof)?
    };
    match hub.call("GET", "/api/providers", None).await {
        Ok(reply) if reply.status == 200 => Ok(Some(hub)),
        Ok(reply) => Err(io::Error::other(format!(
            "The running Hub refused this request ({}). Use the dashboard or verify the Hub credentials in your terminal.",
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
    fn environment_credentials_are_read_case_insensitively_on_windows() {
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
    async fn session_provider_command_does_not_fall_back_to_owner_config_or_local_writes() {
        let (_temp, paths) = trial();
        let config = Config {
            token: "synthetic-owner".into(),
            ..Default::default()
        };
        let environment = vec!["MANY_AI_CLI_SESSION_ID=3".into()];
        let error = discover_with(&environment, &config, &paths, |_| {
            panic!("unexpected probe")
        })
        .await
        .err()
        .expect("a session cannot manage providers");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
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
    async fn owner_token_is_not_sent_to_a_listener_another_user_holds() {
        let (_temp, paths) = trial();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let paths =
            RuntimePaths::trial(paths.root(), port, &paths.root().join("../installed")).unwrap();
        let environment = vec![
            format!("MANY_AI_CLI_HUB_PORT={port}"),
            "MANY_AI_CLI_HUB_TOKEN=synthetic-token".to_owned(),
        ];
        let peer = tokio::spawn(async move {
            // Anyone may connect; what matters is that no request arrives.
            let mut received = 0;
            while let Ok(Ok((mut socket, _))) =
                tokio::time::timeout(Duration::from_millis(500), listener.accept()).await
            {
                let mut buffer = [0; 1024];
                if let Ok(Ok(n)) =
                    tokio::time::timeout(Duration::from_millis(300), socket.read(&mut buffer)).await
                {
                    received += n;
                }
            }
            received
        });
        let error = discover_with(&environment, &Config::default(), &paths, |_| Ok(false))
            .await
            .err()
            .expect("a listener this user does not hold is an error, not no Hub");
        assert!(!error.to_string().contains("synthetic-token"));
        assert_eq!(peer.await.unwrap(), 0, "nothing may be sent to it");
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
