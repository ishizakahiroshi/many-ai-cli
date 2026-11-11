use super::*;
use crate::{
    config::RuntimePaths,
    terminal::{events::CoreEventBus, journal::JournalOptions},
};
#[tokio::test]
async fn memo_marker_is_chunked_once_and_old_timeout_cannot_cancel_next_request() {
    let (root, engine, binding, at) = fixture("claude").await;
    let path = root.path().join("memo.md");
    let seq = engine.begin_handoff_note(binding, path.clone()).unwrap();
    assert_eq!(
        engine.begin_handoff_note(binding, path.clone()),
        Err("already_pending")
    );
    let first = engine
        .observe_output(
            binding,
            OutputChunk {
                bytes: b"\x1b[32m[MANY-AI-CLI-HANDOFF-".to_vec(),
                total_pty_bytes: 0,
            },
            at,
        )
        .unwrap();
    assert!(
        !first
            .0
            .iter()
            .any(|e| matches!(e, CoreEffect::Notify(CoreEvent::HandoffNoteWritten { .. })))
    );
    let second = engine
        .observe_output(
            binding,
            OutputChunk {
                bytes: b"NOTE] written [/MANY-AI-CLI-HANDOFF-NOTE]\x1b[0m".to_vec(),
                total_pty_bytes: 0,
            },
            at,
        )
        .unwrap();
    assert_eq!(second.0.iter().filter(|e|matches!(e,CoreEffect::Notify(CoreEvent::HandoffNoteWritten{path:p,..}) if p==&path)).count(),1);
    assert!(engine.expire_handoff_note(binding, seq).0.is_empty());
    let newer = engine.begin_handoff_note(binding, path).unwrap();
    assert!(engine.expire_handoff_note(binding, seq).0.is_empty());
    let expired = engine.expire_handoff_note(binding, newer);
    assert!(expired.0.iter().any(|e|matches!(e,CoreEffect::SendUiBestEffort {message,..} if message.r#type=="handoff_note" && !message.note_ok && message.note_path.is_empty())));
    assert!(engine.expire_handoff_note(binding, newer).0.is_empty());
}
#[tokio::test]
async fn silent_memo_expiry_is_explicit_and_shell_is_unwritable() {
    let (root, engine, binding, _) = fixture("claude").await;
    let seq = engine
        .begin_handoff_note(binding, root.path().join("memo.md"))
        .unwrap();
    assert_eq!(engine.expire_handoff_note(binding, seq).0.len(), 1);
    let (root, shell, binding, _) = fixture("shell").await;
    assert_eq!(
        shell.begin_handoff_note(binding, root.path().join("memo.md")),
        Err("session_not_writable")
    );
}
#[tokio::test]
async fn routine_codex_read_only_completion_has_no_git_gate_and_empty_message_is_record_only() {
    let (_root, engine, binding, at) = fixture("codex").await;
    let empty = proto::DoneSummary {
        provider: "codex".into(),
        at: timestamp(at).unwrap(),
        ..Default::default()
    };
    let effects = engine
        .observe_codex_completion_with_routine(binding, empty.clone(), at, "", true)
        .unwrap();
    assert_eq!(effects.0.len(), 1);
    assert!(
        matches!(&effects.0[0],CoreEffect::Notify(CoreEvent::RoutineCompleted{summary,..}) if summary.text.is_empty() && summary.kind=="unknown" && summary.session_id==binding.session.0 && summary.provider=="codex")
    );
    assert!(engine.details(binding.session).unwrap().done.is_none());
    let message = proto::DoneSummary {
        text: "read-only analysis finished".into(),
        ..empty
    };
    let effects = engine
        .observe_codex_completion_with_routine(binding, message, at, "", true)
        .unwrap();
    assert!(
        !effects
            .0
            .iter()
            .any(|e| matches!(e, CoreEffect::Notify(CoreEvent::GitTurnCapture { .. })))
    );
    assert!(effects.0.iter().any(|e|matches!(e,CoreEffect::SendUiBestEffort {message,..} if message.r#type=="done_summary" && message.done_summary.as_ref().unwrap().kind=="unknown")));
}
#[tokio::test]
async fn stale_routine_label_cannot_bypass_git_work_predicate() {
    let (_root, engine, binding, at) = fixture("codex").await;
    let summary = proto::DoneSummary {
        text: "complete".into(),
        at: timestamp(at).unwrap(),
        ..Default::default()
    };
    let effects = engine
        .observe_codex_completion_with_routine(
            binding,
            summary.clone(),
            at,
            "different-label",
            true,
        )
        .unwrap();
    assert!(
        effects
            .0
            .iter()
            .any(|e| matches!(e, CoreEffect::Notify(CoreEvent::GitTurnCapture { .. })))
    );
    assert!(
        engine
            .completion_after_git(binding, 0, &summary.at)
            .unwrap()
            .0
            .is_empty()
    );
}
struct Io;
impl WrapperTransport for Io {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Ok(()) })
    }
}
impl CoreEffectSink for Io {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { Ok(()) })
    }
}
impl WrappedSessionSpawner for Io {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { SpawnWaitOutcome::Failed("synthetic has no provider".into()) })
    }
}
async fn fixture(provider: &str) -> (tempfile::TempDir, SessionEngine, SessionBinding, Timestamp) {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49674, &root.path().join("installed")).unwrap();
    let engine = SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
        Arc::new(Io),
        Arc::new(Io),
        Arc::new(Io),
        CoreEventBus::new(64).unwrap(),
    );
    let at = Timestamp::from_unix(1791158400, 0).unwrap();
    engine
        .attach_ui(
            UiBinding {
                connection: UiConnectionId(41),
                auth_epoch: engine.auth_epoch(),
            },
            None,
            None,
        )
        .unwrap();
    engine
        .finish_ui_priming(UiBinding {
            connection: UiConnectionId(41),
            auth_epoch: engine.auth_epoch(),
        })
        .unwrap();
    let binding = engine
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: provider.into(),
                    cwd: runtime.to_string_lossy().into_owned(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            at,
        )
        .await
        .unwrap()
        .binding;
    (root, engine, binding, at)
}
fn output(
    engine: &SessionEngine,
    binding: SessionBinding,
    text: &str,
    at: Timestamp,
) -> CoreEffects {
    engine
        .observe_output(
            binding,
            OutputChunk {
                bytes: text.as_bytes().to_vec(),
                total_pty_bytes: 0,
            },
            at,
        )
        .unwrap()
}
fn schedule(engine: &SessionEngine, binding: SessionBinding, at: Timestamp) -> String {
    output(engine, binding, "● changed task\r\n", at);
    let end = at + Duration::from_secs(4);
    let effects = engine.evaluate_idle(end);
    assert!(effects.0.iter().any(|effect| matches!(
        effect,
        CoreEffect::Notify(CoreEvent::GitTurnCapture {
            ended_at: Some(_),
            ..
        })
    )));
    timestamp(end).unwrap()
}

