//! Work the board's owner handed over (see `application::board_sync`), for the
//! dashboard. The caller applies the ordinary Hub token/PIN, method, Host and
//! Origin guard before invoking this authenticated boundary.
//!
//! Reading the list starts nothing. A session for a piece of handed-over work
//! is started by the person at the dashboard through the ordinary `/api/spawn`
//! (see `spawn_routes`), which is the confirmation.
use super::http::{Request, Response, require_method};
use crate::application::board_sync::{BoardSync, TakeBack};
use serde::Deserialize;
use std::sync::Arc;

pub const PATH: &str = "/api/board-handoffs";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TakeBackBody {
    item_id: String,
    request_id: String,
}

pub struct BoardHandoffHttp {
    board: Arc<BoardSync>,
}
impl BoardHandoffHttp {
    pub fn new(board: Arc<BoardSync>) -> Self {
        Self { board }
    }
    pub async fn authenticated(&self, request: &Request) -> Response {
        if request.method == "POST" {
            return self.take_back_authenticated(request).await;
        }
        self.list_authenticated(request).await
    }

    /// Withdraws a request nobody has taken yet. The person at the dashboard
    /// asked for it, so this is the confirmation; the board decides the rest.
    async fn take_back_authenticated(&self, request: &Request) -> Response {
        let body = &request.body[..request.body.len().min(super::http::JSON_BODY_LIMIT)];
        let Ok(named) = serde_json::from_slice::<TakeBackBody>(body) else {
            return Response::error(
                400,
                "board_handoff_invalid",
                "name the item_id and request_id to take back",
            );
        };
        match self
            .board
            .take_back(&named.item_id, &named.request_id)
            .await
        {
            TakeBack::Done => Response::json(200, &serde_json::json!({"ok": true})),
            TakeBack::Gone | TakeBack::Refused => Response::error(
                409,
                "board_handoff_gone",
                "the handed-over work is no longer open",
            ),
            TakeBack::Unreached => {
                Response::error(502, "board_unreachable", "the board could not be reached")
            }
        }
    }

    pub async fn list_authenticated(&self, request: &Request) -> Response {
        if let Err(error) = require_method(request, &["GET"]) {
            return error;
        }
        let work = self.board.handed_work().await;
        Response::json(200, &serde_json::json!({"ok": true, "work": work}))
    }
}
