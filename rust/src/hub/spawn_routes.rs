//! Router applies source method/Host/Origin/token/PIN guards before this adapter.
use super::http::{Request, Response, decode_json};
use crate::{
    application::ordinary_spawn::{OrdinarySpawnRequest, OrdinarySpawnService},
    proto::{
        core::{HttpWaitCancellation, VerifiedConfirmationRequest},
        time::Timestamp,
    },
};
use std::sync::Arc;
pub struct SpawnHttp {
    service: Arc<OrdinarySpawnService>,
}
impl SpawnHttp {
    pub fn new(service: Arc<OrdinarySpawnService>) -> Self {
        Self { service }
    }
    pub async fn spawn_authenticated(
        &self,
        request: &Request,
        verified: &VerifiedConfirmationRequest,
        waiter: &HttpWaitCancellation,
        now: Timestamp,
        offset_seconds: i32,
    ) -> Response {
        let body: OrdinarySpawnRequest = match decode_json(request) {
            Ok(body) => body,
            Err(response) => return response,
        };
        match self
            .service
            .spawn(body, verified, waiter, now, offset_seconds)
            .await
        {
            Ok(Some(id)) => {
                Response::json(200, &serde_json::json!({"ok":true,"orchestration_id":id.0}))
            }
            Ok(None) => Response::json(200, &serde_json::json!({"ok":true})),
            Err(error) => Response::error(error.status, error.code, error.detail),
        }
    }
}