fn codex_activity_record(
    engine: &SessionEngine,
    binding: SessionBinding,
    parser: &mut crate::approval::transcript::parser::ParseState,
    kind: &str,
    turn_id: &str,
    at: Timestamp,
) -> CoreEffects {
    parser.begin_batch();
    parser.parse_record(
        crate::approval::transcript::parser::ProviderFormat::Codex,
        serde_json::json!({"type":"event_msg", "timestamp":timestamp(at).unwrap(),
            "payload":{"type":kind, "turn_id":turn_id}})
        .to_string()
        .as_bytes(),
    );
    engine
        .observe_codex_turn_activity(binding, parser.codex_turn_active().unwrap(), at)
        .unwrap()
}

#[tokio::test]
async fn codex_turn_activity_keeps_silent_work_running_and_real_completion_stops_it() {
    let (_root, engine, binding, at) = fixture("codex").await;
    output(&engine, binding, "working", at);
    // Original failure: terminal silence alone still means standby without
    // provider evidence. A task_started must keep the same quiet turn running.
    engine.evaluate_idle(at + Duration::from_secs(4));
    assert_eq!(engine.snapshot(binding.session).unwrap().state, "standby");
    let mut parser = crate::approval::transcript::parser::ParseState::default();
    codex_activity_record(&engine, binding, &mut parser, "task_started", "turn-1", at);
    engine.evaluate_idle(at + Duration::from_secs(60));
    let snapshot = engine.snapshot(binding.session).unwrap();
    assert!(snapshot.activity.output_idle);
    assert!(snapshot.activity.workflow_active);
    assert_eq!(snapshot.state, "running");
    // Existing initial/reconnect snapshots contain the authoritative state.
    let priming = engine
        .attach_ui(
            UiBinding {
                connection: UiConnectionId(42),
                auth_epoch: engine.auth_epoch(),
            },
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        priming
            .sessions
            .iter()
            .find(|s| s.id == binding.session)
            .unwrap()
            .state,
        "running"
    );
    codex_activity_record(
        &engine,
        binding,
        &mut parser,
        "task_complete",
        "turn-1",
        at + Duration::from_secs(61),
    );
    assert_eq!(engine.snapshot(binding.session).unwrap().state, "standby");
}

