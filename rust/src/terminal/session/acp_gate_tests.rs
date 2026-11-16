//! A provider declared `connection: acp` is driven by a person only: the
//! engine refuses every start, input and approval answer that does not come
//! from the authenticated dashboard. Synthetic provider ids only.
use super::observer_tests::{Io, now};
use super::*;
use crate::{
    config::RuntimePaths,
    profile::{declared, registry::Registry},
    proto::provider::{Definition, LaunchDefinition, Layers},
    terminal::{events::CoreEventBus, journal::JournalOptions},
};

fn acp_registry() -> Registry {
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
fn engine() -> (tempfile::TempDir, SessionEngine, UiBinding) {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49691, &root.path().join("installed")).unwrap();
    let core = SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
        Arc::new(Io),
        Arc::new(Io),
        Arc::new(Io),
        CoreEventBus::new(64).unwrap(),
    );
    let ui = UiBinding {
        connection: UiConnectionId(77),
        auth_epoch: core.auth_epoch(),
    };
    core.attach_ui(ui, None, None).unwrap();
    (root, core, ui)
}
async fn register(core: &SessionEngine, provider: &str, pid: i64) -> SessionBinding {
    core.register(
        RegisterRequest {
            message: proto::Message {
                provider: provider.into(),
                pid,
                cwd: "synthetic-cwd".into(),
                cols: 80,
                rows: 24,
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
async fn send(
    core: &SessionEngine,
    binding: SessionBinding,
    authority: InputAuthority,
) -> InputDisposition {
    core.submit(
        binding,
        InputRequest {
            bytes: b"hello\r".to_vec(),
            authority,
        },
        now(),
        &TaskCancellation::default(),
    )
    .await
    .disposition
}

#[tokio::test]
async fn only_the_dashboard_may_type_into_an_acp_session() {
    let _guard = declared::install_test_registry(acp_registry());
    let (_root, core, ui) = engine();
    let acp = register(&core, "synthetic-acp", 4101).await;
    for authority in [
        InputAuthority::Internal,
        InputAuthority::GuardedInternal,
        InputAuthority::InitialPrompt,
    ] {
        assert!(
            matches!(
                send(&core, acp, authority).await,
                InputDisposition::Failed { .. }
            ),
            "an automated input must be refused"
        );
    }
    let d = send(&core, acp, InputAuthority::Ui(ui)).await;
    assert!(
        !matches!(d, InputDisposition::Failed { .. }),
        "{}",
        match &d {
            InputDisposition::Failed { detail, .. } => detail.clone(),
            _ => "other".into(),
        }
    );
    // A built-in provider is untouched by the gate.
    let plain = register(&core, "claude", 4102).await;
    assert!(matches!(
        send(&core, plain, InputAuthority::Internal).await,
        InputDisposition::TransportWritten { .. }
    ));
}

#[tokio::test]
async fn an_acp_provider_is_never_started_as_a_terminal_session() {
    let _guard = declared::install_test_registry(acp_registry());
    let (_root, core, _ui) = engine();
    let spec = WrappedSpawnSpec {
        registration_metadata: Default::default(),
        spawn_attempt: None,
        registration_proof: None,
        provider: "synthetic-acp".into(),
        cwd: std::path::PathBuf::from("synthetic-cwd"),
        model: String::new(),
        model_selection: String::new(),
        risk_confirmed: false,
        label: "synthetic".into(),
        permission_mode: String::new(),
        sandbox: String::new(),
        ask_for_approval: String::new(),
        route: String::new(),
        utf8_session: false,
        effort: String::new(),
        execution_mode: "interactive".into(),
        permission_preset: String::new(),
        initial_prompt: String::new(),
        subscription_profile_id: String::new(),
        subscription_login: false,
        usage_probe: false,
        grants: InternalSpawnGrants::default(),
        cancellation: TaskCancellation::default(),
    };
    let outcome = core
        .spawn_and_wait(
            spec,
            Duration::from_secs(1),
            &HttpWaitCancellation::default(),
        )
        .await;
    assert!(
        matches!(&outcome, SpawnWaitOutcome::Failed(m) if m == declared::ACP_HUMAN_ONLY),
        "spawn_and_wait must refuse"
    );
}

#[tokio::test]
async fn only_a_person_answers_the_permission_of_an_acp_session() {
    let _guard = declared::install_test_registry(acp_registry());
    let (_root, core, _ui) = engine();
    let acp = register(&core, "synthetic-acp", 4103).await;
    for origin in [
        NativeActionOrigin::Batch,
        NativeActionOrigin::OneTap { nonce: "n".into() },
        NativeActionOrigin::Automatic {
            rule_id: "r".into(),
        },
    ] {
        let result = core
            .prepare_and_send(
                NativeActionRequest {
                    binding: ApprovalActionBinding {
                        session: acp,
                        candidate_key: "k".into(),
                        source_epoch: Default::default(),
                        sig: "s".into(),
                    },
                    selection: NativeActionSelection::ApproveOnce,
                    origin,
                },
                &TaskCancellation::default(),
            )
            .await;
        assert!(matches!(result, Err(ApprovalActionError::HumanOnly)));
    }
}
