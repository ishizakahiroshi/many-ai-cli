//! Git display state stays under the sole session lock. Workers carry IDs and
//! cwd, not a wrapper generation: Go applies results to same-cwd replacements.
use super::*;

pub(crate) const REFRESH_AFTER: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(super) struct RefreshState {
    pub checked_at: Option<Timestamp>,
    checked: bool,
    changes: (i64, i64, i64),
    project_checked: bool,
}

impl SessionEngine {
    pub(crate) fn branch_refresh_requests(&self, now: Timestamp) -> Vec<(LiveSessionId, String)> {
        let mut state = lock(&self.state);
        state
            .sessions
            .values_mut()
            .filter_map(|session| {
                let due = session
                    .branch_refresh
                    .checked_at
                    .is_none_or(|at| now.duration_since(at).is_ok_and(|age| age >= REFRESH_AFTER));
                if !due {
                    return None;
                }
                // Enqueue time, not completion time. Terminal sessions remain eligible.
                session.branch_refresh.checked_at = Some(now);
                Some((session.binding.session, session.snapshot.cwd.clone()))
            })
            .collect()
    }

    pub(crate) fn branch_project_needed(&self, cwd: &str, ids: &[LiveSessionId]) -> bool {
        let state = lock(&self.state);
        ids.iter().any(|id| {
            state.sessions.get(id).is_some_and(|session| {
                session.snapshot.cwd == cwd && !session.branch_refresh.project_checked
            })
        })
    }

    pub(crate) fn apply_branch_refresh(
        &self,
        cwd: &str,
        ids: &[LiveSessionId],
        observation: SessionObservation,
    ) -> Result<CoreEffects, SessionError> {
        if !matches!(observation, SessionObservation::Branch { .. }) {
            return Err(SessionError::InvalidRequest(
                "expected branch observation".into(),
            ));
        }
        let mut state = lock(&self.state);
        let mut effects = CoreEffects::default();
        for id in ids {
            let Some(session) = state.sessions.get_mut(id) else {
                continue;
            };
            if session.snapshot.cwd != cwd {
                continue;
            }
            if session.apply_branch_observation(&observation) {
                effects
                    .0
                    .push(CoreEffect::Broadcast(session.branch_update_message()));
            }
        }
        // UI priming and hidden-probe suppression are the same shared route.
        Ok(state.route(effects))
    }
}

impl Session {
    pub(super) fn apply_branch_observation(&mut self, observation: &SessionObservation) -> bool {
        let SessionObservation::Branch {
            branch,
            git_root,
            changes,
            project_id,
        } = observation
        else {
            return false;
        };
        let project_changed = project_id.is_some() && !self.branch_refresh.project_checked;
        let changed = self.snapshot.branch != *branch
            || !self.branch_refresh.checked
            || self.branch_refresh.changes != *changes
            || project_changed;
        if project_changed {
            self.snapshot.project_id = project_id.clone().expect("resolved project identity");
            self.branch_refresh.project_checked = true;
            self.git_root = git_root.clone();
        }
        if !changed {
            return false;
        }
        self.snapshot.branch.clone_from(branch);
        self.branch_refresh.checked = true;
        self.branch_refresh.changes = *changes;
        true
    }

    pub(super) fn branch_update_message(&self) -> proto::Message {
        let s = &self.snapshot;
        proto::Message {
            r#type: "session_update".into(),
            session_id: s.id.0,
            provider: s.provider.clone(),
            display_name: s.display.clone(),
            cwd: s.cwd.clone(),
            branch: s.branch.clone(),
            project_id: s.project_id.clone(),
            label: s.label.clone(),
            model: s.model.clone(),
            route: s.route.clone(),
            state: s.state.clone(),
            last_output_at: s.last_output_at.clone(),
            started_at: s.started_at.clone(),
            first_message: s.first_message.clone(),
            last_message: s.last_message.clone(),
            git_checked: true,
            git_files: self.branch_refresh.changes.0,
            git_added: self.branch_refresh.changes.1,
            git_deleted: self.branch_refresh.changes.2,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests;
