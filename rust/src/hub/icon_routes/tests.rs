use super::*;
use crate::{
    config::{Config, ConfigStore, Resource, RuntimePaths},
    profile::store::ProviderRegistryStore,
};
fn fixture() -> (
    tempfile::TempDir,
    RuntimePaths,
    Arc<ProviderAssets>,
    Arc<IconHttp>,
) {
    let t = tempfile::tempdir().unwrap();
    let run = t.path().join("runtime");
    let old = t.path().join("installed");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::create_dir_all(&old).unwrap();
    let p = RuntimePaths::trial(&run, 49284, &old).unwrap();
    let cfg = Arc::new(ConfigStore::new(p.clone(), Config::default()).unwrap());
    let registry = Arc::new(ProviderRegistryStore::new(&p, cfg).unwrap());
    let owner = ProviderAssets::new(&p, registry, Arc::new(|_, _| {}));
    let http = IconHttp::new(owner.clone());
    (t, p, owner, http)
}
fn request(method: &str, id: &str, body: Vec<u8>) -> Request {
    Request {
        method: method.into(),
        path: format!("/api/provider-icons/{id}"),
        body,
        headers: vec![("Content-Type".into(), "image/png".into())],
        ..Default::default()
    }
}
#[test]
fn sniff_ignores_declared_type_bounds_bytes_and_restricts_ids() {
    let (_t, p, _owner, http) = fixture();
    assert_eq!(
        http.handle_authenticated(&request("PUT", "codex", b"<svg/>".to_vec()))
            .unwrap()
            .status,
        415
    );
    assert_eq!(
        http.handle_authenticated(&request("PUT", "codex", vec![0; ICON_LIMIT + 1]))
            .unwrap()
            .status,
        413
    );
    for id in ["", "../codex", "codex.x", "a/b"] {
        assert_eq!(
            http.handle_authenticated(&request("GET", id, vec![]))
                .unwrap()
                .status,
            404
        )
    }
    assert!(!p.resource(Resource::ProviderIcons).exists());
    assert_eq!(
        http.handle_authenticated(&request("PUT", "absent", b"GIF89a".to_vec()))
            .unwrap()
            .status,
        404
    );
}
#[test]
fn actual_write_read_version_summary_and_delete_share_private_storage() {
    let (_t, p, owner, http) = fixture();
    let bytes = b"\x89PNG\r\n\x1a\nfixture".to_vec();
    assert_eq!(
        http.handle_authenticated(&request("PUT", "codex", bytes.clone()))
            .unwrap()
            .status,
        200
    );
    let response = http
        .handle_authenticated(&request("GET", "codex", vec![]))
        .unwrap();
    assert_eq!(response.body, bytes);
    assert_eq!(response.headers["Content-Type"], "image/png");
    assert_eq!(response.headers["Cache-Control"], "max-age=3600");
    assert_eq!(response.headers["X-Content-Type-Options"], "nosniff");
    assert!(!owner.version("codex").is_empty());
    assert_eq!(
        owner.version("codex"),
        crate::application::provider_assets::icon_version(&p, "codex")
    );
    let list = super::super::provider_routes::handle(
        &Request {
            method: "GET".into(),
            path: "/api/providers".into(),
            ..Default::default()
        },
        &owner.registry,
        &p,
        &|_| true,
        &|_, _, _| {},
    )
    .unwrap();
    let list: serde_json::Value = serde_json::from_slice(&list.body).unwrap();
    let codex = list["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "codex")
        .unwrap();
    assert_eq!(codex["icon_image_version"], owner.version("codex"));
    assert_eq!(
        http.handle_authenticated(&request("DELETE", "codex", vec![]))
            .unwrap()
            .status,
        200
    );
    assert!(owner.version("codex").is_empty());
    assert_eq!(
        http.handle_authenticated(&request("GET", "codex", vec![]))
            .unwrap()
            .status,
        404
    );
    assert_eq!(
        http.handle_authenticated(&request("DELETE", "codex", vec![]))
            .unwrap()
            .status,
        200
    );
}
#[test]
fn builtin_reset_removes_image_and_restore_reloads_actual_revision() {
    let (_t, p, owner, http) = fixture();
    let baseline = owner.registry.baseline("codex").unwrap();
    let revision = owner
        .registry
        .history
        .save_override(
            "codex",
            crate::proto::provider::Definition {
                id: "codex".into(),
                display_name: "Custom Codex fixture".into(),
                ..Default::default()
            },
            &baseline,
            "",
            "edit",
        )
        .unwrap();
    owner.registry.reload().unwrap();
    assert_eq!(
        http.handle_authenticated(&request("PUT", "codex", b"GIF89a".to_vec()))
            .unwrap()
            .status,
        200
    );
    let invoke = |path: &str, body: serde_json::Value| {
        super::super::provider_routes::handle(
            &Request {
                method: "POST".into(),
                path: format!("/api/providers/codex/{path}"),
                body: serde_json::to_vec(&body).unwrap(),
                ..Default::default()
            },
            &owner.registry,
            &p,
            &|_| true,
            &|_, _, _| {},
        )
        .unwrap()
    };
    let stale = invoke("reset", serde_json::json!({"expected_revision":"stale"}));
    assert_eq!(stale.status, 409);
    assert!(!owner.version("codex").is_empty());
    let reset = invoke(
        "reset",
        serde_json::json!({"expected_revision":revision.revision}),
    );
    assert_eq!(reset.status, 200);
    assert!(owner.version("codex").is_empty());
    let current = owner.registry.history.current("codex").unwrap();
    assert_eq!(current.reason, "reset");
    assert_eq!(
        owner
            .registry
            .snapshot()
            .unwrap()
            .registry
            .lookup("codex")
            .unwrap()
            .definition
            .display_name,
        baseline.display_name
    );
    let restore = invoke(
        "restore",
        serde_json::json!({"revision":revision.revision,"expected_revision":current.revision}),
    );
    assert_eq!(restore.status, 200);
    assert_eq!(
        owner
            .registry
            .snapshot()
            .unwrap()
            .registry
            .lookup("codex")
            .unwrap()
            .definition
            .display_name,
        "Custom Codex fixture"
    );
    assert_eq!(
        owner.registry.history.current("codex").unwrap().reason,
        "restore"
    );
}
