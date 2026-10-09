//! Raw audio proxy: each dial is pinned to validated loopback addresses.
use super::{
    http::{Request, Response},
    network::{self, Policy},
};
use crate::{
    config::{ConfigStore, VoiceWhisperConfig},
    process::Cancellation,
    proto::{core::CoreFuture, unicode},
};
use futures_util::StreamExt;
use std::{net::IpAddr, sync::Arc, time::Duration};

pub const PATH: &str = "/api/voice/transcribe";
pub const AUDIO_LIMIT: usize = 25 * 1024 * 1024;
pub trait ManagedWhisper: Send + Sync {
    fn ensure<'a>(
        &'a self,
        config: VoiceWhisperConfig,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<VoiceWhisperConfig, Response>>;
}
pub struct VoiceHttp {
    config: Arc<ConfigStore>,
    managed: Arc<dyn ManagedWhisper>,
}
impl ManagedWhisper for Arc<crate::application::whisper::WhisperManager> {
    fn ensure<'a>(
        &'a self,
        config: VoiceWhisperConfig,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<VoiceWhisperConfig, Response>> {
        Box::pin(async move {
            crate::application::whisper::WhisperManager::ensure(self, &config, cancel)
                .await
                .map_err(|error| Response::error(error.status, error.code, &error.detail))
        })
    }
}
impl VoiceHttp {
    pub fn new(config: Arc<ConfigStore>, managed: Arc<dyn ManagedWhisper>) -> Self {
        Self { config, managed }
    }
    pub async fn handle_authenticated(&self, request: &Request, cancel: &Cancellation) -> Response {
        let config = match self.prepare(cancel).await {
            Ok(config) => config,
            Err(response) => return response,
        };
        self.transcribe_audio(&config, &request.body, cancel).await
    }
    pub async fn prepare(&self, cancel: &Cancellation) -> Result<VoiceWhisperConfig, Response> {
        let mut config = match self.config.snapshot() {
            Ok(snapshot) => snapshot.config.voice.whisper,
            Err(_) => {
                return Err(Response::error(
                    500,
                    "internal",
                    "configuration unavailable",
                ));
            }
        };
        if config.managed {
            config = match self.managed.ensure(config, cancel).await {
                Ok(config) => config,
                Err(response) => return Err(response),
            };
        }
        Ok(config)
    }
    pub async fn transcribe_audio(
        &self,
        config: &VoiceWhisperConfig,
        audio: &[u8],
        cancel: &Cancellation,
    ) -> Response {
        if audio.len() > AUDIO_LIMIT {
            return Response::error(413, "audio_too_large", "audio too large");
        }
        match transcribe(config, audio, cancel).await {
            Ok(text)
                if hallucination(
                    &text,
                    config.hallucination_phrases.as_deref().unwrap_or_default(),
                ) =>
            {
                Response::error(
                    422,
                    "hallucination",
                    "transcript matched a known hallucination phrase",
                )
            }
            Ok(text) => Response::json(200, &serde_json::json!({"ok":true,"text":text})),
            Err(response) => response,
        }
    }
}
pub fn normalized(text: &str) -> String {
    unicode::simple_lower(text)
        .chars()
        .filter(|r| !r.is_whitespace())
        .collect::<String>()
        .trim_end_matches(|r| "。．.!！?？、,，'\"“”‘’「」『』（）()[]{}".contains(r))
        .to_owned()
}
pub fn hallucination(text: &str, phrases: &[String]) -> bool {
    let text = normalized(text);
    !text.is_empty() && phrases.iter().any(|phrase| text == normalized(phrase))
}
pub fn target_url(server: &str, path: &str) -> Result<url::Url, String> {
    let mut url = url::Url::parse(server.trim()).map_err(|e| e.to_string())?;
    // Go URL.Path is percent-decoded, and setting Path invalidates the old
    // RawPath once the request suffix is appended. Preserve every leading slash.
    let mut base = decode_path(url.path());
    while base.last() == Some(&b'/') {
        base.pop();
    }
    if !path.starts_with('/') {
        base.push(b'/');
    }
    base.extend_from_slice(path.as_bytes());
    url.set_path(&encode_path(&base));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}
