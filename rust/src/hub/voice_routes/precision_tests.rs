use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct Server {
    port: u16,
    stop: Cancellation,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl Server {
    async fn new(replies: Vec<(&'static [u8], bool)>) -> Self {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Cancellation::default();
        let cancelled = stop.clone();
        let task = tokio::spawn(async move {
            for (reply, hold) in replies {
                let (mut stream, _) = tokio::select! {_=cancelled.cancelled()=>return,result=listener.accept()=>result.unwrap()};
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                loop {
                    let count = stream.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&request[..end]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                stream.write_all(reply).await.unwrap();
                stream.flush().await.unwrap();
                if hold {
                    cancelled.cancelled().await;
                    return;
                }
                stream.shutdown().await.unwrap();
            }
        });
        Self {
            port,
            stop,
            task: Some(task),
        }
    }
    fn config(&self) -> VoiceWhisperConfig {
        VoiceWhisperConfig {
            server_url: format!("http://127.0.0.1:{}", self.port),
            timeout_seconds: 2,
            ..Default::default()
        }
    }
    async fn join(mut self) {
        self.stop.cancel();
        self.task.take().unwrap().await.unwrap();
    }
}
fn code(response: &Response) -> String {
    serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["error"]
        .as_str()
        .unwrap()
        .into()
}
#[tokio::test]
async fn complete_json_returns_while_response_body_is_still_open() {
    let server=Server::new(vec![(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n11\r\n{\"text\":\"ready\"}\n\r\n",true)]).await;
    let config = server.config();
    let result = transcribe(&config, b"fixture", &Cancellation::default()).await;
    server.join().await;
    assert_eq!(result.unwrap(), "ready");
}
#[tokio::test]
async fn partial_404_read_error_keeps_status_and_enters_real_second_request() {
    let server = Server::new(vec![
        (
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 100\r\n\r\npartial detail",
            false,
        ),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 19\r\n\r\n{\"text\":\"fallback\"}",
            false,
        ),
    ])
    .await;
    let result = transcribe(&server.config(), b"fixture", &Cancellation::default()).await;
    server.join().await;
    assert_eq!(result.unwrap(), "fallback");
}
#[tokio::test]
async fn same_absolute_deadline_maps_headers_to_504_and_started_json_body_to_502() {
    for (body, expected) in [
        (b"" as &'static [u8], "whisper_timeout"),
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"text\":",
            "whisper_failed",
        ),
    ] {
        let server = Server::new(vec![(body, true)]).await;
        let config = server.config();
        let (_, result) = post(
            &config,
            b"fixture",
            "/inference",
            tokio::time::Instant::now() + Duration::from_secs(1),
            &Cancellation::default(),
        )
        .await;
        server.join().await;
        let error = result.unwrap_err();
        assert_eq!(code(&error), expected);
        assert_eq!(
            error.status,
            if expected == "whisper_timeout" {
                504
            } else {
                502
            }
        );
    }
}
#[tokio::test]
async fn non_2xx_partial_body_deadline_retains_detail_and_status() {
    let server = Server::new(vec![(
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 100\r\n\r\npartial detail",
        true,
    )])
    .await;
    let (status, result) = post(
        &server.config(),
        b"fixture",
        "/inference",
        tokio::time::Instant::now() + Duration::from_secs(1),
        &Cancellation::default(),
    )
    .await;
    server.join().await;
    assert_eq!(status, 404);
    let response = result.unwrap_err();
    assert_eq!(code(&response), "whisper_failed");
    let data: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(data["detail"], "partial detail");
}
#[test]
fn paths_follow_pinned_go_stdlib_observations() {
    for (server, path, expected) in [
        (
            "http://127.0.0.1:7777/base/?old=yes#f",
            "inference",
            "http://127.0.0.1:7777/base/inference",
        ),
        (
            "http://127.0.0.1:7777/a%2Fb/",
            "//inference",
            "http://127.0.0.1:7777/a/b//inference",
        ),
        (
            "http://127.0.0.1:7777/a%25b",
            "/what?x#y",
            "http://127.0.0.1:7777/a%25b/what%3Fx%23y",
        ),
        (
            "http://127.0.0.1:7777/%FF",
            "/inference",
            "http://127.0.0.1:7777/%FF/inference",
        ),
    ] {
        assert_eq!(target_url(server, path).unwrap().as_str(), expected);
    }
}
#[test]
fn mixed_dns_answers_are_rejected_before_any_dial_and_literal_credentials_are_blocked() {
    let url = target_url("http://fixture.invalid", "/inference").unwrap();
    assert!(
        network::validated_target(
            url,
            &["127.0.0.1".parse().unwrap(), "192.0.2.10".parse().unwrap()],
            Policy::ManagedLoopbackHttp
        )
        .is_err()
    );
    assert!(
        network::validate_url(
            "http://user@127.0.0.1/inference",
            Policy::ManagedLoopbackHttp
        )
        .is_err()
    );
}
#[tokio::test]
async fn scalar_array_success_is_decode_failure_and_null_is_missing_text() {
    for (body, missing) in [
        (
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]" as &'static [u8],
            false,
        ),
        (b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nnull", true),
    ] {
        let server = Server::new(vec![(body, false)]).await;
        let response = transcribe(&server.config(), b"fixture", &Cancellation::default())
            .await
            .unwrap_err();
        server.join().await;
        assert_eq!(code(&response), "whisper_failed");
        let data: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        let detail = data["detail"].as_str().unwrap();
        assert_eq!(detail == "whisper response missing text", missing);
        if !missing {
            assert!(detail.starts_with("decode whisper response failed:"));
        }
    }
}
#[tokio::test]
async fn fallback_uses_original_deadline_after_partial_404_body_expires() {
    let server = Server::new(vec![(
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 100\r\n\r\npartial",
        true,
    )])
    .await;
    let mut config = server.config();
    config.timeout_seconds = 1;
    let response = transcribe(&config, b"fixture", &Cancellation::default())
        .await
        .unwrap_err();
    server.join().await;
    assert_eq!(response.status, 504);
    assert_eq!(code(&response), "whisper_timeout");
}
#[test]
fn first_json_value_boundary_matches_go_decoder_without_requiring_object_eof() {
    assert!(
        decode_success(br#"{"text":"complete"}"#, false)
            .unwrap()
            .is_some()
    );
    assert!(decode_success(b"null", false).unwrap().is_none());
    assert!(decode_success(b"null ", false).unwrap().unwrap().is_null());
    assert!(decode_success(b"null", true).unwrap().unwrap().is_null());
    assert!(decode_success(b"[]", false).is_err());
    assert!(
        decode_success(br#"{"text":"complete"} trailing junk"#, false)
            .unwrap()
            .is_some()
    );
    assert!(decode_success(br#"{"text":"cutoff""#, true).is_err());
}
