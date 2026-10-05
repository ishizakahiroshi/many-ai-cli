//! Source completion gates belong to the session, including asynchronous Git
//! fallback admission. Workers retain parser cursors, never a second session map.
use super::*;
use crate::orchestration::initial_prompt::InitialPromptOutcome;

const DONE_OPEN: &str = "[MANY-AI-CLI-DONE]";
const DONE_CLOSE: &str = "[/MANY-AI-CLI-DONE]";
const SUMMARY_OPEN: &str = "[MANY-AI-CLI-TURN-SUMMARY]";
const SUMMARY_CLOSE: &str = "[/MANY-AI-CLI-TURN-SUMMARY]";
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct CompletionState {
    pub(super) confirmed_turn: u64,
    marker_seen: bool,
    last_notify: Option<Timestamp>,
    buffer: String,
    fallback: Option<FallbackCandidate>,
    summary: Option<SummaryAwait>,
    initial: Option<(InitialPromptOutcome, Timestamp)>,
    codex: Option<(u64, Timestamp, proto::DoneSummary)>,
    launch_arg: bool,
    thread_checked: Option<Timestamp>,
    text_question: Option<crate::approval::text_question::TextQuestion>,
    note: Option<NoteAwait>,
    note_seq: u64,
}
struct NoteAwait {
    seq: u64,
    path: PathBuf,
    buffer: String,
}
struct FallbackCandidate {
    turn: u64,
    at: Timestamp,
    screen: Vec<String>,
}
struct SummaryAwait {
    turn: i64,
    deadline: Timestamp,
    buffer: String,
}
impl CompletionState {
    pub(super) fn discard_text_question(&mut self) {
        self.text_question = None;
    }
    pub(super) fn user_turn(&mut self) {
        self.confirmed_turn = self.confirmed_turn.wrapping_add(1);
        self.marker_seen = false;
        self.fallback = None;
        self.codex = None;
        self.text_question = None;
    }
}
impl SessionEngine {
    pub fn begin_handoff_note(
        &self,
        binding: SessionBinding,
        path: PathBuf,
    ) -> Result<u64, &'static str> {
        let mut state = lock(&self.state);
        let s = state.session(binding).map_err(|_| "session_not_writable")?;
        if !s.connected || !s.ai_provider() {
            return Err("session_not_writable");
        }
        if s.completion.note.is_some() {
            return Err("already_pending");
        }
        s.completion.note_seq = s.completion.note_seq.wrapping_add(1);
        let seq = s.completion.note_seq;
        s.completion.note = Some(NoteAwait {
            seq,
            path,
            buffer: String::new(),
        });
        Ok(seq)
    }
    pub fn expire_handoff_note(&self, binding: SessionBinding, seq: u64) -> CoreEffects {
        let mut state = lock(&self.state);
        let Ok(s) = state.session(binding) else {
            return CoreEffects::default();
        };
        if s.completion
            .note
            .as_ref()
            .is_none_or(|note| note.seq != seq)
        {
            return CoreEffects::default();
        }
        s.completion.note = None;
        state.route(CoreEffects(vec![CoreEffect::Broadcast(proto::Message {
            r#type: "handoff_note".into(),
            session_id: binding.session.0,
            ..Default::default()
        })]))
    }
    /// Detect before evaluate_idle can publish standby/fallback. Ledger I/O is
    /// outside the session lock; both output and user-turn generation must still
    /// match when the recovered answer or newly opened record is applied.
    pub(super) fn open_idle_text_questions(&self, now: Timestamp) -> CoreEffects {
        let openings = {
            let mut state = lock(&self.state);
            let mut openings = Vec::new();
            for s in state.sessions.values_mut() {
                if terminal(&s.snapshot.state)
                    || s.snapshot.activity.output_idle
                    || s.last_output.is_some_and(|at| {
                        now.duration_since(at)
                            .is_ok_and(|elapsed| elapsed < self.options.idle_after)
                    })
                {
                    continue;
                }
                let held = s.completion.text_question.take();
                let source = if s.marker_source.is_transcript(&s.snapshot.provider) {
                    "transcript"
                } else {
                    "go_vt"
                };
                let question = if source == "transcript" {
                    held
                } else if s.usage_probe
                    || s.custom_provider
                    || !s.ai_provider()
                    || s.resize_debounce.is_some_and(|until| now < until)
                    || crate::approval::marker::extract_vt(&s.vt).is_some()
                {
                    None
                } else {
                    crate::approval::text_question::detect(&s.vt.tail_lines(90))
                };
                if let Some(question) = question {
                    let candidate = s.text_candidate(&question);
                    let known = s
                        .approval
                        .consumed()
                        .is_some_and(|consumed| consumed.key == candidate.key)
                        || s.approval
                            .record()
                            .is_some_and(|record| record.data().candidate.key == candidate.key);
                    openings.push((
                        s.binding,
                        s.completion.confirmed_turn,
                        s.output_generation,
                        question,
                        source,
                        candidate,
                        known,
                    ));
                }
            }
            openings
        };
        let mut all = CoreEffects::default();
        for (binding, turn, generation, question, source, candidate, known) in openings {
            let latest = if source == "go_vt" && !known {
                self.journal.storage().and_then(|storage| {
                    storage
                        .latest_approval_of_kinds(
                            binding.session,
                            "go_vt",
                            &[
                                "marker".into(),
                                "plain_yes_no".into(),
                                "sequential_choice".into(),
                                "hub_choice".into(),
                            ],
                        )
                        .ok()
                        .flatten()
                })
            } else {
                None
            };
            let mut state = lock(&self.state);
            let Ok(s) = state.session(binding) else {
                continue;
            };
            if s.completion.confirmed_turn != turn || s.output_generation != generation {
                continue;
            }
            if source == "go_vt"
                && s.approval
                    .restore_answered_vt_question(&candidate, latest.as_ref())
            {
                continue;
            }
            let opened = s.open_text_question(question, source, now);
            all.0.extend(state.route(opened).0);
        }
        all
    }
    pub fn note_registration_prompt(
        &self,
        binding: SessionBinding,
        metadata: &SpawnRegistrationMetadata,
    ) -> Result<(), SessionError> {
        lock(&self.state).session(binding)?.completion.launch_arg = metadata.prompt_at_launch;
        Ok(())
    }
    pub fn initial_prompt_outcome(
        &self,
        binding: SessionBinding,
    ) -> Result<Option<(InitialPromptOutcome, Timestamp)>, SessionError> {
        Ok(lock(&self.state)
            .session(binding)?
            .completion
            .initial
            .clone())
    }
    pub fn codex_thread_follow_view(
        &self,
        binding: SessionBinding,
        now: Timestamp,
    ) -> Result<Option<(TranscriptSessionIdentity, Vec<TranscriptSessionIdentity>)>, SessionError>
    {
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        if s.snapshot.provider != "codex"
            || s.completion.thread_checked.is_some_and(|at| {
                now.duration_since(at)
                    .is_ok_and(|elapsed| elapsed < Duration::from_secs(5))
            })
        {
            return Ok(None);
        }
        s.completion.thread_checked = Some(now);
        let identity = s.transcript.clone();
        let peers = state
            .sessions
            .values()
            .filter(|other| {
                other.binding.session != binding.session
                    && other.snapshot.provider == "codex"
                    && !terminal(&other.snapshot.state)
            })
            .flat_map(|other| {
                let peer = other.transcript.clone();
                let resolved = other
                    .marker_source
                    .resolved_path
                    .to_string_lossy()
                    .into_owned();
                if resolved.is_empty() || resolved == peer.native_log_path {
                    vec![peer]
                } else if peer.native_log_path.is_empty() {
                    let mut resolved_peer = peer;
                    resolved_peer.native_log_path = resolved;
                    vec![resolved_peer]
                } else {
                    // Go claims both the declared native log and the parser's
                    // current path while a thread switch is being observed.
                    let mut resolved_peer = peer.clone();
                    resolved_peer.native_log_path = resolved;
                    vec![peer, resolved_peer]
                }
            })
            .collect();
        Ok(Some((identity, peers)))
    }
    pub fn apply_codex_thread_follow(
        &self,
        binding: SessionBinding,
        current: &PathBuf,
        next: Option<PathBuf>,
        ambiguous: bool,
    ) -> Result<(), SessionError> {
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        if s.snapshot.provider != "codex"
            || (!s.transcript.native_log_path.is_empty()
                && PathBuf::from(&s.transcript.native_log_path) != *current)
        {
            return Err(SessionError::StaleBinding);
        }
        s.marker_source.codex_thread_ambiguous = ambiguous;
        if let Some(next) = next {
            s.transcript.native_log_path = next.to_string_lossy().into_owned();
        }
        Ok(())
    }
    pub fn is_subscription_login(&self, binding: SessionBinding) -> Result<bool, SessionError> {
        Ok(lock(&self.state).session(binding)?.subscription_login)
    }
    pub fn wrapper_pid(&self, binding: SessionBinding) -> Result<Option<u32>, SessionError> {
        let pid = lock(&self.state).session(binding)?.pid;
        Ok(u32::try_from(pid).ok().filter(|pid| *pid > 0))
    }
    pub fn is_usage_probe(&self, binding: SessionBinding) -> Result<bool, SessionError> {
        Ok(lock(&self.state).session(binding)?.usage_probe)
    }
    pub fn observe_transcript_miss(&self, binding: SessionBinding) -> Result<bool, SessionError> {
        Ok(lock(&self.state).session(binding)?.marker_source.miss())
    }
    pub fn observe_transcript_batch(
        &self,
        binding: SessionBinding,
        path: PathBuf,
        messages: &[crate::approval::transcript::parser::AgentChatMessage],
        prime: bool,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        s.marker_source.resolved(path);
        if s.completion.launch_arg
            && s.completion.initial.is_none()
            && messages.iter().any(|message| message.role == "user")
        {
            s.completion.initial=Some((InitialPromptOutcome::Delivered { evidence:crate::orchestration::initial_prompt::DeliveryEvidence::TranscriptUserObserved,attempts:0,composer_enter_retried:false },now));
        }
        if crate::approval::marker::provider_features(&s.snapshot.provider).approval_marker
            != crate::approval::marker::FeatureSource::Native
        {
            return Ok(CoreEffects::default());
        }
        let messages = if prime {
            &messages[messages.len().saturating_sub(1)..]
        } else {
            messages
        };
        let mut effects = CoreEffects::default();
        let mut warnings = Vec::new();
        for message in messages {
            if message.role == "user" && (prime || !message.text.is_empty()) {
                s.completion.text_question = None;
                if s.approval
                    .record()
                    .is_some_and(|record| record.data().origin == "marker")
                {
                    let candidate = s
                        .approval
                        .record()
                        .expect("marker record checked")
                        .data()
                        .candidate
                        .clone();
                    s.approval.mark_consumed(candidate);
                    effects.0.extend(
                        s.approval
                            .close(ApprovalCloseReason::AnsweredTerminal, &message.text, now)
                            .0,
                    );
                }
                s.approval.transcript_user_boundary();
            } else if message.role == "assistant" {
                if let Some(marker) = crate::approval::marker::extract(&message.text) {
                    effects.0.extend(s.set_transcript_question(None, now).0);
                    let (admitted, warning) = s.observe_marker(Some(marker), "transcript", now)?;
                    effects.0.extend(admitted.0);
                    if let Some(error) = warning {
                        warnings.push(error);
                    }
                } else {
                    let question = if !message.text.is_empty() && message.tools.is_empty() {
                        crate::approval::text_question::detect(
                            &message.text.lines().map(str::to_owned).collect::<Vec<_>>(),
                        )
                    } else {
                        None
                    };
                    effects.0.extend(s.set_transcript_question(question, now).0);
                }
            }
        }
        if let Some(update) = s.refresh_approval_activity() {
            effects.0.push(CoreEffect::Broadcast(update));
        }
        let effects = state.route(effects);
        drop(state);
        for error in warnings {
            self.warn("approval marker suppressed: corrupt block", &error);
        }
        Ok(effects)
    }
    pub fn record_initial_prompt_outcome(
        &self,
        binding: SessionBinding,
        outcome: &InitialPromptOutcome,
        at: Timestamp,
    ) -> Result<(), SessionError> {
        let mut state = lock(&self.state);
        let session = state.session(binding)?;
        session.completion.initial = Some((outcome.clone(), at));
        Ok(())
    }
    pub fn begin_turn_summary(
        &self,
        binding: SessionBinding,
        turn: i64,
        now: Timestamp,
    ) -> Result<bool, SessionError> {
        let mut state = lock(&self.state);
        let session = state.session(binding)?;
        if !session.connected || !session.ai_provider() || session.completion.summary.is_some() {
            return Ok(false);
        }
        session.completion.summary = Some(SummaryAwait {
            turn,
            deadline: now + Duration::from_secs(60),
            buffer: String::new(),
        });
        Ok(true)
    }
    /// Applies a Git-proven work predicate to the candidate captured under the
    /// session lock at running -> standby, before any intervening new input.
    pub fn completion_after_git(
        &self,
        binding: SessionBinding,
        files: i64,
        ended_at: &str,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        if let Some((turn, at, summary)) = s.completion.codex.take() {
            if summary.at != ended_at {
                s.completion.codex = Some((turn, at, summary));
            } else if files > 0
                && turn == s.completion.confirmed_turn
                && s.snapshot.provider == "codex"
                && s.completion.last_notify.is_none_or(|last| last <= at)
            {
                s.completion.last_notify = Some(at);
                let effects = s.publish_completion(summary, false, at)?;
                return Ok(state.route(effects));
            }
        }
        let Some(candidate) = s.completion.fallback.take() else {
            return Ok(CoreEffects::default());
        };
        if timestamp(candidate.at)? != ended_at {
            // Another capture cannot consume this candidate's callback.
            s.completion.fallback = Some(candidate);
            return Ok(CoreEffects::default());
        }
        if files == 0
            || !s.fallback_provider()
            || s.approval.record().is_some()
            || s.completion.marker_seen
            || s.completion.confirmed_turn != candidate.turn
            || s.completion.last_notify.is_some_and(|at| at > candidate.at)
        {
            return Ok(CoreEffects::default());
        }
        s.completion.last_notify = Some(candidate.at);
        let last = last_useful_line(&candidate.screen);
        let text = if last.is_empty() {
            "ターン終了（完了サマリーなし）".into()
        } else {
            format!("ターン終了（完了サマリーなし）。最後の出力: {last}")
        };
        let summary = proto::DoneSummary {
            session_id: binding.session.0,
            provider: s.snapshot.provider.clone(),
            title: s.done_title(),
            text,
            kind: "unknown".into(),
            at: timestamp(candidate.at)?,
            fallback: true,
        };
        // The Git gate is already reserved by the caller; a recursive capture
        // here would race the next confirmed turn's baseline.
        let effects = s.publish_completion(summary, false, candidate.at)?;
        Ok(state.route(effects))
    }
    pub fn observe_completion(
        &self,
        binding: SessionBinding,
        summary: proto::DoneSummary,
        capture_git: bool,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        s.completion.last_notify = Some(now);
        let effects = s.publish_completion(summary, capture_git, now)?;
        Ok(state.route(effects))
    }
    pub fn observe_codex_completion(
        &self,
        binding: SessionBinding,
        summary: proto::DoneSummary,
        at: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        self.observe_codex_completion_with_routine(binding, summary, at, "", false)
    }
    pub fn observe_codex_completion_with_routine(
        &self,
        binding: SessionBinding,
        mut summary: proto::DoneSummary,
        at: Timestamp,
        expected_label: &str,
        active_routine: bool,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        if s.snapshot.provider != "codex" {
            return Ok(CoreEffects::default());
        }
        if active_routine && s.snapshot.launch_label == expected_label {
            s.completion.last_notify = Some(Timestamp::now());
            summary.session_id = binding.session.0;
            summary.provider = "codex".into();
            summary.kind = "unknown".into();
            if summary.text.trim().is_empty() {
                return Ok(state.route(CoreEffects(vec![CoreEffect::Notify(
                    CoreEvent::RoutineCompleted { binding, summary },
                )])));
            }
            let effects = s.publish_completion(summary, false, at)?;
            return Ok(state.route(effects));
        }
        if summary.text.trim().is_empty() {
            summary.text = "Codex ターン完了".into();
        }
        summary.kind = classify(&summary.text).into();
        let ended_at = summary.at.clone();
        s.completion.codex = Some((s.completion.confirmed_turn, at, summary));
        Ok(state.route(CoreEffects(vec![CoreEffect::Notify(
            CoreEvent::GitTurnCapture {
                binding,
                started_at: String::new(),
                ended_at: Some(ended_at),
            },
        )])))
    }
}
impl Session {
    fn text_candidate(
        &self,
        question: &crate::approval::text_question::TextQuestion,
    ) -> CandidateIdentity {
        crate::approval::identity::candidate(
            &self.snapshot.provider,
            &question.kind,
            &question.question,
            "",
            &question.options,
            self.approval.epoch(),
        )
    }
    fn open_text_question(
        &mut self,
        question: crate::approval::text_question::TextQuestion,
        source: &str,
        now: Timestamp,
    ) -> CoreEffects {
        if source == "go_vt" && self.approval.blocks_vt_marker() {
            return CoreEffects::default();
        }
        let candidate = self.text_candidate(&question);
        let mut effects = self.approval.observe(
            ApprovalRecordData {
                candidate,
                sig: question.sig,
                origin: "marker".into(),
                source: source.into(),
                kind: question.kind,
                block: question.block,
                question: question.question,
                context: String::new(),
                options: question.options,
                summary: Default::default(),
                detected_at: now,
            },
            None,
            now,
        );
        if let Some(update) = self.refresh_approval_activity() {
            effects.0.push(CoreEffect::Broadcast(update));
        }
        effects
    }
    fn set_transcript_question(
        &mut self,
        question: Option<crate::approval::text_question::TextQuestion>,
        now: Timestamp,
    ) -> CoreEffects {
        let candidate = question
            .as_ref()
            .map(|question| self.text_candidate(question));
        let close = self.approval.record().is_some_and(|record| {
            let record = record.data();
            record.origin == "marker"
                && record.kind != "marker"
                && record.source == "transcript"
                && candidate
                    .as_ref()
                    .is_none_or(|candidate| candidate.key != record.candidate.key)
        });
        let mut effects = if close {
            self.approval.close(ApprovalCloseReason::Vanished, "", now)
        } else {
            CoreEffects::default()
        };
        self.completion.text_question = question;
        if self.snapshot.activity.output_idle
            && let Some(question) = self.completion.text_question.take()
        {
            effects
                .0
                .extend(self.open_text_question(question, "transcript", now).0);
        }
        effects
    }
    fn ai_provider(&self) -> bool {
        !self.snapshot.provider.is_empty() && self.snapshot.provider != "shell"
    }
    fn fallback_provider(&self) -> bool {
        self.ai_provider() && self.snapshot.provider != "codex"
    }
    fn done_title(&self) -> String {
        let mut title = self.snapshot.display.trim();
        if title.is_empty() {
            title = self.snapshot.provider.trim();
        }
        if title.is_empty() {
            title = "many-ai-cli";
        }
        if self.snapshot.label.is_empty() {
            format!("{title} #{}", self.binding.session.0)
        } else {
            format!(
                "{title} #{} [{}]",
                self.binding.session.0, self.snapshot.label
            )
        }
    }
    pub(super) fn schedule_fallback(
        &mut self,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        if !self.fallback_provider() || self.approval.record().is_some() {
            return Ok(CoreEffects::default());
        }
        self.completion.fallback = Some(FallbackCandidate {
            turn: self.completion.confirmed_turn,
            at: now,
            screen: self.vt.tail_lines(self.size.rows.max(0) as usize),
        });
        Ok(CoreEffects(vec![CoreEffect::Notify(
            CoreEvent::GitTurnCapture {
                binding: self.binding,
                started_at: String::new(),
                ended_at: Some(timestamp(now)?),
            },
        )]))
    }
    pub(super) fn scan_completion(
        &mut self,
        clean: &str,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        let mut effects = CoreEffects::default();
        if let Some(note) = self.completion.note.as_mut() {
            append_bounded(&mut note.buffer, clean, 16 * 1024);
            if !extract_blocks(
                &note.buffer,
                "[MANY-AI-CLI-HANDOFF-NOTE]",
                "[/MANY-AI-CLI-HANDOFF-NOTE]",
            )
            .is_empty()
            {
                let note = self.completion.note.take().expect("pending note checked");
                effects
                    .0
                    .push(CoreEffect::Notify(CoreEvent::HandoffNoteWritten {
                        binding: self.binding,
                        path: note.path,
                    }));
            }
        }
        append_bounded(&mut self.completion.buffer, clean, 16 * 1024);
        if self.completion.buffer.contains(DONE_OPEN) && self.completion.buffer.contains(DONE_CLOSE)
        {
            let buffer = std::mem::take(&mut self.completion.buffer);
            let texts = extract_blocks(&buffer, DONE_OPEN, DONE_CLOSE);
            if !texts.is_empty() && self.ai_provider() {
                self.completion.marker_seen = true;
                if self.completion.last_notify.is_none_or(|at| {
                    now.duration_since(at)
                        .is_ok_and(|elapsed| elapsed >= Duration::from_secs(60))
                }) {
                    self.completion.last_notify = Some(now);
                    for text in texts {
                        let summary = proto::DoneSummary {
                            session_id: self.binding.session.0,
                            provider: self.snapshot.provider.clone(),
                            title: self.done_title(),
                            text,
                            at: timestamp(now)?,
                            ..Default::default()
                        };
                        effects
                            .0
                            .extend(self.publish_completion(summary, true, now)?.0);
                    }
                }
            }
        }
        if let Some(summary) = self.completion.summary.as_mut() {
            if now > summary.deadline {
                self.completion.summary = None;
            } else {
                append_bounded(&mut summary.buffer, clean, 16 * 1024);
                if let Some(text) = extract_blocks(&summary.buffer, SUMMARY_OPEN, SUMMARY_CLOSE)
                    .into_iter()
                    .next()
                {
                    let turn = summary.turn;
                    self.completion.summary = None;
                    effects.0.push(CoreEffect::Notify(CoreEvent::TurnSummary {
                        binding: self.binding,
                        turn,
                        text: mask_secrets(&text),
                    }));
                }
            }
        }
        Ok(effects)
    }
    pub(super) fn publish_completion(
        &mut self,
        mut summary: proto::DoneSummary,
        capture_git: bool,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        summary.text = truncate(&mask_secrets(&summary.text));
        if summary.text.is_empty() {
            return Ok(CoreEffects::default());
        }
        summary.session_id = self.binding.session.0;
        if summary.provider.is_empty() {
            summary.provider.clone_from(&self.snapshot.provider);
        }
        if summary.title.is_empty() {
            summary.title = self.done_title();
        }
        if summary.kind.is_empty() {
            summary.kind = classify(&summary.text).into();
        }
        if summary.at.is_empty() {
            summary.at = timestamp(now)?;
        }
        self.done = Some(summary.clone());
        let mut effects = CoreEffects::default();
        if capture_git {
            effects
                .0
                .push(CoreEffect::Notify(CoreEvent::GitTurnCapture {
                    binding: self.binding,
                    started_at: String::new(),
                    ended_at: Some(summary.at.clone()),
                }));
        }
        let fallback = summary.fallback;
        effects
            .0
            .push(CoreEffect::Notify(CoreEvent::CompletionRecord {
                binding: self.binding,
                summary: summary.clone(),
            }));
        effects.0.push(CoreEffect::Broadcast(proto::Message {
            r#type: "done_summary".into(),
            session_id: self.binding.session.0,
            provider: summary.provider.clone(),
            done_summary: Some(summary.clone()),
            ..Default::default()
        }));
        effects.0.push(CoreEffect::Notify(CoreEvent::Completed {
            binding: self.binding,
            summary,
            fallback,
        }));
        Ok(effects)
    }
}
fn append_bounded(buffer: &mut String, text: &str, max: usize) {
    buffer.push_str(text);
    if buffer.len() > max {
        let mut start = buffer.len() - max;
        while !buffer.is_char_boundary(start) {
            start += 1;
        }
        buffer.drain(..start);
    }
}
pub fn extract_blocks(mut text: &str, open: &str, close: &str) -> Vec<String> {
    let mut result = Vec::new();
    while let Some(start) = text.find(open) {
        text = &text[start + open.len()..];
        let Some(end) = text.find(close) else {
            break;
        };
        let block = text[..end].trim();
        if !block.is_empty() {
            result.push(block.split_whitespace().collect::<Vec<_>>().join(" "));
        }
        text = &text[end + close.len()..];
    }
    result
}
pub fn truncate(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= 320 {
        text
    } else {
        format!("{}…", text.chars().take(320).collect::<String>().trim())
    }
}
fn classify(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    for (kind, terms) in [
        (
            "needs_action",
            &["要判断", "要対応", "確認が必要", "blocked", "判断が必要"][..],
        ),
        (
            "aborted",
            &["中断", "キャンセル", "aborted", "cancelled", "canceled"][..],
        ),
        ("failure", &["失敗", "エラー", "failed", "error"][..]),
    ] {
        if terms.iter().any(|term| lower.contains(term)) {
            return kind;
        }
    }
    "success"
}
fn chrome(line: &str) -> bool {
    line.chars().all(|c| {
        c.is_whitespace() || ('\u{2500}'..='\u{259f}').contains(&c) || "-=_~>$❯›".contains(c)
    })
}
fn last_useful_line(screen: &[String]) -> String {
    let mut end = screen.len();
    if let Some(bottom) = (screen.len().saturating_sub(8)..screen.len())
        .rev()
        .find(|&i| screen[i].trim().starts_with(['╰', '└', '┗', '╚']) && chrome(screen[i].trim()))
    {
        end = (0..bottom)
            .rev()
            .find(|&i| {
                screen[i].trim().starts_with(['╭', '┌', '┏', '╔']) && chrome(screen[i].trim())
            })
            .unwrap_or(bottom);
    }
    for line in screen[..end].iter().rev() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with(['│', '┃', '['])
            || line
                .chars()
                .next()
                .is_some_and(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
            || chrome(line)
        {
            continue;
        }
        return truncate(line.trim_start_matches(|c| "│┃┆┊╎▏|⏺●•◦· \t".contains(c)));
    }
    String::new()
}