fn decode_path(path: &str) -> Vec<u8> {
    let mut output = Vec::new();
    let mut bytes = path.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().and_then(|b| (b as char).to_digit(16));
            let low = bytes.next().and_then(|b| (b as char).to_digit(16));
            if let (Some(high), Some(low)) = (high, low) {
                output.push((high * 16 + low) as u8);
            }
        } else {
            output.push(byte);
        }
    }
    output
}
fn encode_path(bytes: &[u8]) -> String {
    let mut output = String::new();
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || b"-_.~$&+,/:;=@".contains(byte) {
            output.push(*byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}
pub async fn transcribe(
    config: &VoiceWhisperConfig,
    audio: &[u8],
    cancel: &Cancellation,
) -> Result<String, Response> {
    if config.server_url.trim().is_empty() {
        return Err(Response::error(
            400,
            "whisper_not_configured",
            "whisper server_url is not configured",
        ));
    }
    if audio.is_empty() {
        return Err(Response::error(400, "bad_request", "empty audio"));
    }
    let timeout = if config.timeout_seconds <= 0 {
        60
    } else {
        config.timeout_seconds as u64
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    let paths = if config.request_path.trim().is_empty() {
        vec!["/v1/audio/transcriptions", "/inference"]
    } else {
        vec![config.request_path.trim()]
    };
    for (index, path) in paths.iter().enumerate() {
        let (status, result) = post(config, audio, path, deadline, cancel).await;
        if status == 404 && index == 0 && paths.len() == 2 {
            continue;
        }
        return result;
    }
    Err(Response::error(502, "whisper_failed", "whisper failed"))
}
async fn post(
    config: &VoiceWhisperConfig,
    audio: &[u8],
    path: &str,
    deadline: tokio::time::Instant,
    cancel: &Cancellation,
) -> (u16, Result<String, Response>) {
    let result = async {
        let mut url = target_url(&config.server_url, path)
            .map_err(|e| Response::error(502, "whisper_failed", &e))?;
        let mut method = reqwest::Method::POST;
        for previous in 0..3 {
            network::validate_url(url.as_str(), Policy::ManagedLoopbackHttp)
                .map_err(unreachable)?;
            let host = url.host_str().unwrap().trim_matches(['[', ']']);
            let port = url.port_or_known_default().unwrap();
            let addresses: Vec<IpAddr> = if let Ok(ip) = host.parse() {
                vec![ip]
            } else {
                headers(tokio::net::lookup_host((host, port)), deadline, cancel)
                    .await?
                    .map(|a| a.ip())
                    .collect()
            };
            let target =
                network::validated_target(url.clone(), &addresses, Policy::ManagedLoopbackHttp)
                    .map_err(unreachable)?;
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(host, &target.addresses)
                .build()
                .map_err(unreachable)?;
            let mut request = client.request(method.clone(), url.clone());
            if method == reqwest::Method::POST {
                let mut form = reqwest::multipart::Form::new()
                    .part(
                        "file",
                        reqwest::multipart::Part::bytes(audio.to_vec()).file_name("audio.wav"),
                    )
                    .text("response_format", "json");
                if path == "/v1/audio/transcriptions" {
                    form = form.text("model", "whisper-1");
                }
                let language = config.language.trim();
                if !language.is_empty()
                    && unicode::simple_fold_key(language) != unicode::simple_fold_key("auto")
                {
                    form = form.text("language", language.to_owned());
                }
                request = request.multipart(form);
            }
            let response = headers(request.send(), deadline, cancel).await?;
            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(location) = response.headers().get(reqwest::header::LOCATION)
            {
                if previous >= 2 {
                    return Err(Response::error(
                        502,
                        "whisper_unreachable",
                        "whisper unreachable: too many redirects",
                    ));
                }
                url = url
                    .join(location.to_str().map_err(unreachable)?)
                    .map_err(unreachable)?;
                if matches!(status, 301..=303) {
                    method = reqwest::Method::GET;
                }
                continue;
            }
            if !(200..300).contains(&status) {
                let bytes = error_body(response, 4096, deadline, cancel).await;
                let detail = String::from_utf8_lossy(&bytes).trim().to_owned();
                return Ok((
                    status,
                    Err(Response::error(
                        502,
                        "whisper_failed",
                        &if detail.is_empty() {
                            format!("whisper returned HTTP {status}")
                        } else {
                            detail
                        },
                    )),
                ));
            }
            let data = success_body(response, 1024 * 1024, deadline, cancel).await?;
            let text = data
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_owned();
            if text.is_empty() {
                return Ok((
                    status,
                    Err(Response::error(
                        502,
                        "whisper_failed",
                        "whisper response missing text",
                    )),
                ));
            }
            return Ok((status, Ok(text)));
        }
        Err(Response::error(502, "whisper_failed", "whisper failed"))
    }
    .await;
    result.unwrap_or_else(|response| (0, Err(response)))
}
fn unreachable(error: impl std::fmt::Display) -> Response {
    Response::error(
        502,
        "whisper_unreachable",
        &format!("whisper unreachable: {error}"),
    )
}
async fn headers<T, E: std::fmt::Display>(
    future: impl std::future::Future<Output = Result<T, E>>,
    deadline: tokio::time::Instant,
    cancel: &Cancellation,
) -> Result<T, Response> {
    if cancel.is_cancelled() {
        return Err(unreachable("request cancelled"));
    }
    if tokio::time::Instant::now() >= deadline {
        return Err(Response::error(
            504,
            "whisper_timeout",
            "whisper request timed out",
        ));
    }
    tokio::select! {_=cancel.cancelled()=>Err(unreachable("request cancelled")),value=tokio::time::timeout_at(deadline,future)=>match value{Ok(value)=>value.map_err(unreachable),Err(_)=>Err(Response::error(504,"whisper_timeout","whisper request timed out"))}}
}
async fn error_body(
    response: reqwest::Response,
    limit: usize,
    deadline: tokio::time::Instant,
    cancel: &Cancellation,
) -> Vec<u8> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while bytes.len() < limit {
        let chunk = tokio::select! {_=cancel.cancelled()=>break,value=tokio::time::timeout_at(deadline,stream.next())=>match value{Ok(value)=>value,Err(_)=>break}};
        let Some(Ok(chunk)) = chunk else {
            break;
        };
        bytes.extend_from_slice(&chunk[..chunk.len().min(limit - bytes.len())]);
    }
    bytes
}
fn decode_failure(detail: impl std::fmt::Display) -> Response {
    Response::error(
        502,
        "whisper_failed",
        &format!("decode whisper response failed: {detail}"),
    )
}
fn decode_success(bytes: &[u8], eof: bool) -> Result<Option<serde_json::Value>, Response> {
    let raw = match crate::proto::wire::first_http_raw(bytes) {
        Ok(raw) => raw,
        Err(error) if error.is_eof() && !eof => return Ok(None),
        Err(error) => return Err(decode_failure(error)),
    };
    // Go Decoder special-cases closing object/array delimiters. Scalar tokens
    // need a following byte or EOF to establish the token boundary.
    if !eof
        && !matches!(raw.get().as_bytes().first(), Some(b'{' | b'['))
        && bytes
            .iter()
            .skip_while(|byte| byte.is_ascii_whitespace())
            .count()
            == raw.get().len()
    {
        return Ok(None);
    }
    let value = crate::proto::decode_http_value(bytes).map_err(decode_failure)?;
    if value.is_object() || value.is_null() {
        Ok(Some(value))
    } else {
        Err(decode_failure(
            "cannot unmarshal JSON value into map[string]interface {}",
        ))
    }
}
async fn success_body(
    response: reqwest::Response,
    limit: usize,
    deadline: tokio::time::Instant,
    cancel: &Cancellation,
) -> Result<serde_json::Value, Response> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        if !bytes.is_empty()
            && let Some(value) = decode_success(&bytes, false)?
        {
            return Ok(value);
        }
        if bytes.len() == limit {
            return decode_success(&bytes, true)?.ok_or_else(|| decode_failure("unexpected EOF"));
        }
        let chunk = tokio::select! {_=cancel.cancelled()=>return Err(decode_failure("request cancelled")),value=tokio::time::timeout_at(deadline,stream.next())=>value.map_err(|_|decode_failure("context deadline exceeded"))?};
        match chunk {
            Some(Ok(chunk)) => {
                bytes.extend_from_slice(&chunk[..chunk.len().min(limit - bytes.len())])
            }
            Some(Err(error)) => return Err(decode_failure(error)),
            None => {
                return decode_success(&bytes, true)?
                    .ok_or_else(|| decode_failure("unexpected EOF"));
            }
        }
    }
}

