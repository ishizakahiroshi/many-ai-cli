use super::*;
use crate::config::{Config, NotifyBackendConfig};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Deserialize)]
struct Case {
    name: String,
    method: String,
    body: String,
    #[serde(default)]
    initial: Value,
    #[serde(default)]
    policy: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    backends: usize,
    #[serde(default)]
    push: usize,
    #[serde(default)]
    fail_save: bool,
}
struct Fixture {
    _temp: tempfile::TempDir,
    paths: RuntimePaths,
    store: Arc<ConfigStore>,
}
fn fixture(initial: Value, backends: usize) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::production(temp.path()).unwrap();
    std::fs::create_dir_all(paths.root()).unwrap();
    let prefs = config::decode_user_prefs_http_json(initial.to_string().as_bytes()).unwrap();
    let mut cfg = Config {
        user_prefs: prefs,
        ..Default::default()
    };
    cfg.notify.backends = Some(vec![NotifyBackendConfig::default(); backends]);
    let store = Arc::new(ConfigStore::new(paths.clone(), cfg).unwrap());
    Fixture {
        _temp: temp,
        paths,
        store,
    }
}
fn request(method: &str, body: &str, policy: &str, version: &str) -> Request {
    Request {
        method: method.into(),
        path: PATH.into(),
        body: body.as_bytes().to_vec(),
        headers: vec![
            ("X-Template-Write".into(), policy.into()),
            ("X-Template-Version".into(), version.into()),
        ],
        ..Default::default()
    }
}
fn response(f: &Fixture, request: &Request) -> Response {
    handle_authenticated(request, &f.store, &f.paths, || false).unwrap()
}
fn body(response: &Response) -> Value {
    serde_json::from_slice(&response.body).unwrap()
}
fn normalized(value: Value, root: &str) -> Value {
    match value {
        Value::String(text) => text.replace(root, "<root>").into(),
        Value::Array(items) => items
            .into_iter()
            .map(|item| normalized(item, root))
            .collect(),
        Value::Object(items) => Value::Object(
            items
                .into_iter()
                .map(|(key, item)| (key.replace(root, "<root>"), normalized(item, root)))
                .collect(),
        ),
        other => other,
    }
}
fn persisted(f: &Fixture) -> Value {
    let text = std::fs::read_to_string(f.paths.resource(Resource::Config)).unwrap();
    Config::from_yaml(&text, &f.paths)
        .unwrap()
        .user_prefs
        .public_json()
}

fn native_expected_paths(value: Value) -> Value {
    match value {
        Value::String(text) => match text.as_str() {
            "<root>\\user_avatar.bin" => {
                format!("<root>{}user_avatar.bin", std::path::MAIN_SEPARATOR).into()
            }
            "<root>\\notify_sound_custom.bin" => {
                format!("<root>{}notify_sound_custom.bin", std::path::MAIN_SEPARATOR).into()
            }
            _ => text.into(),
        },
        Value::Array(items) => items.into_iter().map(native_expected_paths).collect(),
        Value::Object(items) => items
            .into_iter()
            .map(|(key, value)| (key, native_expected_paths(value)))
            .collect(),
        other => other,
    }
}

