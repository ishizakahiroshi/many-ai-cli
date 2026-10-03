//! Optional remote PIN sessions. Cookies are device-bound and live only while
//! their nonce exists in this Hub instance. No plaintext PIN is stored or logged.
use super::{
    auth::*,
    http::{Request, Response, decode_json, random_hex},
};
use crate::{
    config::{Config, ConfigStore},
    proto::wire::{Field, GoWire, Schema},
};
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Mutex};

pub const UI_ORIGIN_COOKIE: &str = "MANY_AI_CLI_ui_origin";
const PIN_TTL: i64 = 12 * 60 * 60;
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
fn sign(secret: &str, payload: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC arbitrary key length");
    mac.update(payload.as_bytes());
    hex(mac.finalize().into_bytes())
}
fn signed(secret: &str, payload: &str, purpose: &str) -> String {
    format!("{payload}.{}", sign(secret, &format!("{purpose}{payload}")))
}
fn verify_signed<'a>(secret: &str, value: &'a str, purpose: &str) -> Option<&'a str> {
    if secret.is_empty() {
        return None;
    }
    let (payload, sig) = value.rsplit_once('.')?;
    if payload.is_empty() || !valid_token(sig, &sign(secret, &format!("{purpose}{payload}"))) {
        return None;
    }
    Some(payload)
}
pub fn valid_pin_format(pin: &str) -> bool {
    (6..=32).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_digit())
}
fn user_agent_bucket(ua: &str) -> String {
    let ua = ua.to_lowercase();
    let brand = [
        "edg/",
        "chrome/",
        "firefox/",
        "safari/",
        "curl/",
        "many-ai-cli",
    ]
    .into_iter()
    .find(|v| ua.contains(v))
    .unwrap_or("other")
    .trim_end_matches('/');
    let os = ["windows", "android", "iphone", "ipad", "mac os", "linux"]
        .into_iter()
        .find(|v| ua.contains(v))
        .unwrap_or("other");
    format!("{brand}/{os}")
}
pub fn device_hash(request: &Request) -> String {
    let login = request.header("Tailscale-User-Login").trim().to_lowercase();
    let identity = if is_loopback(&request.remote_addr) && !login.is_empty() {
        format!("tailscale:{login}")
    } else {
        remote_ip(&request.remote_addr)
            .map(|v| v.to_string())
            .unwrap_or_else(|| request.remote_addr.trim().into())
    };
    hex(Sha256::digest(
        format!(
            "{identity}\0{}",
            user_agent_bucket(request.header("User-Agent"))
        )
        .as_bytes(),
    ))[..16]
        .into()
}
fn rate_key(request: &Request) -> String {
    if is_loopback(&request.remote_addr) && logically_remote(request) {
        let login = request.header("Tailscale-User-Login").trim().to_lowercase();
        if login.is_empty() {
            return String::new();
        }
        return format!(
            "ts:{}",
            &hex(Sha256::digest(format!("tailscale:{login}").as_bytes()))[..16]
        );
    }
    remote_ip(&request.remote_addr)
        .map(|v| v.to_string())
        .unwrap_or_else(|| request.remote_addr.trim().into())
}
#[derive(Default)]
struct Attempt {
    fails: u32,
    level: usize,
    until: i64,
    last: i64,
}
#[derive(Default)]
pub struct Limiter {
    ips: BTreeMap<String, Attempt>,
    global_fails: u32,
    global_until: i64,
    global_seen: i64,
}
impl Limiter {
    pub fn retry_after(&mut self, key: &str, now: i64) -> i64 {
        self.ips
            .retain(|_, a| now - a.last <= 3600 || now <= a.until);
        if now < self.global_until {
            return self.global_until - now + 1;
        }
        self.ips
            .get(key)
            .filter(|a| now < a.until)
            .map(|a| a.until - now + 1)
            .unwrap_or(0)
    }
    pub fn begin(&mut self, key: &str, now: i64) -> i64 {
        let retry = self.retry_after(key, now);
        if retry > 0 {
            return retry;
        }
        if !key.is_empty() {
            let a = self.ips.entry(key.into()).or_default();
            a.last = now;
            a.fails += 1;
            if a.fails >= 5 {
                a.until = now + [60, 300, 1800][a.level.min(2)];
                a.level = (a.level + 1).min(2);
                a.fails = 0;
            }
        }
        if now - self.global_seen > 3600 {
            self.global_fails = 0;
        }
        self.global_seen = now;
        self.global_fails += 1;
        if self.global_fails >= 30 {
            self.global_until = now + 60;
            self.global_fails = 0;
        }
        0
    }
    pub fn success(&mut self, key: &str) {
        if !key.is_empty() {
            self.ips.remove(key);
        }
    }
}
#[derive(Clone)]
struct Session {
    expiry: i64,
    device: String,
    issued: u64,
}
#[derive(Default)]
struct State {
    sessions: BTreeMap<String, Session>,
    limiter: Limiter,
    ui_secret: String,
    sequence: u64,
}
#[derive(Default)]
pub struct AuthService {
    state: Mutex<State>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct PinRequest {
    pin: String,
    clear: bool,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct LoginRequest {
    pin: String,
}
impl GoWire for LoginRequest {
    const GO_TYPE: &'static str = "LoginRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "LoginRequest",
        fields: &[Field {
            name: "pin",
            kind: "string",
        }],
    }];
}
impl GoWire for PinRequest {
    const GO_TYPE: &'static str = "PinRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "PinRequest",
        fields: &[
            Field {
                name: "pin",
                kind: "string",
            },
            Field {
                name: "clear",
                kind: "bool",
            },
        ],
    }];
}

