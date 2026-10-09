use super::http::{Request, Response};
use crate::{application::session_usage::SessionUsage, proto::time::Timestamp};
use std::sync::Arc;
pub const PATH: &str = "/api/session-usage";
pub fn is_route(path: &str) -> bool {
    path == PATH
}
pub struct UsageHttp {
    owner: Arc<SessionUsage>,
}
impl UsageHttp {
    pub fn new(owner: Arc<SessionUsage>) -> Self {
        Self { owner }
    }
    pub async fn handle_authenticated(
        &self,
        request: &Request,
        now: Timestamp,
    ) -> Option<Response> {
        if !is_route(&request.path) {
            return None;
        }
        Some(self.owner.receive(request, now).await)
    }
}
