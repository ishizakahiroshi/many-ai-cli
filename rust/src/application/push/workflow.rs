use super::*;
use crate::{application::session_observations::workflow_state, proto::WorkflowProgress};
fn payload(details: &SessionDetails, progress: &WorkflowProgress) -> ApprovalPayload {
    let snapshot = &details.snapshot;
    let provider = if snapshot.provider.is_empty() {
        "claude"
    } else {
        snapshot.provider.as_str()
    };
    let name = [
        snapshot.display.trim(),
        snapshot.provider.trim(),
        "many-ai-cli",
    ]
    .into_iter()
    .find(|name| !name.is_empty())
    .unwrap();
    let title = if snapshot.label.is_empty() {
        format!("{name} #{}", details.binding.session.0)
    } else {
        format!("{name} #{} [{}]", details.binding.session.0, snapshot.label)
    };
    let body = if progress.total > 0 {
        format!(
            "Workflow completed ({}/{} agents)",
            progress.done, progress.total
        )
    } else if progress.done > 0 {
        format!("Workflow completed ({} agents)", progress.done)
    } else {
        "Workflow completed".into()
    };
    let signature = workflow_state::signature(progress);
    let id = format!(
        "workflow-{}-{}",
        details.binding.session.0,
        &signature[..16]
    );
    let risk = crate::approval::summary::summarize(&body, "").risk;
    ApprovalPayload {
        id,
        session_id: details.binding.session.0,
        provider: provider.into(),
        title,
        body: mask_secrets(&body)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        url: format!("/?session_id={}", details.binding.session.0),
        risk,
        ..Default::default()
    }
}
impl PushManager {
    /// Exact aggregate workflow delivery. No journal IDs, transcript content,
    /// agent detail or native approval action tokens cross this boundary.
    pub fn notify_workflow_completion(
        self: &Arc<Self>,
        core: &SessionEngine,
        binding: SessionBinding,
        progress: &WorkflowProgress,
        enabled: bool,
    ) -> Result<(), SessionError> {
        if !enabled || !progress.settled {
            return Ok(());
        }
        let Some(details) = core.details(binding.session) else {
            return Ok(());
        };
        if details.binding != binding {
            return Err(SessionError::StaleBinding);
        }
        self.send_approval(payload(&details, progress))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workflow_payload_contains_only_aggregate_counts_and_no_action_tokens() {
        let details = SessionDetails {
            binding: SessionBinding {
                session: LiveSessionId(7),
                incarnation: SessionIncarnation(1),
                wrapper: WrapperConnectionId(1),
            },
            last_output_at: None,
            snapshot: SessionSnapshot {
                provider: "claude".into(),
                display: "Claude".into(),
                label: "synthetic".into(),
                last_message: "SYNTHETIC_TRANSCRIPT_NEVER_SENT".into(),
                ..Default::default()
            },
            db_id: None,
            git_root: None,
            transcript: TranscriptSessionIdentity::default(),
            approval: ApprovalSessionSnapshot {
                session: LiveSessionId(7),
                version: ApprovalStateVersion(0),
                record: None,
            },
            workflow: None,
            subagents: None,
            done: None,
            connected: true,
        };
        let progress = WorkflowProgress {
            detected: true,
            total: 3,
            done: 2,
            settled: true,
            settled_by: "timeout".into(),
            ..Default::default()
        };
        let payload = payload(&details, &progress);
        assert_eq!(payload.body, "Workflow completed (2/3 agents)");
        assert_eq!(payload.title, "Claude #7 [synthetic]");
        assert_eq!(payload.url, "/?session_id=7");
        assert_eq!(
            payload.id,
            format!("workflow-7-{}", &workflow_state::signature(&progress)[..16])
        );
        assert!(payload.approve_token.is_empty() && payload.reject_token.is_empty());
        assert!(!payload.body.contains("SYNTHETIC_TRANSCRIPT"));
    }
}
