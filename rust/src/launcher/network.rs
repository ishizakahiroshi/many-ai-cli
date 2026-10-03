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
pub async fn probe_hub(hub_url: &str, timeout: Duration) -> bool {
    let Ok(url) = hub_endpoint(hub_url, "/api/info") else {
        return false;
    };
    let Ok(client) = client(timeout) else {
        return false;
    };
    client
        .get(url)
        .send()
        .await
        .is_ok_and(|r| r.status() == reqwest::StatusCode::OK)
}
pub async fn post_hub_endpoint(
    hub_url: &str,
    path: &str,
    cancel: &Cancellation,
) -> Result<(), String> {
    let url = hub_endpoint(hub_url, path).map_err(|error| format!("{path}: {error}"))?;
    let client =
        client(Duration::from_secs(3)).map_err(|_| format!("{path}: HTTP client unavailable"))?;
    let result = tokio::select! {
        _=cancel.cancelled()=>return Err(format!("{path}: cancelled")),
        result=client.post(url).send()=>result,
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
        tokio::select! { _=cancel.cancelled()=>return Err("cancelled".into()), response=client.get(&url).send()=>{
            if response.is_ok_and(|r|r.status()==reqwest::StatusCode::OK) { return Ok(()); }
        }}
        tokio::select! { _=cancel.cancelled()=>return Err("cancelled".into()), _=tokio::time::sleep(interval)=>{} }
    }
    Err(format!("timed out after {tries} polls"))
}
pub async fn post_net_hint(port: i64, token: &str, host: &str, cancel: &Cancellation) {
    let Ok(client) = client(Duration::from_secs(5)) else {
        return;
    };
    let url = format!(
        "http://127.0.0.1:{port}/api/net-hint?token={}",
        query_escape(token)
    );
    let body = serde_json::json!({"ssh":true,"host_label":host,"env_kind":"remote-tunnel"});
    tokio::select! { _=cancel.cancelled()=>{}, _=client.post(url).json(&body).send()=>{} }
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
