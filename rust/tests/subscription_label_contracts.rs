//! Fixed Go d8fbf859: subscription.go resolves display metadata when the wrapper
//! registers or reattaches. These tests use the actual application config and
//! subscription services, with isolated roots and no provider/auth processes.
use many_ai_cli::{
    application::main_program::{HubComposition, MainContext, ServeOptions},
    config::{Config, ConfigStore, Resource, RuntimePaths},
    proto::{self, core::*, time::Timestamp},
};
use serde_json::{Value, json};
use std::sync::Arc;

struct Fixture {
    hub: HubComposition,
    context: MainContext,
    _root: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let listener = loop {
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            if listener.local_addr().unwrap().port() != 47777 {
                break listener;
            }
        };
        let port = listener.local_addr().unwrap().port();
        let runtime = root.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let paths =
            RuntimePaths::trial(&runtime, port, &root.path().join("installed/.many-ai-cli"))
                .unwrap();
        let mut config = Config::from_yaml(
            r#"
token: synthetic
subscriptions:
  claude:
    - {id: ' MAIN ', name: '  Claude Max Main  ', enabled: false}
    - {id: main, name: 'Duplicate must not win'}
    - {id: i, name: 'Unicode normalized'}
    - {id: k, name: 'Kelvin normalized'}
    - {id: blank, name: '   '}
  codex:
    - {id: main, name: 'Codex Team'}
  ' Codex ':
    - {id: main, name: 'Exact configured provider key'}
"#,
            &paths,
        )
        .unwrap();
        config.hub.port = i64::from(port);
        config.hub.idle_timeout_min = 0;
        config.hub.log_dir = runtime.join("logs").to_string_lossy().into_owned();
        config.log.session_enabled = true;
        let context = MainContext {
            config: Arc::new(ConfigStore::new(paths.clone(), config).unwrap()),
            paths,
            cwd: runtime.clone(),
            executable: root.path().join("must-not-be-started"),
            application_home: root.path().join("installed"),
            vendor_home: runtime.clone(),
            environment: vec![
                format!("HOME={}", runtime.display()),
                format!("USERPROFILE={}", runtime.display()),
                "PATH=".into(),
            ],
            shell: String::new(),
            terminal_size: TerminalSize { cols: 80, rows: 24 },
        };
        drop(listener);
        let hub = HubComposition::new(context.clone(), ServeOptions::default())
            .await
            .unwrap();
        let ui = UiBinding {
            connection: UiConnectionId(1),
            auth_epoch: hub.dependencies.core.auth_epoch(),
        };
        hub.dependencies.core.attach_ui(ui, None, None).unwrap();
        assert!(
            hub.dependencies
                .core
                .finish_ui_priming(ui)
                .unwrap()
                .0
                .is_empty()
        );
        Self {
            hub,
            context,
            _root: root,
        }
    }

    fn message(&self, provider: &str, id: &str) -> proto::Message {
        proto::Message {
            provider: provider.into(),
            subscription_id: id.into(),
            // The wrapper's proposed display name is not the source of truth.
            subscription_name: "Untrusted wrapper name".into(),
            cwd: self.context.cwd.to_string_lossy().into_owned(),
            pid: 7,
            cols: 80,
            rows: 24,
            ..Default::default()
        }
    }

    async fn register(&self, provider: &str, id: &str) -> Registration {
        self.hub
            .dependencies
            .core
            .register(
                RegisterRequest {
                    message: self.message(provider, id),
                    spawn_proof: None,
                },
                WrapperConnectionId(1),
                Timestamp::UNIX_EPOCH,
            )
            .await
            .unwrap()
    }

    async fn reattach(&self, session: LiveSessionId, provider: &str, id: &str) -> Reattachment {
        self.hub
            .dependencies
            .core
            .reattach(
                ReattachRequest {
                    message: proto::Message {
                        session_id: session.0,
                        ..self.message(provider, id)
                    },
                    restored_metadata: None,
                },
                WrapperConnectionId(2),
                Timestamp::UNIX_EPOCH,
            )
            .await
            .unwrap()
    }

    fn check(&self, snapshot: &SessionSnapshot, effects: &CoreEffects, id: &str, name: &str) {
        assert_eq!(snapshot.subscription_profile_id, id);
        assert_eq!(snapshot.subscription_profile_name, name);
        let snapshot_wire = serde_json::to_value(snapshot).unwrap();
        assert_optional(&snapshot_wire, "subscription_profile_id", id);
        assert_optional(&snapshot_wire, "subscription_profile_name", name);
        let update = effects
            .0
            .iter()
            .find_map(|effect| match effect {
                CoreEffect::SendUi { message, .. }
                | CoreEffect::SendUiBestEffort { message, .. }
                    if message.r#type == "session_update"
                        && message.session_id == snapshot.id.0 =>
                {
                    Some(message)
                }
                _ => None,
            })
            .expect("the existing UI must receive a session_update");
        let update_wire = serde_json::to_value(update).unwrap();
        assert_optional(&update_wire, "subscription_id", id);
        assert_optional(&update_wire, "subscription_name", name);
        // This row exists synchronously, before the optional history effects.
        let row = self
            .hub
            .dependencies
            .journal
            .storage()
            .unwrap()
            .session_overview_by_live_session(snapshot.id)
            .unwrap();
        assert_eq!(row.subscription_id, id);
        assert_eq!(
            self.hub
                .dependencies
                .core
                .snapshot(snapshot.id)
                .unwrap()
                .subscription_profile_name,
            name
        );
        assert!(
            !self
                .context
                .paths
                .resource(Resource::Subscriptions)
                .exists()
        );
    }
}