#[tokio::test]
async fn codex_turn_activity_rejects_old_end_and_handles_native_next_turn_and_abort() {
    let (_root, engine, binding, at) = fixture("codex").await;
    let mut parser = crate::approval::transcript::parser::ParseState::default();
    codex_activity_record(&engine, binding, &mut parser, "task_started", "old", at);
    codex_activity_record(
        &engine,
        binding,
        &mut parser,
        "task_started",
        "new",
        at + Duration::from_secs(10),
    );
    codex_activity_record(
        &engine,
        binding,
        &mut parser,
        "task_complete",
        "old",
        at + Duration::from_secs(11),
    );
    engine.evaluate_idle(at + Duration::from_secs(60));
    assert_eq!(engine.snapshot(binding.session).unwrap().state, "running");
    codex_activity_record(
        &engine,
        binding,
        &mut parser,
        "task_complete",
        "new",
        at + Duration::from_secs(61),
    );
    assert_eq!(engine.snapshot(binding.session).unwrap().state, "standby");
    // A new native-terminal turn needs no Hub input counter or Git changes.
    codex_activity_record(
        &engine,
        binding,
        &mut parser,
        "task_started",
        "native",
        at + Duration::from_secs(70),
    );
    engine.evaluate_idle(at + Duration::from_secs(100));
    assert_eq!(engine.snapshot(binding.session).unwrap().state, "running");
    codex_activity_record(
        &engine,
        binding,
        &mut parser,
        "turn_aborted",
        "",
        at + Duration::from_secs(101),
    );
    assert_eq!(engine.snapshot(binding.session).unwrap().state, "standby");
}

#[tokio::test]
async fn codex_turn_activity_does_not_override_other_providers_or_replay_redraw_as_work() {
    let (_root, engine, binding, at) = fixture("claude").await;
    engine
        .observe_codex_turn_activity(binding, true, at)
        .unwrap();
    output(&engine, binding, "redraw", at);
    engine.evaluate_idle(at + Duration::from_secs(4));
    assert_eq!(engine.snapshot(binding.session).unwrap().state, "standby");
    let (_root, engine, binding, at) = fixture("codex").await;
    let mut parser = crate::approval::transcript::parser::ParseState::default();
    codex_activity_record(
        &engine,
        binding,
        &mut parser,
        "task_started",
        "finished",
        at,
    );
    codex_activity_record(
        &engine,
        binding,
        &mut parser,
        "task_complete",
        "finished",
        at + Duration::from_secs(10),
    );
    // Neither a repeated old start nor a terminal redraw reopens a finished turn.
    codex_activity_record(
        &engine,
        binding,
        &mut parser,
        "task_started",
        "finished",
        at,
    );
    output(&engine, binding, "redraw", at + Duration::from_secs(11));
    engine.evaluate_idle(at + Duration::from_secs(15));
    assert_eq!(engine.snapshot(binding.session).unwrap().state, "standby");
}