pub fn cookie(name: &str, value: &str, strict: bool, max_age: i64, request: &Request) -> String {
    format!(
        "{name}={value}; Path=/; Max-Age={max_age}; HttpOnly{}; SameSite={}",
        if uses_https(request) { "; Secure" } else { "" },
        if strict { "Strict" } else { "Lax" }
    )
}
impl AuthService {
    pub fn has_valid_pin(&self, request: &Request, config: &Config, now: i64) -> bool {
        let Some(value) = request.cookie(PIN_COOKIE) else {
            return false;
        };
        let Some(payload) = verify_signed(&config.auth_cookie_secret, value, "") else {
            return false;
        };
        let parts: Vec<_> = payload.split('.').collect();
        if parts.len() != 3 || parts[1].is_empty() {
            return false;
        }
        let Ok(expiry) = parts[0].parse::<i64>() else {
            return false;
        };
        if expiry <= now || !valid_token(parts[2], &device_hash(request)) {
            return false;
        }
        let Ok(mut s) = self.state.lock() else {
            return false;
        };
        s.sessions.retain(|_, v| v.expiry > now);
        s.sessions
            .get(parts[1])
            .is_some_and(|v| v.expiry == expiry && valid_token(&v.device, parts[2]))
    }
    pub fn guard(
        &self,
        request: &Request,
        config: &Config,
        port: i64,
        methods: &[&str],
        now: i64,
    ) -> Result<(), Response> {
        guard_base(request, config, port, methods)?;
        if logically_remote(request)
            && !config.remote_pin_hash.trim().is_empty()
            && !self.has_valid_pin(request, config, now)
        {
            return Err(Response::error(401, "pin_required", "remote pin required").no_store());
        }
        Ok(())
    }
    fn issue(&self, request: &Request, secret: &str, now: i64) -> Result<String, Response> {
        let nonce = random_hex(16)
            .map_err(|_| Response::error(500, "internal", "failed to create pin session"))?;
        let expiry = now + PIN_TTL;
        let device = device_hash(request);
        let mut s = self
            .state
            .lock()
            .map_err(|_| Response::error(500, "internal", "authentication unavailable"))?;
        s.sessions.retain(|_, v| v.expiry > now);
        while s.sessions.len() >= 256 {
            let key = s
                .sessions
                .iter()
                .min_by_key(|(_, v)| v.issued)
                .map(|(k, _)| k.clone())
                .unwrap();
            s.sessions.remove(&key);
        }
        s.sequence = s.sequence.saturating_add(1);
        let issued = s.sequence;
        s.sessions.insert(
            nonce.clone(),
            Session {
                expiry,
                device: device.clone(),
                issued,
            },
        );
        Ok(signed(secret, &format!("{expiry}.{nonce}.{device}"), ""))
    }
    pub fn status(&self, request: &Request, config: &Config, now: i64) -> Response {
        let enabled = !config.remote_pin_hash.trim().is_empty();
        let remote = logically_remote(request);
        let authed = !enabled || !remote || self.has_valid_pin(request, config, now);
        let retry = if enabled && remote {
            self.state
                .lock()
                .map(|mut s| s.limiter.retry_after(&rate_key(request), now))
                .unwrap_or(1)
        } else {
            0
        };
        Response::json(200,&json!({"pin_enabled":enabled,"remote":remote,"remote_exposed":!config.hub.allowed_hosts.is_empty()||!config.hub.trusted_networks.is_empty(),"authed":authed,"locked":retry>0,"retry_after":retry})).no_store()
    }
    pub fn login(&self, request: &Request, config: &Config, now: i64) -> Response {
        let result = (|| {
            if config.remote_pin_hash.trim().is_empty() {
                return Response::json(200, &json!({"ok":true,"pin_enabled":false}));
            }
            if config.auth_cookie_secret.is_empty() {
                return Response::error(500, "internal", "pin not configured");
            }
            let body = match decode_json::<LoginRequest>(request) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let key = rate_key(request);
            let retry = self
                .state
                .lock()
                .map(|mut s| s.limiter.begin(&key, now))
                .unwrap_or(1);
            if retry > 0 {
                return locked(retry);
            }
            if !valid_pin_format(&body.pin) {
                return Response::error(401, "bad_pin", "incorrect pin");
            }
            if !bcrypt::verify(&body.pin, &config.remote_pin_hash).unwrap_or(false) {
                let retry = self
                    .state
                    .lock()
                    .map(|mut s| s.limiter.retry_after(&key, now))
                    .unwrap_or(1);
                return if retry > 0 {
                    locked(retry)
                } else {
                    Response::error(401, "bad_pin", "incorrect pin")
                };
            }
            if let Ok(mut s) = self.state.lock() {
                s.limiter.success(&key);
            }
            let value = match self.issue(request, &config.auth_cookie_secret, now) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let mut response = Response::json(200, &json!({"ok":true}));
            response
                .cookies
                .push(cookie(PIN_COOKIE, &value, true, PIN_TTL, request));
            response
        })();
        result.no_store()
    }
    pub fn logout(&self, request: &Request, config: &Config) -> Response {
        if let Some(value) = request.cookie(PIN_COOKIE)
            && let Some(payload) = verify_signed(&config.auth_cookie_secret, value, "")
            && let Some(nonce) = payload.split('.').nth(1)
            && let Ok(mut s) = self.state.lock()
        {
            s.sessions.remove(nonce);
        }
        let mut response = Response::json(200, &json!({"ok":true})).no_store();
        response
            .cookies
            .push(cookie(TOKEN_COOKIE, "", false, 0, request));
        response
            .cookies
            .push(cookie(PIN_COOKIE, "", true, 0, request));
        response
    }
    pub fn set_pin(&self, request: &Request, store: &ConfigStore, now: i64) -> Response {
        let result = (|| {
            let mut snapshot = match store.snapshot() {
                Ok(s) => s,
                Err(_) => return Response::error(500, "internal", "configuration unavailable"),
            };
            if logically_remote(request) && !self.has_valid_pin(request, &snapshot.config, now) {
                return Response::error(
                    403,
                    "forbidden",
                    "set-pin from a remote address requires existing PIN authentication or a local (loopback) session",
                );
            }
            let body = match decode_json::<PinRequest>(request) {
                Ok(v) => v,
                Err(e) => return e,
            };
            if body.clear || body.pin.trim().is_empty() {
                snapshot.config.remote_pin_hash.clear();
                return match store.persist(snapshot.revision, snapshot.config) {
                    Ok(_) => Response::json(200, &json!({"ok":true,"pin_enabled":false})),
                    Err(_) => Response::error(500, "internal", "failed to persist config"),
                };
            }
            if !valid_pin_format(&body.pin) {
                return Response::error(400, "bad_pin_format", "pin must be 6 or more digits");
            }
            // golang.org/x/crypto/bcrypt.DefaultCost is 10; crate default is 12.
            let hash = match bcrypt::hash(&body.pin, 10) {
                Ok(v) => v,
                Err(_) => return Response::error(500, "internal", "failed to hash pin"),
            };
            let candidate = match random_hex(32) {
                Ok(v) => v,
                Err(_) => return Response::error(500, "internal", "failed to init secret"),
            };
            snapshot.config.remote_pin_hash = hash;
            if snapshot.config.auth_cookie_secret.is_empty() {
                snapshot.config.auth_cookie_secret = candidate;
            }
            let secret = snapshot.config.auth_cookie_secret.clone();
            if store.persist(snapshot.revision, snapshot.config).is_err() {
                return Response::error(500, "internal", "failed to persist config");
            }
            let value = match self.issue(request, &secret, now) {
                Ok(v) => v,
                Err(e) => return e,
            };
            let mut response = Response::json(200, &json!({"ok":true,"pin_enabled":true}));
            response
                .cookies
                .push(cookie(PIN_COOKIE, &value, true, PIN_TTL, request));
            response
        })();
        result.no_store()
    }
    /// Core UI invalidation must follow a successful disk rotation; the caller
    /// cannot acknowledge revocation before receiving `Ok` from this method.
    pub fn rotate(
        &self,
        request: &Request,
        store: &ConfigStore,
        port: i64,
        now: i64,
    ) -> Result<Response, Response> {
        let mut snap = store
            .snapshot()
            .map_err(|_| Response::error(500, "internal", "configuration unavailable"))?;
        if logically_remote(request) && !self.has_valid_pin(request, &snap.config, now) {
            return Err(Response::error(403,"forbidden","revoke-all from a remote address requires existing PIN authentication or a local (loopback) session").no_store());
        }
        let token = random_hex(32)
            .map_err(|_| Response::error(500, "internal", "failed to generate token"))?;
        let secret = random_hex(32)
            .map_err(|_| Response::error(500, "internal", "failed to generate secret"))?;
        snap.config.token = token.clone();
        snap.config.auth_cookie_secret = secret;
        store
            .persist(snap.revision, snap.config)
            .map_err(|_| Response::error(500, "internal", "failed to persist config"))?;
        if let Ok(mut s) = self.state.lock() {
            s.sessions.clear();
            s.ui_secret.clear();
        }
        Ok(Response::json(
            200,
            &json!({"token":token,"hub_url":format!("http://127.0.0.1:{port}/?token={token}")}),
        )
        .no_store())
    }
    fn ui_secret(&self, config: &Config) -> Option<String> {
        if !config.auth_cookie_secret.is_empty() {
            return Some(config.auth_cookie_secret.clone());
        }
        let mut s = self.state.lock().ok()?;
        if s.ui_secret.is_empty() {
            s.ui_secret = random_hex(32).ok()?;
        }
        Some(s.ui_secret.clone())
    }
    pub fn issue_ui_cookie(&self, request: &Request, config: &Config, now: i64) -> Option<String> {
        let secret = self.ui_secret(config)?;
        let payload = format!("{}.{}", now + 86400, random_hex(16).ok()?);
        Some(cookie(
            UI_ORIGIN_COOKIE,
            &signed(&secret, &payload, "ui-origin-v1:"),
            false,
            86400,
            request,
        ))
    }
    pub fn valid_ui_cookie(&self, request: &Request, config: &Config, now: i64) -> bool {
        let Some(secret) = self.ui_secret(config) else {
            return false;
        };
        let Some(value) = request.cookie(UI_ORIGIN_COOKIE) else {
            return false;
        };
        let Some(payload) = verify_signed(&secret, value.trim(), "ui-origin-v1:") else {
            return false;
        };
        let parts: Vec<_> = payload.split('.').collect();
        parts.len() == 2 && !parts[1].is_empty() && parts[0].parse::<i64>().is_ok_and(|v| now < v)
    }
}
fn locked(retry: i64) -> Response {
    let mut response = Response::error(429, "locked_out", "too many attempts");
    response
        .headers
        .insert("Retry-After".into(), retry.to_string());
    response
}
