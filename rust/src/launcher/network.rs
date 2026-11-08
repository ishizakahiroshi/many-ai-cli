use super::commands::{normalize_hub_token, query_escape};
use crate::process::Cancellation;
use std::{io, time::Duration};

pub(crate) fn client(timeout: Duration) -> io::Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(io::Error::other)
}
/// Registry URLs are only local forwarded Hub endpoints. Refuse arbitrary URLs
/// before sending their opaque token anywhere (including HTTP redirects).
pub fn hub_endpoint(hub_url: &str, path: &str) -> Result<url::Url, String> {
    let mut url = url::Url::parse(hub_url).map_err(|_| "invalid Hub URL")?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_none()
    {
        return Err("Hub URL must be an explicit loopback HTTP endpoint".into());
    }
    url.set_path(path);
    url.set_fragment(None);
    Ok(url)
}
/// The token goes only to a port this OS user holds: the launcher's own SSH
/// forward or Hub (U01).
fn own_port(port: i64) -> bool {
    u16::try_from(port)
        .is_ok_and(|port| crate::process::listener::loopback_listener_is_own(port).unwrap_or(false))
}
/// The remote local proof the launcher keeps in the fragment of a stored Hub
/// URL. A fragment never goes over HTTP; the launcher sends the proof as a
/// header so a remote Hub with a PIN set lets its calls through (U09).
const PROOF_FRAGMENT: &str = "local=";
pub fn local_proof_of(hub_url: &str) -> Option<String> {
    let fragment = url::Url::parse(hub_url).ok()?.fragment()?.to_owned();
    let proof = fragment.strip_prefix(PROOF_FRAGMENT)?;
    (!proof.is_empty() && proof.bytes().all(|b| b.is_ascii_hexdigit())).then(|| proof.to_owned())
}
/// `hub_url` with `proof` kept in its fragment (or none).
pub fn with_local_proof(hub_url: &str, proof: Option<&str>) -> String {
    let Ok(mut url) = url::Url::parse(hub_url) else {
        return hub_url.into();
    };
    url.set_fragment(
        proof
            .map(|proof| format!("{PROOF_FRAGMENT}{proof}"))
            .as_deref(),
    );
    url.into()
}
/// `hub_url` without the proof, for display and for a browser.
pub fn without_local_proof(hub_url: &str) -> String {
    with_local_proof(hub_url, None)
}
fn proof_headers(proof: Option<&str>) -> reqwest::header::HeaderMap {
    crate::application::hub_runtime::local_proof_headers(proof)
}
fn own_endpoint(url: &url::Url) -> bool {
    url.port().is_some_and(|port| own_port(i64::from(port)))
}
pub async fn probe_hub(hub_url: &str, timeout: Duration) -> bool {
    let Ok(url) = hub_endpoint(hub_url, "/api/info") else {
        return false;
    };
    if !own_endpoint(&url) {
        return false;
    }
    let Ok(client) = client(timeout) else {
        return false;
    };
    client
        .get(url)
        .headers(proof_headers(local_proof_of(hub_url).as_deref()))
        .send()
        .await
        .is_ok_and(|r| r.status() == reqwest::StatusCode::OK)
}
/// The address to open in a browser for `hub_url`. Its token is traded
/// through the forwarded port for a one-time launch code, because a browser's
/// command line can be read by other users of this machine (U04). The other
/// parameters stay. If no code can be had, the token is left out.
pub async fn browser_url(hub_url: &str) -> String {
    let proof = local_proof_of(hub_url);
    let Ok(mut url) = url::Url::parse(hub_url) else {
        return hub_url.into();
    };
    url.set_fragment(None);
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let Some(token) = pairs
        .iter()
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.clone())
    else {
        return hub_url.into();
    };
    let code = match url.port() {
        Some(port) => {
            crate::application::hub_runtime::request_launch_code(port, &token, proof.as_deref())
                .await
        }
        None => None,
    };
    {
        let mut query = url.query_pairs_mut();
        query.clear();
        if let Some(code) = &code {
            query.append_pair("launch", code);
        }
        for (key, value) in pairs.iter().filter(|(key, _)| key != "token") {
            query.append_pair(key, value);
        }
    }
    if url.query() == Some("") {
        url.set_query(None);
    }
    url.into()
}
pub async fn post_hub_endpoint(
    hub_url: &str,
    path: &str,
    cancel: &Cancellation,
) -> Result<(), String> {
    let url = hub_endpoint(hub_url, path).map_err(|error| format!("{path}: {error}"))?;
    if !own_endpoint(&url) {
        return Err(format!(
            "{path}: the forwarded port is not held by this OS user; the token was not sent"
        ));
    }
    let client =
        client(Duration::from_secs(3)).map_err(|_| format!("{path}: HTTP client unavailable"))?;
    let result = tokio::select! {
        _=cancel.cancelled()=>return Err(format!("{path}: cancelled")),
        result=client.post(url).headers(proof_headers(local_proof_of(hub_url).as_deref())).header(crate::hub::lifecycle::STOP_SOURCE_HEADER,"launcher").send()=>result,
    };
    let response = result.map_err(|error| format!("{path}: {}", error.without_url()))?;
    if !response.status().is_success() {
        return Err(format!("{path}: {}", response.status()));
    }
    Ok(())
}
pub async fn poll_until_ready(
    port: i64,
    token: &str,
    proof: Option<&str>,
    cancel: &Cancellation,
    interval: Duration,
    tries: usize,
) -> Result<(), String> {
    normalize_hub_token(token)?;
    let url = format!(
        "http://127.0.0.1:{port}/api/info?token={}",
        query_escape(token)
    );
    let client = client(interval).map_err(|_| "HTTP client unavailable")?;
    for _ in 0..tries {
        // Until this user's listener is up, nothing is sent (U01).
        if own_port(port) {
            tokio::select! { _=cancel.cancelled()=>return Err("cancelled".into()), response=client.get(&url).headers(proof_headers(proof)).send()=>{
                if response.is_ok_and(|r|r.status()==reqwest::StatusCode::OK) { return Ok(()); }
            }}
        }
        tokio::select! { _=cancel.cancelled()=>return Err("cancelled".into()), _=tokio::time::sleep(interval)=>{} }
    }
    Err(format!("timed out after {tries} polls"))
}
pub async fn post_net_hint(
    port: i64,
    token: &str,
    proof: Option<&str>,
    host: &str,
    cancel: &Cancellation,
) {
    let Ok(client) = client(Duration::from_secs(5)) else {
        return;
    };
    let url = format!(
        "http://127.0.0.1:{port}/api/net-hint?token={}",
        query_escape(token)
    );
    if !own_port(port) {
        return;
    }
    let body = serde_json::json!({"ssh":true,"host_label":host,"env_kind":"remote-tunnel"});
    tokio::select! { _=cancel.cancelled()=>{}, _=client.post(url).headers(proof_headers(proof)).json(&body).send()=>{} }
}
pub async fn port_in_use(port: u16) -> bool {
    match tokio::time::timeout(
        Duration::from_millis(200),
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)),
    )
    .await
    {
        Ok(Ok(_)) => true,
        Ok(Err(error)) => error.kind() != io::ErrorKind::ConnectionRefused,
        Err(_) => true,
    }
}
pub async fn pick_port(base: u16) -> u16 {
    for i in 0..10u16 {
        if let Some(port) = base.checked_add(i * 100)
            && !port_in_use(port).await
        {
            return port;
        }
    }
    base
}

