//! Source SEC-C known-device admission. Invoke only after successful authentication.
use super::PushManager;
use crate::{
    hub::{auth, http::Request},
    notify::Manager,
    proto::{time::Timestamp, unicode::simple_lower},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

pub struct SecurityNotifications {
    devices: Mutex<BTreeMap<String, Timestamp>>,
    notifications: Arc<Manager>,
    push: Option<Arc<PushManager>>,
    warning: Arc<dyn Fn(&'static str) + Send + Sync>,
}
impl SecurityNotifications {
    pub fn new(
        notifications: Arc<Manager>,
        push: Option<Arc<PushManager>>,
        warning: Arc<dyn Fn(&'static str) + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            devices: Mutex::default(),
            notifications,
            push,
            warning,
        })
    }
    /// `via` is the source caller's page, ws, or pin_login tag. Admission refreshes
    /// the timestamp even when transport preflight and dispatch both call it.
    pub fn note_authenticated(&self, request: &Request, via: &str, now: Timestamp) {
        let Some((title, body)) = admit(&self.devices, request, via, now) else {
            return;
        };
        if self
            .notifications
            .send_security(title.clone(), body.clone())
            .is_err()
        {
            (self.warning)("security notification task unavailable");
        }
        if let Some(push) = &self.push
            && push.send_security(title, body).is_err()
        {
            (self.warning)("security push task unavailable");
        }
    }
}
pub fn request_device_hash(request: &Request) -> String {
    let mut identity = request.remote_addr.clone();
    if auth::is_loopback(&identity) {
        let login = simple_lower(request.header("Tailscale-User-Login").trim());
        if !login.is_empty() {
            identity = format!("tailscale:{login}");
        }
    }
    let identity = auth::remote_ip(&identity)
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| identity.trim().into());
    let bucket = user_agent_bucket(request.header("User-Agent"));
    let digest = Sha256::digest(format!("{identity}\0{bucket}").as_bytes());
    digest[..8].iter().map(|v| format!("{v:02x}")).collect()
}
fn user_agent_bucket(raw: &str) -> String {
    let lower = simple_lower(raw);
    let brand = [
        "edg/",
        "chrome/",
        "firefox/",
        "safari/",
        "curl/",
        "many-ai-cli",
    ]
    .into_iter()
    .find(|s| lower.contains(s))
    .unwrap_or("other")
    .trim_end_matches('/');
    let os = ["windows", "android", "iphone", "ipad", "mac os", "linux"]
        .into_iter()
        .find(|s| lower.contains(s))
        .unwrap_or("other");
    format!("{brand}/{os}")
}
fn admit(
    devices: &Mutex<BTreeMap<String, Timestamp>>,
    request: &Request,
    via: &str,
    now: Timestamp,
) -> Option<(String, String)> {
    if !auth::logically_remote(request) {
        return None;
    }
    let key = request_device_hash(request);
    {
        let mut devices = devices.lock().unwrap_or_else(|e| e.into_inner());
        devices.retain(|_, at| {
            now.duration_since(*at)
                .map_or(true, |age| age <= Duration::from_secs(24 * 3600))
        });
        let known = devices.contains_key(&key);
        if !known
            && devices.len() >= 256
            && let Some(oldest) = devices
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(key, _)| key.clone())
        {
            devices.remove(&oldest);
        }
        devices.insert(key, now);
        if known {
            return None;
        }
    }
    let ip = auth::remote_ip(&request.remote_addr)
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".into());
    let ua = request.header("User-Agent").trim();
    // Go byte slicing followed by JSON encoding replaces an incomplete scalar.
    let ua = if ua.len() > 80 {
        String::from_utf8_lossy(&ua.as_bytes()[..80])
            .trim()
            .to_owned()
    } else {
        ua.into()
    };
    Some((
        "many-ai-cli: new device connected".into(),
        format!("New remote connection from {ip} ({ua}) via {via}"),
    ))
}

#[cfg(test)]
mod tests;
