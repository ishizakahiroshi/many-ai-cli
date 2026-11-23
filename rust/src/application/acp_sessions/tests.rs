//! ACP session tests over the scripted agent (`crate::acp::mock_agent`). Synthetic
//! provider ids and folders only; no real agent, no sign-in.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use crate::acp::mock_agent::{MockConfig, MockHandle, connect};
use crate::profile::registry::Registry;
use crate::proto::core::{AuthEpoch, VerifiedUiOrigin};
use crate::proto::provider::{Definition, LaunchDefinition, Layers};

const WAIT: Duration = Duration::from_secs(20);

pub(crate) fn human() -> VerifiedUiOrigin {
    VerifiedUiOrigin::after_server_verification(AuthEpoch(1))
}

pub(crate) fn acp_registry() -> Registry {
    let definition = Definition {
        schema_version: 1,
        id: "synthetic-acp".into(),
        display_name: "Synthetic ACP".into(),
        connection: "acp".into(),
        launch: Some(LaunchDefinition {
            executable: "synthetic-acp-bin".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    Registry::build(
        Layers {
            embedded: Some(crate::profile::registry::embedded_definitions().unwrap()),
            user: Some(vec![definition]),
            ..Default::default()
        },
        &crate::profile::registry::default_adapters(),
    )
}

pub(crate) struct Fixture {
    pub sessions: AcpSessions,
    pub handles: Arc<Mutex<Vec<MockHandle>>>,
    pub connects: Arc<AtomicUsize>,
    _registry: declared::TestRegistryGuard,
    pub provider: &'static str,
    pub folder: tempfile::TempDir,
    pub logs: tempfile::TempDir,
}

pub(crate) fn fixture(config: MockConfig) -> Fixture {
    fixture_for(acp_registry(), "synthetic-acp", config)
}

pub(crate) fn fixture_for(
    registry: Registry,
    provider: &'static str,
    config: MockConfig,
) -> Fixture {
    let registry = declared::install_test_registry(registry);
    let handles: Arc<Mutex<Vec<MockHandle>>> = Arc::default();
    let connects = Arc::new(AtomicUsize::new(0));
    let connector: Connector = {
        let handles = handles.clone();
        let connects = connects.clone();
        Arc::new(move |_provider, _cwd| {
            connects.fetch_add(1, Ordering::SeqCst);
            let (client, events, handle) = connect(config.clone());
            handles.lock().unwrap().push(handle);
            Ok((client, events))
        })
    };
    let logs = tempfile::tempdir().unwrap();
    Fixture {
        sessions: AcpSessions::new(connector, Some(logs.path())),
        handles,
        connects,
        _registry: registry,
        provider,
        folder: tempfile::tempdir().unwrap(),
        logs,
    }
}

impl Fixture {
    pub async fn open(&self) -> SessionView {
        self.sessions
            .open(
                &human(),
                self.provider,
                self.folder.path().to_str().unwrap(),
                None,
            )
            .await
            .expect("open")
    }
    pub fn mock(&self, index: usize) -> MockHandle {
        self.handles.lock().unwrap()[index].clone()
    }
    /// Polls until `done` holds for the events seen so far.
    pub async fn until(&self, id: u64, done: impl Fn(&EventsView) -> bool) -> EventsView {
        let mut after = 0;
        let mut all: Option<EventsView> = None;
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let view = self
                .sessions
                .events(&human(), id, after, Duration::from_millis(200))
                .await
                .unwrap();
            after = view.last_seq;
            match &mut all {
                None => all = Some(view),
                Some(seen) => {
                    seen.events.extend(view.events);
                    seen.pending_permissions = view.pending_permissions;
                    seen.session = view.session;
                    seen.last_seq = view.last_seq;
                }
            }
            if done(all.as_ref().unwrap()) {
                return all.unwrap();
            }
            assert!(tokio::time::Instant::now() < deadline, "timed out");
        }
    }
}

fn kinds(view: &EventsView) -> Vec<&'static str> {
    view.events.iter().map(|e| e.kind).collect()
}

fn has_kind(view: &EventsView, kind: &str) -> bool {
    view.events.iter().any(|e| e.kind == kind)
}

#[tokio::test]
async fn a_session_starts_prompts_and_records_the_turn_in_order() {
    let f = fixture(MockConfig::default());
    let session = f.open().await;
    assert_eq!(session.provider, "synthetic-acp");
    assert_eq!(session.phase, Phase::Idle);
    f.sessions
        .prompt(&human(), session.id, "plain")
        .await
        .unwrap();
    let view = f.until(session.id, |v| has_kind(v, "turn_end")).await;
    let order = kinds(&view);
    assert_eq!(order.first(), Some(&"meta"));
    // Both chunks are recorded before the end of the turn, never after it.
    let end = order.iter().position(|k| *k == "turn_end").unwrap();
    assert_eq!(order.iter().filter(|k| **k == "update").count(), 2);
    assert_eq!(order[..end].iter().filter(|k| **k == "update").count(), 2);
    assert_eq!(view.session.phase, Phase::Idle);
    assert_eq!(
        f.mock(0).methods(),
        ["initialize", "session/new", "session/prompt"]
    );
}

#[tokio::test]
async fn the_session_is_saved_to_a_log_without_secrets() {
    let f = fixture(MockConfig::default());
    let session = f.open().await;
    // The scripted agent answers an unknown prompt with an error; the turn still
    // ends, and the log holds what the person sent, masked.
    f.sessions
        .prompt(
            &human(),
            session.id,
            "Bearer abcdefghijklmnopqrstuvwxyz0123456789",
        )
        .await
        .unwrap();
    f.until(session.id, |v| has_kind(v, "turn_error")).await;
    let name = session.log_file.expect("log file");
    assert!(name.starts_with("acp_synthetic-acp_") && name.ends_with(".jsonl"));
    let text = std::fs::read_to_string(f.logs.path().join("sessions").join(&name)).unwrap();
    assert!(text.lines().next().unwrap().contains("\"kind\":\"meta\""));
    assert!(text.contains("user_prompt"));
    assert!(!text.contains("abcdefghijklmnopqrstuvwxyz0123456789"));
    for line in text.lines() {
        serde_json::from_str::<Value>(line).expect("every log line is JSON");
    }
    let saved = f.sessions.saved();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].agent_session_id, "sess_mock_1");
    assert_eq!(saved[0].provider, "synthetic-acp");
}