impl SessionEngine {
    /// Relay completion uses the shared redaction/handoff/notification path,
    /// but does not advance the parent's ordinary turn clock or Git capture.
    pub fn publish_relay_done(&self, mut summary: proto::DoneSummary) -> CoreEffects {
        summary.text = truncate(&mask_secrets(&summary.text));
        if summary.text.is_empty() {
            return CoreEffects::default();
        }
        if summary.kind.is_empty() {
            summary.kind = classify(&summary.text).into();
        }
        if summary.at.is_empty() {
            summary.at = timestamp(Timestamp::now()).unwrap_or_default();
        }
        let mut state = lock(&self.state);
        let mut effects = CoreEffects::default();
        let binding = state
            .sessions
            .get(&LiveSessionId(summary.session_id))
            .map(|session| session.binding);
        if let Some(binding) = binding {
            effects
                .0
                .push(CoreEffect::Notify(CoreEvent::RoutineCompleted {
                    binding,
                    summary: summary.clone(),
                }));
            effects
                .0
                .push(CoreEffect::Notify(CoreEvent::CompletionRecord {
                    binding,
                    summary: summary.clone(),
                }));
        }
        effects.0.push(CoreEffect::Broadcast(proto::Message {
            r#type: "done_summary".into(),
            session_id: summary.session_id,
            provider: summary.provider.clone(),
            done_summary: Some(summary.clone()),
            ..Default::default()
        }));
        if let Some(binding) = binding {
            effects.0.push(CoreEffect::Notify(CoreEvent::Completed {
                binding,
                fallback: summary.fallback,
                summary,
            }));
        }
        state.route(effects)
    }
}