#[test]
fn fixed_go_get_put_decoder_sanitizers_hashes_and_saved_snapshots() {
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/services/user-preferences/cases.json"
    ))
    .unwrap();
    let observed: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/services/user-preferences/go_observations.json"
    ))
    .unwrap();
    assert_eq!(
        observed["baseline"],
        "21d0bc7935a2c4696fb89ccff2e324157a528c2d"
    );
    assert_eq!(cases.len(), observed["cases"].as_array().unwrap().len());
    for (case, expected) in cases.into_iter().zip(observed["cases"].as_array().unwrap()) {
        let expected = native_expected_paths(expected.clone());
        assert_eq!(case.name, expected["name"], "case sequence");
        let f = fixture(case.initial, case.backends);
        if case.fail_save {
            std::fs::create_dir(f.paths.resource(Resource::Config)).unwrap();
        }
        let root = f.paths.root().to_str().unwrap();
        // Insert native roots into a JSON string without changing its escapes.
        let quoted_root = serde_json::to_string(root).unwrap();
        let body_text = case
            .body
            .replace("<root>", &quoted_root[1..quoted_root.len() - 1]);
        let original_version =
            template_version(&f.store.snapshot().unwrap().config.user_prefs.templates);
        assert_eq!(
            original_version, expected["initial_version"],
            "{} initial hash",
            case.name
        );
        let version = if case.version == "current" {
            &original_version
        } else {
            &case.version
        };
        let req = request(&case.method, &body_text, &case.policy, version);
        let actual = handle_authenticated(&req, &f.store, &f.paths, || case.push > 0).unwrap();
        assert_eq!(
            actual.status,
            expected["status"].as_u64().unwrap() as u16,
            "{} status: {}",
            case.name,
            String::from_utf8_lossy(&actual.body)
        );
        let mut actual_body = normalized(body(&actual), root);
        if actual.status == 500 {
            assert!(
                actual_body["detail"]
                    .as_str()
                    .unwrap()
                    .starts_with("save failed:")
            );
            actual_body["detail"] = "save failed: <owned filesystem error>".into();
        }
        assert_eq!(actual_body, expected["body"], "{} body", case.name);
        assert_eq!(
            actual.headers.get("Content-Type").unwrap(),
            expected["content_type"].as_str().unwrap(),
            "{} content type",
            case.name
        );
        assert_eq!(
            actual
                .headers
                .get("X-Template-Version")
                .map(String::as_str)
                .unwrap_or(""),
            expected["version"].as_str().unwrap(),
            "{} response hash",
            case.name
        );
        assert_eq!(
            normalized(
                f.store.snapshot().unwrap().config.user_prefs.public_json(),
                root
            ),
            expected["runtime"],
            "{} published prefs",
            case.name
        );
        let saved = if expected["saved"].is_null() {
            assert!(
                !f.paths.resource(Resource::Config).is_file(),
                "{} unexpected save",
                case.name
            );
            Value::Null
        } else {
            normalized(persisted(&f), root)
        };
        assert_eq!(saved, expected["saved"], "{} persisted prefs", case.name);
    }
}

#[test]
fn get_effective_notification_is_detached_and_short_circuits_push_reader() {
    for (explicit, backends, push, effective, reads) in [
        (None, 0, false, false, 1),
        (None, 0, true, true, 1),
        (None, 2, false, true, 0),
        (Some(false), 2, true, false, 0),
        (Some(true), 0, false, true, 0),
    ] {
        let f = fixture(
            json!({"done_summary_notify":{"enabled":explicit}}),
            backends,
        );
        let before = f.store.snapshot().unwrap();
        let count = AtomicUsize::new(0);
        let actual = handle_authenticated(&request("GET", "", "", ""), &f.store, &f.paths, || {
            count.fetch_add(1, Ordering::SeqCst);
            push
        })
        .unwrap();
        assert_eq!(body(&actual)["done_summary_notify"]["enabled"], effective);
        assert_eq!(count.load(Ordering::SeqCst), reads);
        let after = f.store.snapshot().unwrap();
        assert_eq!(before.revision, after.revision);
        assert_eq!(
            after.config.user_prefs.done_summary_notify.enabled,
            explicit
        );
        assert!(!f.paths.resource(Resource::Config).exists());
    }
}

#[test]
fn unrelated_config_revision_retries_without_fabricating_template_conflict() {
    let f = fixture(json!({"templates":[{"body":"original"}]}), 0);
    let version = template_version(&f.store.snapshot().unwrap().config.user_prefs.templates);
    let req = request(
        "PUT",
        r#"{"templates":[{"body":"accepted"}]}"#,
        "compare",
        &version,
    );
    let prefs = config::decode_user_prefs_http_json(&req.body).unwrap();
    let mut attempts = 0;
    let actual = commit_preferences(&req, &f.store, prefs, |revision, next| {
        attempts += 1;
        if attempts == 1 {
            let mut other = f.store.snapshot().unwrap();
            other.config.input.deferred_enter_ms = 231;
            f.store.persist(other.revision, other.config).unwrap();
        }
        f.store.persist(revision, next)
    });
    assert_eq!(actual.status, 200);
    assert_eq!(attempts, 2);
    let snapshot = f.store.snapshot().unwrap();
    assert_eq!(snapshot.config.input.deferred_enter_ms, 231);
    assert_eq!(snapshot.config.user_prefs.templates[0].body, "accepted");
    let saved = Config::from_yaml(
        &std::fs::read_to_string(f.paths.resource(Resource::Config)).unwrap(),
        &f.paths,
    )
    .unwrap();
    assert_eq!(saved.input.deferred_enter_ms, 231);
    assert_eq!(saved.user_prefs, snapshot.config.user_prefs);
}