#[tokio::test]
async fn a_permission_request_waits_for_a_person_and_is_never_answered_by_the_hub() {
    let f = fixture(MockConfig::default());
    let session = f.open().await;
    f.sessions
        .prompt(&human(), session.id, "permission")
        .await
        .unwrap();
    let view = f
        .until(session.id, |v| !v.pending_permissions.is_empty())
        .await;
    let pending = view.pending_permissions[0].clone();
    assert_eq!(pending.options.len(), 2);
    // Give an automatic answer every chance to happen.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        f.mock(0).find(|m| m["id"] == 900).is_none(),
        "the Hub must not answer a permission request by itself"
    );
    // The person answers from the dashboard.
    f.sessions
        .answer_permission(
            &human(),
            session.id,
            pending.request_key,
            PermissionDecision::Select("allow-once".into()),
        )
        .await
        .unwrap();
    let view = f.until(session.id, |v| has_kind(v, "turn_end")).await;
    assert!(view.pending_permissions.is_empty());
    assert!(has_kind(&view, "permission_answered"));
    let answer = f.mock(0).find(|m| m["id"] == 900).expect("answer sent");
    assert_eq!(answer["result"]["outcome"]["optionId"], "allow-once");
}

#[tokio::test]
async fn an_unknown_permission_request_is_refused() {
    let f = fixture(MockConfig::default());
    let session = f.open().await;
    let error = f
        .sessions
        .answer_permission(&human(), session.id, 99, PermissionDecision::Cancelled)
        .await
        .unwrap_err();
    assert!(matches!(error, AcpSessionError::BadRequest(_)));
}

#[tokio::test]
async fn cancel_stops_the_turn_and_closes_open_requests_without_approving() {
    let f = fixture(MockConfig::default());
    let session = f.open().await;
    f.sessions
        .prompt(&human(), session.id, "permission")
        .await
        .unwrap();
    f.until(session.id, |v| !v.pending_permissions.is_empty())
        .await;
    f.sessions.cancel(&human(), session.id).await.unwrap();
    let view = f.until(session.id, |v| has_kind(v, "turn_end")).await;
    assert!(view.pending_permissions.is_empty());
    assert!(has_kind(&view, "permission_closed"));
    let end = view.events.iter().find(|e| e.kind == "turn_end").unwrap();
    assert_eq!(end.data["stop_reason"], "cancelled");
    let answer = f.mock(0).find(|m| m["id"] == 900).unwrap();
    assert_eq!(answer["result"]["outcome"]["outcome"], "cancelled");
}

