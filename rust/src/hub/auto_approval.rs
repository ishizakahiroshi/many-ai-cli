//! Read-only automatic approval status and source history simulation.
use super::http::{Request, Response, require_method};
use crate::application::approval_rules::{ApprovalRules, Candidate};
use crate::approval::policy::Policy;
use std::sync::Arc;
pub const STATUS: &str = "/api/auto-approval/status";
pub const SIMULATE: &str = "/api/auto-approval/simulate";
pub struct AutoApprovalHttp {
    rules: Arc<ApprovalRules>,
}
impl AutoApprovalHttp {
    pub fn new(rules: Arc<ApprovalRules>) -> Self {
        Self { rules }
    }
    pub fn handle_authenticated(&self, request: &Request) -> Option<Response> {
        if !matches!(request.path.as_str(), STATUS | SIMULATE) {
            return None;
        }
        if let Err(error) = require_method(request, &["GET"]) {
            return Some(error);
        }
        let policy = self.rules.policy.snapshot();
        Some(if request.path == STATUS {
            Response::json(
                200,
                &serde_json::json!({
                    "enabled":self.rules.policy.enabled(), "path":self.rules.policy.path(),
                    "active_rules":policy.active_rules(),
                    "warnings":(!policy.warnings.is_empty()).then_some(&policy.warnings),
                }),
            )
        } else {
            simulation(&policy, self.rules.history(), &request.query("n"))
        })
    }
}
fn query_count(value: &str) -> isize {
    // fmt.Sscanf(...,"%d",&n) changes n only after a valid decimal prefix;
    // whitespace, a sign and trailing text preserve the source scan behavior.
    let value = value.trim_start();
    let sign = usize::from(value.starts_with(['+', '-']));
    let end = sign + value[sign..].bytes().take_while(u8::is_ascii_digit).count();
    value
        .get(..end)
        .and_then(|part| part.parse().ok())
        .unwrap_or(100)
}
fn simulation(policy: &Policy, mut items: Vec<Candidate>, count: &str) -> Response {
    let count = query_count(count);
    if count > 0 && (count as usize) < items.len() {
        items.drain(..items.len() - count as usize);
    }
    let mut matched = 0;
    for item in &mut items {
        item.decision = policy.evaluate(&item.summary.command, &item.cwd, &item.summary.risk);
        matched += usize::from(item.decision.allowed);
    }
    Response::json(
        200,
        &serde_json::json!({"total":items.len(),"matched":matched,"items":(!items.is_empty()).then_some(items)}),
    )
}
#[cfg(test)]
mod tests;
