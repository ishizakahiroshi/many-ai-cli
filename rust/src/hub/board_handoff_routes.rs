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
/// The `local-claude` taker: GET its state, POST `{"paused": bool}`.
pub const TAKER_PATH: &str = "/api/local-taker";
/// GET `?request_id=` a kept run's steps.
pub const TAKER_STREAM_PATH: &str = "/api/local-taker/stream";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TakeBackBody {
    item_id: String,
    request_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PauseBody {
    paused: bool,
}

pub struct BoardHandoffHttp {
    board: Arc<BoardSync>,
}
impl BoardHandoffHttp {
    pub fn new(board: Arc<BoardSync>) -> Self {
        Self { board }
    }
    pub async fn authenticated(&self, request: &Request) -> Response {
        if request.path == TAKER_PATH {
            return self.taker_authenticated(request);
        }
        if request.path == TAKER_STREAM_PATH {
            return self.taker_stream_authenticated(request);
        }
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

    /// The taker's state; a POST from the person at the dashboard pauses or
    /// resumes it (that press is the confirmation).
    fn taker_authenticated(&self, request: &Request) -> Response {
        if let Err(error) = require_method(request, &["GET", "POST"]) {
            return error;
        }
        if request.method == "POST" {
            let body = &request.body[..request.body.len().min(super::http::JSON_BODY_LIMIT)];
            let Ok(named) = serde_json::from_slice::<PauseBody>(body) else {
                return Response::error(
                    400,
                    "local_taker_invalid",
                    "send {\"paused\": true|false}",
                );
            };
            if self.board.set_local_taker_paused(named.paused).is_err() {
                return Response::error(
                    500,
                    "local_taker_unsaved",
                    "the setting could not be saved",
                );
            }
        }
        Response::json(
            200,
            &serde_json::json!({"ok": true, "taker": self.board.local_taker_status()}),
        )
    }

    fn taker_stream_authenticated(&self, request: &Request) -> Response {
        if let Err(error) = require_method(request, &["GET"]) {
            return error;
        }
        match self.board.local_taker_stream(&request.query("request_id")) {
            Some(entries) => {
                Response::json(200, &serde_json::json!({"ok": true, "entries": entries}))
            }
            None => Response::error(
                404,
                "local_taker_stream_missing",
                "no kept run for that request",
            ),
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
