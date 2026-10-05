use super::*;
use crate::{
    config::Config,
    hub::{http::Request, nvidia_routes::NvidiaHttp},
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
struct FakeIo {
    calls: AtomicUsize,
    result: Mutex<Result<Vec<u8>, CatalogError>>,
}
impl NvidiaIo for FakeIo {
    fn catalog<'a>(
        &'a self,
        key: &'a str,
        parse_body: bool,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<Vec<u8>, CatalogError>> {
        Box::pin(async move {
            assert!(!key.trim().is_empty());
            assert!(!key.contains(['\0', '\r', '\n']));
            self.calls.fetch_add(1, Ordering::SeqCst);
            if cancel.is_cancelled() {
                return Err(CatalogError::Cancelled);
            }
            let reply = self.result.lock().unwrap().clone()?;
            Ok(if parse_body { reply } else { vec![] })
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    paths: RuntimePaths,
    config: Arc<ConfigStore>,
    owner: Arc<NvidiaNim>,
    io: Arc<FakeIo>,
}
fn fixture(environment: Vec<String>) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("runtime");
    let installed = temp.path().join("installed");
    std::fs::create_dir(&run).unwrap();
    std::fs::create_dir(&installed).unwrap();
    let paths = RuntimePaths::trial(&run, 49338, &installed).unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::defaults(&paths)).unwrap());
    let io = Arc::new(FakeIo {
        calls: AtomicUsize::new(0),
        result: Mutex::new(Ok(br#"{"data":[{"id":"vendor/model"}]}"#.to_vec())),
    });
    let owner = NvidiaNim::new(NvidiaDependencies {
        paths: paths.clone(),
        config: config.clone(),
        environment,
        io: io.clone(),
    })
    .unwrap();
    Fixture {
        _temp: temp,
        paths,
        config,
        owner,
        io,
    }
}
#[test]
fn private_key_save_status_priority_and_delete_never_serialize_secret() {
    let f = fixture(vec![]);
    assert!(!key_configured(&f.paths, &[]).unwrap());
    let status = f
        .owner
        .save_settings(true, " synthetic-private-key ")
        .unwrap();
    assert!(status.enabled && status.api_key_configured);
    assert_eq!(status.api_key_source, KeySource::File);
    assert!(
        !serde_json::to_string(&status)
            .unwrap()
            .contains("synthetic-private-key")
    );
    let root = Dir::open(f.paths.root())
        .unwrap()
        .child_dir("secrets", false)
        .unwrap();
    assert_eq!(
        root.read("nvidia_api_key", KEY_LIMIT).unwrap(),
        b"synthetic-private-key"
    );
    assert!(key_configured(&f.paths, &[]).unwrap());
    assert!(!f.owner.delete_key().unwrap().api_key_configured);
    assert!(f.owner.delete_key().is_ok());
}
#[test]
fn environment_prevents_file_mutation_and_invalid_env_does_not_fall_back() {
    let f = fixture(vec!["NVIDIA_API_KEY=synthetic-env".into()]);
    assert_eq!(f.owner.status().unwrap().api_key_source, KeySource::Env);
    assert_eq!(
        f.owner.save_settings(true, "synthetic-file").unwrap_err(),
        SettingsError::Environment
    );
    assert_eq!(
        f.owner.delete_key().unwrap_err(),
        SettingsError::Environment
    );
    assert!(!f.paths.root().join("secrets").exists());
    assert!(!f.config.snapshot().unwrap().config.nvidia_nim.enabled);
    assert!(key_configured(&f.paths, &["NVIDIA_API_KEY=invalid\nkey".into()]).is_err());
}
#[test]
fn malformed_private_file_invalid_text_and_limit_are_rejected_before_config_publish() {
    let f = fixture(vec![]);
    for key in [" ", "internal\rkey", "internal\nkey", "internal\0key"] {
        assert_eq!(
            f.owner.save_settings(true, key).unwrap_err(),
            SettingsError::InvalidKey
        );
    }
    assert!(!f.config.snapshot().unwrap().config.nvidia_nim.enabled);
    let dir = Dir::open(f.paths.root())
        .unwrap()
        .child_dir("secrets", true)
        .unwrap();
    dir.replace("nvidia_api_key", &vec![b'x'; KEY_LIMIT + 1], 0o600)
        .unwrap();
    assert!(key_configured(&f.paths, &[]).is_err());
    assert_eq!(f.owner.status().unwrap_err(), SettingsError::Status);
    assert!(!f.config.snapshot().unwrap().config.nvidia_nim.enabled);
}
#[tokio::test]
async fn test_ignores_body_and_disabled_flag_preserves_source_codes() {
    let f = fixture(vec![]);
    let route = NvidiaHttp::new(f.owner.clone());
    let request = Request {
        method: "POST".into(),
        path: "/api/nvidia-nim/test".into(),
        body: b"ignored malformed body".to_vec(),
        ..Default::default()
    };
    assert_eq!(
        route
            .handle_authenticated(&request, &Cancellation::default())
            .await
            .unwrap()
            .status,
        400
    );
    assert_eq!(f.io.calls.load(Ordering::SeqCst), 0);
    f.owner.save_settings(false, "synthetic-key").unwrap();
    for (error, expected) in [
        (CatalogError::Http(401), "unauthorized"),
        (CatalogError::Http(402), "payment_required"),
        (CatalogError::Http(408), "timeout"),
        (CatalogError::Http(429), "rate_limited"),
        (CatalogError::Http(503), "server_error"),
        (CatalogError::Parse, "connection_failed"),
    ] {
        *f.io.result.lock().unwrap() = Err(error);
        let response = route
            .handle_authenticated(&request, &Cancellation::default())
            .await
            .unwrap();
        assert_eq!(response.status, 200);
        let wire: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(wire["code"], expected);
        assert_eq!(wire["ok"], false);
    }
    *f.io.result.lock().unwrap() = Ok(b"invalid body success test does not parse".to_vec());
    assert_eq!(
        f.owner.test(&Cancellation::default()).await.unwrap(),
        Ok(())
    );
}
#[test]
fn catalog_parser_retains_order_and_rejects_missing_null_duplicates_traversal_and_non_ascii() {
    assert_eq!(
        parse_models(br#"{"data":[{"id":" B/one "},{"id":"A/two"}]}"#).unwrap(),
        vec!["B/one", "A/two"]
    );
    assert_eq!(
        parse_models(br#"{"data":[]}"#).unwrap(),
        Vec::<String>::new()
    );
    for data in [
        r#"{}"#,
        r#"{"data":null}"#,
        r#"{"data":[{}]}"#,
        r#"{"data":[{"id":"a/../b"}]}"#,
        r#"{"data":[{"id":"a/b"},{"id":"a/b"}]}"#,
        r#"{"data":[{"id":"模型"}]}"#,
    ] {
        assert_eq!(parse_models(data.as_bytes()), Err(CatalogError::Parse));
    }
}
#[tokio::test]
async fn native_trial_never_attempts_live_catalog() {
    let f = fixture(vec![]);
    let native = NativeNvidiaIo { paths: f.paths };
    assert_eq!(
        native
            .catalog("synthetic-key", true, &Cancellation::default())
            .await,
        Err(CatalogError::Request)
    );
}

#[tokio::test]
async fn settings_http_omitted_key_preserves_file_whitespace_is_invalid_and_method_guard_is_first()
{
    let f = fixture(vec![]);
    f.owner
        .save_settings(true, "synthetic-private-key")
        .unwrap();
    let route = NvidiaHttp::new(f.owner.clone());
    let response = route
        .handle_authenticated(
            &Request {
                method: "PUT".into(),
                path: "/api/nvidia-nim".into(),
                body: br#"{"enabled":false}"#.to_vec(),
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert!(!f.owner.status().unwrap().enabled);
    assert!(f.owner.status().unwrap().api_key_configured);
    let response = route
        .handle_authenticated(
            &Request {
                method: "PUT".into(),
                path: "/api/nvidia-nim".into(),
                body: br#"{"enabled":true,"api_key":" "}"#.to_vec(),
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(response.status, 400);
    assert!(!f.owner.status().unwrap().enabled);
    let response = route
        .handle_authenticated(
            &Request {
                method: "PATCH".into(),
                path: "/api/nvidia-nim".into(),
                body: b"invalid JSON".to_vec(),
                ..Default::default()
            },
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(response.status, 405);
    assert_eq!(f.io.calls.load(Ordering::SeqCst), 0);
    let wire = serde_json::to_string(&f.config.snapshot().unwrap().config).unwrap();
    assert!(!wire.contains("synthetic-private-key"));
}

#[test]
fn shared_key_resolver_trims_outer_crlf_but_rejects_internal_controls_like_source() {
    let f = fixture(vec![]);
    assert_eq!(
        resolve_key(&f.paths, &["NVIDIA_API_KEY=\r\n synthetic-key \r\n".into()]).unwrap(),
        "synthetic-key"
    );
    for key in [
        "NVIDIA_API_KEY=synthetic\rkey",
        "NVIDIA_API_KEY=synthetic\nkey",
        "NVIDIA_API_KEY=synthetic\0key",
    ] {
        assert!(resolve_key(&f.paths, &[key.into()]).is_err());
    }
}
