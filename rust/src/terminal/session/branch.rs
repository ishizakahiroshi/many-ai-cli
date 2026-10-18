//! Git display state stays under the sole session lock. Workers carry IDs and
//! cwd, not a wrapper generation: Go applies results to same-cwd replacements.
use super::*;

pub(crate) const REFRESH_AFTER: Duration = Duration::from_secs(2);
const IDLE_REFRESH_AFTER: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct RefreshState {
    pub checked_at: Option<Timestamp>,
    checked: bool,
    changes: (i64, i64, i64),
    project_checked: bool,
    final_checked: bool,
}

impl Session {
    fn branch_refresh_due(&mut self, now: Timestamp) -> bool {
        let ended = terminal(&self.snapshot.state);
        let Some(at) = self.branch_refresh.checked_at else {
            self.branch_refresh.final_checked = ended;
            return true;
        };
        let age = now.duration_since(at).unwrap_or_default();
        if ended {
            let due = age >= REFRESH_AFTER && !self.branch_refresh.final_checked;
            self.branch_refresh.final_checked |= due;
            return due;
        }
        self.branch_refresh.final_checked = false;
        if age >= IDLE_REFRESH_AFTER {
            return true;
        }
        let git_not_answered = self.snapshot.branch.is_empty()
            && !(self.branch_refresh.project_checked && self.snapshot.project_id.is_empty());
        age >= REFRESH_AFTER && (self.last_output.is_some_and(|out| out > at) || git_not_answered)
    }
}

impl SessionEngine {
    pub(crate) fn branch_refresh_requests(&self, now: Timestamp) -> Vec<(LiveSessionId, String)> {
        let mut state = lock(&self.state);
        state
            .sessions
            .values_mut()
            .filter_map(|session| {
                if !session.branch_refresh_due(now) {
                    return None;
                }
                // Enqueue time, not completion time. Live sessions poll every 2s only
                // after new output or while the branch is unknown, else every 30s; an
                // ended session gets one last check, then stops.
                session.branch_refresh.checked_at = Some(now);
                Some((session.binding.session, session.snapshot.cwd.clone()))
            })
            .collect()
    }

    /// Make every session due now, so a screen that attaches after none was
    /// watching gets fresh counts rather than the ones from before the pause.
    pub(crate) fn restart_branch_refresh(&self) {
        for session in lock(&self.state).sessions.values_mut() {
            session.branch_refresh.checked_at = None;
        }
    }

    /// The working directory of every session that has not ended, for the
    /// repositories to watch and match changes to (`files::change_watch`).
    pub(crate) fn branch_refresh_cwds(&self) -> Vec<(LiveSessionId, String)> {
        lock(&self.state)
            .sessions
            .values()
            .filter(|session| !terminal(&session.snapshot.state))
            .map(|session| (session.binding.session, session.snapshot.cwd.clone()))
            .collect()
    }

    /// Make these sessions due now: their repository changed.
    pub(crate) fn restart_branch_refresh_for(&self, ids: &[LiveSessionId]) {
        let mut state = lock(&self.state);
        for id in ids {
            if let Some(session) = state.sessions.get_mut(id) {
                session.branch_refresh.checked_at = None;
            }
        }
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
            let changed = session.apply_branch_observation(&observation);
            // 一時観測 rust-branch-probe: 画面へ送る更新を作ったかを記録するだけ。
            crate::logging::branch_probe::note_applied(id.0, changed);
            if changed {
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
