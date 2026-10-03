//! One-tap HTTP operations over the single SessionCore action lease.
//!
//! Source: internal/hub/approval_action.go at the fixed Go oracle. The router
//! checks path syntax before method/Host/Origin, and then invokes this guarded
//! boundary. This route intentionally does not require the ordinary Hub token.
//! Verification does not consume a nonce. The injected manager is shared with
//! notification issuance, and is consumed exactly once, after the bound send.
use super::http::{Request, Response, decode_json};
use crate::{
    approval::token::{OneTapAction, OneTapManager, TokenError, VerifiedClaim},
    proto::{
        core::{
            ApprovalActionBinding, ApprovalActionError, CoreEffectFailure, CoreEffectSink,
            CoreFuture, NativeActionOrigin, NativeActionRequest, NativeActionSelection,
            ReservedApprovalAction, SessionCore, SessionError, TaskCancellation,
        },
        time::Timestamp,
        wire::{Field, GoWire, Schema},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

pub const ONE_TAP_PREFIX: &str = "/api/approval-action/";
pub type ApprovalClock = Arc<dyn Fn() -> Timestamp + Send + Sync>;
pub type ApprovalWarning = Arc<dyn Fn(&CoreEffectFailure) + Send + Sync>;

/// Required Hub lifecycle boundary. `start` synchronously takes ownership of
/// the job before returning a completion waiter. Dropping that waiter MUST NOT
/// drop/cancel the job. The owner tracks every job, including completion errors.
/// Stop HTTP admission before calling `drain`; drain all transferred work before
/// shutting down the journal, socket writers or Tokio runtime. A no-op owner is
/// not a compatible implementation and must not be used by a production Hub.
pub trait ApprovalEffectOwner: Send + Sync {
    fn start(&self, job: CoreFuture<'static, ()>) -> CoreFuture<'static, Result<(), SessionError>>;
    fn drain(&self) -> CoreFuture<'_, ()>;
}

/// Successful implementations persist AddRule, reload its policy and publish
/// that policy to the actual automatic-action evaluator. The fixed Go handler
/// ignores a reload error after a successful add. No default or in-memory-only
/// implementation is supplied here; callers must inject the real integration.
pub trait ApprovalBatchRules: Send + Sync {
    fn add_and_reload(&self, command: &str, cwd: &str) -> Result<ApprovalAutoRule, String>;
}

/// Source: internal/autoapproval/policy.go Rule JSON response.
#[derive(Clone, Default, Serialize)]
pub struct ApprovalAutoRule {
    pub id: String,
    pub command: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub risk: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub working_dir: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct BatchRequest {
    signature: String,
    action: String,
    session_id: i64,
}
impl GoWire for BatchRequest {
    const GO_TYPE: &'static str = "ApprovalBatchRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "ApprovalBatchRequest",
        fields: &[
            Field {
                name: "signature",
                kind: "string",
            },
            Field {
                name: "action",
                kind: "string",
            },
            Field {
                name: "session_id",
                kind: "int",
            },
        ],
    }];
}

pub fn batch_signature(
    provider: &str,
    cwd: &str,
    summary: &crate::proto::ApprovalSummary,
) -> String {
    // Go uses Unicode simple case mapping, not full case expansion (notably İ).
    let provider: String = provider
        .trim()
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    format!(
        "{provider}\0{}\0{}\0{}",
        summary.command.trim(),
        summary.risk,
        cwd.trim()
    )
}

/// Must run before the route's method/Host/Origin checks. In particular an empty
/// token or slash-containing suffix returns 401 even for a GET request.
pub fn one_tap_token(path: &str) -> Result<&str, Response> {
    match path.strip_prefix(ONE_TAP_PREFIX) {
        Some(token) if !token.is_empty() && !token.contains('/') => Ok(token),
        _ => Err(Response::error(
            401,
            "invalid_action_token",
            "invalid action token",
        )),
    }
}

pub struct ApprovalActionHttp {
    core: Arc<dyn SessionCore>,
    effects: Arc<dyn CoreEffectSink>,
    effect_owner: Arc<dyn ApprovalEffectOwner>,
    one_tap: Arc<OneTapManager>,
    clock: ApprovalClock,
    warning: ApprovalWarning,
}
impl ApprovalActionHttp {
    pub fn new(
        core: Arc<dyn SessionCore>,
        effects: Arc<dyn CoreEffectSink>,
        effect_owner: Arc<dyn ApprovalEffectOwner>,
        one_tap: Arc<OneTapManager>,
        clock: ApprovalClock,
        warning: ApprovalWarning,
    ) -> Self {
        Self {
            core,
            effects,
            effect_owner,
            one_tap,
            clock,
            warning,
        }
    }

    /// The router has already applied the one-tap POST/Host/Origin guards.
    /// Cancellation can abandon queued input, but cannot undo accepted input or
    /// stop closure effects after commit. Do not impose an HTTP-body requirement.
    pub async fn one_tap_guarded(&self, token: &str, cancel: &TaskCancellation) -> Response {
        let claim = match self.one_tap.verify(token, (self.clock)()) {
            Ok(claim) => claim,
            Err(error) => return token_error(error),
        };
        let binding = match self.binding(&claim) {
            Ok(binding) => binding,
            Err(error) => return action_error(error),
        };
        let selection = match claim.action() {
            OneTapAction::Approve => NativeActionSelection::ApproveOnce,
            OneTapAction::Reject => NativeActionSelection::RejectOnce,
        };
        let action = match self
            .core
            .prepare_and_send(
                NativeActionRequest {
                    binding,
                    selection,
                    origin: NativeActionOrigin::OneTap {
                        nonce: claim.nonce().into(),
                    },
                },
                cancel,
            )
            .await
        {
            Ok(action) => action,
            Err(error) => return action_error(error),
        };
        // Keep a release guard over every early-return/unwind path. There is no
        // suspension point between obtaining the lease and the commit below.
        let lease = ActionLease::new(self.core.clone(), action);
        if let Err(error) = self.one_tap.consume(&claim, (self.clock)()) {
            return token_error(error);
        }
        let effects = match lease.commit((self.clock)()) {
            Ok(effects) => effects,
            Err(error) => return action_error(error),
        };
        self.finish_effects(effects).await;
        Response::json(
            200,
            &json!({
                "ok": true,
                "session_id": claim.session().0,
                "action": match claim.action() {
                    OneTapAction::Approve => "approve",
                    OneTapAction::Reject => "reject",
                },
            }),
        )
    }

    /// The ordinary token/PIN, POST, Host and Origin guard belongs to the router.
    /// Matching snapshots come from the same core and actions still revalidate
    /// after its FIFO. A required rule adapter prevents fake auto_rule success.
    pub async fn batch_authenticated(
        &self,
        request: &Request,
        rules: &dyn ApprovalBatchRules,
        cancel: &TaskCancellation,
    ) -> Response {
        let request: BatchRequest = match decode_json(request) {
            Ok(request) => request,
            Err(error) => return error,
        };
        if request.signature.trim().is_empty() && request.action != "deny_session" {
            return Response::error(400, "invalid_batch_request", "signature is required");
        }
        let matched: Vec<_> = self
            .core
            .pending_native_approval_actions()
            .into_iter()
            .filter(|item| {
                // Preserve Go's OR, even for deny_session with a supplied signature:
                // that request can match other sessions as well as the requested ID.
                (request.action == "deny_session"
                    && item.binding.session.session.0 == request.session_id)
                    || batch_signature(&item.provider, &item.cwd, &item.summary)
                        == request.signature
            })
            .collect();
        if request.action == "auto_rule" {
            let Some(first) = matched.first().filter(|item| item.summary.risk == "low") else {
                return Response::error(
                    403,
                    "auto_rule_requires_low_risk",
                    "only a pending low-risk approval can be added",
                );
            };
            return match rules.add_and_reload(&first.summary.command, &first.cwd) {
                Ok(rule) => {
                    Response::json(200, &json!({"ok":true,"matched":matched.len(),"rule":rule}))
                }
                Err(error) => Response::error(400, "auto_rule_not_added", &error),
            };
        }
        let reject = request.action == "deny_session" && request.session_id > 0;
        if request.action != "approve" && !reject {
            return Response::error(400, "invalid_batch_action", "unsupported batch action");
        }
        let mut applied = 0;
        for item in &matched {
            if !reject && item.summary.risk != "low" {
                continue;
            }
            let action = self
                .core
                .prepare_and_send(
                    NativeActionRequest {
                        binding: item.binding.clone(),
                        selection: if reject {
                            NativeActionSelection::RejectOnce
                        } else {
                            NativeActionSelection::ApproveOnce
                        },
                        origin: NativeActionOrigin::Batch,
                    },
                    cancel,
                )
                .await;
            if let Ok(action) = action {
                let lease = ActionLease::new(self.core.clone(), action);
                if let Ok(effects) = lease.commit((self.clock)()) {
                    self.finish_effects(effects).await;
                    applied += 1;
                }
            }
        }
        Response::json(
            200,
            &json!({"ok":true,"matched":matched.len(),"applied":applied}),
        )
    }

    async fn finish_effects(&self, effects: crate::proto::core::CoreEffects) {
        // Go's StoreApprovalConsumed/broadcast failures do not undo a committed
        // action or turn its response into failure. Apply once, report failure,
        // and never retry the batch. Own this drain independently of the HTTP
        // waiter so a disconnect cannot discard pending ledger/UI effects.
        let sink = self.effects.clone();
        let warning = self.warning.clone();
        let drain = self.effect_owner.start(Box::pin(async move {
            if let Err(error) = sink.apply(effects).await {
                warning(&error);
            }
        }));
        if let Err(error) = drain.await {
            (self.warning)(&CoreEffectFailure {
                index: 0,
                error: crate::proto::core::SessionError::Transport(format!(
                    "approval closure effect task failed: {error:?}"
                )),
            });
        }
    }

    fn binding(&self, claim: &VerifiedClaim) -> Result<ApprovalActionBinding, ApprovalActionError> {
        if claim.approval_id() != claim.signature() {
            return Err(ApprovalActionError::StaleCandidate);
        }
        let details = self
            .core
            .details(claim.session())
            .ok_or(ApprovalActionError::NotFound)?;
        let record = details
            .approval
            .record
            .ok_or(ApprovalActionError::StaleCandidate)?;
        let data = record.data();
        if data.origin != "native" || data.sig != claim.signature() {
            return Err(ApprovalActionError::StaleCandidate);
        }
        Ok(ApprovalActionBinding {
            session: details.binding,
            candidate_key: data.candidate.key.clone(),
            source_epoch: claim.epoch(),
            sig: claim.signature().into(),
        })
    }
}

/// The shared lease is not itself RAII. This caller-local guard closes that
/// ownership gap without making another session map or action reservation.
struct ActionLease {
    core: Arc<dyn SessionCore>,
    action: Option<ReservedApprovalAction>,
}
impl ActionLease {
    fn new(core: Arc<dyn SessionCore>, action: ReservedApprovalAction) -> Self {
        Self {
            core,
            action: Some(action),
        }
    }
    fn commit(
        mut self,
        now: Timestamp,
    ) -> Result<crate::proto::core::CoreEffects, ApprovalActionError> {
        let action = self.action.as_ref().expect("owned action lease");
        // commit consumes its argument even on failure. Retain only the same
        // release identity until success, so a failing commit still releases.
        let committed = self.core.commit(
            ReservedApprovalAction {
                reservation: action.reservation,
                binding: action.binding.clone(),
                selected_text: action.selected_text.clone(),
                origin: action.origin.clone(),
            },
            now,
        );
        if committed.is_ok() {
            self.action.take();
        }
        committed
    }
}
impl Drop for ActionLease {
    fn drop(&mut self) {
        if let Some(action) = self.action.take() {
            self.core.release(action);
        }
    }
}

fn token_error(error: TokenError) -> Response {
    match error {
        TokenError::Consumed => Response::error(409, "action_already_used", "action already used"),
        TokenError::Invalid | TokenError::Expired => Response::error(
            401,
            "invalid_action_token",
            "invalid or expired action token",
        ),
        TokenError::RandomUnavailable => action_not_applied(),
    }
}
fn action_error(error: ApprovalActionError) -> Response {
    match error {
        ApprovalActionError::NotFound | ApprovalActionError::StaleCandidate => {
            Response::error(409, "approval_not_pending", "approval is no longer pending")
        }
        ApprovalActionError::HighRisk => Response::error(
            403,
            "high_risk_requires_in_app_confirmation",
            "open Hub to confirm this high-risk approval",
        ),
        _ => action_not_applied(),
    }
}
fn action_not_applied() -> Response {
    Response::error(409, "action_not_applied", "approval action was not applied")
}

#[cfg(test)]
#[path = "approval_actions_tests.rs"]
mod tests;