#[tokio::test]
async fn codex_turn_activity_resumes_after_question_without_a_silent_standby_gap() {
    use crate::approval::transcript::parser::{AgentChatMessage, AgentChatTool, ParseState};
    let (_root, engine, binding, at) = fixture("codex").await;
    let mut parser = ParseState::default();
    codex_activity_record(&engine, binding, &mut parser, "task_started", "one", at);
    engine
        .observe_transcript_batch(
            binding,
            PathBuf::from("owned.jsonl"),
            &[AgentChatMessage {
                role: "assistant".into(),
                text: "Proceed with this change? (Y:1/N:0)".into(),
                ..Default::default()
            }],
            false,
            at,
        )
        .unwrap();
    engine.evaluate_idle(at + Duration::from_secs(4));
    assert_eq!(engine.snapshot(binding.session).unwrap().state, "waiting");
    engine
        .observe_transcript_batch(
            binding,
            PathBuf::from("owned.jsonl"),
            &[AgentChatMessage {
                role: "assistant".into(),
                tools: vec![AgentChatTool {
                    name: "synthetic".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            false,
            at + Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(engine.snapshot(binding.session).unwrap().state, "running");
}
#[tokio::test]
async fn git_work_predicate_and_marker_arriving_during_capture_prevent_false_fallback() {
    let (_root, engine, binding, at) = fixture("claude").await;
    let ended = schedule(&engine, binding, at);
    assert!(
        engine
            .completion_after_git(binding, 0, &ended)
            .unwrap()
            .0
            .is_empty()
    );
    let ended = schedule(&engine, binding, at + Duration::from_secs(10));
    output(
        &engine,
        binding,
        "[MANY-AI-CLI-DONE] fixed [/MANY-AI-CLI-DONE]",
        at + Duration::from_secs(15),
    );
    assert!(
        engine
            .completion_after_git(binding, 1, &ended)
            .unwrap()
            .0
            .is_empty()
    );
    assert_eq!(
        engine.details(binding.session).unwrap().done.unwrap().text,
        "fixed"
    );
}
#[tokio::test]
async fn new_confirmed_ui_input_invalidates_pending_fallback() {
    let (_root, engine, binding, at) = fixture("claude").await;
    let ended = schedule(&engine, binding, at);
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: engine.auth_epoch(),
    };
    engine.attach_ui(ui, None, None).unwrap();
    let receipt = engine
        .submit(
            binding,
            InputRequest {
                bytes: b"second confirmed turn\r".to_vec(),
                authority: InputAuthority::Ui(ui),
            },
            at + Duration::from_secs(5),
            &TaskCancellation::default(),
        )
        .await;
    assert!(matches!(
        receipt.disposition,
        InputDisposition::TransportWritten { .. }
    ));
    assert!(
        engine
            .completion_after_git(binding, 1, &ended)
            .unwrap()
            .0
            .is_empty()
    );
}
#[tokio::test]
async fn approval_arriving_during_git_capture_suppresses_fallback() {
    let (_root, engine, binding, at) = fixture("claude").await;
    let ended = schedule(&engine, binding, at);
    let marker =
        crate::approval::marker::extract("[MANY-AI-CLI]\ncontinue? (Y:1/N:0)\n[/MANY-AI-CLI]")
            .unwrap();
    engine
        .observe_transcript_marker(binding, Some(marker), at + Duration::from_secs(5))
        .unwrap();
    assert!(
        engine
            .details(binding.session)
            .unwrap()
            .approval
            .record
            .is_some()
    );
    assert!(
        engine
            .completion_after_git(binding, 1, &ended)
            .unwrap()
            .0
            .is_empty()
    );
}
#[tokio::test]
async fn codex_completion_requires_changed_files_and_prime_is_not_completion() {
    let (_root, engine, binding, at) = fixture("codex").await;
    let summary = proto::DoneSummary {
        text: "task completed".into(),
        at: timestamp(at).unwrap(),
        ..Default::default()
    };
    engine
        .observe_codex_completion(binding, summary.clone(), at)
        .unwrap();
    assert!(engine.details(binding.session).unwrap().done.is_none());
    assert!(
        engine
            .completion_after_git(binding, 0, &summary.at)
            .unwrap()
            .0
            .is_empty()
    );
    engine
        .observe_codex_completion(binding, summary.clone(), at)
        .unwrap();
    let effects = engine
        .completion_after_git(binding, 1, &summary.at)
        .unwrap();
    assert!(effects.0.iter().any(|effect| matches!(
        effect,
        CoreEffect::Notify(CoreEvent::Completed {
            fallback: false,
            ..
        })
    )));
    assert_eq!(
        engine.details(binding.session).unwrap().done.unwrap().text,
        "task completed"
    );
}
#[tokio::test]
async fn transcript_plain_question_waits_for_idle_and_tools_clear_it() {
    let (_root, engine, binding, at) = fixture("claude").await;
    output(&engine, binding, "working", at);
    let question = crate::approval::transcript::parser::AgentChatMessage {
        role: "assistant".into(),
        text: "Proceed with this change? (Y:1/N:0)".into(),
        ..Default::default()
    };
    engine
        .observe_transcript_batch(
            binding,
            PathBuf::from("owned.jsonl"),
            &[question],
            false,
            at,
        )
        .unwrap();
    assert!(
        engine
            .details(binding.session)
            .unwrap()
            .approval
            .record
            .is_none()
    );
    let effects = engine.evaluate_idle(at + Duration::from_secs(4));
    assert!(!effects.0.iter().any(|effect| matches!(
        effect,
        CoreEffect::Notify(CoreEvent::GitTurnCapture {
            ended_at: Some(_),
            ..
        })
    )));
    let record = engine
        .details(binding.session)
        .unwrap()
        .approval
        .record
        .unwrap();
    assert_eq!(record.data().kind, "plain_yes_no");
    let tools = crate::approval::transcript::parser::AgentChatMessage {
        role: "assistant".into(),
        tools: vec![crate::approval::transcript::parser::AgentChatTool {
            name: "synthetic".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    engine
        .observe_transcript_batch(
            binding,
            PathBuf::from("owned.jsonl"),
            &[tools],
            false,
            at + Duration::from_secs(5),
        )
        .unwrap();
    assert!(
        engine
            .details(binding.session)
            .unwrap()
            .approval
            .record
            .is_none()
    );
}
#[tokio::test]
async fn transcript_prime_ignores_old_question_and_launch_user_acceptance_is_once() {
    let (_root, engine, binding, at) = fixture("claude").await;
    engine
        .note_registration_prompt(
            binding,
            &SpawnRegistrationMetadata {
                prompt_at_launch: true,
                ..Default::default()
            },
        )
        .unwrap();
    let messages = vec![
        crate::approval::transcript::parser::AgentChatMessage {
            role: "user".into(),
            text: "launch instruction".into(),
            ..Default::default()
        },
        crate::approval::transcript::parser::AgentChatMessage {
            role: "assistant".into(),
            text: "Old question? (Y:1/N:0)".into(),
            ..Default::default()
        },
        crate::approval::transcript::parser::AgentChatMessage {
            role: "assistant".into(),
            text: "I continued without needing an answer.".into(),
            ..Default::default()
        },
    ];
    engine
        .observe_transcript_batch(binding, PathBuf::from("owned.jsonl"), &messages, true, at)
        .unwrap();
    assert!(
        engine
            .details(binding.session)
            .unwrap()
            .approval
            .record
            .is_none()
    );
    let (outcome, delivered_at) = engine.initial_prompt_outcome(binding).unwrap().unwrap();
    assert!(matches!(
        outcome,
        InitialPromptOutcome::Delivered {
            evidence:
                crate::orchestration::initial_prompt::DeliveryEvidence::TranscriptUserObserved,
            attempts: 0,
            ..
        }
    ));
    assert_eq!(delivered_at, at);
    engine
        .observe_transcript_batch(
            binding,
            PathBuf::from("owned.jsonl"),
            &messages[..1],
            false,
            at + Duration::from_secs(8),
        )
        .unwrap();
    assert_eq!(
        engine.initial_prompt_outcome(binding).unwrap().unwrap().1,
        at
    );
}
#[tokio::test]
async fn reattachment_preserves_done_marker_gate_and_rejects_stale_worker_binding() {
    let (_root, engine, binding, at) = fixture("claude").await;
    let ended = schedule(&engine, binding, at);
    output(
        &engine,
        binding,
        "[MANY-AI-CLI-DONE] fixed [/MANY-AI-CLI-DONE]",
        at + Duration::from_secs(5),
    );
    let snapshot = engine.snapshot(binding.session).unwrap();
    let attached = engine
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: binding.session.0,
                    provider: "claude".into(),
                    cwd: snapshot.cwd,
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    started_at: snapshot.started_at,
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            at + Duration::from_secs(6),
        )
        .await
        .unwrap();
    assert!(matches!(
        engine.completion_after_git(binding, 1, &ended),
        Err(SessionError::StaleBinding)
    ));
    assert!(
        engine
            .completion_after_git(attached.binding, 1, &ended)
            .unwrap()
            .0
            .is_empty()
    );
    assert_eq!(
        engine.details(binding.session).unwrap().done.unwrap().text,
        "fixed"
    );
}
fn marker_text(body: &str) -> String {
    format!(
        "{} {body} {}",
        crate::approval::marker::OPEN,
        crate::approval::marker::CLOSE
    )
}
fn observe_body(
    engine: &SessionEngine,
    binding: SessionBinding,
    body: &str,
    at: Timestamp,
) -> CoreEffects {
    let marker = crate::approval::marker::extract(&marker_text(body)).unwrap();
    engine
        .observe_transcript_marker(binding, Some(marker), at)
        .unwrap()
}
fn suppressed_broadcasts(effects: &CoreEffects) -> usize {
    effects
        .0
        .iter()
        .filter(|effect| {
            matches!(effect, CoreEffect::SendUiBestEffort { message, .. }
                if message.r#type == "approval_marker_suppressed")
        })
        .count()
}
#[tokio::test]
async fn marker_with_question_only_opens_a_yes_no_record_not_a_broken_marker() {
    let (_root, engine, binding, at) = fixture("claude").await;
    let body = "この形でpush・Issue作成・dotsへの送信まで進めていいですか？";
    let effects = observe_body(&engine, binding, body, at);
    assert_eq!(suppressed_broadcasts(&effects), 0);
    let record = engine
        .details(binding.session)
        .unwrap()
        .approval
        .record
        .expect("a question inside the marker must open a record");
    let data = record.data();
    assert_eq!(data.kind, "plain_yes_no");
    assert_eq!(data.origin, "marker");
    assert_eq!(data.source, "transcript");
    assert_eq!(data.question, body);
    assert_eq!(
        data.block,
        marker_text(body),
        "block stays the marker block"
    );
    let options = data
        .options
        .iter()
        .map(|o| (o.num, o.label.as_str(), o.is_current, o.preserve_order))
        .collect::<Vec<_>>();
    assert_eq!(
        options,
        [(1, "Yes (1)", true, true), (0, "No (0)", false, true)]
    );
}
#[tokio::test]
async fn marker_without_question_or_choices_opens_no_record_and_no_suppression_notice() {
    let (_root, engine, binding, at) = fixture("claude").await;
    let effects = observe_body(
        &engine,
        binding,
        "決まりを直しました。\n- 1億円以上は億で書きます",
        at,
    );
    assert_eq!(suppressed_broadcasts(&effects), 0);
    assert!(
        engine
            .details(binding.session)
            .unwrap()
            .approval
            .record
            .is_none()
    );
}
#[tokio::test]
async fn marker_with_numbered_choices_keeps_the_marker_record_and_bad_start_is_still_suppressed() {
    let (_root, engine, binding, at) = fixture("claude").await;
    let body = "どちらにしますか?\n1. A (Recommended)\n2. B\nN. User specifies";
    let effects = observe_body(&engine, binding, body, at);
    assert_eq!(suppressed_broadcasts(&effects), 0);
    let record = engine
        .details(binding.session)
        .unwrap()
        .approval
        .record
        .unwrap();
    assert_eq!(record.data().kind, "marker");
    assert!(record.data().options.is_empty());
    assert!(record.data().question.is_empty());
    let (_root, engine, binding, at) = fixture("claude").await;
    let effects = observe_body(&engine, binding, "どれにしますか?\n3. A\n4. B", at);
    assert_eq!(suppressed_broadcasts(&effects), 1);
    assert!(
        engine
            .details(binding.session)
            .unwrap()
            .approval
            .record
            .is_none()
    );
}

/// 2026-11-08: a diff-rendered TUI dropped one character of a closing marker
/// in the ANSI-stripped stream ("[/MANY-AI-LI-DONE]"). The block must not run
/// on into the screen that follows and into the next turn's summary.
#[test]
fn a_broken_closing_marker_does_not_swallow_the_next_turn() {
    let stream = "[MANY-AI-CLI-DONE] first turn done. [/MANY-AI-LI-DONE] \
        ✻ Cogitated for 53s · done 11:06 ❯ next prompt from the user \
        [MANY-AI-CLI-DONE] second turn done. [/MANY-AI-CLI-DONE]";
    assert_eq!(
        extract_blocks(stream, DONE_OPEN, DONE_CLOSE),
        vec!["second turn done.".to_string()]
    );
}

fn completed(effects: &CoreEffects) -> usize {
    effects
        .0
        .iter()
        .filter(|e| matches!(e, CoreEffect::Notify(CoreEvent::Completed { .. })))
        .count()
}

/// The stripped stream reads "[/MANY-AI-CXI-DONE]L", but after the cursor move
/// the screen shows the intact closing marker: the summary is the screen's.
#[tokio::test]
async fn the_summary_is_read_from_the_screen_when_the_stream_is_garbled() {
    let (_root, engine, binding, at) = fixture("claude").await;
    let effects = output(
        &engine,
        binding,
        "\x1b[1;1H[MANY-AI-CLI-DONE] first turn [/MANY-AI-CXI-DONE]\x1b[1;42HL",
        at,
    );
    assert_eq!(completed(&effects), 1);
    assert_eq!(
        engine.details(binding.session).unwrap().done.unwrap().text,
        "first turn"
    );
}

/// A redraw that shows an earlier block again (scrolling back to the bottom,
/// for example) publishes nothing; the next turn's block still is published.
#[tokio::test]
async fn a_redrawn_summary_is_not_published_twice() {
    let (_root, engine, binding, at) = fixture("claude").await;
    let first = "\x1b[1;1H[MANY-AI-CLI-DONE] first turn [/MANY-AI-CLI-DONE]";
    assert_eq!(completed(&output(&engine, binding, first, at)), 1);
    let later = at + Duration::from_secs(120);
    assert_eq!(completed(&output(&engine, binding, first, later)), 0);
    let next = "\x1b[2J\x1b[1;1H[MANY-AI-CLI-DONE] second turn [/MANY-AI-CLI-DONE]";
    let after = at + Duration::from_secs(240);
    assert_eq!(completed(&output(&engine, binding, next, after)), 1);
    assert_eq!(
        engine.details(binding.session).unwrap().done.unwrap().text,
        "second turn"
    );
}

/// Wrapped Japanese joins without a space; other wrapped text with one.
#[test]
fn the_screen_joins_wrapped_lines_like_the_text_they_hold() {
    let lines = vec![
        "  [MANY-AI-CLI-DONE] 看板に載ったことを確か".to_string(),
        "  めました。Checked on the".to_string(),
        "  board. [/MANY-AI-CLI-DONE]".to_string(),
    ];
    assert_eq!(
        extract_blocks(&join_screen(&lines), DONE_OPEN, DONE_CLOSE),
        vec!["看板に載ったことを確かめました。Checked on the board.".to_string()]
    );
}

#[test]
fn a_summary_is_classified_by_how_its_first_sentence_ends() {
    for (text, expected) in [
        // The three examples in the rules given to the AI.
        (
            "files タブの検索バグを修正しました。変更: web/src/app/files-view.ts。テスト: bun run check は成功しました。",
            "success",
        ),
        (
            "テスト追加は未完了です。internal/hub/server_test.go の既存失敗 3 件を確認しました。",
            "failure",
        ),
        (
            "実装は停止しています。破壊的な migration の適用可否についてユーザー判断が必要です。",
            "needs_action",
        ),
        // Finished work that names what it fixed or counted (reports like
        // these went out as failures on 2026-11-11).
        (
            "説明書の追記、試験が一時フォルダを残す件（281 個 → 0 個）、混んだ機械で Git の試験が落ちる件（3 本同時で 2/3 失敗 → 0/3）を直しました。",
            "success",
        ),
        (
            "files::branch の 3 件を直して develop に入れました（9ad0f94b、push なし。6 本同時で 6/6 失敗 → 0/6）。空のフォルダは残っています。",
            "success",
        ),
        (
            "古い試験の一時フォルダ 6,921 個を消し、.tmp フォルダは 0 個になりました（消す前に対象を数え直し、失敗 0 件）。",
            "success",
        ),
        (
            "一時フォルダを手放していない入れ物を、試験の実行時に見つけて失敗にする検査を足し、develop に入れました。",
            "success",
        ),
        (
            "検証スクリプトでリンク切れ・はみ出し・JS エラーが 0 件であることを確認しました。",
            "success",
        ),
        ("失敗の原因を調べ、直しました。", "success"),
        (
            "修正を push しました。CI の check は失敗しています。",
            "success",
        ),
        ("All 12 tests passed, 0 failed.", "success"),
        // Work that did not end well says so in its conclusion.
        ("ビルドに失敗しました。原因は調査中です。", "failure"),
        (
            "3 件を直しましたが、残り 1 件はエラーで止まりました。",
            "failure",
        ),
        ("The build failed. See the log.", "failure"),
        ("作業を中断しました。続きは次回です。", "aborted"),
        ("中断していた作業を再開し、最後まで終えました。", "success"),
        ("Cancelled by the user.", "aborted"),
        // A wait for the user counts wherever it is written.
        (
            "案を 3 つ出しました。どれにするかは要判断です。",
            "needs_action",
        ),
    ] {
        assert_eq!(classify(text), expected, "{text}");
    }
}