#[test]
fn preserve_reloads_changed_templates_and_compare_rejects_the_same_race() {
    for policy in ["preserve", "compare"] {
        let f = fixture(json!({"templates":[{"body":"original"}]}), 0);
        let version = template_version(&f.store.snapshot().unwrap().config.user_prefs.templates);
        let req = request(
            "PUT",
            r#"{"templates":[{"body":"stale edit"}],"display_name":"new name"}"#,
            policy,
            &version,
        );
        let prefs = config::decode_user_prefs_http_json(&req.body).unwrap();
        let mut attempts = 0;
        let actual = commit_preferences(&req, &f.store, prefs, |revision, next| {
            attempts += 1;
            if attempts == 1 {
                let mut other = f.store.snapshot().unwrap();
                other.config.user_prefs.templates[0].body = "concurrent edit".into();
                f.store.persist(other.revision, other.config).unwrap();
            }
            f.store.persist(revision, next)
        });
        let snapshot = f.store.snapshot().unwrap();
        assert_eq!(
            snapshot.config.user_prefs.templates[0].body,
            "concurrent edit"
        );
        if policy == "preserve" {
            assert_eq!(actual.status, 200);
            assert_eq!(attempts, 2);
            assert_eq!(snapshot.config.user_prefs.display_name, "new name");
        } else {
            assert_eq!(actual.status, 409);
            assert_eq!(body(&actual)["error"], "template_conflict");
            assert_eq!(attempts, 1);
            assert_eq!(snapshot.config.user_prefs.display_name, "");
        }
        assert_eq!(persisted(&f), snapshot.config.user_prefs.public_json());
    }
}

#[test]
fn response_describes_own_committed_snapshot_even_after_another_save() {
    let f = fixture(json!({}), 0);
    let req = request(
        "PUT",
        r#"{"templates":[{"body":"own edit"}],"display_name":"own name"}"#,
        "",
        "",
    );
    let prefs = config::decode_user_prefs_http_json(&req.body).unwrap();
    let actual = commit_preferences(&req, &f.store, prefs, |revision, next| {
        let committed = f.store.persist(revision, next)?;
        let mut another = committed.clone();
        another.config.user_prefs.templates[0].body = "later edit".into();
        another.config.user_prefs.display_name = "later name".into();
        f.store.persist(another.revision, another.config)?;
        Ok(committed)
    });
    assert_eq!(actual.status, 200);
    assert_eq!(body(&actual)["display_name"], "own name");
    let saved = config::decode_user_prefs_http_json(&actual.body).unwrap();
    assert_eq!(
        actual.headers["X-Template-Version"],
        template_version(&saved.templates)
    );
    assert_eq!(saved.templates[0].body, "own edit");
    assert_eq!(
        f.store.snapshot().unwrap().config.user_prefs.templates[0].body,
        "later edit"
    );
}

#[test]
fn failed_save_keeps_revision_prefs_and_template_version_then_retry_succeeds() {
    let f = fixture(
        json!({"templates":[{"body":"original"}],"display_name":"before"}),
        0,
    );
    let before = f.store.snapshot().unwrap();
    let version = template_version(&before.config.user_prefs.templates);
    std::fs::create_dir(f.paths.resource(Resource::Config)).unwrap();
    let req = request(
        "PUT",
        r#"{"templates":[{"body":"new"}],"display_name":"after"}"#,
        "compare",
        &version,
    );
    let failed = response(&f, &req);
    assert_eq!(failed.status, 500);
    assert_eq!(body(&failed)["error"], "save_failed");
    assert!(!failed.headers.contains_key("X-Template-Version"));
    let after = f.store.snapshot().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.config.user_prefs, before.config.user_prefs);
    std::fs::remove_dir(f.paths.resource(Resource::Config)).unwrap();
    let accepted = response(&f, &req);
    assert_eq!(accepted.status, 200);
    assert_ne!(accepted.headers["X-Template-Version"], version);
    assert_eq!(persisted(&f), body(&accepted));
}

