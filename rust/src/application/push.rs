//! Private VAPID/subscription persistence and actual encrypted Web Push delivery.
pub mod crypto;
pub mod native;
pub mod security;
mod workflow;
use crate::{
    config::RuntimePaths,
    files::safe_fs::Dir,
    hub::{
        network::{self, Policy},
        task_owner::HubTaskHandle,
    },
    proto::{
        core::*,
        time::{self, Timestamp},
    },
    storage::mask_secrets,
    terminal::session::SessionEngine,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub auth: String,
    pub p256dh: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Subscription {
    pub endpoint: String,
    pub keys: Keys,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub user_agent: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub last_seen: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    vapid_public_key: String,
    vapid_private_key: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    subscriptions: Vec<Subscription>,
}
use crate::proto::wire::{Field, GoWire, Schema};
const SCHEMAS: &[Schema] = &[
    Schema {
        name: "PushStoreFile",
        fields: &[
            Field {
                name: "vapid_public_key",
                kind: "string",
            },
            Field {
                name: "vapid_private_key",
                kind: "string",
            },
            Field {
                name: "subscriptions",
                kind: "[]PushSubscription",
            },
        ],
    },
    Schema {
        name: "PushSubscription",
        fields: &[
            Field {
                name: "endpoint",
                kind: "string",
            },
            Field {
                name: "keys",
                kind: "PushKeys",
            },
            Field {
                name: "user_agent",
                kind: "string",
            },
            Field {
                name: "created_at",
                kind: "string",
            },
            Field {
                name: "last_seen",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "PushKeys",
        fields: &[
            Field {
                name: "auth",
                kind: "string",
            },
            Field {
                name: "p256dh",
                kind: "string",
            },
        ],
    },
];
impl GoWire for Store {
    const GO_TYPE: &'static str = "PushStoreFile";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Serialize)]
pub struct Status {
    pub supported: bool,
    pub public_key: String,
    pub subscriptions: usize,
}
#[derive(Clone, Default)]
pub struct ApprovalPayload {
    pub id: String,
    pub session_id: i64,
    pub provider: String,
    pub title: String,
    pub body: String,
    pub url: String,
    pub risk: String,
    pub approve_token: String,
    pub reject_token: String,
}
pub trait PushHttpClient: Send + Sync {
    fn send<'a>(
        &'a self,
        endpoint: &'a str,
        request: crypto::EncodedPush,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, io::Result<u16>>;
}
pub type Warning = Arc<dyn Fn(&'static str, &str) + Send + Sync>;
struct State {
    store: Store,
    sent: BTreeMap<String, Timestamp>,
}
pub struct PushManager {
    state: Mutex<State>,
    directory: Dir,
    client: Arc<dyn PushHttpClient>,
    tasks: HubTaskHandle,
    warning: Warning,
}
impl PushManager {
    pub fn new(
        paths: &RuntimePaths,
        tasks: HubTaskHandle,
        client: Arc<dyn PushHttpClient>,
        warning: Warning,
    ) -> io::Result<Arc<Self>> {
        let directory = Dir::open_or_create_private(paths.root())?;
        let mut store = match directory.read("push_store.json", usize::MAX) {
            Ok(bytes) if bytes.is_empty() => Store::default(),
            Ok(bytes) => crate::proto::wire::decode::<Store>(&bytes)
                .map_err(|_| io::Error::other("parse push store: invalid JSON"))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Store::default(),
            Err(_) => return Err(io::Error::other("read push store failed")),
        };
        if store.vapid_public_key.trim().is_empty() || store.vapid_private_key.trim().is_empty() {
            let (private, public) = crypto::generate()?;
            store.vapid_private_key = private;
            store.vapid_public_key = public;
            save(&directory, &store)?;
        }
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                store,
                sent: BTreeMap::new(),
            }),
            directory,
            client,
            tasks,
            warning,
        }))
    }
    pub fn status(&self) -> Status {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        Status {
            supported: !state.store.vapid_public_key.is_empty(),
            public_key: state.store.vapid_public_key.clone(),
            subscriptions: state.store.subscriptions.len(),
        }
    }
    pub fn public_key(&self) -> String {
        self.status().public_key
    }
    pub fn upsert(&self, mut sub: Subscription, now: Timestamp) -> io::Result<()> {
        sub.endpoint = sub.endpoint.trim().into();
        sub.keys.auth = sub.keys.auth.trim().into();
        sub.keys.p256dh = sub.keys.p256dh.trim().into();
        if sub.endpoint.is_empty() || sub.keys.auth.is_empty() || sub.keys.p256dh.is_empty() {
            return Err(io::Error::other(
                "subscription endpoint and keys are required",
            ));
        }
        network::validate_url(&sub.endpoint, Policy::ExternalHttps)
            .map_err(|e| io::Error::other(format!("invalid subscription endpoint: {e}")))?;
        let now = time::format_rfc3339(now).map_err(io::Error::other)?;
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = state
            .store
            .subscriptions
            .iter_mut()
            .find(|s| s.endpoint == sub.endpoint)
        {
            sub.created_at = if existing.created_at.is_empty() {
                now.clone()
            } else {
                existing.created_at.clone()
            };
            sub.last_seen = now;
            *existing = sub;
        } else {
            sub.created_at = now.clone();
            sub.last_seen = now;
            state.store.subscriptions.push(sub);
        }
        save(&self.directory, &state.store)
    }
    pub fn delete(&self, endpoint: &str) -> io::Result<()> {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            return Err(io::Error::other("subscription endpoint is required"));
        }
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.store.subscriptions.retain(|s| s.endpoint != endpoint);
        save(&self.directory, &state.store)
    }
    pub fn notify_approval(
        self: &Arc<Self>,
        core: &SessionEngine,
        session: LiveSessionId,
        record: &ImmutableApprovalRecord,
        one_tap: Option<&crate::approval::token::OneTapManager>,
        target_url: &str,
    ) -> Result<(), SessionError> {
        let Some(details) = core.details(session) else {
            return Ok(());
        };
        let data = record.data();
        if !details
            .approval
            .record
            .as_ref()
            .is_some_and(|current| current.data().candidate.same_candidate(&data.candidate))
        {
            return Ok(());
        }
        let snapshot = details.snapshot;
        let name = [
            snapshot.display.trim(),
            snapshot.provider.trim(),
            "many-ai-cli",
        ]
        .into_iter()
        .find(|v| !v.is_empty())
        .unwrap();
        let title = if snapshot.label.is_empty() {
            format!("{name} #{}", session.0)
        } else {
            format!("{name} #{} [{}]", session.0, snapshot.label)
        };
        let text = [
            data.question.trim(),
            data.context.trim(),
            snapshot.last_message.trim(),
            snapshot.first_message.trim(),
            snapshot.cwd.trim(),
            "Approval is waiting.",
        ]
        .into_iter()
        .find(|v| !v.is_empty())
        .unwrap();
        let body = mask_secrets(text)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let id = if data.sig.is_empty() {
            format!("session-{}-{body}", session.0)
        } else {
            format!("{}#{}", data.sig, data.candidate.source_epoch.0)
        };
        let summary = crate::approval::summary::summarize(&data.question, &data.context);
        let mut payload = ApprovalPayload {
            id,
            session_id: session.0,
            provider: snapshot.provider,
            title,
            body,
            url: if target_url.is_empty() {
                format!("/?session_id={}", session.0)
            } else {
                target_url.into()
            },
            risk: summary.risk.clone(),
            ..Default::default()
        };
        if data.origin == "native"
            && !data.options.is_empty()
            && !data.sig.is_empty()
            && let Some(tokens) = one_tap
        {
            use crate::approval::token::OneTapAction;
            payload.reject_token = tokens
                .issue(
                    session,
                    &data.sig,
                    &data.sig,
                    data.candidate.source_epoch,
                    OneTapAction::Reject,
                    Timestamp::now(),
                )
                .unwrap_or_default();
            if summary.risk != "high" {
                payload.approve_token = tokens
                    .issue(
                        session,
                        &data.sig,
                        &data.sig,
                        data.candidate.source_epoch,
                        OneTapAction::Approve,
                        Timestamp::now(),
                    )
                    .unwrap_or_default();
            }
        }
        self.send_approval(payload)
    }
    pub fn send_approval(
        self: &Arc<Self>,
        mut payload: ApprovalPayload,
    ) -> Result<(), SessionError> {
        payload.id = payload.id.trim().into();
        let now = Timestamp::now();
        if payload.id.is_empty() {
            payload.id = format!("session-{}-{:x}", payload.session_id, now.unix_nanos());
        }
        let store = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            state.sent.retain(|_, at| {
                now.duration_since(*at)
                    .map_or(true, |age| age <= Duration::from_secs(3600))
            });
            if state.sent.contains_key(&payload.id) {
                return Ok(());
            }
            state.store.clone()
        };
        if store.subscriptions.is_empty()
            || store.vapid_public_key.is_empty()
            || store.vapid_private_key.is_empty()
        {
            return Ok(());
        }
        payload.body = truncate(&payload.body, 180);
        let body=serde_json::to_vec(&serde_json::json!({"type":"approval_waiting","id":payload.id,"session_id":payload.session_id,"provider":payload.provider,"title":payload.title,"body":payload.body,"url":payload.url,"risk":payload.risk,"approve_token":payload.approve_token,"reject_token":payload.reject_token})).map_err(|_|SessionError::Transport("push payload invalid".into()))?;
        let owner = self.clone();
        let permit = self.tasks.effect_permit()?;
        let cancel = permit.cancellation();
        drop(permit.start(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(10),
                owner.deliver(store, payload.id, body, true, &cancel),
            )
            .await;
        }));
        Ok(())
    }
    pub fn send_security(
        self: &Arc<Self>,
        title: String,
        body: String,
    ) -> Result<(), SessionError> {
        let store = self
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .store
            .clone();
        if store.subscriptions.is_empty()
            || store.vapid_public_key.is_empty()
            || store.vapid_private_key.is_empty()
        {
            return Ok(());
        }
        let body = truncate(&body, 180);
        let id = format!("security-{}", hash(&format!("{title}\0{body}"), 16));
        let encoded=serde_json::to_vec(&serde_json::json!({"type":"security_alert","id":id,"title":title,"body":body,"url":"/"})).map_err(|_|SessionError::Transport("push payload invalid".into()))?;
        let owner = self.clone();
        let permit = self.tasks.effect_permit()?;
        let cancel = permit.cancellation();
        drop(permit.start(async move {
            let _ = tokio::time::timeout(
                Duration::from_secs(10),
                owner.deliver(store, id, encoded, false, &cancel),
            )
            .await;
        }));
        Ok(())
    }
    async fn deliver(
        &self,
        store: Store,
        id: String,
        body: Vec<u8>,
        dedup: bool,
        cancel: &TaskCancellation,
    ) {
        let mut expired = BTreeSet::new();
        let mut success = false;
        for sub in &store.subscriptions {
            if cancel.token().is_cancelled() {
                return;
            }
            let result = match crypto::encode(
                &store,
                &sub.keys,
                &sub.endpoint,
                &body,
                hash(&id, 24),
                Timestamp::now().unix_seconds(),
            ) {
                Ok(request) => {
                    tokio::select! {result=tokio::time::timeout(Duration::from_secs(5),self.client.send(&sub.endpoint,request,cancel))=>result.unwrap_or_else(|_|Err(io::Error::new(io::ErrorKind::TimedOut,"push subscription send timed out"))),_=cancel.token().cancelled()=>return}
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(404 | 410) => {
                    expired.insert(sub.endpoint.clone());
                }
                Ok(code) if (200..300).contains(&code) => success = true,
                Ok(_) => (self.warning)("web push non-2xx response", &hash(&sub.endpoint, 12)),
                Err(_) => (self.warning)("web push send failed", &hash(&sub.endpoint, 12)),
            }
        }
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if dedup && success {
            state.sent.insert(id, Timestamp::now());
        }
        if !expired.is_empty() {
            state
                .store
                .subscriptions
                .retain(|sub| !expired.contains(&sub.endpoint));
            if save(&self.directory, &state.store).is_err() {
                (self.warning)("save push store after expiry failed", "");
            }
        }
    }
}
fn save(dir: &Dir, store: &Store) -> io::Result<()> {
    dir.replace(
        "push_store.json",
        &serde_json::to_vec_pretty(store)
            .map_err(|_| io::Error::other("marshal push store failed"))?,
        0o600,
    )
}
pub(super) fn hash(raw: &str, len: usize) -> String {
    let bytes = Sha256::digest(raw.as_bytes());
    let text = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    text[..len].into()
}
pub(super) fn truncate(raw: &str, max: usize) -> String {
    if raw.len() <= max {
        return raw.into();
    }
    let mut end = max;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw[..end].trim().into()
}
#[cfg(test)]
mod tests;