#[tokio::test]
async fn only_one_turn_runs_at_a_time() {
    let f = fixture(MockConfig::default());
    let session = f.open().await;
    f.sessions
        .prompt(&human(), session.id, "hang")
        .await
        .unwrap();
    let error = f
        .sessions
        .prompt(&human(), session.id, "plain")
        .await
        .unwrap_err();
    assert!(matches!(error, AcpSessionError::Busy));
    // The prompt is written by the turn task; cancel only after the agent has it.
    for _ in 0..200 {
        if f.mock(0).methods().contains(&"session/prompt".to_string()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    f.sessions.cancel(&human(), session.id).await.unwrap();
    f.until(session.id, |v| has_kind(v, "turn_end")).await;
    f.sessions
        .prompt(&human(), session.id, "plain")
        .await
        .unwrap();
}

#[tokio::test]
async fn a_conversation_is_resumed_through_the_agents_own_resume() {
    let f = fixture(MockConfig::default());
    let session = f
        .sessions
        .open(
            &human(),
            "synthetic-acp",
            f.folder.path().to_str().unwrap(),
            Some("sess_a"),
        )
        .await
        .unwrap();
    assert_eq!(session.agent_session_id, "sess_a");
    assert_eq!(f.mock(0).methods(), ["initialize", "session/resume"]);
    let sent = f.mock(0).find(|m| m["method"] == "session/resume").unwrap();
    assert_eq!(sent["params"]["sessionId"], "sess_a");
    // The agent's own list of conversations is available when it offers one.
    let list = f
        .sessions
        .agent_sessions(&human(), session.id, None)
        .await
        .unwrap();
    assert_eq!(list["sessions"][0]["agent_session_id"], "sess_a");
    assert_eq!(list["next_cursor"], "page2");
}

#[tokio::test]
async fn a_loaded_conversation_marks_its_history_as_replay() {
    let f = fixture(MockConfig {
        no_resume: true,
        ..Default::default()
    });
    let session = f
        .sessions
        .open(
            &human(),
            "synthetic-acp",
            f.folder.path().to_str().unwrap(),
            Some("sess_a"),
        )
        .await
        .unwrap();
    assert_eq!(f.mock(0).methods(), ["initialize", "session/load"]);
    let view = f.until(session.id, |v| has_kind(v, "update")).await;
    let history = view.events.iter().find(|e| e.kind == "update").unwrap();
    assert_eq!(history.data["replay"], true);
    // Updates after the replay are not marked.
    f.sessions
        .prompt(&human(), session.id, "plain")
        .await
        .unwrap();
    let view = f.until(session.id, |v| has_kind(v, "turn_end")).await;
    let fresh: Vec<_> = view
        .events
        .iter()
        .filter(|e| e.kind == "update" && e.data.get("replay").is_none())
        .collect();
    assert_eq!(fresh.len(), 2);
}

#[tokio::test]
async fn an_agent_without_resume_or_load_cannot_reopen_a_conversation() {
    let f = fixture(MockConfig {
        bare_capabilities: true,
        ..Default::default()
    });
    let error = f
        .sessions
        .open(
            &human(),
            "synthetic-acp",
            f.folder.path().to_str().unwrap(),
            Some("sess_a"),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AcpSessionError::Unsupported(_)));
    let session = f.open().await;
    let error = f
        .sessions
        .agent_sessions(&human(), session.id, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AcpSessionError::Unsupported(_)));
    assert!(!session.can_resume && !session.can_close);
}

#[tokio::test]
async fn ending_closes_the_conversation_and_refuses_more_prompts() {
    let f = fixture(MockConfig::default());
    let session = f.open().await;
    let ended = f.sessions.end(&human(), session.id).await.unwrap();
    assert_eq!(ended.phase, Phase::Ended);
    assert!(f.mock(0).methods().contains(&"session/close".to_string()));
    let error = f
        .sessions
        .prompt(&human(), session.id, "plain")
        .await
        .unwrap_err();
    assert!(matches!(error, AcpSessionError::Ended(_)));
    // The log is kept so the conversation can be resumed later.
    assert_eq!(f.sessions.saved().len(), 1);
}