#[test]
fn parallel_compare_preserve_and_other_config_writers_keep_one_template_winner() {
    let f = fixture(json!({"templates":[{"body":"original"}]}), 0);
    let version = template_version(&f.store.snapshot().unwrap().config.user_prefs.templates);
    let start = Arc::new(Barrier::new(25));
    let mut jobs = vec![];
    for i in 0..24 {
        let (store, paths, start, version) = (
            f.store.clone(),
            f.paths.clone(),
            start.clone(),
            version.clone(),
        );
        jobs.push(std::thread::spawn(move|| {
            start.wait();
            if i>=16 {
                loop {
                    let mut snap=store.snapshot().unwrap();
                    snap.config.input.deferred_enter_ms=100+i;
                    match store.persist(snap.revision,snap.config) {
                        Ok(_)=>break,
                        Err(ConfigError::Conflict{..})=>continue,
                        Err(error)=>panic!("non-preference save failed: {error}"),
                    }
                }
                return None;
            }
            let compare=i%2==0;
            let req=request("PUT",&json!({"templates":[{"body":format!("edit {i}")}],"open_project":format!("/synthetic/{i}")}).to_string(),if compare{"compare"}else{"preserve"},&version);
            Some((compare,handle_authenticated(&req,&store,&paths,||false).unwrap()))
        }));
    }
    start.wait();
    let mut winners = 0;
    for job in jobs {
        let Some((compare, reply)) = job.join().unwrap() else {
            continue;
        };
        if compare && reply.status == 409 {
            continue;
        }
        assert_eq!(
            reply.status,
            200,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        if compare {
            winners += 1;
        }
        let prefs = config::decode_user_prefs_http_json(&reply.body).unwrap();
        assert_eq!(
            reply.headers["X-Template-Version"],
            template_version(&prefs.templates)
        );
    }
    assert_eq!(winners, 1);
    let final_snapshot = f.store.snapshot().unwrap();
    let saved = Config::from_yaml(
        &std::fs::read_to_string(f.paths.resource(Resource::Config)).unwrap(),
        &f.paths,
    )
    .unwrap();
    assert_eq!(saved.user_prefs, final_snapshot.config.user_prefs);
    assert_eq!(saved.input, final_snapshot.config.input);
}

#[test]
fn route_and_method_body_limits_are_bounded_without_claiming_upload_routes() {
    let f = fixture(json!({}), 0);
    for path in [
        "/api/user-prefs/",
        "/api/user-prefs/avatar",
        "/api/user-prefs/notify-sound-custom",
        "/api/avatar",
    ] {
        let req = Request {
            path: path.into(),
            ..request("PUT", "{", "", "")
        };
        assert!(handle_authenticated(&req, &f.store, &f.paths, || false).is_none());
    }
    assert_eq!(response(&f, &request("POST", "{", "", "")).status, 405);
    assert_eq!(
        response(
            &f,
            &request(
                "PUT",
                &format!("{{\"display_name\":\"{}\"}}", "x".repeat(JSON_BODY_LIMIT)),
                "",
                ""
            )
        )
        .status,
        400
    );
    let mut trailing = request("PUT", "{}", "", "");
    trailing.body.extend(vec![b' '; JSON_BODY_LIMIT + 10]);
    assert_eq!(response(&f, &trailing).status, 200);
}

#[test]
fn native_path_comparison_and_mime_filter_preserve_source_spelling_rules() {
    assert_eq!(candidate_key("/a/b/../c", false), "/a/c");
    assert_eq!(
        candidate_key(r"C:\SYNTHETIC\prefs\..\user_avatar.bin", true),
        r"c:\synthetic\user_avatar.bin"
    );
    assert_eq!(
        candidate_key(r"C:\İ\USER_AVATAR.BIN", true),
        "c:\\i\\user_avatar.bin"
    );
    for mime in ["audio/wav", " AUDIO/MP3 ; codec=x ", "audio/"] {
        assert!(notify_sound_allowed(mime));
    }
    for mime in ["", "text/html", " audio ; audio/wav ", "application/audio"] {
        assert!(!notify_sound_allowed(mime));
    }
}

#[test]
fn trial_preferences_persist_only_in_explicit_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("trial");
    std::fs::create_dir(&root).unwrap();
    let installed = temp.path().join("synthetic-installed");
    let paths = RuntimePaths::trial(&root, 49962, &installed).unwrap();
    let store = ConfigStore::new(paths.clone(), Config::default()).unwrap();
    let req=request("PUT",&json!({"avatar":paths.resource(Resource::Avatar),"notify_sound":{"custom_file":paths.resource(Resource::NotifySound),"custom_mime":"audio/wav"}}).to_string(),"","");
    let actual = handle_authenticated(&req, &store, &paths, || false).unwrap();
    assert_eq!(actual.status, 200);
    assert!(paths.resource(Resource::Config).is_file());
    assert!(!installed.exists());
    assert!(!paths.resource(Resource::Avatar).exists());
    assert!(!paths.resource(Resource::NotifySound).exists());
}
