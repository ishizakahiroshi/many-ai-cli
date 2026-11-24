use super::*;
use crate::config::{Config, ConfigStore};
use std::sync::Arc;

#[test]
fn history_diff_reads_verified_current_and_selected_records_without_mutation() {
    let (_temp, paths, store) = setup();
    let baseline = store.baseline("claude").unwrap();
    let first = store
        .history
        .save_override(
            "claude",
            Definition {
                id: "claude".into(),
                display_name: "First".into(),
                ..Default::default()
            },
            &baseline,
            "",
            "edit",
        )
        .unwrap();
    let second = store
        .history
        .save_override(
            "claude",
            Definition {
                id: "claude".into(),
                display_name: "Second".into(),
                ..Default::default()
            },
            &baseline,
            &first.revision,
            "edit",
        )
        .unwrap();
    let endpoint = format!("/api/providers/claude/history/{}/diff", first.revision);
    assert_eq!(
        response(&request("POST", &endpoint, json!(null)), &store, &paths).status,
        405
    );
    let diff = response(&request("GET", &endpoint, json!(null)), &store, &paths);
    assert_eq!(diff.status, 200);
    let body = value(&diff);
    assert_eq!(body["revision"], first.revision);
    assert_eq!(body["current_revision"], second.revision);
    assert_eq!(body["diff"][0]["status"], "changed");
    let change = body["diff"][0]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|change| change["field"] == "display_name")
        .unwrap();
    assert_eq!(change["current"], "Second");
    assert_eq!(change["candidate"], "First");
    assert_eq!(change["conflict"], false);
    assert_eq!(
        store.history.current("claude").unwrap().revision,
        second.revision
    );
    assert_eq!(
        response(
            &request(
                "GET",
                "/api/providers/claude/history/missing/diff",
                json!(null)
            ),
            &store,
            &paths
        )
        .status,
        404
    );
}

#[test]
fn backup_routes_preserve_nullable_listing_verify_method_and_restore_cas() {
    let (_temp, paths, store) = setup();
    let listing = request("GET", "/api/providers/claude/backups", json!(null));
    assert_eq!(
        value(&response(&listing, &store, &paths))["backups"],
        json!(null)
    );
    Dir::open_or_create_private(&paths.resource(Resource::ProviderBackups))
        .unwrap()
        .child_dir("claude", true)
        .unwrap();
    assert_eq!(
        value(&response(&listing, &store, &paths))["backups"],
        json!([])
    );
    let first = store.history.reset("claude", "").unwrap();
    let second = store.history.reset("claude", &first.revision).unwrap();
    store.reload().unwrap();
    let verify = format!("/api/providers/claude/backups/{}/verify", first.revision);
    assert_eq!(
        response(&request("POST", &verify, json!(null)), &store, &paths).status,
        405
    );
    let verified = response(&request("GET", &verify, json!(null)), &store, &paths);
    assert_eq!(verified.status, 200);
    assert_eq!(value(&verified)["backup"]["revision"], first.revision);
    let restore = format!("/api/providers/claude/backups/{}/restore", first.revision);
    assert_eq!(
        response(&request("POST", &restore, json!({})), &store, &paths).status,
        400
    );
    assert_eq!(
        response(
            &request(
                "POST",
                &restore,
                json!({"expected_revision":first.revision})
            ),
            &store,
            &paths
        )
        .status,
        409
    );
    assert_eq!(
        store.history.current("claude").unwrap().revision,
        second.revision
    );
    let restored = response(
        &request(
            "POST",
            &restore,
            json!({"expected_revision":second.revision}),
        ),
        &store,
        &paths,
    );
    assert_eq!(restored.status, 200);
    let current = store.history.current("claude").unwrap();
    assert_eq!(current.reason, "restore");
    assert_eq!(current.parent_revision, second.revision);
    assert!(current.payload == first.payload);
    assert_eq!(
        value(&restored)["revision"],
        store.snapshot().unwrap().registry.revision()
    );
}

