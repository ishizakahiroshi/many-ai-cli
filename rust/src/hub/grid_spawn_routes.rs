//! Router supplies the ordinary source POST/Host/Origin/token/PIN guards.
use super::http::{Request, Response, decode_json};
use crate::{
    application::grid_spawn::{GridRequest, GridSpawn},
    proto::core::VerifiedConfirmationRequest,
};
use std::sync::Arc;
pub struct GridHttp {
    service: Arc<GridSpawn>,
}
impl GridHttp {
    pub fn new(service: Arc<GridSpawn>) -> Self {
        Self { service }
    }
    pub async fn spawn_authenticated(
        &self,
        request: &Request,
        verified: &VerifiedConfirmationRequest,
    ) -> Response {
        let body: GridRequest = match decode_json(request) {
            Ok(body) => body,
            Err(response) => return response,
        };
        match self.service.spawn(body, verified).await {
            Ok(result) => Response::json(
                200,
                &serde_json::json!({"ok":true,"layout":result.layout,"count":result.count}),
            ),
            Err(error) => Response::error(error.status, error.code, error.detail),
        }
    }
}
