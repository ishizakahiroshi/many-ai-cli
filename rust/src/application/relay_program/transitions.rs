use super::*;
use crate::orchestration::relay::text::{Verdict, VerdictError, count_done_lines, parse_verdict};
impl RelayProgram {
    pub async fn child_progress(
        &self,
        board: &str,
        child: LiveSessionId,
        text: &str,
        now: Timestamp,
        cancel: TaskCancellation,
    ) -> Result<(), RelayError> {
        let handle = lock(&self.state)
            .runs
            .get(board)
            .cloned()
            .ok_or_else(RelayError::missing)?;
        let mut run = handle.lock().await;
        if !run.file.terminal()
            && !run.awaiting_reconnect
            && let Some(role) = run.file.role_of(child.0)
        {
            if run.file.headless(role) {
                if count_done_lines(text, role, run.file.progress_id(role)).count
                    > run.file.baseline(role)
                {
                    run.timers
                        .entry(child.0)
                        .or_default()
                        .pending_done_at
                        .get_or_insert(now);
                }
            } else {
                self.advance(&mut run, child.0, text, false, now, &cancel)
                    .await;
            }
        }
        Ok(())
    }
    pub(super) async fn advance(
        &self,
        run: &mut Run,
        child: i64,
        text: &str,
        exited: bool,
        now: Timestamp,
        cancel: &TaskCancellation,
    ) {
        let mut transaction = TransitionGuard {
            owner: self,
            run,
            armed: true,
        };
        self.advance_inner(transaction.run, child, text, exited, now, cancel)
            .await;
        transaction.armed = false;
    }
    async fn advance_inner(
        &self,
        run: &mut Run,
        child: i64,
        text: &str,
        exited: bool,
        now: Timestamp,
        cancel: &TaskCancellation,
    ) {
        let Some(role) = run.file.role_of(child) else {
            return;
        };
        let mut done = count_done_lines(text, role, run.file.progress_id(role));
        if done.count <= run.file.baseline(role) {
            if !exited {
                return;
            }
            done.escalate = false;
        }
        match run.file.state.as_str() {
            "implementing" | "fixing" => {
                if !matches!(role, IMPLEMENTATION | STRONG) {
                    return;
                }
                if role != run.file.active_implementer {
                    run.file.set_baseline(role, done.count);
                    self.board(run,&format!("ignored DONE from waiting implementer role={role} session={child} (active={})",run.file.active_implementer),now);
                    return;
                }
                run.file.set_baseline(role, done.count);
                if run.file.state == "implementing" && role == IMPLEMENTATION && done.escalate {
                    run.file.final_seen |= done.final_seen;
                    if self
                        .hand_to_strong(run, "implement", &Verdict::default(), now, cancel)
                        .await
                    {
                        self.transition(run, "implementing", "", now).await;
                        return;
                    }
                    if let Err(error) = self
                        .dispatch(
                            run,
                            IMPLEMENTATION,
                            run.file.prompts().self_implement_text(),
                            cancel,
                            now,
                        )
                        .await
                    {
                        self.finish(
                            run,
                            "stopped",
                            failure_reason(&error),
                            &error.to_string(),
                            now,
                        )
                        .await;
                        return;
                    }
                    self.transition(run, "implementing", "", now).await;
                    return;
                }
                self.implementation_done(run, done.final_seen, now, cancel)
                    .await;
            }
            "reviewing" if role == REVIEW => {
                run.file.set_baseline(role, done.count);
                self.review_done(run, text, done.count, now, cancel).await;
            }
            _ => {}
        }
    }
    async fn implementation_done(
        &self,
        run: &mut Run,
        final_seen: bool,
        now: Timestamp,
        cancel: &TaskCancellation,
    ) {
        let fixed = run.file.state == "fixing";
        run.file.round = if fixed { run.file.round + 1 } else { 1 };
        run.file.final_seen |= final_seen;
        let (commit, files) = if run.file.mode == "worktree" {
            (
                self.git
                    .head(Path::new(&run.file.child_cwd), cancel)
                    .await
                    .unwrap_or_default(),
                self.git
                    .changed(
                        Path::new(&run.file.child_cwd),
                        &run.file.last_reviewed_commit,
                        cancel,
                    )
                    .await
                    .unwrap_or_default(),
            )
        } else {
            (
                String::new(),
                self.git
                    .changed(Path::new(&run.file.child_cwd), "", cancel)
                    .await
                    .unwrap_or_default(),
            )
        };
        let c = run.file.current_c();
        let text = if fixed {
            format!(
                "C{c} fix done by {} (round {})",
                run.file.active_implementer,
                run.file.round - 1
            )
        } else {
            format!("C{c} done by {}", run.file.active_implementer)
        };
        run.file.event(
            "c_done",
            format!(
                "{text}{}",
                if run.file.final_seen {
                    " final=true"
                } else {
                    ""
                }
            ),
            now,
        );
        if let Some(event) = run.file.events.last_mut() {
            event.commit = commit;
            event.files_changed = files;
            event.review_path.clear();
        }
        run.file.review_path = run.file.prompts().review_file(run.file.round);
        let prompt = if fixed && run.file.review_session_id != 0 {
            run.file.prompts().rereview_text()
        } else {
            run.file.prompts().review_prompt()
        };
        let result = if run.file.review_session_id == 0 {
            self.spawn(run, REVIEW, prompt, None, cancel, now).await
        } else {
            self.dispatch(run, REVIEW, prompt, cancel, now).await
        };
        if let Err(error) = result {
            self.finish(
                run,
                "stopped",
                failure_reason(&error),
                &error.to_string(),
                now,
            )
            .await;
            return;
        }
        run.file.event("review_started", String::new(), now);
        self.transition(run, "reviewing", "", now).await;
    }
    async fn review_done(
        &self,
        run: &mut Run,
        text: &str,
        count: i64,
        now: Timestamp,
        cancel: &TaskCancellation,
    ) {
        let default = run.file.prompts().review_file(run.file.round);
        let verdict = match parse_verdict(
            text,
            count,
            &run.file.board_dir(),
            Path::new(&default),
            |path| self.store.review_exists(&run.file, path),
        ) {
            Ok(verdict) => verdict,
            Err(VerdictError::ReviewFileMissing(verdict)) => {
                run.file.review_path = verdict.file;
                self.finish(
                    run,
                    "stopped",
                    "review_file_missing",
                    "review file missing",
                    now,
                )
                .await;
                return;
            }
            Err(error) => {
                self.finish(run, "stopped", "verdict_missing", &error.to_string(), now)
                    .await;
                return;
            }
        };
        if !verdict.file.is_empty() {
            run.file.review_path = verdict.file.clone();
        }
        run.file.last_verdict = Some(verdict.clone());
        let text = match verdict.kind.as_str() {
            "findings" => format!("findings must={} should={}", verdict.must, verdict.should),
            "blocked" => format!("blocked reason={}", verdict.reason),
            _ => verdict.kind.clone(),
        };
        run.file.event("verdict", text.clone(), now);
        let c = run.file.current_c();
        if verdict.kind == "pass" || (verdict.kind == "findings" && verdict.must == 0) {
            if let Ok(head) = self.git.head(Path::new(&run.file.child_cwd), cancel).await
                && !head.trim().is_empty()
            {
                run.file.last_reviewed_commit = head;
            }
            run.file.completed_cs += 1;
            if run.file.final_seen {
                self.finish(run, "completed", "", &format!("C{c} passed (final)"), now)
                    .await;
                return;
            }
            run.file.round = 0;
            run.file.active_implementer = IMPLEMENTATION.into();
            let prompt = run.file.prompts().proceed_text();
            if let Err(error) = self
                .dispatch(run, IMPLEMENTATION, prompt, cancel, now)
                .await
            {
                self.finish(
                    run,
                    "stopped",
                    failure_reason(&error),
                    &error.to_string(),
                    now,
                )
                .await;
                return;
            }
            run.file.event(
                "proceed_sent",
                format!("C{c} passed; proceed to C{}", run.file.current_c()),
                now,
            );
            self.transition(run, "implementing", "", now).await;
        } else if verdict.kind == "findings" {
            if run.file.active_implementer == IMPLEMENTATION
                && run.file.round >= run.file.escalate_after
                && self.hand_to_strong(run, "fix", &verdict, now, cancel).await
            {
                self.transition(run, "fixing", "", now).await;
                return;
            }
            if run.file.round >= run.file.max_rounds {
                self.finish(
                    run,
                    "stopped",
                    "max_rounds",
                    &format!(
                        "must={} still open after round {} of {} ({})",
                        verdict.must,
                        run.file.round,
                        run.file.max_rounds,
                        run.file.active_implementer
                    ),
                    now,
                )
                .await;
                return;
            }
            let role = run.file.active_implementer.clone();
            let prompt = run.file.prompts().fix_text(&verdict);
            if let Err(error) = self.dispatch(run, &role, prompt, cancel, now).await {
                self.finish(
                    run,
                    "stopped",
                    failure_reason(&error),
                    &error.to_string(),
                    now,
                )
                .await;
                return;
            }
            run.file.event(
                "fix_sent",
                format!("{text} → {}", run.file.active_implementer),
                now,
            );
            self.transition(run, "fixing", "", now).await;
        } else {
            self.finish(run, "stopped", "blocked", &verdict.reason, now)
                .await;
        }
    }
    async fn hand_to_strong(
        &self,
        run: &mut Run,
        kind: &str,
        verdict: &Verdict,
        now: Timestamp,
        cancel: &TaskCancellation,
    ) -> bool {
        if !run.file.roles.contains_key(STRONG) {
            return false;
        }
        let reason = if kind == "implement" {
            "plan_hint"
        } else {
            "review_failures"
        };
        let result = if run.file.strong_session_id == 0 {
            let prompt = run.file.prompts().strong_prompt(kind, verdict);
            self.spawn(run, STRONG, prompt, None, cancel, now).await
        } else {
            let prompt = run.file.prompts().strong_text(kind, verdict);
            self.dispatch(run, STRONG, prompt, cancel, now).await
        };
        if let Err(error) = result {
            self.board(
                run,
                &format!(
                    "escalation skipped: {} ({error}) reason={reason}",
                    if error.code == "orchestration_limit" {
                        "child limit"
                    } else {
                        "spawn error"
                    }
                ),
                now,
            );
            return false;
        }
        run.file.active_implementer = STRONG.into();
        run.file.round = 0;
        run.file.event("escalated", format!("reason={reason}"), now);
        if let Some(event) = run.file.events.last_mut() {
            event.review_path = verdict.file.clone();
        }
        if run.file.headless(IMPLEMENTATION) {
            self.board(run,&format!("hand-over to {STRONG}: no wait notice sent (the headless implementer's process already exited)"),now);
        } else {
            let prompt = run.file.prompts().wait_text();
            if let Err(error) = self.inject(run, IMPLEMENTATION, prompt, cancel, now).await {
                self.warn("relay wait text not delivered", error);
            }
        }
        true
    }
    pub(super) async fn exited(
        &self,
        run: &mut Run,
        child: i64,
        state: &str,
        now: Timestamp,
        cancel: &TaskCancellation,
    ) {
        let Some(role) = run.file.role_of(child) else {
            return;
        };
        if run.file.terminal() || run.awaiting_reconnect {
            return;
        }
        let detail = format!("{role} #{child} state={state}");
        if !run.file.headless(role) {
            self.finish(run, "stopped", "child_exited", &detail, now)
                .await;
            return;
        }
        let text = self
            .store
            .read_progress(&run.file, role)
            .map(|p| p.0)
            .unwrap_or_default();
        if state != "completed" {
            if !text.trim().is_empty() {
                self.board(run,&format!("headless {role} session={child} exited state={state}; its progress file said:\n{text}"),now);
            }
            self.finish(run, "stopped", "child_exited", &detail, now)
                .await;
            return;
        }
        self.board(run,&format!("headless {role} session={child} finished its instruction (process exit, state={state})"),now);
        self.advance(run, child, &text, true, now, cancel).await;
    }
}
