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