#[tokio::test]
async fn a_crash_ends_the_session_and_nothing_is_reconnected_or_resent() {
    let f = fixture(MockConfig::default());
    let session = f.open().await;
    f.sessions
        .prompt(&human(), session.id, "crash")
        .await
        .unwrap();
    let view = f
        .until(session.id, |v| v.session.phase == Phase::Ended)
        .await;
    assert!(has_kind(&view, "ended"));
    assert!(view.pending_permissions.is_empty());
    // Wait for the failed turn's report; it must not start anything new.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(f.connects.load(Ordering::SeqCst), 1, "no reconnect");
    assert_eq!(
        f.mock(0).methods(),
        ["initialize", "session/new", "session/prompt"],
        "no resend"
    );
    let error = f
        .sessions
        .prompt(&human(), session.id, "plain")
        .await
        .unwrap_err();
    assert!(matches!(error, AcpSessionError::Ended(_)));
    assert_eq!(f.connects.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ending_with_an_open_permission_drops_the_record_unanswered() {
    let f = fixture(MockConfig::default());
    let session = f.open().await;
    f.sessions
        .prompt(&human(), session.id, "permission")
        .await
        .unwrap();
    f.until(session.id, |v| !v.pending_permissions.is_empty())
        .await;
    f.sessions.end(&human(), session.id).await.unwrap();
    let view = f
        .sessions
        .events(&human(), session.id, 0, Duration::ZERO)
        .await
        .unwrap();
    assert!(view.pending_permissions.is_empty());
    assert!(has_kind(&view, "permission_closed"));
    assert!(f.mock(0).find(|m| m["id"] == 900).is_none());
}

#[tokio::test]
async fn a_missing_sign_in_creates_no_session_and_is_not_retried() {
    let f = fixture(MockConfig {
        auth_required: true,
        ..Default::default()
    });
    let error = f
        .sessions
        .open(
            &human(),
            "synthetic-acp",
            f.folder.path().to_str().unwrap(),
            None,
        )
        .await
        .unwrap_err();
    match error {
        AcpSessionError::AuthRequired { methods, .. } => assert_eq!(methods.len(), 2),
        other => panic!("{other}"),
    }
    assert!(f.sessions.list().is_empty());
    assert_eq!(f.connects.load(Ordering::SeqCst), 1, "no retry");
    let methods = f.mock(0).methods();
    assert!(
        !methods.iter().any(|m| m.contains("auth")),
        "no authenticate"
    );
    assert_eq!(methods, ["initialize", "session/new"]);
}

#[tokio::test]
async fn only_an_acp_provider_in_an_existing_folder_opens() {
    let f = fixture(MockConfig::default());
    let folder = f.folder.path().to_str().unwrap();
    for provider in ["claude", "no-such-provider"] {
        let error = f
            .sessions
            .open(&human(), provider, folder, None)
            .await
            .unwrap_err();
        assert!(matches!(error, AcpSessionError::NotAcp(_)));
    }
    let error = f
        .sessions
        .open(&human(), "synthetic-acp", "relative/folder", None)
        .await
        .unwrap_err();
    assert!(matches!(error, AcpSessionError::BadRequest(_)));
    assert_eq!(f.connects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn several_sessions_are_independent_and_only_as_many_as_were_opened() {
    let f = fixture(MockConfig::default());
    let first = f.open().await;
    let second = f.open().await;
    let third = f.open().await;
    assert_eq!(f.sessions.list().len(), 3);
    assert_eq!(f.connects.load(Ordering::SeqCst), 3);
    assert_ne!(first.log_file, second.log_file);
    f.sessions
        .prompt(&human(), second.id, "plain")
        .await
        .unwrap();
    f.until(second.id, |v| has_kind(v, "turn_end")).await;
    let untouched = f
        .sessions
        .events(&human(), third.id, 0, Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(kinds(&untouched), ["meta"]);
    f.sessions.end(&human(), first.id).await.unwrap();
    let views = f.sessions.list();
    assert_eq!(views.iter().filter(|v| v.phase == Phase::Ended).count(), 1);
    assert_eq!(
        f.connects.load(Ordering::SeqCst),
        3,
        "nothing opened on its own"
    );
}

#[test]
fn the_manager_takes_the_dashboards_proof_and_never_signs_in() {
    let source = include_str!("../acp_sessions.rs");
    let code: String = source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!code.contains(".authenticate("));
    assert!(!code.contains("credential"));
    for entry in [
        "pub async fn open",
        "pub async fn events",
        "pub async fn prompt",
        "pub async fn answer_permission",
        "pub async fn cancel",
        "pub async fn agent_sessions",
        "pub async fn end",
    ] {
        let at = code.find(entry).unwrap_or_else(|| panic!("{entry}"));
        assert!(
            code[at..at + 200].contains("VerifiedUiOrigin"),
            "{entry} must take the dashboard's proof"
        );
    }
}

/// The shipped sample definition for Antigravity, added the way a person (or an
/// AI they asked) adds it, then driven end to end over the scripted agent. The
/// real `agy acp` is never started and nothing is signed in.
#[tokio::test]
async fn the_antigravity_sample_runs_end_to_end_and_every_automatic_path_refuses_it() {
    use crate::orchestration::child_options::RegisteredProviders;
    use crate::profile::registry::default_adapters;
    use crate::profile::validation::validate_definition;
    use crate::proto::time::Timestamp;

    // 1. Add: the sample validates and the registry resolves it as person-only.
    let raw = include_str!("../../../../docs/samples/acp-agent-sample.json");
    let (definition, diagnostics) =
        validate_definition(raw.as_bytes(), &default_adapters()).unwrap();
    assert!(diagnostics.iter().all(|d| d.severity.as_str() != "error"));
    let id: &'static str = Box::leak(definition.id.clone().into_boxed_str());
    let registry = Registry::build(
        Layers {
            embedded: Some(crate::profile::registry::embedded_definitions().unwrap()),
            user: Some(vec![definition]),
            ..Default::default()
        },
        &default_adapters(),
    );
    let f = fixture_for(registry.clone(), id, MockConfig::default());
    assert!(declared::is_acp(id));
    assert_eq!(
        declared::automation_refusal(id),
        Some(declared::ACP_HUMAN_ONLY)
    );

    // 2. Start (a person's proof), send, stream, and a permission nobody answers
    //    for the person.
    let session = f.open().await;
    assert_eq!(session.provider, id);
    f.sessions
        .prompt(&human(), session.id, "plain")
        .await
        .unwrap();
    let view = f.until(session.id, |v| has_kind(v, "turn_end")).await;
    assert!(has_kind(&view, "update"));
    f.sessions
        .prompt(&human(), session.id, "permission")
        .await
        .unwrap();
    let view = f
        .until(session.id, |v| !v.pending_permissions.is_empty())
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        f.mock(0).find(|m| m["id"] == 900).is_none(),
        "nothing answers a permission request on the person's behalf"
    );
    let pending = view.pending_permissions[0].clone();

    // 3. Cancel closes the open request as cancelled, never as an approval.
    f.sessions.cancel(&human(), session.id).await.unwrap();
    let view = f
        .until(session.id, |v| v.pending_permissions.is_empty())
        .await;
    assert!(!has_kind(&view, "permission_answered"));
    if let Some(answer) = f.mock(0).find(|m| m["id"] == 900) {
        assert_eq!(answer["result"]["outcome"]["outcome"], "cancelled");
    }
    let _ = pending;

    // 4. A second, separate session resumes a saved conversation, then ends.
    let resumed = f
        .sessions
        .open(
            &human(),
            id,
            f.folder.path().to_str().unwrap(),
            Some("sess_a"),
        )
        .await
        .unwrap();
    assert_eq!(resumed.agent_session_id, "sess_a");
    f.sessions.end(&human(), resumed.id).await.unwrap();
    f.sessions.end(&human(), session.id).await.unwrap();
    assert!(
        f.sessions
            .prompt(&human(), session.id, "plain")
            .await
            .is_err(),
        "an ended session takes no more prompts"
    );

    // 5. Every automatic path refuses it.
    let registered = RegisteredProviders::from_registry(&registry);
    assert!(
        registered.is_acp(id),
        "children and relay roles are refused"
    );
    assert!(!registered.is_valid(id));
    assert!(!declared::handoff_candidates().contains(&id.to_string()));
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut routine = crate::routine::model::Definition {
        name: "nightly".into(),
        cwd: project.to_string_lossy().into_owned(),
        provider: id.into(),
        prompt: "do it".into(),
        ..Default::default()
    };
    assert_eq!(
        crate::routine::validation::validate(
            &mut routine,
            &dir.path().join("home"),
            Timestamp::now()
        )
        .unwrap_err(),
        declared::ACP_HUMAN_ONLY
    );
}
