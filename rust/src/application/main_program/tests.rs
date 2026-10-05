use super::*;
use crate::{
    application::hub_runtime::RuntimeLedger,
    config::{Config, ConfigStore, RuntimePaths},
    proto::core::TerminalSize,
};
use std::{sync::Arc, time::Duration};
async fn fixture_context(root: &tempfile::TempDir) -> MainContext {
    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    assert_ne!(port, 47777);
    drop(socket);
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    let paths =
        RuntimePaths::trial(&runtime, port, &root.path().join("installed/.many-ai-cli")).unwrap();
    let mut config = Config {
        token: "synthetic-loopback-token".into(),
        ..Default::default()
    };
    config.hub.idle_timeout_min = 0;
    config.hub.port = i64::from(port);
    let config = Arc::new(ConfigStore::new(paths.clone(), config).unwrap());
    let environment = vec![
        format!("HOME={}", runtime.display()),
        format!("USERPROFILE={}", runtime.display()),
        "PATH=".into(),
    ];
    MainContext {
        config,
        paths,
        cwd: runtime.clone(),
        executable: std::env::current_exe().unwrap(),
        application_home: root.path().join("installed"),
        vendor_home: runtime,
        environment,
        shell: if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }.into(),
        terminal_size: TerminalSize { cols: 80, rows: 24 },
    }
}
#[tokio::test]
async fn actual_owned_loopback_composition_serves_info_and_removes_its_runtime_ledger_after_drain()
{
    let root = tempfile::tempdir().unwrap();
    let context = fixture_context(&root).await;
    let paths = context.paths.clone();
    let port = paths.port();
    let token = context.config.snapshot().unwrap().config.token;
    let composition = HubComposition::new(context, ServeOptions::default())
        .await
        .unwrap();
    let tasks = composition.dependencies.tasks.clone();
    let shutdown = Cancellation::default();
    let command_shutdown = shutdown.clone();
    let command = tokio::spawn(async move { composition.run(&command_shutdown).await });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let mut url = url::Url::parse(&format!("http://127.0.0.1:{port}/api/info")).unwrap();
    url.query_pairs_mut().append_pair("token", &token);
    let response = client.get(url).send().await.unwrap();
    assert!(response.status().is_success());
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["version"], env!("MANY_AI_BUILD_VERSION"));
    assert!(
        RuntimeLedger::open(&paths)
            .unwrap()
            .read()
            .unwrap()
            .is_some()
    );
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(15), command)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        RuntimeLedger::open(&paths)
            .unwrap()
            .read()
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        tasks.effect_permit(),
        Err(crate::proto::core::SessionError::Shutdown)
    ));
    assert!(
        !paths
            .resource(crate::config::Resource::Database)
            .as_os_str()
            .is_empty()
    );
}
#[tokio::test]
async fn explicit_trial_port_conflict_fails_before_publication_and_dev_debug_have_actual_owners() {
    let root = tempfile::tempdir().unwrap();
    let context = fixture_context(&root).await;
    let paths = context.paths.clone();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, paths.port()))
        .await
        .unwrap();
    assert!(
        HubComposition::new(context, ServeOptions::default())
            .await
            .is_err()
    );
    assert!(
        RuntimeLedger::open(&paths)
            .unwrap()
            .read()
            .unwrap()
            .is_none()
    );
    drop(listener);
    let root = tempfile::tempdir().unwrap();
    let context = fixture_context(&root).await;
    assert!(
        HubComposition::new(
            context,
            ServeOptions {
                dev: true,
                debug: true,
                ..Default::default()
            }
        )
        .await
        .is_ok()
    );
}
#[tokio::test]
async fn hub_originated_wrapper_skips_all_bootstrap_probes_and_child_startup() {
    let root = tempfile::tempdir().unwrap();
    let mut context = fixture_context(&root).await;
    context.environment.push("MANY_AI_CLI=1".into());
    context.executable = root.path().join("must-not-be-started.exe");
    let before = context.config.snapshot().unwrap().config.hub.port;
    ensure_hub(&mut context, &Cancellation::default())
        .await
        .unwrap();
    assert_eq!(context.config.snapshot().unwrap().config.hub.port, before);
    assert!(
        RuntimeLedger::open(&context.paths)
            .unwrap()
            .read()
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn provider_bootstrap_reuses_authenticated_owned_hub_without_spawning_or_persisting_port() {
    let root = tempfile::tempdir().unwrap();
    let mut context = fixture_context(&root).await;
    let observer_context = context.clone();
    context.executable = root.path().join("must-not-be-started.exe");
    let composition = HubComposition::new(observer_context, ServeOptions::default())
        .await
        .unwrap();
    let shutdown = Cancellation::default();
    let stopped = shutdown.clone();
    let serve = tokio::spawn(async move { composition.run(&stopped).await });
    let revision = context.config.snapshot().unwrap().revision;
    // Observe the actual existing owner ready before testing discovery reuse.
    // A newly scheduled run has not yet published its runtime ledger.
    let mut info = url::Url::parse(&format!(
        "http://127.0.0.1:{}/api/info",
        context.paths.port()
    ))
    .unwrap();
    info.query_pairs_mut()
        .append_pair("token", &context.config.snapshot().unwrap().config.token);
    let ready = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get(info)
        .send()
        .await
        .unwrap();
    assert!(ready.status().is_success());
    let ready: serde_json::Value = ready.json().await.unwrap();
    assert_ne!(
        ready["binary_stale"], true,
        "existing owner must not request stale-binary restart"
    );
    let runtime = RuntimeLedger::open(&context.paths)
        .unwrap()
        .read()
        .unwrap()
        .expect("existing owner must publish runtime");
    assert_eq!(runtime.port, i64::from(context.paths.port()));
    assert_eq!(runtime.pid, i64::from(std::process::id()));
    assert!(crate::process::pid_alive(runtime.pid));
    assert_eq!(
        running_port(&context).await.unwrap(),
        Some(context.paths.port()),
        "actual authenticated discovery must find the existing owner before autostart"
    );
    ensure_hub(&mut context, &Cancellation::default())
        .await
        .unwrap();
    assert_eq!(context.config.snapshot().unwrap().revision, revision);
    assert!(
        RuntimeLedger::open(&context.paths)
            .unwrap()
            .read()
            .unwrap()
            .is_some()
    );
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(15), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn routine_routes_persist_in_selected_runtime_and_routine_prompt_metadata_uses_shared_owner()
{
    use crate::routine::runner::RoutineLaunchPreparation;
    let root = tempfile::tempdir().unwrap();
    let context = fixture_context(&root).await;
    let project = context.paths.root().join("project");
    std::fs::create_dir(&project).unwrap();
    let paths = context.paths.clone();
    let port = paths.port();
    let token = context.config.snapshot().unwrap().config.token;
    let composition = HubComposition::new(context, ServeOptions::default())
        .await
        .unwrap();
    let runner = composition.dependencies.routines.clone();
    let store = composition.dependencies.routine_store.clone();
    let preparation = super::routines::RoutinePreparation {
        paths: paths.clone(),
        orchestration: composition.dependencies.orchestration.clone(),
        warning: Arc::new(|_, _| {}),
    };
    let run = crate::routine::model::Run {
        provider: "copilot".into(),
        cwd: project.to_string_lossy().into_owned(),
        session_label: "routine-synthetic".into(),
        model: "synthetic-model".into(),
        ..Default::default()
    };
    let spec = preparation
        .prepare(
            &run,
            "synthetic instruction".into(),
            crate::proto::core::TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert!(spec.initial_prompt.is_empty());
    assert_eq!(
        spec.registration_metadata.initial_prompt,
        "synthetic instruction"
    );
    assert!(!spec.registration_metadata.prompt_at_launch);
    assert_eq!(spec.label, run.session_label);
    assert!(spec.spawn_attempt.is_none());
    assert!(spec.registration_proof.is_none());
    let stopped = Cancellation::default();
    let shutdown = stopped.clone();
    let serve = tokio::spawn(async move { composition.run(&shutdown).await });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let mut url = url::Url::parse(&format!("http://127.0.0.1:{port}/api/routines")).unwrap();
    url.query_pairs_mut().append_pair("token", &token);
    let saved=client.post(url.clone()).json(&serde_json::json!({"name":"synthetic routine","cwd":project,"provider":"copilot","prompt":"read synthetic files","enabled":false,"schedule":{"kind":"manual"}})).send().await.unwrap();
    assert!(saved.status().is_success());
    let result: serde_json::Value = client.get(url).send().await.unwrap().json().await.unwrap();
    assert_eq!(result["routines"].as_array().unwrap().len(), 1);
    assert!(store.ready());
    assert!(paths.root().join("routines.json").is_file());
    stopped.cancel();
    tokio::time::timeout(Duration::from_secs(15), serve)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    runner.join_launches().await;
    let loaded = crate::routine::store::RoutineStore::open(Arc::new(
        crate::files::safe_fs::Dir::open(paths.root()).unwrap(),
    ));
    assert_eq!(loaded.definitions().unwrap().len(), 1);
}

#[tokio::test]
async fn corrupt_history_and_obstructed_logs_do_not_abort_all_hub_composition() {
    for obstruct_logs in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let context = fixture_context(&root).await;
        let database = context.paths.resource(crate::config::Resource::Database);
        std::fs::write(&database, b"synthetic non-SQLite input\n").unwrap();
        if obstruct_logs {
            std::fs::write(
                context.paths.resource(crate::config::Resource::Logs),
                b"synthetic log obstruction",
            )
            .unwrap();
        }
        let composition = HubComposition::new(context, ServeOptions::default())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(&database).unwrap(),
            b"synthetic non-SQLite input\n"
        );
        drop(composition);
    }
}
#[tokio::test]
async fn actual_registration_ack_observes_instruction_file_and_shutdown_restores_original() {
    use futures_util::{SinkExt, StreamExt};
    let root = tempfile::tempdir().unwrap();
    let context = fixture_context(&root).await;
    let paths = context.paths.clone();
    let project = context.cwd.clone();
    let target = project.join("AGENTS.md");
    std::fs::write(&target, b"synthetic original\n").unwrap();
    let composition = HubComposition::new(context, ServeOptions::default())
        .await
        .unwrap();
    composition
        .dependencies
        .instruction_rules
        .change("enable", &Cancellation::default())
        .await
        .unwrap();
    let shutdown = Cancellation::default();
    let cancel = shutdown.clone();
    let task = tokio::spawn(async move { composition.run(&cancel).await });
    let (mut wrapper, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}/ws", paths.port()))
            .await
            .unwrap();
    wrapper
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({"type":"register","provider":"copilot","pid":3011,
            "cwd":project,"home_dir":paths.root(),"token":"synthetic-loopback-token"})
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let ack = tokio::time::timeout(Duration::from_secs(5), wrapper.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ack: serde_json::Value = serde_json::from_str(ack.to_text().unwrap()).unwrap();
    assert_eq!(ack["type"], "registered");
    let prepared = std::fs::read_to_string(&target).unwrap();
    assert_ne!(
        prepared, "synthetic original\n",
        "ACK must follow actual instruction mutation"
    );
    assert!(prepared.contains("synthetic original"));
    wrapper.close(None).await.unwrap();
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(15), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"synthetic original\n");
}

#[tokio::test]
async fn production_log_spelling_is_preserved_while_selected_cwd_controls_actual_storage() {
    for (configured, relative_database) in [
        ("logs", Some("logs/any-ai-cli.db")),
        ("one/logs", Some("one/any-ai-cli.db")),
        ("", None),
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut context = fixture_context(&root).await;
        let port = context.paths.port();
        // The production instruction-home walk starts at the filesystem root
        // and uses O_NOFOLLOW, so it does not expand macOS /var, /tmp, or /etc.
        // tempfile sits on that alias; hold the same directory by its physical spelling.
        let home = crate::config::paths::native_system_path(root.path()).join("synthetic-home");
        std::fs::create_dir(&home).unwrap();
        let paths = RuntimePaths::production(&home).unwrap();
        let cwd = context.cwd.clone();
        let mut config = Config {
            token: "synthetic-production-fixture-token".into(),
            ..Default::default()
        };
        config.hub.port = i64::from(port);
        config.hub.log_dir = configured.into();
        config.log.enabled = true;
        context.paths = paths.clone();
        context.config = Arc::new(ConfigStore::new(paths, config).unwrap());
        context.environment = vec![
            format!("HOME={}", home.display()),
            format!("USERPROFILE={}", home.display()),
            "PATH=".into(),
        ];
        context.vendor_home = home.clone();
        context.application_home = home;
        assert_eq!(
            context.config.snapshot().unwrap().config.hub.log_dir,
            configured
        );
        let composition = HubComposition::new(context, ServeOptions::default())
            .await
            .unwrap_or_else(|error| {
                panic!("HubComposition failed for log_dir={configured:?}: {error}")
            });
        assert_eq!(
            composition
                .dependencies
                .config
                .snapshot()
                .unwrap()
                .config
                .hub
                .log_dir,
            configured
        );
        if let Some(relative) = relative_database {
            assert!(cwd.join(relative).is_file());
        } else {
            assert!(!cwd.join("any-ai-cli.db").exists());
            assert!(
                cwd.join("hub.log").is_file(),
                "empty configured logs use cwd, never default runtime/logs"
            );
        }
        drop(composition);
    }
}

#[tokio::test]
async fn actual_trial_composition_keeps_native_notifications_disabled_after_config_refresh() {
    use crate::config::NotifyBackendConfig;
    let root = tempfile::tempdir().unwrap();
    let context = fixture_context(&root).await;
    let receiver = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let url = format!(
        "http://127.0.0.1:{}/synthetic",
        receiver.local_addr().unwrap().port()
    );
    let composition = HubComposition::new(context, ServeOptions::default())
        .await
        .unwrap();
    let config = crate::config::NotifyConfig {
        events: Some(vec!["approval".into(), "done".into()]),
        backends: Some(vec![NotifyBackendConfig {
            r#type: "webhook".into(),
            url,
            ..Default::default()
        }]),
        ..Default::default()
    };
    composition.dependencies.notifications.update_config(config);
    assert!(
        !composition
            .dependencies
            .notifications
            .send_security("synthetic".into(), "synthetic".into())
            .unwrap()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(25), receiver.accept())
            .await
            .is_err()
    );
    drop(composition);
}

#[tokio::test]
async fn actual_http_composition_keeps_relay_guards_and_trial_notification_denial() {
    let root = tempfile::tempdir().unwrap();
    let context = fixture_context(&root).await;
    let port = context.paths.port();
    let receiver = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let backend = serde_json::json!({"type":"webhook","url":format!(
        "http://127.0.0.1:{}/synthetic",receiver.local_addr().unwrap().port())});
    let composition = HubComposition::new(context, ServeOptions::default())
        .await
        .unwrap();
    let shutdown = Cancellation::default();
    let cancelled = shutdown.clone();
    let task = tokio::spawn(async move { composition.run(&cancelled).await });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let endpoint =
        |path: &str| format!("http://127.0.0.1:{port}{path}?token=synthetic-loopback-token");
    // Each action reaches the real relay owner, with no parent/provider launch.
    for (method, action, expected) in [
        ("GET", "relay", 404),
        ("POST", "relay", 404),
        ("POST", "relay-stop", 404),
        ("POST", "relay-resume", 404),
        ("POST", "relay-cleanup", 400),
    ] {
        let reply = client
            .request(
                method.parse().unwrap(),
                endpoint(&format!("/api/sessions/999/{action}")),
            )
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status().as_u16(), expected, "{method} {action}");
        let body: serde_json::Value = reply.json().await.unwrap();
        assert_ne!(body["error"], "migration_unimplemented");
    }
    assert_eq!(
        client
            .get(format!("http://127.0.0.1:{port}/api/sessions/999/relay"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );
    assert_eq!(
        client
            .get(endpoint("/api/sessions/999/relay-stop"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        405
    );
    // Prefix shape/id errors precede Host/method guards for unknown suffixes.
    for (path, expected) in [
        ("/api/sessions/999/unknown", 404),
        ("/api/sessions/no/unknown", 400),
    ] {
        assert_eq!(
            client
                .request(reqwest::Method::TRACE, endpoint(path))
                .header("Host", "wrong.invalid")
                .send()
                .await
                .unwrap()
                .status()
                .as_u16(),
            expected
        );
    }
    assert_eq!(
        client
            .request(
                reqwest::Method::TRACE,
                endpoint("/api/provider-distributions/unknown")
            )
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        404
    );
    assert_eq!(
        client
            .post(endpoint("/api/jev/evaluate"))
            .body("{")
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        503
    );
    assert!(
        client
            .post(endpoint("/api/notify-config"))
            .json(&serde_json::json!({"events":["approval","done"],"backends":[backend.clone()]}))
            .send()
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let denied = client
        .post(endpoint("/api/notify-test"))
        .json(&serde_json::json!({"backend":backend}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status().as_u16(), 502);
    assert!(
        tokio::time::timeout(Duration::from_millis(25), receiver.accept())
            .await
            .is_err()
    );
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(15), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
