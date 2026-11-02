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
        .observe_codex_completion_with_routine(binding, empty.clone(), at, "", true, "")
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
        .observe_codex_completion_with_routine(binding, message, at, "", true, "")
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
            "",
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
async fn hub_confirmed_turn_start_travels_with_the_codex_turn_end_once_and_is_consumed() {
    let (_root, engine, binding, at) = fixture("codex").await;
    let ui = UiBinding {
        connection: UiConnectionId(41),
        auth_epoch: engine.auth_epoch(),
    };
    let receipt = engine
        .submit(
            binding,
            InputRequest {
                bytes: b"do the thing\r".to_vec(),
                authority: InputAuthority::Ui(ui),
            },
            at,
            &TaskCancellation::default(),
        )
        .await;
    assert!(matches!(
        receipt.disposition,
        InputDisposition::TransportWritten { .. }
    ));
    let ended = at + Duration::from_secs(30);
    let summary = proto::DoneSummary {
        text: "task completed".into(),
        at: timestamp(ended).unwrap(),
        ..Default::default()
    };
    let effects = engine
        .observe_codex_completion(binding, summary.clone(), ended)
        .unwrap();
    // The provider-reported end travels with the Hub's own start for the turn, and
    // it sits next to the Git capture in the same batch: the signal does not depend
    // on the diff measurement succeeding.
    let ends: Vec<_> = effects
        .0
        .iter()
        .filter_map(|effect| match effect {
            CoreEffect::Notify(CoreEvent::TurnEnded {
                turn_id,
                provider,
                started_at,
                ended_at,
                ..
            }) => Some((
                turn_id.clone(),
                provider.clone(),
                started_at.clone(),
                ended_at.clone(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(ends.len(), 1, "one provider end must not duplicate its signal");
    assert_eq!(ends[0].0, "", "no provider identity was reported here");
    assert_eq!(ends[0].1, "codex");
    assert_eq!(ends[0].2, timestamp(at).unwrap());
    assert_eq!(ends[0].3, timestamp(ended).unwrap());
    assert!(
        effects
            .0
            .iter()
            .any(|effect| matches!(effect, CoreEffect::Notify(CoreEvent::GitTurnCapture { .. })))
    );
    // The start is consumed: a repeat of the same completion cannot re-measure this
    // turn from an older start, and a turn whose input the Hub never confirmed
    // (native terminal input) reports its end with no start at all.
    let resent = engine
        .observe_codex_completion(binding, summary, ended)
        .unwrap();
    assert_eq!(
        resent.0.iter().find_map(|effect| match effect {
            CoreEffect::Notify(CoreEvent::TurnEnded { started_at, .. }) => {
                Some(started_at.clone())
            }
            _ => None,
        }),
        Some(String::new())
    );
}
#[tokio::test]
async fn provider_turn_start_pairs_with_its_own_end_and_is_never_attached_to_another() {
    // parent review R6: the provider's own turn_id decides which start an end may
    // use. A new turn's start must not be attached to an older turn's end, and it
    // must not be consumed by it either.
    let (_root, engine, binding, at) = fixture("codex").await;
    let first = at;
    let second = at + Duration::from_secs(10);
    let ended = at + Duration::from_secs(20);
    engine
        .observe_codex_turn_start(binding, "turn-1".into(), first, true)
        .unwrap();
    let started = engine
        .observe_codex_turn_start(binding, "turn-2".into(), second, true)
        .unwrap();
    assert!(
        matches!(
            &started.0[0],
            CoreEffect::Notify(CoreEvent::TurnStarted { turn_id, started_at, .. })
                if turn_id == "turn-2" && started_at == &timestamp(second).unwrap()
        ),
        "a new provider turn must be announced to clients"
    );
    // turn-1's end arrives after turn-2 started: it has no start of its own, and
    // turn-2's start stays pending.
    let old = codex_end(&engine, binding, "turn-1", ended).await;
    assert_eq!(old.0, String::new());
    assert_eq!(old.1, "turn-1");
    let current = codex_end(&engine, binding, "turn-2", ended).await;
    assert_eq!(current.0, timestamp(second).unwrap());
    assert_eq!(current.1, "turn-2");
}

#[tokio::test]
async fn repeated_hub_turn_numbers_do_not_collapse_two_real_provider_turns() {
    // parent review R4: the Hub's own counter stayed at the same value for input
    // typed in the terminal, so a client saw two real turns as one turn's resend.
    // The provider identity is what travels now, and each end keeps its own start.
    let (_root, engine, binding, at) = fixture("codex").await;
    let first_end = at + Duration::from_secs(10);
    let second_start = first_end;
    let second_end = at + Duration::from_secs(20);
    engine
        .observe_codex_turn_start(binding, "turn-1".into(), at, true)
        .unwrap();
    let first = codex_end(&engine, binding, "turn-1", first_end).await;
    assert_eq!(first.1, "turn-1");
    assert_eq!(first.0, timestamp(at).unwrap());
    engine
        .observe_codex_turn_start(binding, "turn-2".into(), second_start, true)
        .unwrap();
    let second = codex_end(&engine, binding, "turn-2", second_end).await;
    assert_eq!(second.1, "turn-2");
    assert_eq!(second.0, timestamp(second_start).unwrap());
}

#[tokio::test]
async fn a_connecting_ui_learns_the_turn_the_hub_still_has_open() {
    // parent review R5: a reconnecting client restores the Hub's state instead of
    // its own memory, so an unseen stop mark stays hidden while the open turn runs.
    let (_root, engine, binding, at) = fixture("codex").await;
    engine
        .observe_codex_turn_start(binding, "turn-9".into(), at, false)
        .unwrap();
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
    assert_eq!(priming.open_turns.len(), 1);
    let open = &priming.open_turns[0];
    assert_eq!(open.r#type, TurnStartType::TurnStart);
    assert_eq!(open.session_id, binding.session.0);
    assert_eq!(open.turn_id, "turn-9");
    assert_eq!(open.started_at, timestamp(at).unwrap());
    // The matching end clears it again.
    engine
        .close_codex_open_turn(binding, "turn-9")
        .unwrap();
    let priming = engine
        .attach_ui(
            UiBinding {
                connection: UiConnectionId(43),
                auth_epoch: engine.auth_epoch(),
            },
            None,
            None,
        )
        .unwrap();
    assert!(priming.open_turns.is_empty());
}

/// The `turn_id` and `started_at` of the turn end this observation produced.
async fn codex_end(
    engine: &SessionEngine,
    binding: SessionBinding,
    turn_id: &str,
    ended: Timestamp,
) -> (String, String) {
    let summary = proto::DoneSummary {
        text: "task completed".into(),
        at: timestamp(ended).unwrap(),
        ..Default::default()
    };
    let effects = engine
        .observe_codex_completion_with_routine(binding, summary, ended, "", false, turn_id)
        .unwrap();
    effects
        .0
        .iter()
        .find_map(|effect| match effect {
            CoreEffect::Notify(CoreEvent::TurnEnded {
                started_at,
                turn_id,
                ..
            }) => Some((started_at.clone(), turn_id.clone())),
            _ => None,
        })
        .expect("every provider end reports itself")
}

#[tokio::test]
async fn silence_fallback_and_done_marker_never_produce_a_turn_end_signal() {
    let (_root, engine, binding, at) = fixture("claude").await;
    // The silence fallback candidate is the Git path plus a Completed record; it is
    // not evidence that a provider confirmed a turn end.
    let ended = schedule(&engine, binding, at);
    let fallback = engine.completion_after_git(binding, 1, &ended).unwrap();
    assert!(
        fallback.0.iter().any(|effect| matches!(
            effect,
            CoreEffect::Notify(CoreEvent::Completed {
                fallback: true,
                ..
            })
        )),
        "expected the silence fallback to complete normally"
    );
    assert!(!fallback.0.iter().any(|effect| matches!(
        effect,
        CoreEffect::Notify(CoreEvent::TurnEnded { .. })
    )));
    // The DONE card is another end record, still without a provider turn end.
    let (_root, engine, binding, at) = fixture("claude").await;
    let marker = output(
        &engine,
        binding,
        "[MANY-AI-CLI-DONE] fixed [/MANY-AI-CLI-DONE]",
        at,
    );
    assert!(marker.0.iter().any(|effect| matches!(
        effect,
        CoreEffect::Notify(CoreEvent::Completed {
            fallback: false,
            ..
        })
    )));
    assert!(!marker.0.iter().any(|effect| matches!(
        effect,
        CoreEffect::Notify(CoreEvent::TurnEnded { .. })
    )));
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
