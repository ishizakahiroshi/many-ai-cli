//! Outbound notifications, retained by the Hub effect owner. Request waiters
//! never own delivery; an aborted delivery releases its deduplication claim.
use crate::{
    config::{NotifyBackendConfig, NotifyConfig},
    hub::task_owner::HubTaskHandle,
    proto::{core::SessionError, time::Timestamp},
    storage::mask_secrets,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Default)]
pub struct ApprovalPayload {
    pub id: String,
    pub session_id: i64,
    pub title: String,
    pub body: String,
    pub risk: String,
    pub approve_url: String,
    pub reject_url: String,
    pub open_url: String,
}
#[derive(Clone, Default)]
pub struct DonePayload {
    pub id: String,
    pub session_id: i64,
    pub title: String,
    pub summary: String,
    pub kind: String,
}
#[derive(Default)]
struct State {
    sent: BTreeMap<String, Timestamp>,
    pending: std::collections::BTreeSet<String>,
}
pub type Warning = dyn Fn(&str, &'static str) + Send + Sync;
pub struct Manager {
    config: Mutex<NotifyConfig>,
    state: Mutex<State>,
    client: reqwest::Client,
    tasks: HubTaskHandle,
    warning: Arc<Warning>,
}
fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|error| error.into_inner())
}
struct Claim {
    owner: Arc<Manager>,
    id: String,
}
impl Drop for Claim {
    fn drop(&mut self) {
        lock(&self.owner.state).pending.remove(&self.id);
    }
}
impl Manager {
    pub fn new(
        config: NotifyConfig,
        tasks: HubTaskHandle,
        warning: Arc<Warning>,
    ) -> Result<Arc<Self>, reqwest::Error> {
        Ok(Arc::new(Self {
            config: Mutex::new(config),
            state: Mutex::default(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()?,
            tasks,
            warning,
        }))
    }
    pub fn update_config(&self, config: NotifyConfig) {
        *lock(&self.config) = config;
    }
    pub fn send_approval(self: &Arc<Self>, payload: ApprovalPayload) -> Result<bool, SessionError> {
        let config = lock(&self.config).clone();
        if !event(&config, "approval") {
            return Ok(false);
        }
        let body = if config.include_body {
            truncate(&mask_secrets(&payload.body), 200)
        } else {
            format!("session #{}: 承認要求", payload.session_id)
        };
        self.enqueue(
            config,
            payload.id.clone(),
            "approval",
            payload.session_id,
            payload.title.clone(),
            body,
            Some(payload),
            None,
        )
    }
    pub fn send_done(self: &Arc<Self>, payload: DonePayload) -> Result<bool, SessionError> {
        let config = lock(&self.config).clone();
        if !event(&config, "done") {
            return Ok(false);
        }
        let label = kind_label(&payload.kind);
        let title = format!("[{label}] {}", payload.title);
        let body = if config.include_body {
            truncate(&mask_secrets(&payload.summary), 200)
        } else {
            format!("session #{}: {label}", payload.session_id)
        };
        self.enqueue(
            config,
            payload.id,
            "done",
            payload.session_id,
            title,
            body,
            None,
            Some(payload.kind),
        )
    }
    pub fn send_security(
        self: &Arc<Self>,
        title: String,
        body: String,
    ) -> Result<bool, SessionError> {
        let config = lock(&self.config).clone();
        let id = format!("security-{title}-{body}");
        self.enqueue(
            config,
            id,
            "security",
            0,
            title,
            truncate(&body, 200),
            None,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn enqueue(
        self: &Arc<Self>,
        config: NotifyConfig,
        id: String,
        prefix: &str,
        session: i64,
        title: String,
        body: String,
        approval: Option<ApprovalPayload>,
        kind: Option<String>,
    ) -> Result<bool, SessionError> {
        let backends = config.backends.unwrap_or_default();
        if backends.is_empty() {
            return Ok(false);
        }
        let now = Timestamp::now();
        let id = if prefix == "security" {
            id
        } else if id.trim().is_empty() {
            format!(
                "{prefix}-{session}-{}",
                super::hub::http::random_hex(12).map_err(|_| SessionError::Transport(
                    "notification identifier unavailable".into()
                ))?
            )
        } else {
            id.trim().to_owned()
        };
        {
            let mut state = lock(&self.state);
            state.sent.retain(|_, at| {
                now.duration_since(*at)
                    .map_or(true, |age| age <= Duration::from_secs(3600))
            });
            if state.sent.contains_key(&id) || !state.pending.insert(id.clone()) {
                return Ok(false);
            }
        }
        let claim = Claim {
            owner: self.clone(),
            id,
        };
        let permit = self.tasks.effect_permit()?;
        let cancellation = permit.cancellation();
        let _receipt = permit.start(async move {
            let mut sends = futures_util::stream::FuturesUnordered::new();
            for backend in backends { sends.push(claim.owner.send(backend, title.clone(), body.clone(), approval.clone(), kind.clone())); }
            use futures_util::StreamExt;
            let mut successful = false;
            loop {
                tokio::select! {
                    _ = cancellation.token().cancelled() => break,
                    next = sends.next() => match next { Some(ok) => successful |= ok, None => break },
                }
            }
            if successful { lock(&claim.owner.state).sent.insert(claim.id.clone(), Timestamp::now()); }
            drop(sends);
            drop(claim);
        });
        Ok(true)
    }
    async fn send(
        &self,
        backend: NotifyBackendConfig,
        title: String,
        body: String,
        approval: Option<ApprovalPayload>,
        kind: Option<String>,
    ) -> bool {
        let request = match backend.r#type.trim().to_ascii_lowercase().as_str() {
            "webhook" => {
                let mut json = serde_json::json!({"title":title,"body":body});
                if let Some(kind) = &kind {
                    json["kind"] = kind.clone().into();
                }
                self.client.post(&backend.url).json(&json)
            }
            "ntfy" => {
                let url = format!(
                    "{}/{}",
                    backend.url.trim_end_matches('/'),
                    backend.topic.trim()
                );
                let (priority, tags) = match kind
                    .as_deref()
                    .map(|value| value.trim().to_ascii_lowercase())
                    .as_deref()
                {
                    Some("success") => ("default", "white_check_mark"),
                    Some("failure") => ("high", "x"),
                    Some("aborted") => ("default", "pause_button"),
                    Some("needs_action") => ("high", "warning"),
                    Some(_) => ("default", "white_check_mark"),
                    None => ("high", "bell"),
                };
                let mut request = self
                    .client
                    .post(url)
                    .header("Title", title)
                    .header("Priority", priority)
                    .header("Tags", tags)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(body);
                if let Some(payload) = approval {
                    let actions = approval_actions(&payload);
                    if !actions.is_empty() {
                        request = request.header("Actions", actions);
                    }
                }
                request
            }
            _ => {
                (self.warning)(&backend.r#type, "unsupported notification backend");
                return false;
            }
        };
        match request.send().await {
            Ok(response) if response.status().is_success() => true,
            Ok(_) => {
                (self.warning)(&backend.r#type, "notification HTTP status failed");
                false
            }
            Err(_) => {
                (self.warning)(&backend.r#type, "notification request failed");
                false
            }
        }
    }
    pub async fn send_test(
        &self,
        backend: NotifyBackendConfig,
        title: String,
        body: String,
    ) -> bool {
        self.send(backend, title, body, None, None).await
    }
}
fn event(config: &NotifyConfig, target: &str) -> bool {
    config
        .events
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|event| event.trim().eq_ignore_ascii_case(target))
}
pub fn kind_label(kind: &str) -> &'static str {
    match kind.trim().to_ascii_lowercase().as_str() {
        "success" => "成功",
        "failure" => "失敗",
        "aborted" => "中断",
        "needs_action" => "要判断",
        _ => "完了",
    }
}
fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_owned();
    }
    let mut cut = max;
    while !value.is_char_boundary(cut) {
        cut -= 1;
    }
    value[..cut].trim().to_owned()
}
fn safe_action(value: &str) -> bool {
    let value = value.trim();
    let userinfo = value
        .split_once("://")
        .map(|(_, rest)| {
            rest.split(['/', '?', '#'])
                .next()
                .unwrap_or_default()
                .contains('@')
        })
        .unwrap_or(false);
    !userinfo
        && reqwest::Url::parse(value).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none_or(str::is_empty)
        })
}
pub fn approval_actions(payload: &ApprovalPayload) -> String {
    let mut actions = Vec::new();
    if safe_action(&payload.approve_url) && !payload.risk.trim().eq_ignore_ascii_case("high") {
        actions.push(format!(
            "http, Approve, {}, method=POST",
            payload.approve_url
        ));
    }
    if safe_action(&payload.reject_url) {
        actions.push(format!("http, Reject, {}, method=POST", payload.reject_url));
    }
    if safe_action(&payload.open_url) {
        actions.push(format!("view, Open, {}", payload.open_url));
    }
    actions.join("; ")
}
pub fn random_topic() -> std::io::Result<String> {
    super::hub::http::random_hex(12)
        .map(|value| format!("anyaicli-{value}"))
        .map_err(|_| std::io::Error::other("notification topic randomness unavailable"))
}
pub fn validate_backend(backend: &NotifyBackendConfig) -> Result<(), &'static str> {
    if !matches!(backend.r#type.trim(), "ntfy" | "webhook") {
        return Err("backend type must be ntfy or webhook");
    }
    if backend.url.trim().is_empty() {
        return Err("backend URL is required");
    }
    let url = reqwest::Url::parse(backend.url.trim()).map_err(|_| "backend URL is invalid")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("backend URL scheme must be http or https");
    }
    if url.host_str().is_none() {
        return Err("backend URL must have a host");
    }
    if backend.r#type.trim() == "ntfy" && backend.topic.trim().is_empty() {
        return Err("ntfy backend requires a non-empty topic");
    }
    Ok(())
}
impl crate::proto::wire::GoWire for NotifyConfig {
    const GO_TYPE: &'static str = "NotifyConfig";
    const SCHEMAS: &'static [crate::proto::wire::Schema] = &[
        crate::proto::wire::Schema {
            name: "NotifyConfig",
            fields: &[
                crate::proto::wire::Field {
                    name: "backends",
                    kind: "[]NotifyBackendConfig",
                },
                crate::proto::wire::Field {
                    name: "events",
                    kind: "[]string",
                },
                crate::proto::wire::Field {
                    name: "include_body",
                    kind: "bool",
                },
            ],
        },
        crate::proto::wire::Schema {
            name: "NotifyBackendConfig",
            fields: &[
                crate::proto::wire::Field {
                    name: "type",
                    kind: "string",
                },
                crate::proto::wire::Field {
                    name: "url",
                    kind: "string",
                },
                crate::proto::wire::Field {
                    name: "topic",
                    kind: "string",
                },
            ],
        },
    ];
}
#[cfg(test)]
mod tests;
