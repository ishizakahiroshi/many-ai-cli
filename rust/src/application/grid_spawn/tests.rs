use super::*;
use crate::{
    config::RuntimePaths,
    profile::registry::{Registry, default_adapters, embedded_definitions},
    proto::{self, provider::Layers},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::{sync::Mutex, time::Duration};

#[test]
fn pinned_go_source_extracted_grid_spec_and_layout_corpus() {
    #[derive(serde::Deserialize)]
    struct Case {
        preset: String,
        count: i64,
        label_prefix: String,
        provider: String,
        layout: String,
        specs: Vec<(String, String)>,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!("go-specs.json")).unwrap();
    assert_eq!(cases.len(), 72);
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("project");
    let home = tmp.path().join("home");
    std::fs::create_dir(&cwd).unwrap();
    std::fs::create_dir(&home).unwrap();
    for case in cases {
        let body = GridRequest {
            preset: case.preset,
            count: case.count,
            label_prefix: case.label_prefix,
            provider: case.provider,
            ..Default::default()
        };
        let plan = body.plan(&cwd, &home, |_| Ok(true)).unwrap();
        assert_eq!(plan.layout, case.layout);
        assert_eq!(plan.specs, case.specs);
    }
}

#[test]
fn source_validation_order_labels_and_layout_boundaries() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let cwd = tmp.path().join("project");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&cwd).unwrap();
    let mut body = GridRequest {
        preset: "invalid".into(),
        count: 0,
        layout: "invalid".into(),
        ..Default::default()
    };
    assert_eq!(
        body.plan(&cwd, &home, |_| panic!(
            "invalid preset must precede registry"
        ))
        .err()
        .unwrap()
        .detail,
        "invalid preset"
    );
    body.preset = "shell".into();
    assert_eq!(
        body.plan(&cwd, &home, |_| panic!()).err().unwrap().detail,
        "invalid layout"
    );
    body.layout.clear();
    assert_eq!(
        body.plan(&cwd, &home, |_| panic!()).err().unwrap().detail,
        "count must be 1-18"
    );
    for (count, expected) in [
        (1, "1x1"),
        (2, "1x2"),
        (3, "2x2"),
        (4, "2x2"),
        (5, "2x3"),
        (6, "2x3"),
        (7, "3x3"),
        (9, "3x3"),
        (10, "4x3"),
        (12, "4x3"),
        (13, "6x3"),
        (18, "6x3"),
    ] {
        body.count = count;
        let plan = body
            .plan(&cwd, &home, |_| panic!("shell ignores AI provider lookup"))
            .unwrap();
        assert_eq!(plan.layout, expected);
        assert_eq!(plan.specs.len(), count as usize);
        assert_eq!(plan.specs.last().unwrap().1, format!("grid-{count}"));
    }
    body.provider = "-ignored".into();
    assert_eq!(
        body.plan(&cwd, &home, |_| panic!()).err().unwrap().detail,
        "invalid provider"
    );
    body.provider = "custom-ai".into();
    body.preset = "ai+shell".into();
    body.count = 3;
    body.label_prefix = "日本語".into();
    let plan = body.plan(&cwd, &home, |p| Ok(p == "custom-ai")).unwrap();
    assert_eq!(
        plan.specs,
        vec![
            ("custom-ai".into(), "日本語-custom-ai-1".into()),
            ("shell".into(), "日本語-shell-1".into()),
            ("shell".into(), "日本語-shell-2".into())
        ]
    );
    body.provider = "shell".into();
    assert_eq!(
        body.plan(&cwd, &home, |_| panic!()).err().unwrap().detail,
        "invalid ai provider for ai+shell preset"
    );
}
#[test]
fn go_wire_rejects_earlier_invalid_duplicate_and_accepts_null() {
    assert!(crate::proto::wire::decode::<GridRequest>(br#"{"count":"bad","count":2}"#).is_err());
    let request = crate::proto::wire::decode::<GridRequest>(
        br#"{"preset":"shell","count":null,"COUNT":2,"provider":null}"#,
    )
    .unwrap();
    assert_eq!(request.count, 2);
    assert_eq!(request.provider, "");
}
struct Capture {
    requests: Mutex<Vec<WrappedStartRequest>>,
    fail_at: usize,
}
impl WrappedSessionSpawner for Capture {
    fn start_wrapped<'a>(
        &'a self,
        request: WrappedStartRequest,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnStartOutcome> {
        Box::pin(async move {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            if requests.len() == self.fail_at {
                SpawnStartOutcome::Failed("synthetic native failure".into())
            } else {
                SpawnStartOutcome::Started {
                    pid: 100 + requests.len() as u32,
                }
            }
        })
    }
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("grid never waits for registration") })
    }
}
struct Boundary;
impl WrapperTransport for Boundary {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("grid has no invented session transport") })
    }
}
impl CoreEffectSink for Boundary {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { panic!("grid has no invented registry") })
    }
}
#[tokio::test]
async fn native_start_only_partial_failure_retains_exact_pending_proof() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().join("project");
    let home = tmp.path().join("home");
    let trial = tmp.path().join("trial");
    let installed = tmp.path().join("installed");
    for path in [&cwd, &home, &trial, &installed] {
        std::fs::create_dir(path).unwrap();
    }
    let paths = RuntimePaths::trial(&trial, 49182, &installed).unwrap();
    let capture = Arc::new(Capture {
        requests: Mutex::new(vec![]),
        fail_at: 2,
    });
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths,
            None,
            JournalOptions {
                session_enabled: false,
                max_bytes: 0,
            },
        )),
        Arc::new(Boundary),
        Arc::new(Boundary),
        capture.clone(),
        CoreEventBus::new(16).unwrap(),
    ));
    let registry = Registry::build(
        Layers {
            embedded: Some(embedded_definitions().unwrap()),
            ..Default::default()
        },
        &default_adapters(),
    );
    let tasks = crate::hub::task_owner::HubTaskOwner::new(tokio::runtime::Handle::current());
    let service = Arc::new(GridSpawn::new(Dependencies {
        core: core.clone(),
        registry: Arc::new(move || Ok(registry.clone())),
        hub_cwd: cwd.clone(),
        home,
        environment: vec!["SYNTHETIC_GRID=1".into()],
        tasks: tasks.handle(),
    }));
    let body = GridRequest {
        preset: "ai+shell".into(),
        provider: "codex".into(),
        count: 3,
        ..Default::default()
    };
    let result = service
        .spawn(
            body,
            &VerifiedConfirmationRequest::after_server_authentication(core.auth_epoch()),
        )
        .await;
    assert_eq!(result.err().unwrap().code, "spawn_error");
    {
        let requests = capture.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let first = &requests[0];
        let second = &requests[1];
        assert_eq!(first.kind, WrappedStartKind::Grid);
        assert_eq!(first.spec.provider, "codex");
        assert_eq!(second.spec.provider, "shell");
        assert!(first.spec.registration_proof.is_some());
        assert_ne!(first.spec.spawn_attempt, second.spec.spawn_attempt);
        assert_eq!(first.policy.base_environment, vec!["SYNTHETIC_GRID=1"]);
        assert!(first.policy.resolved_model.is_empty());
        assert!(first.spec.permission_mode.is_empty());
        assert!(
            core.peek_spawn_metadata(first.spec.spawn_attempt.unwrap(), "codex")
                .is_some()
        );
        assert!(core.begin_provider_update("codex").is_err());
        assert!(core.registered_session_ids().is_empty());
    }
    tasks.stop_requests();
    tasks.drain_requests().await;
}
