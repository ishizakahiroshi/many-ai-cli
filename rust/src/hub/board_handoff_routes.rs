//! Work the board's owner handed over (see `application::board_sync`), for the
//! dashboard. The caller applies the ordinary Hub token/PIN, method, Host and
//! Origin guard before invoking this authenticated boundary.
//!
//! Reading the list starts nothing. A session for a piece of handed-over work
//! is started by the person at the dashboard through the ordinary `/api/spawn`
//! (see `spawn_routes`), which is the confirmation.
use super::http::{Request, Response, require_method};
use crate::application::board_sync::BoardSync;
use std::sync::Arc;

pub const PATH: &str = "/api/board-handoffs";

pub struct BoardHandoffHttp {
    board: Arc<BoardSync>,
}
impl BoardHandoffHttp {
    pub fn new(board: Arc<BoardSync>) -> Self {
        Self { board }
    }
    pub async fn list_authenticated(&self, request: &Request) -> Response {
        if let Err(error) = require_method(request, &["GET"]) {
            return error;
        }
        let work = self.board.handed_work().await;
        Response::json(200, &serde_json::json!({"ok": true, "work": work}))
    }
}
