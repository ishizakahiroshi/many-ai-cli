use super::*;
use crate::{
    config::{Config, ConfigStore, Resource},
    files::safe_fs::Dir,
    proto::provider::{DistributionStatus, to_go_json},
};
fn fixture() -> (tempfile::TempDir, RuntimePaths, Arc<DistributionHttp>) {
    let t = tempfile::tempdir().unwrap();
    let run = t.path().join("runtime");
    let old = t.path().join("installed");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::create_dir_all(&old).unwrap();
    let p = RuntimePaths::trial(&run, 49285, &old).unwrap();
    let cfg = Arc::new(ConfigStore::new(p.clone(), Config::default()).unwrap());
    let registry = Arc::new(ProviderRegistryStore::new(&p, cfg).unwrap());
    let http = DistributionHttp::new(&p, registry);
    (t, p, http)
}
fn request(method: &str, path: &str) -> Request {
    Request {
        method: method.into(),
        path: format!("/api/provider-distributions/{path}"),
        body: b"malformed-json".to_vec(),
        ..Default::default()
    }
}
#[test]
fn empty_keys_fail_before_decoding_and_none_is_real_absence() {
    let (_t, p, http) = fixture();
    for path in ["check", "accept"] {
        let r = http.handle_authenticated(&request("POST", path)).unwrap();
        assert_eq!(r.status, 503);
        assert!(
            String::from_utf8(r.body)
                .unwrap()
                .contains("distribution_keys_unconfigured")
        );
        assert_eq!(
            http.handle_authenticated(&request("GET", path))
                .unwrap()
                .status,
            405
        )
    }
    let r = http
        .handle_authenticated(&request("GET", "status"))
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["state"], "none");
    assert_eq!(v["enabled"], false);
    assert!(!p.resource(Resource::ProviderDistributions).exists());
    assert_eq!(
        http.handle_authenticated(&request("GET", "diff"))
            .unwrap()
            .status,
        400
    );
}
#[test]
fn corrupt_pointer_and_tampered_rollback_return_errors_without_success() {
    let (_t, p, http) = fixture();
    let root = Dir::open_or_create_private(&p.resource(Resource::ProviderDistributions)).unwrap();
    root.replace("accepted.json", b"{", 0o600).unwrap();
    assert_eq!(
        http.handle_authenticated(&request("GET", "status"))
            .unwrap()
            .status,
        500
    );
    let status = DistributionStatus {
        digest: "a".repeat(64),
        state: "accepted".into(),
        ..Default::default()
    };
    root.replace("previous.json", &to_go_json(&status).unwrap(), 0o600)
        .unwrap();
    let r = http
        .handle_authenticated(&request("POST", "rollback"))
        .unwrap();
    assert_eq!(r.status, 422);
    assert_eq!(root.read("accepted.json", usize::MAX).unwrap(), b"{");
}
fn seed(http: &DistributionHttp, root: &Dir, kind: &str, label: &str) -> DistributionStatus {
    use crate::proto::provider::{DistributionBundle, DistributionPayload};
    use sha2::{Digest, Sha256};
    let mut def = http
        .registry
        .snapshot()
        .unwrap()
        .registry
        .lookup("codex")
        .unwrap()
        .definition;
    def.display_name = label.into();
    let bundle = DistributionBundle {
        payload: DistributionPayload {
            schema_version: 1,
            catalog_version: label.into(),
            definitions: Some(vec![def.clone()]),
            digests: Some(
                [(
                    def.id.clone(),
                    crate::profile::store::definition_digest(&def).unwrap(),
                )]
                .into(),
            ),
            ..Default::default()
        },
        ..Default::default()
    };
    let bytes = to_go_json(&bundle).unwrap();
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    root.child_dir(kind, true)
        .unwrap()
        .replace(&format!("{digest}.json"), &bytes, 0o600)
        .unwrap();
    DistributionStatus {
        digest,
        catalog_version: label.into(),
        state: "accepted".into(),
        ..Default::default()
    }
}
#[test]
fn diff_is_real_candidate_and_corrupt_current_is_not_silently_empty() {
    let (_t, p, http) = fixture();
    let root = Dir::open_or_create_private(&p.resource(Resource::ProviderDistributions)).unwrap();
    let old = seed(&http, &root, "accepted", "old fixture");
    let candidate = seed(&http, &root, "downloaded", "new fixture");
    root.replace("accepted.json", &to_go_json(&old).unwrap(), 0o600)
        .unwrap();
    let mut req = request("GET", "diff");
    req.query = format!("digest={}", candidate.digest);
    let result = http.handle_authenticated(&req).unwrap();
    assert_eq!(result.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&result.body).unwrap();
    let changes = body["diff"][0]["changes"].as_array().unwrap();
    let name = changes
        .iter()
        .find(|f| f["field"] == "display_name")
        .unwrap();
    assert_eq!(name["current"], "old fixture");
    assert_eq!(name["candidate"], "new fixture");
    root.replace("accepted.json", b"{", 0o600).unwrap();
    let result = http.handle_authenticated(&req).unwrap();
    assert_eq!(result.status, 500);
    assert!(
        String::from_utf8(result.body)
            .unwrap()
            .contains("distribution_accepted_load_failed")
    );
}
#[test]
fn rollback_reload_failure_restores_pointers_and_live_registry() {
    let (_t, p, http) = fixture();
    let root = Dir::open_or_create_private(&p.resource(Resource::ProviderDistributions)).unwrap();
    let old = seed(&http, &root, "accepted", "old fixture");
    let new = seed(&http, &root, "accepted", "new fixture");
    root.replace("accepted.json", &to_go_json(&new).unwrap(), 0o600)
        .unwrap();
    root.replace("previous.json", &to_go_json(&old).unwrap(), 0o600)
        .unwrap();
    http.registry.reload().unwrap();
    assert_eq!(
        http.registry
            .snapshot()
            .unwrap()
            .registry
            .lookup("codex")
            .unwrap()
            .definition
            .display_name,
        "new fixture"
    );
    std::fs::write(
        p.resource(Resource::ProviderDefinitions),
        b"cannot open as directory",
    )
    .unwrap();
    let result = http
        .handle_authenticated(&request("POST", "rollback"))
        .unwrap();
    assert_eq!(result.status, 500);
    assert!(
        String::from_utf8(result.body)
            .unwrap()
            .contains("distribution rollback was reverted")
    );
    assert_eq!(http.store.status().unwrap().digest, new.digest);
    assert_eq!(
        http.registry
            .snapshot()
            .unwrap()
            .registry
            .lookup("codex")
            .unwrap()
            .definition
            .display_name,
        "new fixture"
    );
    assert_eq!(
        root.read("previous.json", usize::MAX).unwrap(),
        to_go_json(&old).unwrap()
    );
}

#[test]
fn unknown_distribution_suffix_allows_any_method_through_guard_then_returns_not_found() {
    let (_root, _paths, owner) = fixture();
    for method in ["TRACE", "CUSTOM", "POST"] {
        assert_eq!(
            owner
                .handle_authenticated(&request(method, "missing"))
                .unwrap()
                .status,
            404
        );
    }
    assert_eq!(
        owner
            .handle_authenticated(&request("CUSTOM", "status"))
            .unwrap()
            .status,
        405
    );
}
