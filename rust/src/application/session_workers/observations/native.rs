//! Source adapter-key dispatch and bounded native-reader continuation. This
//! state is parser/cache data; session eligibility remains the sole core's.
use super::*;
use crate::{
    application::{agent_history, session_observations::subagents},
    terminal::session::SessionObservationSnapshot,
};
use sha2::{Digest, Sha256};
#[derive(Clone, Default)]
enum Reader {
    #[default]
    None,
    Claude(subagents::claude::State),
    Codex(subagents::codex::State),
    Grok(subagents::grok::State),
}
#[derive(Default)]
pub(super) struct Native {
    pub due: Option<Timestamp>,
    reader: Reader,
    path: Option<PathBuf>,
    path_for: Option<TranscriptSessionIdentity>,
    attempted: Option<Timestamp>,
    signature: String,
    had_tree: bool,
}
pub(super) fn signature(tree: &proto::SubagentTree) -> String {
    let mut text = format!("{}|{}", tree.provider, tree.omitted);
    for n in &tree.nodes {
        text.push_str(&format!(
            "|N:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            n.id,
            n.parent_id,
            n.depth,
            n.label,
            n.agent_type,
            n.model,
            n.state,
            n.started_at,
            n.last_activity_at,
            n.finished_at,
            n.tool_calls,
            n.last_tool_name,
            n.last_tool_summary
        ));
    }
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
impl Native {
    pub fn start(&mut self, now: Timestamp) {
        if self.due.is_none() {
            self.due = Some(now);
        }
    }
    pub fn reattach(&mut self, now: Timestamp) {
        self.signature.clear();
        self.due = Some(now);
    }
    pub fn stop(&mut self) {
        self.due = None;
    }
    pub fn invalidate_publication(&mut self) {
        self.signature.clear();
    }
    pub fn poll(
        &mut self,
        worker: &SessionWorkers,
        snapshot: &SessionObservationSnapshot,
        enabled: bool,
        now: Timestamp,
    ) -> Result<Option<proto::SubagentTree>, SessionError> {
        if self.due.is_none_or(|due| due > now) {
            return Ok(None);
        }
        self.due = None;
        if !enabled
            || matches!(
                snapshot.details.snapshot.state.as_str(),
                "completed" | "error" | "disconnected" | "done" | "timeout" | "dismissed"
            )
        {
            return Ok(None);
        }
        let registry = worker.observation_registry.get().ok_or_else(|| {
            SessionError::InvalidRequest("observation registry is not bound".into())
        })?;
        let registry =
            registry().map_err(|_| storage_error("native observation registry snapshot failed"))?;
        let Some(definition) = registry.lookup(&snapshot.details.snapshot.provider) else {
            return Ok(None);
        };
        let key = definition.definition.adapters.subagents.as_str();
        let provider = match key {
            "subagent:claude-v1" => "claude",
            "subagent:codex-v1" => "codex",
            "subagent:grok-v1" => "grok",
            _ => return Ok(None),
        };
        let identity = &snapshot.details.transcript;
        let changed = self.path.is_some() && self.path_for.as_ref() != Some(identity);
        let valid = self.path.as_ref().is_some_and(|path| {
            !changed && subagents::artifact_metadata(&worker.paths, path).is_ok()
        });
        if !valid
            && (changed
                || self.attempted.is_none_or(|at| {
                    now.duration_since(at)
                        .is_ok_and(|age| age >= Duration::from_secs(30))
                }))
        {
            self.attempted = Some(now);
            let mut resolver_identity = identity.clone();
            resolver_identity.provider = provider.into();
            self.path =
                agent_history::resolve_transcript(&worker.paths, &resolver_identity).map(|path| {
                    if provider == "grok" {
                        path.parent().unwrap_or(Path::new("")).join("updates.jsonl")
                    } else {
                        path
                    }
                });
            self.path_for = self.path.as_ref().map(|_| identity.clone());
        } else if !valid {
            self.path = None;
        }
        self.due = now.checked_add(Duration::from_secs(3));
        let Some(path) = self.path.as_deref() else {
            return Ok(None);
        };
        let budget = subagents::ReadBudget::default();
        let prior = self.reader.clone();
        let result = match (provider, prior) {
            ("claude", Reader::Claude(state)) => subagents::claude::read(
                path,
                snapshot.turn_started_at,
                state,
                &budget,
                &worker.paths,
                now,
            )
            .map(|(tree, state)| (tree, Reader::Claude(state))),
            ("claude", _) => subagents::claude::read(
                path,
                snapshot.turn_started_at,
                Default::default(),
                &budget,
                &worker.paths,
                now,
            )
            .map(|(tree, state)| (tree, Reader::Claude(state))),
            ("codex", Reader::Codex(state)) => subagents::codex::read(
                path,
                snapshot.turn_started_at,
                state,
                &budget,
                &worker.paths,
                now,
            )
            .map(|(tree, state)| (tree, Reader::Codex(state))),
            ("codex", _) => subagents::codex::read(
                path,
                snapshot.turn_started_at,
                Default::default(),
                &budget,
                &worker.paths,
                now,
            )
            .map(|(tree, state)| (tree, Reader::Codex(state))),
            ("grok", Reader::Grok(state)) => subagents::grok::read(
                path,
                snapshot.turn_started_at,
                state,
                &budget,
                &worker.paths,
                now,
            )
            .map(|(tree, state)| (tree, Reader::Grok(state))),
            ("grok", _) => subagents::grok::read(
                path,
                snapshot.turn_started_at,
                Default::default(),
                &budget,
                &worker.paths,
                now,
            )
            .map(|(tree, state)| (tree, Reader::Grok(state))),
            _ => unreachable!("adapter table admitted only three native providers"),
        };
        let Ok((tree, next)) = result else {
            return Ok(None);
        };
        self.reader = next;
        self.had_tree |= snapshot
            .details
            .subagents
            .as_ref()
            .is_some_and(|tree| !tree.nodes.is_empty());
        let tree = if let Some(tree) = tree {
            self.had_tree = true;
            tree
        } else if self.had_tree {
            self.had_tree = false;
            proto::SubagentTree {
                provider: key.into(),
                updated_at: i64::try_from(now.unix_nanos() / 1_000_000).unwrap_or(0),
                ..Default::default()
            }
        } else {
            return Ok(None);
        };
        let signature = signature(&tree);
        if signature == self.signature {
            return Ok(None);
        }
        self.signature = signature;
        Ok(Some(tree))
    }
}
#[cfg(test)]
mod tests;
