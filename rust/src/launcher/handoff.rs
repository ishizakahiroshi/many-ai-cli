//! Who may talk to the launcher's own loopback listeners, and how a URL that
//! carries a launch code reaches the browser (audits N22 and N23).
//!
//! A browser is started with its URL on the command line, and on Linux every
//! user of the machine can read every command line. A loopback listener, in
//! turn, takes connections from every user of the machine. Together that let
//! another OS user read what the launcher handed to the browser and use it
//! first: the launch code of a Hub behind an SSH forward (N22, where the remote
//! Hub cannot see who connected to the local end of the forward), or the token
//! of the launcher's own page (N23).
//!
//! So the launcher's listeners answer only connections this OS user opened
//! ([`own_peer`]; unknown is no), and a launch code never goes on a command
//! line: the browser is started on a page of the launcher's own, and that page
//! is answered with a redirect to the real address ([`open_in_browser`]). The
//! code is asked for when the browser arrives, so its minute starts then.
use super::network;
use std::{io, net::SocketAddr, time::Duration};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

/// How a listener on `port` decides whether `peer` is this OS user.
pub(super) type PeerCheck = fn(SocketAddr, u16) -> bool;

/// Whether the loopback connection from `peer` to this process's listener on
/// `port` was opened by a process of this OS user. An answer the system cannot
/// give is "no": the caller serves nothing.
pub(super) fn own_peer(peer: SocketAddr, port: u16) -> bool {
    peer.ip().is_loopback()
        && crate::process::listener::loopback_peer_is_own(peer.port(), port).unwrap_or(false)
}

/// How long the browser has to arrive at the hand-off page.
pub(super) const WAIT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const HEAD_LIMIT: usize = 8 * 1024;
const REFUSAL: &str = "This page answers only the OS user who started many-ai-cli-launcher.\nIt could not confirm that this connection is theirs.\n";

/// Tells a connection that is not this OS user's why it gets nothing.
pub(super) async fn refuse(mut stream: TcpStream) {
    // Take the request first so that closing does not reset the connection
    // before the answer is read. Nothing in it is used.
    let _ = tokio::time::timeout(Duration::from_millis(250), request_path(&mut stream)).await;
    let response = format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{REFUSAL}",
        REFUSAL.len()
    );
    write(&mut stream, response.as_bytes()).await;
}

/// Opens `hub_url` in the browser without putting its launch code on a command
/// line. `browser` gets the address of a one-time page on a listener of this
/// process; the first request this OS user's browser makes for that page is
/// redirected to `hub_url` with its token traded for a launch code. `Ok(false)`
/// when no such request came within `wait`.
pub(super) async fn open_in_browser(
    hub_url: &str,
    browser: &(dyn Fn(&str) -> io::Result<()> + Send + Sync),
    wait: Duration,
    own: PeerCheck,
) -> io::Result<bool> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    // Not a secret (it is on the command line): it keeps a stray request for
    // another path, such as a favicon, from using up the page.
    let page = format!("/{}", &crate::process::random_token()?[..32]);
    browser(&format!("http://127.0.0.1:{port}{page}"))?;
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let Ok(accepted) = tokio::time::timeout_at(deadline, listener.accept()).await else {
            return Ok(false);
        };
        let (mut stream, peer) = accepted?;
        if !own(peer, port) {
            refuse(stream).await;
            continue;
        }
        let Ok(Some(path)) = tokio::time::timeout_at(deadline, request_path(&mut stream)).await
        else {
            continue;
        };
        if !bool::from(path.as_bytes().ct_eq(page.as_bytes())) {
            respond(&mut stream, "404 Not Found", None).await;
            continue;
        }
        let target = network::browser_url(hub_url).await;
        if target.bytes().any(|byte| byte.is_ascii_control()) {
            respond(&mut stream, "500 Internal Server Error", None).await;
            return Err(io::Error::other("Hub URL cannot be sent as a redirect"));
        }
        respond(&mut stream, "303 See Other", Some(&target)).await;
        return Ok(true);
    }
}

/// The path of a `GET` request, without its query. Anything else is `None`.
async fn request_path(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buffer = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        if head.len() > HEAD_LIMIT {
            return None;
        }
        let read = stream.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    let line = std::str::from_utf8(head.split(|byte| *byte == b'\r').next()?).ok()?;
    let mut parts = line.split(' ');
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    Some(target.split('?').next().unwrap_or_default().to_owned())
}

async fn respond(stream: &mut TcpStream, status: &str, location: Option<&str>) {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: 0\r\nConnection: close\r\n"
    );
    if let Some(location) = location {
        head.push_str(&format!("Location: {location}\r\n"));
    }
    head.push_str("\r\n");
    write(stream, head.as_bytes()).await;
}

async fn write(stream: &mut TcpStream, bytes: &[u8]) {
    let _ = tokio::time::timeout(WRITE_TIMEOUT, async {
        let _ = stream.write_all(bytes).await;
        let _ = stream.shutdown().await;
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // No Hub listens there, so the address to open carries no code and no token.
    const HUB: &str = "http://127.0.0.1:1/?token=synthetic-hub-token&via=ssh";

    fn stranger(_: SocketAddr, _: u16) -> bool {
        false
    }
    async fn get(address: &str, path: &str) -> String {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response).await;
        String::from_utf8_lossy(&response).into_owned()
    }
    /// What the browser was started with: `(host:port, path)`.
    async fn started(opened: &Mutex<Option<String>>) -> (String, String) {
        loop {
            if let Some(url) = opened.lock().unwrap().clone() {
                let rest = url.strip_prefix("http://").unwrap();
                let (address, path) = rest.split_once('/').unwrap();
                return (address.to_owned(), format!("/{path}"));
            }
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn audit_n22_the_launch_address_reaches_only_this_users_browser_and_never_a_command_line()
    {
        let opened = Arc::new(Mutex::new(None));
        let seen = opened.clone();
        let browser = move |url: &str| {
            *seen.lock().unwrap() = Some(url.to_owned());
            Ok(())
        };
        let (came, (other, page)) = tokio::join!(
            open_in_browser(HUB, &browser, Duration::from_secs(10), own_peer),
            async {
                let (address, path) = started(&opened).await;
                // A request for another path does not use the page up.
                let other = get(&address, "/favicon.ico").await;
                (other, get(&address, &path).await)
            }
        );
        assert!(came.unwrap());
        let command_line = opened.lock().unwrap().clone().unwrap();
        assert!(
            !command_line.contains("token") && !command_line.contains("launch"),
            "{command_line}"
        );
        assert!(other.starts_with("HTTP/1.1 404"), "{other}");
        assert!(page.starts_with("HTTP/1.1 303"), "{page}");
        assert!(
            page.contains("\r\nLocation: http://127.0.0.1:1/?via=ssh\r\n"),
            "{page}"
        );
        assert!(page.contains("\r\nCache-Control: no-store\r\n"), "{page}");
    }

    #[tokio::test]
    async fn audit_n22_a_connection_that_is_not_this_users_gets_no_address() {
        let opened = Arc::new(Mutex::new(None));
        let seen = opened.clone();
        let browser = move |url: &str| {
            *seen.lock().unwrap() = Some(url.to_owned());
            Ok(())
        };
        let (came, page) = tokio::join!(
            open_in_browser(HUB, &browser, Duration::from_millis(600), stranger),
            async {
                let (address, path) = started(&opened).await;
                get(&address, &path).await
            }
        );
        assert!(!came.unwrap(), "the page is still unused");
        assert!(page.starts_with("HTTP/1.1 403"), "{page}");
        assert!(!page.contains("Location"), "{page}");
    }
}
