//! Transport-independent HTTP kernel. The Axum adapter preserves guard ordering by
//! dispatching methods here, rather than allowing the framework to emit early 405s.
use serde::Serialize;
use std::collections::BTreeMap;

pub const JSON_BODY_LIMIT: usize = 1024 * 1024;

pub fn decode_json<T: crate::proto::wire::GoWire>(request: &Request) -> Result<T, Response> {
    // Go decodes one value through MaxBytesReader. Bytes after that value are
    // not an EOF requirement and must not turn a valid request into an error.
    crate::proto::decode_http_json(&request.body[..request.body.len().min(JSON_BODY_LIMIT)])
        .map_err(|_| Response::error(400, "bad_request", "invalid json"))
}

#[derive(Clone, Debug, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub host: String,
    pub remote_addr: String,
    pub tls: bool,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Request {
    pub fn header(&self, name: &str) -> &str {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }
    pub fn query(&self, name: &str) -> String {
        url::form_urlencoded::parse(self.query.as_bytes())
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    }
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("cookie"))
            .flat_map(|(_, v)| v.split(';'))
            .filter_map(|part| part.trim().split_once('='))
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.trim().trim_matches('"'))
    }
}

#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub cookies: Vec<String>,
    pub body: Vec<u8>,
}
impl Response {
    pub fn bytes(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        let mut response = Self {
            status,
            headers: BTreeMap::new(),
            cookies: Vec::new(),
            body: body.into(),
        };
        response
            .headers
            .insert("Content-Type".into(), content_type.into());
        response.security_headers();
        response
    }
    pub fn json(status: u16, value: &impl Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(mut bytes) => {
                bytes.push(b'\n');
                Self::bytes(status, "application/json", bytes)
            }
            Err(_) => Self::bytes(
                500,
                "application/json",
                b"{\"ok\":false,\"error\":\"internal\",\"detail\":\"failed to encode response\"}\n"
                    .to_vec(),
            ),
        }
    }
    pub fn error(status: u16, code: &str, detail: &str) -> Self {
        Self::json(
            status,
            &serde_json::json!({"ok":false,"error":code,"detail":detail}),
        )
    }
    pub fn no_store(mut self) -> Self {
        self.headers
            .insert("Cache-Control".into(), "no-store".into());
        self
    }
    pub fn security_headers(&mut self) {
        for (name, value) in [
            ("Content-Security-Policy", content_security_policy(&[])),
            ("X-Frame-Options", "DENY".into()),
            ("X-Content-Type-Options", "nosniff".into()),
            ("Referrer-Policy", "no-referrer".into()),
            ("Cross-Origin-Opener-Policy", "same-origin".into()),
            (
                "Permissions-Policy",
                "camera=(), geolocation=(), payment=(), usb=(), microphone=(self)".into(),
            ),
            ("Service-Worker-Allowed", "/".into()),
        ] {
            self.headers.insert(name.into(), value);
        }
    }
}
pub fn content_security_policy(hosts: &[String]) -> String {
    let mut src = "'self' ws://127.0.0.1:* ws://localhost:*".to_string();
    for raw in hosts {
        let host = raw.trim().trim_end_matches('.');
        if host.is_empty() {
            continue;
        }
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.into()
        };
        src.push_str(&format!(" ws://{host}:* wss://{host}:*"));
    }
    format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' data: blob:; connect-src {src}; font-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"
    )
}

pub fn require_method(request: &Request, methods: &[&str]) -> Result<(), Response> {
    if methods.is_empty() || methods.contains(&request.method.as_str()) {
        Ok(())
    } else {
        Err(Response::error(
            405,
            "method_not_allowed",
            "method not allowed",
        ))
    }
}

pub fn random_hex(bytes: usize) -> Result<String, getrandom::Error> {
    let mut buf = vec![0; bytes];
    getrandom::fill(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}