fn assert_optional(wire: &Value, field: &str, expected: &str) {
    if expected.is_empty() {
        assert!(
            wire.get(field).is_none(),
            "empty {field} must be omitted: {wire}"
        );
    } else {
        assert_eq!(wire[field], expected, "{field}");
    }
}

#[tokio::test]
async fn configured_subscription_name_reaches_registration_snapshot_update_and_normalized_history()
{
    let f = Fixture::new().await;
    let registered = f.register("claude", "\u{a0} MAIN \u{85}").await;
    f.check(
        &registered.snapshot,
        &registered.after_registered,
        "main",
        "Claude Max Main",
    );
    let event = registered
        .after_registered
        .0
        .iter()
        .find_map(|effect| {
            let persistence = match effect {
                CoreEffect::Persist(effect) | CoreEffect::PersistBound { effect, .. } => effect,
                _ => return None,
            };
            match persistence {
                PersistenceEffect::Event { event, .. }
                    if event.0.get("type") == Some(&json!("session_start")) =>
                {
                    Some(event)
                }
                _ => None,
            }
        })
        .expect("registration history must record the resolved ID");
    assert_eq!(event.0["subscription_profile_id"], "main");
    assert!(!event.0.contains_key("subscription_profile_name"));
    assert!(!event.0.contains_key("subscription_name"));
}

#[tokio::test]
async fn subscription_rename_updates_new_registration_and_warm_cold_reattach_only() {
    let f = Fixture::new().await;
    let original = f.register("claude", "main").await;
    let binding = original.binding;
    drop(original.after_registered);
    f.hub
        .dependencies
        .subscriptions
        .update("claude", "MAIN", Some("  Renamed Plan  "), None)
        .unwrap_or_else(|error| panic!("subscription rename failed: {}", error.code));
    assert_eq!(
        f.hub
            .dependencies
            .core
            .snapshot(binding.session)
            .unwrap()
            .subscription_profile_name,
        "Claude Max Main"
    );
    let fresh = f.register("claude", "main").await;
    f.check(
        &fresh.snapshot,
        &fresh.after_registered,
        "main",
        "Renamed Plan",
    );
    drop(fresh.after_registered);
    let warm = f.reattach(binding.session, "claude", " MAIN ").await;
    assert_eq!(warm.binding.session, binding.session);
    assert_eq!(warm.binding.incarnation, binding.incarnation);
    f.check(
        &warm.snapshot,
        &warm.after_reattached,
        "main",
        "Renamed Plan",
    );
    drop(warm.after_reattached);
    let cold = f.reattach(LiveSessionId(70), "claude", " MAIN ").await;
    assert_eq!(cold.binding.session, LiveSessionId(70));
    f.check(
        &cold.snapshot,
        &cold.after_reattached,
        "main",
        "Renamed Plan",
    );
}

#[tokio::test]
async fn subscription_removal_keeps_live_label_until_reattach_then_preserves_only_id() {
    let f = Fixture::new().await;
    let original = f.register("codex", "main").await;
    let session = original.binding.session;
    drop(original.after_registered);
    let removed = f
        .hub
        .dependencies
        .subscriptions
        .remove("codex", "main", false)
        .unwrap_or_else(|error| panic!("subscription removal failed: {}", error.code));
    assert_eq!(removed["credentials_deleted"], false);
    assert_eq!(
        f.hub
            .dependencies
            .core
            .snapshot(session)
            .unwrap()
            .subscription_profile_name,
        "Codex Team"
    );
    let warm = f.reattach(session, "codex", " MAIN ").await;
    f.check(&warm.snapshot, &warm.after_reattached, "main", "");
    drop(warm.after_reattached);
    let cold = f.reattach(LiveSessionId(70), "codex", "main").await;
    f.check(&cold.snapshot, &cold.after_reattached, "main", "");
}

#[tokio::test]
async fn subscription_lookup_matches_go_normalization_validation_and_exact_provider_scope() {
    let f = Fixture::new().await;
    // Fixed Go Find uses the exact provider map key, normalizes profile IDs,
    // and returns the first profile even if disabled. Missing names stay empty.
    let long_id = "a".repeat(65);
    for (provider, raw, id, name) in [
        ("claude", " MAIN ", "main", "Claude Max Main"),
        ("codex", "MAIN", "main", "Codex Team"),
        (" Codex ", "main", "main", "Exact configured provider key"),
        ("Codex", "main", "main", ""),
        (" codex ", "main", "main", ""),
        ("unknown", "main", "main", ""),
        ("claude", "gone", "gone", ""),
        ("claude", "blank", "blank", ""),
        ("claude", " İ ", "i", "Unicode normalized"),
        ("claude", "K", "k", "Kelvin normalized"),
        ("claude", "", "", ""),
        ("claude", "\u{a0}\u{85}", "", ""),
        ("claude", "../escape", "", ""),
        ("claude", "a..b", "", ""),
        ("claude", " AUTO ", "", ""),
        ("claude", "_bad", "", ""),
        ("claude", "a/b", "", ""),
        ("claude", long_id.as_str(), "", ""),
    ] {
        let registered = f.register(provider, raw).await;
        f.check(&registered.snapshot, &registered.after_registered, id, name);
        let session = registered.binding.session;
        drop(registered.after_registered);
        let warm = f.reattach(session, provider, raw).await;
        f.check(&warm.snapshot, &warm.after_reattached, id, name);
        drop(warm.after_reattached);
        // Warm state must not restore the old profile when the newly declared
        // identity is invalid, including a previously configured display name.
        let invalid = f.reattach(session, provider, "../escape").await;
        f.check(&invalid.snapshot, &invalid.after_reattached, "", "");
    }
}
