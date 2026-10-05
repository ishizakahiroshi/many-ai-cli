//! Bounded loopback-only status command. Never starts a Hub or invokes providers.
use crate::config::{Config, RuntimePaths};
use std::{io, time::Duration};
pub async fn write(
    config: &Config,
    paths: &RuntimePaths,
    output: &mut dyn io::Write,
) -> io::Result<()> {
    let port = u16::try_from(config.hub.port).map_err(io::Error::other)?;
    if port == 0 || (paths.is_trial() && port != paths.port()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "status runtime port mismatch",
        ));
    }
    let mut address =
        url::Url::parse(&format!("http://127.0.0.1:{port}/")).map_err(io::Error::other)?;
    address
        .query_pairs_mut()
        .append_pair("token", &config.token);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(io::Error::other)?;
    let running = client
        .get(address.clone())
        .send()
        .await
        .is_ok_and(|response| response.status() == reqwest::StatusCode::OK);
    if !running {
        return writeln!(output, "stopped");
    }
    writeln!(output, "running {address}")?;
    address.set_path("/api/info");
    let stale = async {
        let mut response = client.get(address).send().await.ok()?;
        if response.status() != reqwest::StatusCode::OK {
            return None;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if bytes.len().saturating_add(chunk.len()) > 1024 * 1024 {
                return None;
            }
            bytes.extend_from_slice(&chunk);
            let request = crate::hub::http::Request {
                body: bytes.clone(),
                ..Default::default()
            };
            if let Ok(value) = crate::hub::http::decode_json::<StaleInfo>(&request) {
                return Some(value.binary_stale);
            }
        }
        None
    };
    if tokio::time::timeout(Duration::from_millis(500), stale)
        .await
        .ok()
        .flatten()
        == Some(true)
    {
        writeln!(
            output,
            "WARNING: running Hub is a STALE binary (the on-disk exe differs from the one this Hub started with)."
        )?;
        writeln!(
            output,
            "         Restart the Hub to apply your rebuild: `many-ai-cli stop` then start it again."
        )?;
    }
    Ok(())
}
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct StaleInfo {
    binary_stale: bool,
}
impl crate::proto::wire::GoWire for StaleInfo {
    const GO_TYPE: &'static str = "StatusBinaryInfo";
    const SCHEMAS: &'static [crate::proto::wire::Schema] = &[crate::proto::wire::Schema {
        name: "StatusBinaryInfo",
        fields: &[crate::proto::wire::Field {
            name: "binary_stale",
            kind: "bool",
        }],
    }];
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn isolated_status_uses_root_then_bounded_info_and_refuses_other_trial_port() {
        let root = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::create_dir(root.path().join("trial")).unwrap();
        let paths = RuntimePaths::trial(
            &root.path().join("trial"),
            port,
            &root.path().join("installed"),
        )
        .unwrap();
        let mut config = Config::default();
        config.hub.port = i64::from(port);
        config.token = "synthetic-key".into();
        let peer = tokio::spawn(async move {
            for (expected, payload) in [
                ("GET /?token=synthetic-key", "{}"),
                (
                    "GET /api/info?token=synthetic-key",
                    "{\"BINARY_STALE\":true} trailing ignored",
                ),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let n = socket.read(&mut request).await.unwrap();
                assert!(String::from_utf8_lossy(&request[..n]).starts_with(expected));
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).as_bytes()).await.unwrap();
            }
        });
        let mut output = Vec::new();
        write(&config, &paths, &mut output).await.unwrap();
        peer.await.unwrap();
        assert!(String::from_utf8(output).unwrap().contains("STALE binary"));
        config.hub.port = i64::from(port) + 1;
        assert_eq!(
            write(&config, &paths, &mut Vec::new())
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
