use super::*;
use crate::{
    application::host_actions::PickerKind,
    proto::{self, core::*, time::Timestamp},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use serde::Deserialize;
use std::{io, sync::Mutex, time::Duration};
struct UnusedIo;
impl WrapperTransport for UnusedIo {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("history read must not send wrapper input") })
    }
}
impl CoreEffectSink for UnusedIo {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { panic!("history read must not publish effects") })
    }
}
impl WrappedSessionSpawner for UnusedIo {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("history read must not start providers") })
    }
}
struct Dispatch {
    calls: Mutex<Vec<PathBuf>>,
}
impl HostDispatch for Dispatch {
    fn pick<'a>(&'a self, _: PickerKind) -> CoreFuture<'a, io::Result<String>> {
        Box::pin(async { panic!("unexpected picker") })
    }
    fn open(&self, kind: OpenKind, path: &Path, app: &str) -> io::Result<()> {
        assert_eq!(kind, OpenKind::Directory);
        assert!(app.is_empty());
        self.calls.lock().unwrap().push(path.into());
        Ok(())
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    paths: RuntimePaths,
    core: Arc<SessionEngine>,
    owner: Arc<AgentHistory>,
    dispatch: Arc<Dispatch>,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49331, &root.path().join("installed")).unwrap();
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        Arc::new(UnusedIo),
        Arc::new(UnusedIo),
        Arc::new(UnusedIo),
        CoreEventBus::new(64).unwrap(),
    ));
    let dispatch = Arc::new(Dispatch {
        calls: Mutex::new(vec![]),
    });
    let owner = AgentHistory::new(
        core.clone(),
        paths.clone(),
        paths.root().join("actor"),
        dispatch.clone(),
        Arc::new(|_| panic!("unexpected read warning")),
    );
    Fixture {
        _root: root,
        paths,
        core,
        owner,
        dispatch,
    }
}
fn now() -> Timestamp {
    Timestamp::from_unix(1791158400, 0).unwrap()
}
async fn register(f: &Fixture, provider: &str, pid: i64, agent_id: &str) -> SessionBinding {
    f.core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: provider.into(),
                    pid,
                    cwd: f
                        ._root
                        .path()
                        .join("trial")
                        .join("workspace")
                        .to_string_lossy()
                        .into_owned(),
                    home_dir: f.paths.root().join("actor").to_string_lossy().into_owned(),
                    agent_session_id: agent_id.into(),
                    cols: 100,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(pid as u64),
            now(),
        )
        .await
        .unwrap()
        .binding
}
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
const UUID: &str = "11111111-1111-4111-8111-111111111111";
#[tokio::test]
async fn actual_session_chat_keeps_source_validation_order_and_never_reads_unsupported_provider() {
    let f = fixture();
    let unsupported = register(&f, "copilot", 1301, "").await;
    let response = f
        .owner
        .chat(unsupported.session, "bad", "bad", "bad")
        .unwrap();
    assert_eq!(response["available"], false);
    let claude = register(&f, "claude", 1302, UUID).await;
    assert_eq!(
        f.owner
            .chat(claude.session, "bad", "bad", "")
            .err()
            .unwrap()
            .detail,
        "invalid limit"
    );
    assert_eq!(
        f.owner
            .chat(claude.session, "1", "-2", "")
            .err()
            .unwrap()
            .detail,
        "invalid cursor"
    );
    assert_eq!(
        f.owner.chat(claude.session, "1", "", "").unwrap()["total_known"],
        true
    );
    let identity = f.core.details(claude.session).unwrap().transcript;
    let path = discovery::claude_dir(
        &Path::new(&identity.home_dir).join(".claude"),
        &identity.cwd,
    )
    .join(format!("{UUID}.jsonl"));
    let first = "{\"type\":\"user\",\"message\":{\"content\":\"first\"}}\n";
    let second = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"last ANTHROPIC_API_KEY=synthetic_secret\"}]}}\n";
    write(&path, &(first.to_owned() + second));
    let page = f.owner.chat(claude.session, "1", "", "bad").err().unwrap();
    assert_eq!(page.detail, "invalid cursor");
    let page = f.owner.chat(claude.session, "1", "-1", "bad").unwrap();
    assert_eq!(page["total"], -1);
    assert_eq!(page["total_known"], false);
    assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    assert!(!page.to_string().contains("synthetic_secret"));
    let cursor = page["next_cursor"].as_u64().unwrap();
    assert_eq!(cursor, first.len() as u64);
    let older = f
        .owner
        .chat(claude.session, "1", &cursor.to_string(), "")
        .unwrap();
    assert_eq!(older["messages"][0]["text"], "first");
    assert_eq!(older["next_cursor"], 0);
    assert!(f.dispatch.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn exact_codex_path_precedes_ambiguous_start_lookup_and_trial_open_never_dispatches() {
    let f = fixture();
    let binding = register(&f, "codex", 1303, "").await;
    let path = f.paths.root().join("exact-rollout.jsonl");
    write(
        &path,
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"selected thread\"}}\n",
    );
    f.core
        .apply_observation(
            binding,
            SessionObservation::Transcript {
                path: path.clone(),
                agent_session_id: "thread".into(),
                safe_offset: 0,
                grew_at: String::new(),
            },
            now(),
        )
        .unwrap();
    assert_eq!(PathBuf::from(f.owner.location(binding.session).path), path);
    assert_eq!(
        f.owner
            .open_log(binding.session, &Cancellation::default())
            .err()
            .unwrap()
            .code,
        "open_failed"
    );
    assert!(f.dispatch.calls.lock().unwrap().is_empty());
    let http = crate::hub::agent_history_routes::AgentHistoryHttp::new(f.owner.clone());
    let response = http.handle_authenticated(
        &crate::hub::http::Request {
            method: "POST".into(),
            path: "/api/agent-log/open".into(),
            query: "session_id=bad".into(),
            remote_addr: "192.0.2.1:9".into(),
            ..Default::default()
        },
        &Cancellation::default(),
    );
    assert_eq!(response.status, 403);
}
#[test]
fn metadata_nearest_ignores_bad_candidates_and_refuses_claude_tie_codex_subsecond_ambiguity() {
    let f = fixture();
    let mut identity = TranscriptSessionIdentity {
        provider: "claude".into(),
        cwd: "synthetic/workspace".into(),
        home_dir: f.paths.root().join("actor").to_string_lossy().into_owned(),
        started_at: "2026-10-05T00:00:00Z".into(),
        ..Default::default()
    };
    let dir = discovery::claude_dir(
        &Path::new(&identity.home_dir).join(".claude"),
        &identity.cwd,
    );
    write(&dir.join("00-bad.jsonl"), "invalid\n");
    write(
        &dir.join("a.jsonl"),
        r#"{"cwd":"synthetic/workspace","timestamp":"2026-10-05T00:00:01Z"}"#,
    );
    assert!(discovery::structured_path(&f.paths, &identity).is_some());
    write(
        &dir.join("b.jsonl"),
        r#"{"cwd":"synthetic/workspace","timestamp":"2026-10-05T00:00:01Z"}"#,
    );
    assert!(discovery::structured_path(&f.paths, &identity).is_none());
    identity.provider = "codex".into();
    let date = crate::proto::time::utc(now())
        .unwrap()
        .with_timezone(&chrono::Local);
    use chrono::Datelike;
    let dir = Path::new(&identity.home_dir)
        .join(".codex/sessions")
        .join(format!(
            "{:04}/{:02}/{:02}",
            date.year(),
            date.month(),
            date.day()
        ));
    for (name, stamp) in [
        ("a", "2026-10-05T00:00:00.100Z"),
        ("b", "2026-10-05T00:00:00.500Z"),
    ] {
        write(
            &dir.join(format!("{name}.jsonl")),
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"cwd\":\"synthetic/workspace\",\"timestamp\":\"{stamp}\"}}}}\n"
            ),
        );
    }
    assert!(discovery::structured_path(&f.paths, &identity).is_none());
}
#[derive(Deserialize)]
struct OracleCase {
    name: String,
    body: String,
}
#[derive(Deserialize)]
struct OracleGolden {
    name: String,
    messages: Option<Vec<Value>>,
    error: bool,
}
#[test]
fn pinned_go_grok_history_content_and_scanner_oracle() {
    let cases: Vec<OracleCase> = serde_json::from_str(include_str!("cases.json")).unwrap();
    let golden: Vec<OracleGolden> = serde_json::from_str(include_str!("golden.json")).unwrap();
    assert_eq!(cases.len(), golden.len());
    for (case, golden) in cases.into_iter().zip(golden) {
        assert_eq!(case.name, golden.name);
        let f = fixture();
        let path = f.paths.root().join("grok.jsonl");
        write(&path, &case.body);
        let mut messages = vec![];
        let result = grok::scan(&f.paths, &path, |m| messages.push(m));
        assert_eq!(result.is_err(), golden.error, "{}", case.name);
        if result.is_ok() {
            assert_eq!(
                serde_json::to_value(messages).unwrap(),
                json!(golden.messages.unwrap_or_default()),
                "{}",
                case.name
            );
        }
    }
}
#[test]
fn grok_stream_page_counts_all_source_messages_and_retains_requested_window() {
    let f = fixture();
    let path = f.paths.root().join("grok.jsonl");
    let body = (0..310)
        .map(|i| format!("{{\"type\":\"assistant\",\"content\":\"message{i}\"}}\n"))
        .collect::<String>();
    write(&path, &body);
    let (total, start, messages) = grok::page(&f.paths, &path, None, 50).unwrap();
    assert_eq!((total, start, messages.len()), (310, 260, 50));
    assert_eq!(messages[0].text, "message260");
    let (total, start, messages) = grok::page(&f.paths, &path, Some(9), 3).unwrap();
    assert_eq!((total, start, messages.len()), (310, 9, 3));
    assert_eq!(messages[2].text, "message11");
}

#[test]
fn provider_location_resolves_metadata_only_copilot_cursor_command_and_shared_opencode_store() {
    let f = fixture();
    let home = f.paths.root().join("actor");
    let mut identity = TranscriptSessionIdentity {
        cwd: "synthetic/workspace".into(),
        home_dir: home.to_string_lossy().into_owned(),
        started_at: "2026-10-05T00:00:00Z".into(),
        ..Default::default()
    };
    identity.provider = "copilot".into();
    let path = home.join(".copilot/session-state/one");
    write(
        &path.join("workspace.yaml"),
        "cwd: synthetic/workspace\ncreated_at: '2026-10-05T00:00:01Z'\n",
    );
    let location = discovery::location(&f.paths, &identity);
    assert!(location.available);
    assert_eq!(PathBuf::from(location.path), path);
    identity.provider = "cursor-agent".into();
    let path = home.join(".cursor/chats/hash/one");
    write(
        &path.join("meta.json"),
        r#"{"cwd":"synthetic/workspace","createdAtMs":1791158401000}"#,
    );
    let location = discovery::location(&f.paths, &identity);
    assert!(location.available);
    assert_eq!(PathBuf::from(location.path), path);
    identity.provider = "command-code".into();
    let path =
        discovery::command_dir(&identity.home_dir, &identity.cwd).join(format!("{UUID}.jsonl"));
    write(
        &path,
        r#"{"type":"session","id":"one","cwd":"synthetic/workspace","timestamp":"2026-10-05T00:00:01Z"}"#,
    );
    assert_eq!(
        PathBuf::from(discovery::location(&f.paths, &identity).path),
        path
    );
    identity.provider = "opencode".into();
    let path = home.join(".local/share/opencode/opencode.db");
    write(&path, "BODY_MUST_NOT_BE_READ_OR_RETURNED");
    let location = discovery::location(&f.paths, &identity);
    assert!(location.available);
    assert_eq!(PathBuf::from(&location.path), path);
    assert!(
        !serde_json::to_string(&location)
            .unwrap()
            .contains("BODY_MUST")
    );
}
fn grok_paths(f: &Fixture) -> (TranscriptSessionIdentity, PathBuf, PathBuf) {
    let root = f.paths.root().join("grok-profile");
    let identity = TranscriptSessionIdentity {
        provider: "grok".into(),
        cwd: "synthetic/workspace".into(),
        grok_home: root.to_string_lossy().into_owned(),
        started_at: "2026-10-05T00:00:00Z".into(),
        ..Default::default()
    };
    let cwd = root.join("sessions/synthetic%2Fworkspace");
    (identity, root, cwd)
}
#[test]
fn grok_discovery_prefers_nonempty_candidate_without_falling_back_to_other_profile() {
    let f = fixture();
    let (mut identity, root, cwd) = grok_paths(&f);
    write(
        &root.join("active_sessions.json"),
        r#"[{"session_id":"stub","cwd":"synthetic/workspace","opened_at":"2026-10-05T00:00:00Z"},{"session_id":"body","cwd":"synthetic/workspace","opened_at":"2026-10-05T00:00:02Z"}]"#,
    );
    write(&cwd.join("stub/chat_history.jsonl"), "");
    let body = cwd.join("body/chat_history.jsonl");
    write(&body, "{\"type\":\"assistant\",\"content\":\"safe\"}\n");
    assert_eq!(resolve_grok(&f.paths, &identity), Some(body));
    identity.grok_home = f
        .paths
        .root()
        .join("missing-profile")
        .to_string_lossy()
        .into_owned();
    assert!(resolve_grok(&f.paths, &identity).is_none());
}
#[tokio::test]
async fn grok_history_preserves_missing_file_before_pagination_error_and_user_filter() {
    let f = fixture();
    let binding = register(&f, "grok", 1304, "").await;
    assert_eq!(
        f.owner
            .grok_history(binding.session, "bad", "bad")
            .err()
            .unwrap()
            .detail,
        "grok chat history not found"
    );
    let identity = f.core.details(binding.session).unwrap().transcript;
    let root = Path::new(&identity.home_dir).join(".grok");
    let cwd = crate::application::mobile_connect::query_escape(&identity.cwd).replace('+', "%20");
    let path = root
        .join("sessions")
        .join(cwd)
        .join("active/chat_history.jsonl");
    write(
        &root.join("active_sessions.json"),
        &format!(
            "[{{\"session_id\":\"active\",\"cwd\":{},\"opened_at\":\"2026-10-05T00:00:00Z\"}}]",
            serde_json::to_string(&identity.cwd).unwrap()
        ),
    );
    write(
        &path,
        "{\"type\":\"user\",\"content\":\"<user_info>PRIVATE</user_info><user_query>visible</user_query>\"}\n{\"type\":\"reasoning\",\"content\":\"PRIVATE\"}\n{\"type\":\"assistant\",\"content\":\"answer\"}\n",
    );
    assert_eq!(
        f.owner
            .grok_history(binding.session, "bad", "bad")
            .err()
            .unwrap()
            .detail,
        "invalid limit"
    );
    let response = f.owner.grok_history(binding.session, "1", "-5").unwrap();
    assert_eq!(response["total"], 2);
    assert_eq!(response["offset"], 1);
    assert_eq!(response["messages"][0]["text"], "answer");
    assert!(!response.to_string().contains("PRIVATE"));
}