#[cfg(test)]
#[path = "voice_routes/precision_tests.rs"]
mod precision_tests;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_and_hallucination_follow_source_pinned_lower() {
        assert_eq!(
            target_url("http://127.0.0.1:7777/base/?old=yes#f", "inference")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:7777/base/inference"
        );
        assert!(hallucination("　THANK\tYOU！！　", &["thank you".into()]));
        assert!(hallucination("İ", &["i".into()]));
        assert!(!hallucination(" !!! ", &["".into()]));
        assert!(!hallucination("\u{1c89}", &["\u{1c8a}".into()]));
    }
    #[tokio::test]
    async fn actual_loopback_fallback_preserves_multipart_fields() {
        use axum::{
            extract::{Multipart, State},
            response::IntoResponse,
            routing::post,
        };
        type MultipartFields = Arc<std::sync::Mutex<Vec<(String, String)>>>;
        let fields: MultipartFields = Arc::new(std::sync::Mutex::new(Vec::new()));
        async fn inference(
            State(fields): State<MultipartFields>,
            mut multipart: Multipart,
        ) -> impl IntoResponse {
            while let Some(field) = multipart.next_field().await.unwrap() {
                let name = field.name().unwrap().to_owned();
                let text = field.text().await.unwrap();
                fields.lock().unwrap().push((name, text));
            }
            axum::Json(serde_json::json!({"text":" synthetic transcript "}))
        }
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = axum::Router::new()
            .route("/inference", post(inference))
            .with_state(fields.clone());
        let stop = Cancellation::default();
        let server_stop = stop.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move { server_stop.cancelled().await })
                .await
                .unwrap()
        });
        let config = VoiceWhisperConfig {
            server_url: format!("http://127.0.0.1:{port}"),
            language: "AUTO".into(),
            ..Default::default()
        };
        assert_eq!(
            transcribe(&config, b"synthetic-audio", &Cancellation::default())
                .await
                .unwrap(),
            "synthetic transcript"
        );
        {
            let fields = fields.lock().unwrap();
            assert!(fields.contains(&("file".into(), "synthetic-audio".into())));
            assert!(fields.contains(&("response_format".into(), "json".into())));
            assert!(
                !fields
                    .iter()
                    .any(|(name, _)| matches!(name.as_str(), "language" | "model"))
            );
        }
        stop.cancel();
        server.await.unwrap();
    }
}
