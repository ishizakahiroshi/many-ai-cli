use super::*;
use crate::{
    config::RuntimePaths,
    hub::task_owner::HubTaskOwner,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::{
    sync::{
        Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;
fn now() -> Timestamp {
    Timestamp::from_unix(1791158400, 0).unwrap()
}
#[test]
fn numeric_clamps_presence_and_observed_timestamp_match_source() {
    let req = UsageRequest {
        session_id: 1,
        provider: "claude".into(),
        tokens_in: 1000,
        tokens_cache: 1200,
        tokens_out: 200,
        cost_from_relay: true,
        cost_usd: 1.25,
        ctx_used_pct: 101.0,
        rl_5h_pct: 140.0,
        rl_7d_pct: f64::INFINITY,
        codex_primary_used_pct: 101.0,
        claude_rate_limits_present: true,
        claude_5h_field_present: true,
        claude_5h_present: true,
        codex_primary_present: true,
        codex_primary_reset: -4,
        codex_primary_window_minutes: 6_000_000,
        usage_observed_at: "2026-10-05T00:01:02.123456789+09:00".into(),
        effort_level: " \nhigh\tlevel\u{7} ".into(),
        codex_plan_type: "  pro\n ".into(),
        model: " \u{7}Opus 5\n ".into(),
        duration_ms: 2_592_000_001,
        ..Default::default()
    };
    let (msg, at) = normalize::normalize(req, "fallback", now()).unwrap();
    assert_eq!((msg.cost_usd, msg.cost_known), (1.25, true));
    assert_eq!(
        (
            msg.ctx_used_pct,
            msg.rl_5h_pct,
            msg.rl_7d_pct,
            msg.codex_primary_used_pct
        ),
        (0.0, 100.0, 100.0, 0.0)
    );
    assert!(msg.claude_5h_present && msg.codex_primary_present);
    assert_eq!(
        (
            msg.codex_primary_reset,
            msg.codex_primary_window_minutes,
            msg.duration_ms
        ),
        (0, 0, 0)
    );
    assert_eq!(msg.effort_level, "highlevel");
    assert_eq!(msg.codex_plan_type, "  pro ");
    assert_eq!(msg.usage_model, "Opus 5");
    assert_eq!(msg.usage_observed_at, "2026-10-04T15:01:02.123456789Z");
    assert_eq!(
        at,
        crate::proto::time::parse_rfc3339(&msg.usage_observed_at).unwrap()
    );
}
#[test]
fn pricing_subtracts_cached_input_and_matches_only_exact_or_space_suffix() {
    let (cost, known) = pricing::cost("gpt-4.1 medium", 1_000_000, 1_000_000, 500_000);
    assert!(known);
    assert_eq!(cost, 9.25);
    assert_eq!(pricing::cost("gpt-4.1", 1000, 0, 2000), (0.0005, true));
    assert_eq!(
        pricing::cost("gpt-4.1-2026-10-05", 1000, 0, 0),
        (0.0, false)
    );
    assert_eq!(pricing::cost("unknown", 1000, 1000, 0), (0.0, false));
}
#[test]
fn malformed_numeric_fields_reject_while_zero_time_falls_back() {
    for req in [
        UsageRequest {
            tokens_in: -1,
            ..Default::default()
        },
        UsageRequest {
            tokens_out: 1_000_000_001,
            ..Default::default()
        },
        UsageRequest {
            cost_usd: f64::NAN,
            ..Default::default()
        },
        UsageRequest {
            ctx_window: -1,
            ..Default::default()
        },
    ] {
        assert!(normalize::normalize(req, "", now()).is_err());
    }
    for raw in [
        "invalid",
        " 2026-10-05T00:00:00Z",
        "0001-01-01T00:00:00Z",
        "",
    ] {
        let (msg, at) = normalize::normalize(
            UsageRequest {
                usage_observed_at: raw.into(),
                ..Default::default()
            },
            "unknown",
            now(),
        )
        .unwrap();
        assert_eq!(at, now());
        assert!(msg.usage_observed_at.is_empty());
    }
}
struct Io {
    core: Mutex<Weak<SessionEngine>>,
    calls: AtomicUsize,
    records: Mutex<Vec<(proto::Message, Timestamp)>>,
    messages: Mutex<Vec<proto::Message>>,
    entered: Semaphore,
    release: Semaphore,
}
impl UsageSubscription for Io {
    fn record_session_usage<'a>(
        &'a self,
        binding: SessionBinding,
        stat: &'a proto::Message,
        at: Timestamp,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let core = self.core.lock().unwrap().upgrade().unwrap();
            assert!(
                core.session_usage(binding)?
                    .is_some_and(|v| v.tokens_in == stat.tokens_in)
            );
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.records.lock().unwrap().push((stat.clone(), at));
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            Ok(())
        })
    }
}
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
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for effect in effects.0 {
                match effect {
                    CoreEffect::SendUiBestEffort { message, .. } => {
                        self.messages.lock().unwrap().push(message)
                    }
                    CoreEffect::Persist(_) | CoreEffect::PersistBound { .. } => {
                        panic!("usage must never persist")
                    }
                    _ => {}
                }
            }
            Ok(())
        })
    }
}
impl WrappedSessionSpawner for Io {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { SpawnWaitOutcome::Failed("synthetic forbids provider".into()) })
    }
}
async fn fixture(
    provider: &str,
) -> (
    tempfile::TempDir,
    HubTaskOwner,
    Arc<SessionEngine>,
    Arc<SessionUsage>,
    Arc<Io>,
    SessionBinding,
) {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49685, &root.path().join("installed")).unwrap();
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let io = Arc::new(Io {
        core: Mutex::new(Weak::new()),
        calls: AtomicUsize::new(0),
        records: Mutex::new(Vec::new()),
        messages: Mutex::new(Vec::new()),
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
        io.clone(),
        io.clone(),
        io.clone(),
        CoreEventBus::new(16).unwrap(),
    ));
    *io.core.lock().unwrap() = Arc::downgrade(&core);
    let binding = core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: provider.into(),
                    model: "gpt-4.1".into(),
                    cwd: runtime.to_string_lossy().into_owned(),
                    codex_home: runtime.join("profile").to_string_lossy().into_owned(),
                    pid: 8,
                    cols: 80,
                    rows: 24,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            now(),
        )
        .await
        .unwrap()
        .binding;
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: core.auth_epoch(),
    };
    core.attach_ui(ui, None, None).unwrap();
    core.finish_ui_priming(ui).unwrap();
    let owner = SessionUsage::new(
        core.clone(),
        io.clone(),
        io.clone(),
        tasks.handle(),
        Arc::new(|_, _| {}),
    );
    (root, tasks, core, owner, io, binding)
}
fn request(binding: SessionBinding, body: serde_json::Value) -> Request {
    let mut body = body;
    body["session_id"] = binding.session.0.into();
    Request {
        method: "POST".into(),
        path: "/api/session-usage".into(),
        body: serde_json::to_vec(&body).unwrap(),
        ..Default::default()
    }
}
async fn bounded<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), f)
        .await
        .expect("synthetic usage lifecycle stalled")
}
#[tokio::test]
async fn provider_mismatch_is_ignored_before_values_or_transcript_are_processed() {
    let (_root, _tasks, core, owner, io, b) = fixture("codex").await;
    let r=owner.receive(&request(b,serde_json::json!({"provider":"claude","tokens_in":-7,"transcript_path":"C:/wrong.jsonl"})),now()).await;
    assert_eq!(r.status, 200);
    assert!(
        String::from_utf8(r.body)
            .unwrap()
            .contains("provider_mismatch")
    );
    assert!(core.session_usage(b).unwrap().is_none());
    assert_eq!(io.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn source_path_is_admitted_before_invalid_tokens_but_numeric_usage_stays_absent() {
    let (root, _tasks, core, owner, io, b) = fixture("codex").await;
    let path = root.path().join("trial/profile/sessions/example.jsonl");
    let r = owner
        .receive(
            &request(
                b,
                serde_json::json!({"tokens_in":-1,"transcript_path":path}),
            ),
            now(),
        )
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(
        std::path::PathBuf::from(core.details(b.session).unwrap().transcript.native_log_path),
        crate::files::scope::clean(&path)
    );
    assert!(core.session_usage(b).unwrap().is_none());
    assert_eq!(io.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn http_abandonment_retains_profile_then_ui_order_and_usage_never_persists() {
    let (_root, tasks, core, owner, io, b) = fixture("codex").await;
    let req = request(
        b,
        serde_json::json!({"provider":"codex","tokens_in":1000,"tokens_cache":250,"tokens_out":100,"usage_observed_at":"2026-10-05T01:02:03Z","prompt":"PRIVATE_BODY","data":"PRIVATE_BODY"}),
    );
    let operation = tokio::spawn(async move { owner.receive(&req, now()).await });
    bounded(io.entered.acquire()).await.unwrap().forget();
    assert!(io.messages.lock().unwrap().is_empty());
    operation.abort();
    let _ = operation.await;
    io.release.add_permits(1);
    let drained = bounded(tasks.drain_effects()).await;
    assert_eq!(
        (drained.running, drained.panicked, drained.cancelled),
        (0, 0, 0)
    );
    let records = io.records.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].1,
        crate::proto::time::parse_rfc3339("2026-10-05T01:02:03Z").unwrap()
    );
    let messages = io.messages.lock().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].r#type, "usage_stat");
    assert!(
        !serde_json::to_string(&messages[0])
            .unwrap()
            .contains("PRIVATE_BODY")
    );
    assert_eq!(core.session_usage(b).unwrap().unwrap().tokens_in, 1000);
    core.disconnected(b, now()).unwrap();
    assert!(core.session_usage(b).unwrap().is_none());
}

#[test]
fn fixed_go_pricing_corpus() {
    #[derive(serde::Deserialize)]
    struct Input {
        model: String,
        input: i64,
        output: i64,
        cache: i64,
    }
    #[derive(serde::Deserialize)]
    struct Output {
        cost: f64,
        known: bool,
    }
    let input: Vec<Input> = serde_json::from_str(include_str!("pricing-input.json")).unwrap();
    let expected: Vec<Output> = serde_json::from_str(include_str!("pricing-go.json")).unwrap();
    assert_eq!(input.len(), expected.len());
    assert_eq!(input.len(), 79);
    for (i, (v, w)) in input.iter().zip(expected).enumerate() {
        let (c, k) = pricing::cost(&v.model, v.input, v.output, v.cache);
        assert_eq!(k, w.known, "known case {i}");
        assert!(
            (c - w.cost).abs() <= 1e-12,
            "cost case {i}: {c} vs {}",
            w.cost
        );
    }
}

#[test]
fn wire_uses_go_fold_null_and_first_value_validation() {
    let request=Request{method:"POST".into(),body:br#"{"SESSION_ID":9,"provider":null,"tokens_in":null,"tokens_in":7,"cost_from_relay":null,"model":null} trailing"#.to_vec(),..Default::default()};
    let value: UsageRequest = decode_json(&request).unwrap();
    assert_eq!((value.session_id, value.tokens_in), (9, 7));
    assert!(value.provider.is_empty() && value.model.is_empty() && !value.cost_from_relay);
    let bad = Request {
        body: br#"{"tokens_in":"bad","tokens_in":7}"#.to_vec(),
        ..Default::default()
    };
    assert_eq!(decode_json::<UsageRequest>(&bad).unwrap_err().status, 400);
}
