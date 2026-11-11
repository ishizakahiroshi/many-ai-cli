//! Router applies source method/Host/Origin/token/PIN guards before this adapter.
//!
//! A start may name a piece of work the board's owner handed over
//! (`board_handoff`). It is checked against the board's open requests and the
//! listed folder, and then tied to the session by the start's request id, so
//! the session takes the request when it registers. The folder and the item
//! are the Hub's and the board's; the browser only names them.
use super::http::{Request, Response, decode_json};
use crate::{
    application::board_sync::BoardSync,
    application::ordinary_spawn::{OrdinarySpawnRequest, OrdinarySpawnService},
    proto::{
        core::{HttpWaitCancellation, VerifiedConfirmationRequest},
        time::Timestamp,
    },
};
use serde::Deserialize;
use std::{path::Path, sync::Arc};
pub struct SpawnHttp {
    service: Arc<OrdinarySpawnService>,
    board: Option<Arc<BoardSync>>,
}
/// The part of a start request that names handed-over work.
#[derive(Default, Deserialize)]
#[serde(default)]
struct HandoffPart {
    client_request_id: String,
    cwd: String,
    board_handoff: Option<HandoffRef>,
}
#[derive(Deserialize)]
struct HandoffRef {
    item_id: String,
    request_id: String,
}
impl SpawnHttp {
    pub fn new(service: Arc<OrdinarySpawnService>) -> Self {
        Self {
            service,
            board: None,
        }
    }
    pub fn with_board(mut self, board: Arc<BoardSync>) -> Self {
        self.board = Some(board);
        self
    }
    async fn claim(&self, request: &Request) -> Result<Option<String>, Response> {
        claim_handoff(self.board.as_ref(), request).await
    }
}
/// Ties the start to the handed-over work it names; the reply is the start's
/// request id. `Ok(None)` = an ordinary start.
pub(crate) async fn claim_handoff(
    board: Option<&Arc<BoardSync>>,
    request: &Request,
) -> Result<Option<String>, Response> {
    {
        // The ordinary decoder reads the fields of the Go-era schema; this
        // part is read by its exact keys, and an unreadable one names no work.
        let body = &request.body[..request.body.len().min(super::http::JSON_BODY_LIMIT)];
        let part: HandoffPart = crate::proto::wire::first_http_raw(body)
            .ok()
            .and_then(|raw| serde_json::from_str(raw.get()).ok())
            .unwrap_or_default();
        let Some(named) = part.board_handoff else {
            return Ok(None);
        };
        let Some(board) = board else {
            return Err(Response::error(
                503,
                "board_unavailable",
                "board service is not initialized",
            ));
        };
        if part.client_request_id.is_empty()
            || !crate::proto::core::valid_client_request_id(&part.client_request_id)
        {
            return Err(Response::error(
                400,
                "board_handoff_invalid",
                "a start for handed-over work needs a client_request_id",
            ));
        }
        let open = board.handed_work().await;
        let Some(work) = open
            .iter()
            .find(|work| work.item_id == named.item_id && work.request_id == named.request_id)
        else {
            return Err(Response::error(
                409,
                "board_handoff_gone",
                "the handed-over work is no longer open",
            ));
        };
        if !work.covers(Path::new(&part.cwd)) {
            return Err(Response::error(
                400,
                "board_handoff_folder",
                "handed-over work starts in its project's listed folder",
            ));
        }
        board.claim(&part.client_request_id, work);
        Ok(Some(part.client_request_id))
    }
}
impl SpawnHttp {
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
        let claimed = match self.claim(request).await {
            Ok(claimed) => claimed,
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
            Err(error) => {
                if let (Some(board), Some(start)) = (&self.board, &claimed) {
                    board.unclaim(start);
                }
                Response::error(error.status, error.code, error.detail)
            }
        }
    }
}