#[test]
fn recovery_routes_classify_actual_head_and_publish_verified_user_revision() {
    let (_temp, paths, store) = setup();
    let status = request("GET", "/api/providers/claude/recovery", json!(null));
    let initial = response(&status, &store, &paths);
    assert_eq!(value(&initial)["state"], "ok");
    assert_eq!(value(&initial)["candidates"], json!([]));
    assert_eq!(initial.headers["Cache-Control"], "no-store");
    let verified = store.history.reset("claude", "").unwrap();
    let directory = Dir::open(&paths.resource(Resource::ProviderOverrides))
        .unwrap()
        .child_dir("claude", false)
        .unwrap();
    directory
        .replace("HEAD", b"missing-revision\n", 0o600)
        .unwrap();
    let broken = response(&status, &store, &paths);
    assert_eq!(value(&broken)["state"], "recovery_required");
    assert_eq!(
        value(&broken)["candidates"][0]["revision"],
        verified.revision
    );
    assert_eq!(value(&broken)["candidates"][1], json!({"kind":"base"}));
    let failed = response(
        &request(
            "POST",
            "/api/providers/claude/recovery",
            json!({"revision":"missing"}),
        ),
        &store,
        &paths,
    );
    assert_eq!(failed.status, 422);
    assert_eq!(directory.read("HEAD", 128).unwrap(), b"missing-revision\n");
    let recovered = response(
        &request(
            "POST",
            "/api/providers/claude/recovery",
            json!({"revision":verified.revision}),
        ),
        &store,
        &paths,
    );
    assert_eq!(recovered.status, 200);
    let current = store.history.current("claude").unwrap();
    assert_eq!(value(&recovered)["revision"], current.revision);
    assert!(current.payload == verified.payload);
    assert!(current.parent_revision.is_empty());
    assert_eq!(value(&response(&status, &store, &paths))["state"], "ok");
    assert_eq!(
        response(
            &request("POST", "/api/providers/claude/recovery", json!({})),
            &store,
            &paths
        )
        .status,
        422
    );
}
fn setup() -> (tempfile::TempDir, RuntimePaths, ProviderRegistryStore) {
    let temp = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(
        temp.path(),
        49819,
        &temp.path().join("../installed-provider-route-fixture"),
    )
    .unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap());
    let store = ProviderRegistryStore::new(&paths, config).unwrap();
    (temp, paths, store)
}
fn definition(id: &str) -> Definition {
    Definition {
        schema_version: 1,
        id: id.into(),
        display_name: "Fixture".into(),
        launch: Some(LaunchDefinition {
            executable: "fixture-agent".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn request(method: &str, path: &str, body: serde_json::Value) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        body: serde_json::to_vec(&body).unwrap(),
        ..Default::default()
    }
}
/// What the router hands over for a request the signed-in dashboard sent.
fn dashboard() -> VerifiedUiOrigin {
    VerifiedUiOrigin::after_server_verification(crate::proto::core::AuthEpoch(1))
}
/// A request from the dashboard (the screen's own calls).
fn response(request: &Request, store: &ProviderRegistryStore, paths: &RuntimePaths) -> Response {
    handle(
        request,
        store,
        paths,
        Some(&dashboard()),
        &|_| true,
        &|_, _, _| {},
    )
    .unwrap()
}
/// A request that holds only the Hub token (a session, `orchestrate`, the CLI).
fn token_only(request: &Request, store: &ProviderRegistryStore, paths: &RuntimePaths) -> Response {
    handle(request, store, paths, None, &|_| true, &|_, _, _| {}).unwrap()
}
fn value(response: &Response) -> serde_json::Value {
    serde_json::from_slice(&response.body).unwrap()
}
#[test]
fn custom_crud_registry_revision_backup_and_icon_cleanup() {
    let (_temp, paths, store) = setup();
    let before = store.snapshot().unwrap();
    let created = response(
        &request(
            "POST",
            "/api/providers",
            serde_json::to_value(definition("custom-cli")).unwrap(),
        ),
        &store,
        &paths,
    );
    assert_eq!(created.status, 201);
    assert_eq!(value(&created)["diagnostics"], serde_json::Value::Null);
    assert!(before.registry.lookup("custom-cli").is_none());
    assert_eq!(
        response(
            &request(
                "POST",
                "/api/providers",
                serde_json::to_value(definition("custom-cli")).unwrap()
            ),
            &store,
            &paths
        )
        .status,
        409
    );
    let mut desired = definition("custom-cli");
    desired.display_name = "Changed".into();
    let stale = response(
        &request(
            "PATCH",
            "/api/providers/custom-cli",
            json!({"expected_revision":"stale","definition":desired}),
        ),
        &store,
        &paths,
    );
    assert_eq!(stale.status, 409);
    let changed = response(
        &request(
            "PATCH",
            "/api/providers/custom-cli",
            json!({"expected_revision":store.snapshot().unwrap().registry.revision(),"definition":desired}),
        ),
        &store,
        &paths,
    );
    assert_eq!(changed.status, 200);
    let detail = response(
        &request("GET", "/api/providers/custom-cli", json!(null)),
        &store,
        &paths,
    );
    assert_eq!(detail.status, 200);
    assert_eq!(value(&detail)["provider"]["display_name"], "Changed");
    assert_eq!(detail.headers["Cache-Control"], "no-store");
    let icons = Dir::open_or_create_private(&paths.resource(Resource::ProviderIcons)).unwrap();
    icons
        .create_new("custom-cli.bin", b"fixture", 0o600)
        .unwrap();
    let list = response(
        &request("GET", "/api/providers", json!(null)),
        &store,
        &paths,
    );
    let json = value(&list);
    assert!(
        !json["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "custom-cli")
            .unwrap()["icon_image_version"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    let mut delete = request("DELETE", "/api/providers/custom-cli", json!(null));
    delete.query = format!(
        "expected_revision={}",
        store.snapshot().unwrap().registry.revision()
    );
    assert_eq!(response(&delete, &store, &paths).status, 200);
    assert!(
        store
            .snapshot()
            .unwrap()
            .registry
            .lookup("custom-cli")
            .is_none()
    );
    assert!(
        !paths
            .resource(Resource::ProviderIcons)
            .join("custom-cli.bin")
            .exists()
    );
    let backups = Dir::open(&paths.resource(Resource::ProviderBackups))
        .unwrap()
        .child_dir("custom-cli", false)
        .unwrap();
    assert!(backups.entries().unwrap().len() >= 3);
}
#[test]
fn builtin_first_edit_empty_revision_disable_and_stale_conflict() {
    let (_temp, paths, store) = setup();
    let mut desired = store
        .snapshot()
        .unwrap()
        .registry
        .lookup("claude")
        .unwrap()
        .definition;
    desired.display_name = "Edited Claude".into();
    assert_eq!(
        response(
            &request(
                "PATCH",
                "/api/providers/claude",
                json!({"definition":desired})
            ),
            &store,
            &paths
        )
        .status,
        400
    );
    assert_eq!(
        response(
            &request(
                "PATCH",
                "/api/providers/claude",
                json!({"expected_revision":null,"definition":desired})
            ),
            &store,
            &paths
        )
        .status,
        400
    );
    let edit = response(
        &request(
            "PATCH",
            "/api/providers/claude",
            json!({"expected_revision":"","definition":desired}),
        ),
        &store,
        &paths,
    );
    assert_eq!(edit.status, 200);
    let history = store.history.current("claude").unwrap();
    assert!(history.payload.launch.is_none());
    let mut delete = request("DELETE", "/api/providers/claude", json!(null));
    assert_eq!(response(&delete, &store, &paths).status, 400);
    delete.query = "expected_revision=".into();
    assert_eq!(response(&delete, &store, &paths).status, 409);
    delete.query = format!("expected_revision={}", history.revision);
    let disabled = response(&delete, &store, &paths);
    assert_eq!(disabled.status, 200);
    assert_eq!(value(&disabled)["disabled"], true);
    let current = store
        .snapshot()
        .unwrap()
        .registry
        .lookup("claude")
        .unwrap()
        .definition;
    assert_eq!(current.enabled, Some(false));
    assert_eq!(current.display_name, "Edited Claude");
}
#[test]
fn source_backup_failure_rolls_back_create_before_reload() {
    let (_temp, paths, store) = setup();
    let before = store.snapshot().unwrap().registry.revision().to_string();
    std::fs::create_dir_all(paths.resource(Resource::ProviderBackups).parent().unwrap()).unwrap();
    std::fs::write(
        paths.resource(Resource::ProviderBackups),
        b"ordinary failed backup fixture",
    )
    .unwrap();
    let result = response(
        &request(
            "POST",
            "/api/providers",
            serde_json::to_value(definition("new-cli")).unwrap(),
        ),
        &store,
        &paths,
    );
    assert_eq!(result.status, 500);
    assert_eq!(value(&result)["error"], "provider_backup_failed");
    assert_eq!(
        value(&result)["detail"],
        "could not back up the provider before changing it"
    );
    assert!(
        !paths
            .resource(Resource::ProviderDefinitions)
            .join("new-cli.json")
            .exists()
    );
    assert_eq!(store.snapshot().unwrap().registry.revision(), before);
}
#[test]
fn an_approval_source_the_hub_would_never_read_is_refused_by_validate_and_create() {
    // A CLI that runs under another HOME writes its phrase file under its own
    // root; the Hub can read only sources under its own root or on raw.githubusercontent.com.
    let (temp, paths, store) = setup();
    let elsewhere = temp.path().join("..").join("other-home").join("x.md");
    let inside = paths.root().join("approval-patterns").join("ok-cli.md");
    let with_source = |source: &std::path::Path| {
        let mut def = definition("ok-cli");
        def.approval_pattern_source = source.to_string_lossy().into_owned();
        def
    };
    let outside = response(
        &request(
            "POST",
            "/api/providers/validate",
            serde_json::to_value(with_source(&elsewhere)).unwrap(),
        ),
        &store,
        &paths,
    );
    assert_eq!(outside.status, 200);
    assert_eq!(value(&outside)["valid"], false);
    assert_eq!(
        value(&outside)["diagnostics"][0]["field"],
        "approval_pattern_source"
    );
    assert!(
        value(&outside)["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("under")
    );
    for ok in [serde_json::to_value(with_source(&inside)).unwrap(), {
        let mut def = definition("ok-cli");
        def.approval_pattern_source =
            "https://raw.githubusercontent.com/synthetic/phrases.md".into();
        serde_json::to_value(def).unwrap()
    }] {
        let valid = response(
            &request("POST", "/api/providers/validate", ok),
            &store,
            &paths,
        );
        assert_eq!(value(&valid)["valid"], true, "{}", value(&valid));
    }
    let created = response(
        &request(
            "POST",
            "/api/providers",
            serde_json::to_value(with_source(&elsewhere)).unwrap(),
        ),
        &store,
        &paths,
    );
    assert_eq!(created.status, 422);
    assert!(
        store
            .snapshot()
            .unwrap()
            .registry
            .lookup("ok-cli")
            .is_none()
    );
}
#[test]
fn schema_validation_and_clear_failure_are_real_errors() {
    let (_temp, paths, store) = setup();
    let invalid = response(
        &request(
            "POST",
            "/api/providers/validate",
            json!({"id":"missing-fields"}),
        ),
        &store,
        &paths,
    );
    assert_eq!(invalid.status, 200);
    assert_eq!(value(&invalid)["valid"], false);
    let valid = response(
        &request(
            "POST",
            "/api/providers/validate",
            serde_json::to_value(definition("valid-cli")).unwrap(),
        ),
        &store,
        &paths,
    );
    assert_eq!(valid.status, 200);
    assert_eq!(value(&valid)["valid"], true);
    assert_eq!(value(&valid)["diagnostics"], serde_json::Value::Null);
    let mut desired = store
        .snapshot()
        .unwrap()
        .registry
        .lookup("claude")
        .unwrap()
        .definition;
    desired.launch.as_mut().unwrap().model_args.clear();
    let result = response(
        &request(
            "PATCH",
            "/api/providers/claude",
            json!({"expected_revision":"","definition":desired}),
        ),
        &store,
        &paths,
    );
    assert_eq!(result.status, 422);
    assert_eq!(value(&result)["error"], "provider_override_clears_value");
    assert!(
        value(&result)["fields"]
            .as_array()
            .unwrap()
            .contains(&json!("launch.model_args"))
    );
    assert!(store.history.current("claude").is_err());
}
#[test]
fn go_wire_patch_folds_merges_duplicate_and_null_fields() {
    let (_temp, paths, store) = setup();
    let mut req = request("PATCH", "/api/providers/wire-cli", json!(null));
    let revision = store.snapshot().unwrap().registry.revision().to_string();
    req.body=format!(r#"{{"EXPECTED_REVISION":"{revision}","expected_revision":null,"DEFINITION":{{"schema_version":1,"id":"wire-cli","display_name":"Wire"}},"definition":{{"launch":{{"executable":"fixture-agent"}}}},"definition":null}}"#).into_bytes();
    // Go decoding null into a nonnil *string leaves it nil, making the
    // explicit expected_revision requirement fail before a mutation.
    assert_eq!(response(&req, &store, &paths).status, 400);
    req.body=format!(r#"{{"EXPECTED_REVISION":"{revision}","DEFINITION":{{"schema_version":1,"id":"wire-cli","display_name":"Wire"}},"definition":{{"launch":{{"executable":"fixture-agent"}}}},"definition":null}}"#).into_bytes();
    let result = response(&req, &store, &paths);
    assert_eq!(result.status, 200);
    assert_eq!(
        store
            .snapshot()
            .unwrap()
            .registry
            .lookup("wire-cli")
            .unwrap()
            .definition
            .display_name,
        "Wire"
    );
}
#[test]
fn command_missing_read_diagnostic_and_real_reset_route_use_history() {
    let (_temp, paths, store) = setup();
    let req = request("GET", "/api/providers", json!(null));
    let result = handle(&req, &store, &paths, None, &|_| false, &|_, _, _| {}).unwrap();
    assert!(
        value(&result)["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "command_missing")
    );
    assert_eq!(
        response(
            &request("POST", "/api/providers/claude/reset", json!({})),
            &store,
            &paths
        )
        .status,
        400
    );
    let reset = response(
        &request(
            "POST",
            "/api/providers/claude/reset",
            json!({"expected_revision":""}),
        ),
        &store,
        &paths,
    );
    assert_eq!(reset.status, 200);
    assert_eq!(store.history.current("claude").unwrap().reason, "reset");
    let history = response(
        &request("GET", "/api/providers/claude/history", json!(null)),
        &store,
        &paths,
    );
    assert_eq!(history.status, 200);
    let history = value(&history);
    assert_eq!(history["revisions"].as_array().unwrap().len(), 1);
    assert_eq!(history["revisions"][0]["reason"], "reset");
}

#[test]
fn custom_invalid_edit_is_backed_up_before_validation_without_changing_definition() {
    let (_temp, paths, store) = setup();
    store
        .definitions
        .create_new(&definition("existing-cli"))
        .unwrap();
    store.reload().unwrap();
    let before = store.snapshot().unwrap().registry.revision().to_string();
    let result = response(
        &request(
            "PATCH",
            "/api/providers/existing-cli",
            json!({"expected_revision":before,"definition":{"id":"existing-cli"}}),
        ),
        &store,
        &paths,
    );
    assert_eq!(result.status, 422);
    assert_eq!(value(&result)["error"], "provider_save_failed");
    assert_eq!(store.snapshot().unwrap().registry.revision(), before);
    assert_eq!(
        store.definitions.load().unwrap().definitions[0].display_name,
        "Fixture"
    );
    let backups = Dir::open(&paths.resource(Resource::ProviderBackups))
        .unwrap()
        .child_dir("existing-cli", false)
        .unwrap();
    assert_eq!(backups.entries().unwrap().len(), 1);
}

#[test]
fn filesystem_warning_is_private_and_runs_after_mutation_lock_release() {
    let (_temp, paths, store) = setup();
    std::fs::create_dir_all(paths.resource(Resource::ProviderBackups).parent().unwrap()).unwrap();
    std::fs::write(
        paths.resource(Resource::ProviderBackups),
        b"ordinary failed backup fixture",
    )
    .unwrap();
    let collected = std::sync::Mutex::new(vec![]);
    let warning = |operation: &'static str, id: &str, error: &StoreError| {
        assert!(store.mutation_gate.try_lock().is_ok());
        assert!(store.definitions.load().is_ok());
        collected
            .lock()
            .unwrap()
            .push((operation, id.to_owned(), error.to_string()));
    };
    let result = handle(
        &request(
            "POST",
            "/api/providers",
            serde_json::to_value(definition("warning-cli")).unwrap(),
        ),
        &store,
        &paths,
        None,
        &|_| true,
        &warning,
    )
    .unwrap();
    assert_eq!(result.status, 500);
    assert_eq!(collected.lock().unwrap().len(), 1);
    assert_eq!(collected.lock().unwrap()[0].0, "provider_backup_failed");
    assert_eq!(collected.lock().unwrap()[0].1, "warning-cli");
    assert_eq!(
        value(&result)["detail"],
        "could not back up the provider before changing it"
    );
}

#[test]
fn empty_history_provider_uses_source_invalid_id_response() {
    let (_temp, paths, store) = setup();
    let result = response(
        &request("GET", "/api/providers//history", json!(null)),
        &store,
        &paths,
    );
    assert_eq!(result.status, 404);
    assert_eq!(value(&result)["error"], "history_not_found");
    assert_eq!(value(&result)["detail"], "id is required");
}

fn change(
    request: &Request,
    store: &ProviderRegistryStore,
    paths: &RuntimePaths,
) -> Option<(String, &'static str)> {
    let response = response(request, store, paths);
    change_of(request, &response).map(|c| (c.id, c.action))
}

#[test]
fn only_successful_registry_mutations_produce_a_change_notice() {
    let (_temp, paths, store) = setup();
    let create = request(
        "POST",
        "/api/providers",
        serde_json::to_value(definition("notice-cli")).unwrap(),
    );
    assert_eq!(
        change(&create, &store, &paths),
        Some(("notice-cli".into(), "created"))
    );
    // Duplicate id (409), reserved id, bad json, validate-only and reads: none.
    assert_eq!(change(&create, &store, &paths), None);
    let reserved = request(
        "POST",
        "/api/providers",
        serde_json::to_value(definition("claude")).unwrap(),
    );
    assert_eq!(change(&reserved, &store, &paths), None);
    let validate = request(
        "POST",
        "/api/providers/validate",
        serde_json::to_value(definition("notice-cli")).unwrap(),
    );
    assert_eq!(response(&validate, &store, &paths).status, 200);
    assert_eq!(change(&validate, &store, &paths), None);
    assert_eq!(
        change(
            &request("GET", "/api/providers", json!(null)),
            &store,
            &paths
        ),
        None
    );
    let stale = request(
        "PATCH",
        "/api/providers/notice-cli",
        json!({"expected_revision":"stale","definition":definition("notice-cli")}),
    );
    assert_eq!(change(&stale, &store, &paths), None);
    let mut renamed = definition("notice-cli");
    renamed.display_name = "Renamed".into();
    let patch = request(
        "PATCH",
        "/api/providers/notice-cli",
        json!({"expected_revision":store.snapshot().unwrap().registry.revision(),"definition":renamed}),
    );
    assert_eq!(
        change(&patch, &store, &paths),
        Some(("notice-cli".into(), "updated"))
    );
    let mut delete = request("DELETE", "/api/providers/notice-cli", json!(null));
    delete.query = "expected_revision=stale".into();
    assert_eq!(change(&delete, &store, &paths), None);
    delete.query = format!(
        "expected_revision={}",
        store.snapshot().unwrap().registry.revision()
    );
    assert_eq!(
        change(&delete, &store, &paths),
        Some(("notice-cli".into(), "removed"))
    );
    // A built-in is disabled, not removed (its revision is the history head).
    let mut edited = store
        .snapshot()
        .unwrap()
        .registry
        .lookup("codex")
        .unwrap()
        .definition;
    edited.display_name = "Edited Codex".into();
    let edit = request(
        "PATCH",
        "/api/providers/codex",
        json!({"expected_revision":"","definition":edited}),
    );
    assert_eq!(
        change(&edit, &store, &paths),
        Some(("codex".into(), "updated"))
    );
    let mut disable = request("DELETE", "/api/providers/codex", json!(null));
    disable.query = format!(
        "expected_revision={}",
        store.history.current("codex").unwrap().revision
    );
    assert_eq!(
        change(&disable, &store, &paths),
        Some(("codex".into(), "disabled"))
    );
}

#[test]
fn the_declarations_route_gives_the_screen_every_providers_declared_values() {
    let (_temp, paths, store) = setup();
    assert_eq!(
        response(
            &request("POST", "/api/providers/declarations", json!(null)),
            &store,
            &paths
        )
        .status,
        405
    );
    let reply = response(
        &request("GET", "/api/providers/declarations", json!(null)),
        &store,
        &paths,
    );
    assert_eq!(reply.status, 200);
    let body = value(&reply);
    let list = body["declarations"].as_array().unwrap();
    assert_eq!(list.len(), 7);
    let claude = list.iter().find(|d| d["id"] == "claude").unwrap();
    assert_eq!(claude["connection"], "pty");
    assert_eq!(claude["builtin"], true);
    assert_eq!(claude["presentation"]["letter"], "C");
    assert_eq!(claude["presentation"]["order"], 1);
    assert_eq!(claude["presentation"]["label"], "Claude");
    assert_eq!(claude["presentation"]["long_label"], "Claude Code");
    assert_eq!(
        claude["links"]["usage_url"],
        "https://claude.ai/settings/usage"
    );
    assert!(claude["permissions"]["modes"].get("bounded").is_some());
    let codex = list.iter().find(|d| d["id"] == "codex").unwrap();
    assert_eq!(codex["input"]["slow_paste"], true);
    // The list of what each provider can and cannot do is carried too.
    for entry in list {
        let lines = entry["capability_report"].as_array().unwrap();
        let state = |key: &str| lines.iter().find(|l| l["key"] == key).unwrap()["state"].clone();
        assert_eq!(state("launch"), "available", "{}", entry["id"]);
        assert!(
            ["available", "unavailable", "unsupported"]
                .contains(&state("usage_meter").as_str().unwrap())
        );
    }
    // No launch command leaves the Hub through this route.
    for entry in list {
        assert!(entry.get("launch").is_none(), "{}", entry["id"]);
    }
}

#[test]
fn a_token_only_caller_cannot_change_a_builtin_definition() {
    let (_temp, paths, store) = setup();
    let shipped = store
        .snapshot()
        .unwrap()
        .registry
        .lookup("claude")
        .unwrap()
        .definition;
    let mut swapped = shipped.clone();
    swapped.launch.as_mut().unwrap().executable = "synthetic-other-executable".into();
    let mut delete = request("DELETE", "/api/providers/claude", json!(null));
    delete.query = "expected_revision=".into();
    for refused in [
        request(
            "PATCH",
            "/api/providers/claude",
            json!({"expected_revision":"","definition":swapped}),
        ),
        delete,
        request(
            "POST",
            "/api/providers/claude/reset",
            json!({"expected_revision":""}),
        ),
        request(
            "POST",
            "/api/providers/claude/restore",
            json!({"revision":"synthetic","expected_revision":"synthetic"}),
        ),
        request(
            "POST",
            "/api/providers/claude/backups/synthetic/restore",
            json!({"expected_revision":""}),
        ),
        request(
            "POST",
            "/api/providers/claude/recovery",
            json!({"revision":""}),
        ),
    ] {
        let reply = token_only(&refused, &store, &paths);
        assert_eq!(reply.status, 403, "{} {}", refused.method, refused.path);
        assert_eq!(value(&reply)["error"], "ui_required");
        assert_eq!(change_of(&refused, &reply), None);
    }
    assert!(store.history.current("claude").is_err());
    let now = store
        .snapshot()
        .unwrap()
        .registry
        .lookup("claude")
        .unwrap()
        .definition;
    assert!(now.launch == shipped.launch);
    assert_eq!(now.enabled, shipped.enabled);
    // Reads of a built-in still need only the token.
    for read in [
        "/api/providers/claude",
        "/api/providers/claude/history",
        "/api/providers/claude/recovery",
        "/api/providers/claude/backups",
    ] {
        assert_ne!(
            token_only(&request("GET", read, json!(null)), &store, &paths).status,
            403,
            "{read}"
        );
    }
    // The same edit from the dashboard is saved.
    let mut renamed = shipped;
    renamed.display_name = "Edited from the dashboard".into();
    let saved = response(
        &request(
            "PATCH",
            "/api/providers/claude",
            json!({"expected_revision":"","definition":renamed}),
        ),
        &store,
        &paths,
    );
    assert_eq!(saved.status, 200);
}

#[test]
fn a_registered_provider_keeps_its_token_only_routes() {
    let (_temp, paths, store) = setup();
    let created = token_only(
        &request(
            "POST",
            "/api/providers",
            serde_json::to_value(definition("token-cli")).unwrap(),
        ),
        &store,
        &paths,
    );
    assert_eq!(created.status, 201);
    let mut desired = definition("token-cli");
    desired.display_name = "Changed by token".into();
    let changed = token_only(
        &request(
            "PATCH",
            "/api/providers/token-cli",
            json!({"expected_revision":store.snapshot().unwrap().registry.revision(),"definition":desired}),
        ),
        &store,
        &paths,
    );
    assert_eq!(changed.status, 200);
    let mut delete = request("DELETE", "/api/providers/token-cli", json!(null));
    delete.query = format!(
        "expected_revision={}",
        store.snapshot().unwrap().registry.revision()
    );
    assert_eq!(token_only(&delete, &store, &paths).status, 200);
}

#[test]
fn a_provider_id_cannot_shadow_the_declarations_route() {
    assert!(crate::profile::store::validate_user_id("declarations").is_err());
    assert!(crate::profile::store::validate_user_id("my-agent").is_ok());
}
