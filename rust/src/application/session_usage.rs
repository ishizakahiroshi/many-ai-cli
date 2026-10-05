//! Source session-usage endpoint: numeric memory, profile observation, UI broadcast.
mod normalize;
mod pricing;
mod request;
#[cfg(test)]
mod tests;
use crate::{
    application::event_observer::EventWarning,
    hub::{
        http::{Request, Response, decode_json, require_method},
        task_owner::HubTaskHandle,
    },
    proto::{self, core::*, time::Timestamp},
    terminal::session::SessionEngine,
};
pub use request::UsageRequest;
use std::sync::Arc;
pub trait UsageSubscription: Send + Sync {
    fn record_session_usage<'a>(
        &'a self,
        binding: SessionBinding,
        stat: &'a proto::Message,
        record_at: Timestamp,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
}
pub struct SessionUsage {
    core: Arc<SessionEngine>,
    effects: Arc<dyn CoreEffectSink>,
    subscription: Arc<dyn UsageSubscription>,
    tasks: HubTaskHandle,
    warning: EventWarning,
}
impl SessionUsage {
    pub fn new(
        core: Arc<SessionEngine>,
        effects: Arc<dyn CoreEffectSink>,
        subscription: Arc<dyn UsageSubscription>,
        tasks: HubTaskHandle,
        warning: EventWarning,
    ) -> Arc<Self> {
        Arc::new(Self {
            core,
            effects,
            subscription,
            tasks,
            warning,
        })
    }
    pub async fn receive(self: &Arc<Self>, request: &Request, now: Timestamp) -> Response {
        if let Err(error) = require_method(request, &["POST"]) {
            return error;
        }
        let req: UsageRequest = match decode_json(request) {
            Ok(req) => req,
            Err(error) => return error,
        };
        if req.session_id <= 0 {
            return Response::error(400, "bad_request", "session_id required");
        }
        let Some(details) = self.core.details(LiveSessionId(req.session_id)) else {
            return Response::error(404, "not_found", "session not found");
        };
        if !req.provider.is_empty()
            && !details.snapshot.provider.is_empty()
            && req.provider != details.snapshot.provider
        {
            (self.warning)(
                "usage relay provider mismatch",
                &SessionError::InvalidRequest("reported provider differs".into()),
            );
            return Response::json(
                200,
                &serde_json::json!({"ok":true,"ignored":"provider_mismatch"}),
            );
        }
        if details.snapshot.provider == "codex"
            && !req.transcript_path.is_empty()
            && self
                .core
                .set_usage_transcript(details.binding, &req.transcript_path)
                .is_err()
        {
            return Response::error(404, "not_found", "session not found");
        }
        let (message, record_at) = match normalize::normalize(req, &details.snapshot.model, now) {
            Ok(value) => value,
            Err(detail) => return Response::error(400, "bad_request", detail),
        };
        let permit = match self.tasks.effect_permit() {
            Ok(permit) => permit,
            Err(_) => {
                return Response::error(503, "usage_unavailable", "usage service unavailable");
            }
        };
        let owner = self.clone();
        let binding = details.binding;
        let task = permit.start(async move {
            let effects = owner.core.record_session_usage(binding, message.clone())?;
            // Required actual profile owner, outside the core state lock.
            owner
                .subscription
                .record_session_usage(binding, &message, record_at)
                .await?;
            owner
                .effects
                .apply(effects)
                .await
                .map_err(|_| SessionError::InvalidRequest("usage publication failed".into()))
        });
        match task.wait().await {
            Ok(Ok(())) => Response::json(200, &serde_json::json!({"ok":true})),
            _ => Response::error(503, "usage_unavailable", "usage service unavailable"),
        }
    }
}