#[cfg(test)]
mod launch_tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn audit_u04_the_launcher_opens_a_launch_code_instead_of_the_token() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let read = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]).into_owned();
            let body = r#"{"code":"abcd1234","expires_in":60}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            request
        });
        let opened = super::browser_url(&format!(
            "http://127.0.0.1:{port}/?token=synthetic-remote&via=ssh&host_label=box&env_kind=remote-tunnel#local=fe9876"
        ))
        .await;
        assert_eq!(
            opened,
            format!(
                "http://127.0.0.1:{port}/?launch=abcd1234&via=ssh&host_label=box&env_kind=remote-tunnel"
            )
        );
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /api/auth/launch-code "));
        assert!(request.contains("authorization: Bearer synthetic-remote"));
        // The remote proof kept in the fragment goes as a header (U09), and
        // neither it nor the token reaches the opened address.
        assert!(request.contains("x-many-ai-cli-local: fe9876"));
        assert!(!opened.contains("fe9876") && !opened.contains('#'));
        assert_eq!(
            super::local_proof_of(&super::with_local_proof(
                "http://127.0.0.1:1/?token=t",
                Some("ab12")
            )),
            Some("ab12".into())
        );
        assert_eq!(
            super::without_local_proof("http://127.0.0.1:1/?token=t#local=ab12"),
            "http://127.0.0.1:1/?token=t"
        );
        assert_eq!(
            super::local_proof_of("http://127.0.0.1:1/#local=not-hex"),
            None
        );
        // Nobody listens any more: the token is left out, not opened.
        let opened = super::browser_url(&format!(
            "http://127.0.0.1:{port}/?token=synthetic-remote&via=ssh"
        ))
        .await;
        assert_eq!(opened, format!("http://127.0.0.1:{port}/?via=ssh"));
    }
}
