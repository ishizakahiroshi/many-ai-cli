use many_ai_cli::{
    config::{Resource, RuntimePaths},
    launcher::*,
    process::Cancellation,
};
use serde_json::Value;
#[cfg(unix)]
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};

fn store_at(root: &Path) -> LauncherStore {
    let installed = root.parent().unwrap().join("installed");
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::create_dir_all(root).unwrap();
    LauncherStore::open(RuntimePaths::trial(root, 49001, &installed).unwrap()).unwrap()
}
fn ssh() -> Profile {
    Profile {
        name: "remote".into(),
        kind: "ssh".into(),
        host: "example.invalid".into(),
        ..Profile::default()
    }
}
#[test]
fn launcher_actual_go_oracle_contracts() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../fixtures/launcher/go-oracle.json")).unwrap();
    assert!(cases.len() > 100, "empty or incomplete Go oracle");
    for (i, row) in cases.iter().enumerate() {
        let input = row["input"].as_str().unwrap_or_default();
        match row["kind"].as_str().unwrap() {
            "yaml" => {
                let got = decode_profiles_yaml(input.as_bytes());
                assert_eq!(
                    got.is_ok(),
                    row["ok"].as_bool().unwrap(),
                    "yaml case {i}: {got:?}"
                );
                if let Ok(file) = got {
                    assert_eq!(
                        serde_json::to_value(&file).unwrap(),
                        row["output"],
                        "yaml case {i}"
                    );
                    assert_eq!(
                        file.validate().err().unwrap_or_default(),
                        row["validation_error"],
                        "yaml validation {i}"
                    );
                }
            }
            "validate" => {
                let profile: Profile = serde_json::from_value(row["input"].clone()).unwrap();
                assert_eq!(
                    profile.validate().err().unwrap_or_default(),
                    row["error"],
                    "validation case {i}"
                );
            }
            "ssh_serve_args" => {
                let profile: Profile = serde_json::from_value(row["profile"].clone()).unwrap();
                let mut expected: Vec<String> =
                    serde_json::from_value(row["output"].clone()).unwrap();
                let source_script = expected.last().unwrap().clone();
                assert_eq!(source_script, ssh_serve_script(&profile, 49001, None));
                *expected.last_mut().unwrap() = shell_quote(&source_script);
                assert_eq!(ssh_serve_args(&profile, 49001, None), expected);
            }
            "quote" => assert_eq!(shell_quote(input), row["output"], "quote case {i}"),
            "tunnel_url" => assert_eq!(
                tunnel_hub_url(
                    49001,
                    row["token"].as_str().unwrap(),
                    row["host"].as_str().unwrap()
                ),
                row["output"]
            ),
            "scan" => {
                for size in [1, 2, 7, 8192] {
                    let mut scanner = UrlScanner::default();
                    let mut output = Vec::new();
                    let mut urls = Vec::new();
                    for chunk in input.as_bytes().chunks(size) {
                        for (line, url) in scanner.feed(chunk, false) {
                            output.extend(line);
                            urls.extend(url);
                        }
                    }
                    for (line, url) in scanner.feed(&[], true) {
                        output.extend(line);
                        urls.extend(url);
                    }
                    assert_eq!(
                        String::from_utf8(output).unwrap(),
                        row["output"],
                        "scanner case {i}"
                    );
                    assert_eq!(serde_json::to_value(urls).unwrap(), row["urls"]);
                }
            }
            "import" => {
                let got = parse_exported_profile(input.as_bytes());
                assert_eq!(got.is_ok(), row["ok"].as_bool().unwrap(), "import case {i}");
                if let Ok(got) = got {
                    assert_eq!(serde_json::to_value(got).unwrap(), row["output"]);
                }
            }
            "banner" => assert_eq!(startup_banner(input), row["output"]),
            "ui_host" => assert_eq!(
                allowed_ui_host(input, 49001),
                row["output"].as_bool().unwrap(),
                "host case {i}: {input}"
            ),
            "ui_origin" => assert_eq!(
                allowed_ui_origin(input, 49001),
                row["output"].as_bool().unwrap(),
                "origin case {i}: {input}"
            ),
            other => panic!("unknown Go corpus type {other}"),
        }
    }
}
#[test]
fn launcher_profile_roundtrip_selection_and_failed_replace() {
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    assert_eq!(store.load_profiles().unwrap(), ProfilesFile::fresh());
    let mut profile = ssh();
    profile.host = "first@example.invalid".into();
    profile.user = "old".into();
    profile.identity_file = "key with spaces".into();
    profile.cwd = "/synthetic/two words".into();
    let file = ProfilesFile {
        version: 1,
        last_used: "missing".into(),
        profiles: Some(vec![profile]),
    };
    file.validate().unwrap();
    store.save_profiles(&file).unwrap();
    let loaded = store.load_profiles().unwrap();
    assert_eq!(loaded.list()[0].user, "first");
    assert_eq!(loaded.list()[0].host, "example.invalid");
    assert!(loaded.select("", true).is_err());
    assert_eq!(loaded.select("remote", true).unwrap().name, "remote");
    let mut invalid = ssh();
    invalid.host = "-option".into();
    assert!(store.replace_profiles(Some(vec![invalid])).is_err());
    assert_eq!(store.load_profiles().unwrap(), loaded);
    let path = store.paths.resource(Resource::LauncherProfiles);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(store.save_profiles(&loaded).is_err());
    assert!(path.is_dir());
    assert!(std::fs::read_dir(store.root()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".many-ai-cli-")
    }));
}
#[test]
fn launcher_load_does_not_validate_or_reset_historical_profile() {
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let path = store.paths.resource(Resource::LauncherProfiles);
    let old=b"version: 0\nlast_used: absent\nprofiles:\n- name: legacy\n  type: future-type\n  unknown_optional: ignored\n";
    std::fs::write(&path, old).unwrap();
    let loaded = store.load_profiles().unwrap();
    assert_eq!(loaded.version, 0);
    assert_eq!(loaded.list()[0].kind, "future-type");
    assert!(loaded.validate().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), old);
    std::fs::write(&path, b"version: 999\n").unwrap();
    assert!(store.load_profiles().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"version: 999\n");
}
#[test]
fn launcher_active_concurrent_updates_and_root_isolation() {
    let t = tempfile::tempdir().unwrap();
    let a = store_at(&t.path().join("a"));
    let b = store_at(&t.path().join("b"));
    b.register_active("unrelated", "http://127.0.0.1:49002/?token=abc")
        .unwrap();
    let handles: Vec<_> = (0..16)
        .map(|n| {
            let a = a.clone();
            std::thread::spawn(move || {
                a.register_active(&format!("p{n}"), "http://127.0.0.1:49001/?token=abc")
                    .unwrap();
                a.collect_active(|_| true, |_| true).unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(a.active_snapshot().unwrap().len(), 16);
    a.unregister_all().unwrap();
    assert!(a.active_snapshot().unwrap().is_empty());
    assert_eq!(b.active_snapshot().unwrap().len(), 1);
}
#[test]
fn launcher_active_double_guard_and_stale_lock() {
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    for (name, pid, url) in [
        ("good", 1, "good"),
        ("reused", 1, "bad"),
        ("dead", -1, "good"),
    ] {
        store
            .register_record(ActiveConnection {
                profile: name.into(),
                pid,
                hub_url: url.into(),
                started_at: "2026-01-01T00:00:00Z".into(),
            })
            .unwrap();
    }
    let live = store
        .collect_active(|pid| pid > 0, |url| url == "good")
        .unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].profile, "good");
    let mut lock = store
        .try_acquire_connect_with("remote", |_| false)
        .unwrap()
        .unwrap();
    assert!(
        store
            .try_acquire_connect_with("remote", |_| true)
            .unwrap()
            .is_none()
    );
    lock.release().unwrap();
    lock.release().unwrap();
    assert!(
        store
            .try_acquire_connect_with("remote", |_| true)
            .unwrap()
            .is_some()
    );
    std::fs::write(
        store.root().join(LauncherStore::startup_lock_name("dead")),
        br#"{"profile":"dead","pid":-1,"started_at":"2026-01-01T00:00:00Z"}"#,
    )
    .unwrap();
    assert!(
        store
            .try_acquire_connect_with("dead", |pid| pid > 0)
            .unwrap()
            .is_some()
    );
    std::fs::write(store.paths.resource(Resource::LauncherActive), b"{broken").unwrap();
    assert!(store.active_snapshot().unwrap().is_empty());
}
#[cfg(unix)]
#[test]
fn launcher_persistence_rejects_symlink_targets_and_pins_root() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    store.save_profiles(&ProfilesFile::fresh()).unwrap();
    assert_eq!(
        std::fs::metadata(store.paths.resource(Resource::LauncherProfiles))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let other = t.path().join("outside");
    std::fs::create_dir(&other).unwrap();
    let moved = t.path().join("moved");
    std::fs::rename(store.root(), &moved).unwrap();
    symlink(&other, store.root()).unwrap();
    store
        .register_active("p", "http://127.0.0.1:49001/?token=abc")
        .unwrap();
    assert!(!other.join("launcher-active.json").exists());
    assert!(moved.join("launcher-active.json").exists());
}
#[test]
fn launcher_export_candidates_order_and_import_completion() {
    let identity = ExportIdentity {
        username: "synthetic-user".into(),
        user: "ignored".into(),
        public_host: " PUBLIC.example. ".into(),
        ssh_connection: "192.0.2.1 1000 192.0.2.2 22".into(),
        hostname: "synthetic-host".into(),
        ipv4: vec![
            "127.0.0.1".parse().unwrap(),
            "169.254.1.1".parse().unwrap(),
            "192.0.2.3".parse().unwrap(),
        ],
        cwd: "/synthetic/work".into(),
        executable: "C:\\synthetic\\many-ai-cli.exe".into(),
    };
    let exported = build_export_profile(
        &ExportOptions {
            public_host: "public.EXAMPLE".into(),
            ..ExportOptions::default()
        },
        &identity,
    );
    assert_eq!(
        exported.host_candidates.as_ref().unwrap(),
        &["public.EXAMPLE", "192.0.2.2", "192.0.2.3", "synthetic-host"]
    );
    assert_eq!(exported.profile.binary, "many-ai-cli.exe");
    assert!(exported.profile.identity_file.is_empty());
    assert_eq!(exported.profile.user, "synthetic-user");
    let existing = ProfilesFile {
        profiles: Some(vec![exported.profile.clone()]),
        ..ProfilesFile::fresh()
    };
    let mut params = FetchParams {
        host: "reachable.example".into(),
        identity_file: "synthetic-key".into(),
        ..FetchParams::default()
    };
    let completed = complete_fetched_profile(exported.clone(), &params, &existing).unwrap();
    assert_eq!(completed.profile.name, "synthetic-host-2");
    assert_eq!(completed.profile.host, "reachable.example");
    assert_eq!(completed.profile.identity_file, "synthetic-key");
    params.host = "-option".into();
    assert!(complete_fetched_profile(exported, &params, &existing).is_err());
}
#[test]
fn launcher_remote_scope_wsl_and_cleanup_are_bounded() {
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("a"));
    let profile = ssh();
    assert!(validate_remote_scope(&store.paths, None).is_err());
    let scope = RemoteTrial::new("/synthetic/candidate A", "/synthetic/candidate.bin");
    validate_remote_scope(&store.paths, Some(&scope)).unwrap();
    let re = regex::Regex::new(&cleanup_pattern(&profile, 49001, Some(&scope))).unwrap();
    assert!(re.is_match("/synthetic/candidate.bin --trial-root /synthetic/candidate A --trial-port 49001 serve --port 49001"));
    for other in [
        "many-ai-cli serve --port 49001",
        "/synthetic/candidate.bin --trial-root /synthetic/candidate B --trial-port 49001 serve --port 49001",
        "/synthetic/candidate.bin --trial-root /synthetic/candidate A --trial-port 49001 serve --port 490010",
    ] {
        assert!(!re.is_match(other));
    }
    let p = Profile {
        name: "wsl".into(),
        kind: "wsl".into(),
        distro: "synthetic-distro".into(),
        cwd: "/two words".into(),
        ..Profile::default()
    };
    let args = wsl_serve_args(&p, 49001, None);
    assert_eq!(&args[..4], ["-d", "synthetic-distro", "--cd", "/two words"]);
    assert!(args.last().unwrap().contains("MANY_AI_CLI_WSL_LAUNCHER=1"));
    #[cfg(not(windows))]
    {
        let config = ConnectorConfig::new(store.paths.clone(), t.path().into());
        assert!(config.start(p).is_err());
    }
}
#[cfg(unix)]
fn executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
#[cfg(unix)]
fn stub_config(store: &LauncherStore, dir: &Path, mode: &str) -> ConnectorConfig {
    let stub = dir.join("fake-ssh.py");
    executable(
        &stub,
        r#"#!/usr/bin/env python3
import json, os, sys, time
args=sys.argv[1:]
with open(os.environ['LAUNCHER_STUB_LOG'],'a') as log: log.write(json.dumps(args)+'\n')
if 'pkill' in args: sys.exit(0)
if '-N' in args:
 while True: time.sleep(0.01)
if '-L' not in args and '-lc' not in args:
 print(os.environ.get('LAUNCHER_STUB_TOKEN','a+b&x=1'));sys.exit(0)
if '-lc' in args:
 print('synthetic shell greeting');print('{"profile":{"name":"imported","type":"ssh","host":"example.invalid"},"host_candidates":["example.invalid"]}');sys.exit(0)
port=int(args[args.index('-L')+1].split(':')[1]);mode=os.environ.get('LAUNCHER_STUB_MODE','ready')
if 'silent.example' in args: mode='silent'
if mode=='early': sys.exit(7)
if mode=='silent':
 while True: time.sleep(0.01)
if mode=='mismatch' and port==49001: port+=1
sys.stderr.write('noise\nhttp://127.0.0.1:%d/?token=abc123\n'%port);sys.stderr.flush()
while True: time.sleep(0.01)
"#,
    );
    let mut config = ConnectorConfig::new(store.paths.clone(), dir.into());
    config.ssh_executable = stub;
    config.remote_trial = Some(RemoteTrial::new(
        "/synthetic/candidate",
        "/synthetic/candidate/many-ai-cli",
    ));
    config.environment.insert(
        "LAUNCHER_STUB_LOG".into(),
        Some(dir.join("argv.jsonl").into_os_string()),
    );
    config
        .environment
        .insert("LAUNCHER_STUB_MODE".into(), Some(mode.into()));
    config
}
#[cfg(unix)]
#[tokio::test]
async fn launcher_fake_ssh_serve_retry_ready_lifetime_and_cleanup() {
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let config = stub_config(&store, t.path(), "mismatch");
    let mut p = ssh();
    p.hub_port = 49001;
    let mut conn = config.start(p).unwrap();
    let ready = tokio::time::timeout(Duration::from_secs(5), conn.ready())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready, "http://127.0.0.1:49101/?token=abc123");
    assert!(
        tokio::time::timeout(Duration::from_millis(30), conn.wait())
            .await
            .is_err()
    );
    conn.cancel();
    conn.cancel();
    tokio::time::timeout(Duration::from_secs(4), conn.wait())
        .await
        .unwrap()
        .unwrap();
    conn.wait().await.unwrap();
    let args: Vec<Vec<String>> = std::fs::read_to_string(t.path().join("argv.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        args.iter().filter(|a| a.iter().any(|s| s == "-t")).count(),
        2
    );
    assert_eq!(
        args.iter()
            .filter(|a| a.iter().any(|s| s == "pkill"))
            .count(),
        2
    );
}
#[cfg(unix)]
#[tokio::test]
async fn launcher_fake_ssh_early_exit_and_import() {
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let config = stub_config(&store, t.path(), "early");
    let mut p = ssh();
    p.hub_port = 49001;
    let mut conn = config.start(p).unwrap();
    assert!(conn.ready().await.is_err());
    assert!(conn.wait().await.is_err());
    let args: Vec<Vec<String>> = std::fs::read_to_string(t.path().join("argv.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        args.iter().filter(|a| a.iter().any(|s| s == "-t")).count(),
        5
    );
    let exported = config
        .fetch_remote_profile(
            &FetchParams {
                host: "first@example.invalid".into(),
                ..FetchParams::default()
            },
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(exported.profile.name, "imported");
}
#[cfg(unix)]
#[tokio::test]
async fn launcher_manager_timeout_only_bounds_connecting_and_closes_own_connection() {
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let mut p = ssh();
    p.hub_port = 49001;
    store
        .save_profiles(&ProfilesFile {
            profiles: Some(vec![p]),
            ..ProfilesFile::fresh()
        })
        .unwrap();
    let config = stub_config(&store, t.path(), "ready");
    let connect_timeout = Duration::from_secs(3);
    let manager = ConnectionManager::with_timeout(store.clone(), config, connect_timeout);
    manager.connect("remote").await.unwrap();
    let connected = tokio::time::timeout(connect_timeout, async {
        loop {
            let status = manager.status().await;
            if status["status"] == "connected" || status["status"] == "error" {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("connection reaches a terminal state within the configured timeout");
    assert_eq!(
        connected["status"],
        "connected",
        "unexpected terminal connection error: {}",
        connected.get("error").unwrap_or(&serde_json::Value::Null)
    );
    tokio::time::sleep(connect_timeout + Duration::from_millis(100)).await;
    assert!(manager.owns("remote").await);
    assert_eq!(store.load_profiles().unwrap().last_used, "remote");
    manager.close_all().await;
    assert!(!manager.owns("remote").await);
    assert!(store.active_snapshot().unwrap().is_empty());
}
#[tokio::test]
async fn launcher_ui_auth_shapes_and_real_loopback_asset() {
    use many_ai_cli::hub::http::Request;
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let manager = ConnectionManager::new(
        store.clone(),
        ConnectorConfig::new(store.paths.clone(), t.path().into()),
    );
    let server = UiServer::from_token(manager, "synthetic-ui".into(), 49001).unwrap();
    let mut request = Request {
        path: "/api/profiles".into(),
        method: "POST".into(),
        host: "evil.example".into(),
        body: b"{broken".to_vec(),
        ..Request::default()
    };
    assert_eq!(
        server
            .handle(&request, &Cancellation::default())
            .await
            .status,
        401
    );
    request.query = "token=synthetic-ui".into();
    assert_eq!(
        server
            .handle(&request, &Cancellation::default())
            .await
            .status,
        403
    );
    request.host = "localhost:49001".into();
    request.body = b"{\"profiles\":[]} trailing".to_vec();
    assert_eq!(
        server
            .handle(&request, &Cancellation::default())
            .await
            .status,
        200
    );
    request.method = "GET".into();
    let response = server.handle(&request, &Cancellation::default()).await;
    assert_eq!(response.status, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["profiles"], Value::Null);
    assert!(body.get("active").is_none());
    let stop = Cancellation::default();
    let mut running = server.serve(stop.clone()).await.unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client.get(&running.url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.headers().get("Content-Security-Policy").is_none());
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        many_ai_cli::assets::LAUNCHER_UI
    );
    stop.cancel();
    running.wait().await.unwrap();
}
#[test]
fn launcher_tokens_scanner_limits_and_browser_argv() {
    for token in ["", "space value", "line\nvalue", "x\u{0085}"] {
        assert!(normalize_hub_token(token).is_err());
    }
    assert_eq!(normalize_hub_token("a+b&日本語").unwrap(), "a+b&日本語");
    let mut scanner = UrlScanner::default();
    assert!(scanner.feed(&vec![b'x'; 1024 * 1024], false).is_empty());
    assert!(
        scanner
            .feed(b"\nhttp://127.0.0.1:49001/?token=abc\n", true)
            .is_empty()
    );
    let (_, args) = browser_command("http://127.0.0.1:49001/?token=abc&via=ssh");
    assert_eq!(
        args.last().unwrap(),
        "http://127.0.0.1:49001/?token=abc&via=ssh"
    );
    assert!(hub_endpoint("http://example.invalid:49001/?token=abc", "/api/info").is_err());
}

#[cfg(unix)]
#[test]
fn launcher_ssh_join_and_remote_shell_deliver_exact_argv_env_cwd() {
    use std::process::Command;
    let t = tempfile::tempdir().unwrap();
    let cwd = t.path().join("working directory ' 日本語;");
    std::fs::create_dir(&cwd).unwrap();
    let binary = t.path().join("candidate ' ; 日本語\napp");
    executable(
        &binary,
        "#!/usr/bin/env python3\nimport json,os,sys\nprint(json.dumps({'argv':sys.argv[1:],'cwd':os.getcwd(),'label':os.environ.get('MANY_AI_CLI_HOST_LABEL')}))\n",
    );
    let fake_bin = t.path().join("fake-bin");
    std::fs::create_dir(&fake_bin).unwrap();
    // Simulate bash -ilc argv, but do NOT read a login shell or real home.
    executable(
        &fake_bin.join("bash"),
        "#!/bin/sh\n[ \"$#\" = 2 ] || exit 90\ncase \"$1\" in -ilc|-lc|-c) ;; *) exit 91;; esac\nexec /bin/bash --noprofile --norc -c \"$2\"\n",
    );
    let p = Profile {
        binary: binary.to_string_lossy().into_owned(),
        cwd: cwd.to_string_lossy().into_owned(),
        host: "host';literal&日本語".into(),
        ..ssh()
    };
    p.validate().unwrap();
    let args = ssh_serve_args(&p, 49001, None);
    let start = args.iter().position(|s| s == "--").unwrap() + 1;
    let joined = args[start..].join(" ");
    let path = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let result = Command::new("/bin/sh")
        .args(["-c", &joined])
        .env("PATH", &path)
        .env_remove("BASH_ENV")
        .current_dir(t.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "remote shell status: {:?}",
        result.status
    );
    let observed: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(observed["argv"], json!(["serve", "--port", "49001"]));
    assert_eq!(
        observed["cwd"],
        cwd.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    assert_eq!(observed["label"], p.host);
    // Actual Go fixed-SHA script lacks the outer quote. The same shell boundary
    // must fail to deliver the intended operation, documenting the required fix.
    let mut broken = args[start..].to_vec();
    *broken.last_mut().unwrap() = ssh_serve_script(&p, 49001, None);
    let result = Command::new("/bin/sh")
        .args(["-c", &broken.join(" ")])
        .env("PATH", &path)
        .env_remove("BASH_ENV")
        .current_dir(t.path())
        .output()
        .unwrap();
    assert!(!result.status.success());
    let imported = ssh_import_args(&p, None, 49001);
    let start = imported.iter().position(|s| s == "--").unwrap() + 1;
    let result = Command::new("/bin/sh")
        .args(["-c", &imported[start..].join(" ")])
        .env("PATH", path)
        .env_remove("BASH_ENV")
        .current_dir(t.path())
        .output()
        .unwrap();
    assert!(result.status.success());
    let observed: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(observed["argv"], json!(["profile-export", "--json"]));
}

#[cfg(unix)]
#[tokio::test]
async fn launcher_fake_tunnel_opaque_token_readiness_redirect_and_quiet() {
    use axum::{
        Router,
        routing::{get, post},
    };
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("candidate");
    let installed = t.path().join("installed");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&installed).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let store = LauncherStore::open(RuntimePaths::trial(&root, port, &installed).unwrap()).unwrap();
    let router = Router::new()
        .route(
            "/api/info",
            get(|uri: axum::http::Uri| async move {
                assert_eq!(uri.query().unwrap(), "token=a%2Bb%26x%3D1");
                "{}"
            }),
        )
        .route("/api/net-hint", post(|| async { "{}" }));
    let stop = Cancellation::default();
    let server_stop = stop.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { server_stop.cancelled().await })
            .await
            .unwrap();
    });
    let mut config = stub_config(&store, t.path(), "ready");
    config.poll_interval = Duration::from_millis(20);
    let p = Profile {
        mode: "tunnel".into(),
        hub_port: i64::from(port),
        token_command: "printf synthetic".into(),
        ..ssh()
    };
    let mut connection = config.start(p).unwrap();
    let url = tokio::time::timeout(Duration::from_secs(3), connection.ready())
        .await
        .unwrap()
        .unwrap();
    assert!(url.contains(
        "?token=a%2Bb%26x%3D1&via=ssh&host_label=example.invalid&env_kind=remote-tunnel"
    ));
    connection.cancel();
    connection.wait().await.unwrap();
    stop.cancel();
    server.await.unwrap();
    let args = std::fs::read_to_string(t.path().join("argv.jsonl")).unwrap();
    assert!(
        !args.contains("pkill"),
        "tunnel-only teardown must not stop persistent Hub"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn launcher_ui_disconnect_failure_still_cleans_owned_only() {
    use axum::{
        Router,
        routing::{get, post},
    };
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let kill_calls = calls.clone();
    let stop_calls = calls.clone();
    let router = Router::new()
        .route("/api/info", get(|| async { "{}" }))
        .route(
            "/api/kill-all",
            post(move || {
                let calls = kill_calls.clone();
                async move {
                    calls.lock().unwrap().push("kill");
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR
                }
            }),
        )
        .route(
            "/api/shutdown",
            post(move || {
                let calls = stop_calls.clone();
                async move {
                    calls.lock().unwrap().push("shutdown");
                    axum::http::StatusCode::BAD_GATEWAY
                }
            }),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut p = ssh();
    p.hub_port = i64::from(port);
    store
        .save_profiles(&ProfilesFile {
            profiles: Some(vec![p]),
            ..ProfilesFile::fresh()
        })
        .unwrap();
    let config = stub_config(&store, t.path(), "ready");
    let manager = ConnectionManager::new(store.clone(), config);
    manager.connect("remote").await.unwrap();
    for _ in 0..100 {
        if manager.owns("remote").await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(manager.owns("remote").await);
    let result = manager
        .disconnect("remote", "all", &Cancellation::default(), "launcher")
        .await
        .unwrap_err();
    assert_eq!(result.status, 502);
    assert!(!manager.owns("remote").await);
    assert!(store.active_snapshot().unwrap().is_empty());
    assert_eq!(*calls.lock().unwrap(), ["kill", "shutdown"]);
    manager.close_all().await;
    server.abort();
}

#[test]
fn launcher_delivery_target_channel_contract_and_hash_readback() {
    use many_ai_cli::launcher::delivery::*;
    use sha2::{Digest, Sha256};
    let t = tempfile::tempdir().unwrap();
    let make = |path: String| {
        let full = t.path().join(&path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        let bytes = format!("synthetic artifact contract fixture: {path}").into_bytes();
        std::fs::write(full, &bytes).unwrap();
        ArtifactFile {
            path,
            sha256: Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            bytes: bytes.len() as u64,
        }
    };
    let mut manifest = DeliveryManifest {
        schema: 1,
        oracle: ORACLE.into(),
        source_revision: "a".repeat(40),
        lockfile_sha256: "b".repeat(64),
        version: "0.0.0-rust-candidate".into(),
        toolchain: "rustc 1.90.0 (synthetic metadata)".into(),
        build_time: "2026-10-03T00:00:00Z".into(),
        binaries: vec![],
        embedded_assets: vec![],
        runtimes: vec![],
    };
    for target in Target::ALL {
        for launcher in [false, true] {
            manifest.binaries.push(BinaryArtifact {
                target,
                launcher,
                file: make(format!("{}/{}", target.suffix(), target.binary(launcher))),
            });
        }
        let npm = package_files(target, Channel::Npm).unwrap();
        assert_eq!(npm.len(), 2);
        assert!(!npm.iter().any(|p| p.contains("launcher")));
        let zip = package_files(target, Channel::Zip).unwrap();
        assert!(zip.contains(&target.binary(true)));
        assert_eq!(
            zip.contains(&"unblock-windows.cmd".into()),
            target == Target::WindowsX64
        );
    }
    for asset in ["web/dist/index.html", "internal/launcher/ui/index.html"] {
        manifest.embedded_assets.push(make(asset.into()));
    }
    assert!(
        manifest.validate().is_err(),
        "missing verified runtime input must block packaging"
    );
    for name in [
        "vcomp140.dll",
        "msvcp140.dll",
        "vcruntime140.dll",
        "vcruntime140_1.dll",
    ] {
        manifest.runtimes.push(RuntimeArtifact {
            target: Target::WindowsX64,
            identity: "synthetic-runtime".into(),
            license: "synthetic-license".into(),
            verification: "synthetic-only, no real signature claim".into(),
            file: make(format!("runtime/{name}")),
        });
    }
    manifest.validate().unwrap();
    manifest.oracle = "21d0bc7935a2c4696fb89ccff2e324157a528c2d".into();
    assert!(
        manifest.validate().is_err(),
        "the previous oracle cannot label a version25 candidate"
    );
    manifest.oracle = ORACLE.into();
    manifest.verify_files(t.path()).unwrap();
    let artifacts = manifest
        .goreleaser_artifacts("rust/candidate-dist")
        .unwrap();
    assert_eq!(artifacts.as_array().unwrap().len(), 8);
    assert_eq!(
        artifacts
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["extra"]["ID"] == "many-ai-cli")
            .count(),
        4
    );
    assert!(package_files(Target::MacosIntel, Channel::Deb).is_err());
    assert!(package_files(Target::LinuxX64, Channel::Homebrew).is_err());
    assert!(package_files(Target::MacosAppleSilicon, Channel::Winget).is_err());
    std::fs::write(t.path().join(&manifest.binaries[0].file.path), b"tampered").unwrap();
    assert!(manifest.verify_files(t.path()).is_err());
    manifest.binaries.pop();
    assert!(manifest.validate().is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn launcher_trial_cleanup_distinguishes_profiles_even_on_same_port() {
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let config = stub_config(&store, t.path(), "ready");
    let mut p = ssh();
    p.hub_port = 49001;
    let mut other = p.clone();
    other.name = "second-profile".into();
    let mut first = config.start(p).unwrap();
    let mut second = config.start(other).unwrap();
    first.ready().await.unwrap();
    second.ready().await.unwrap();
    first.cancel();
    first.wait().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), second.wait())
            .await
            .is_err()
    );
    second.cancel();
    second.wait().await.unwrap();
    let args: Vec<Vec<String>> = std::fs::read_to_string(t.path().join("argv.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let marker = regex::Regex::new(r"many-ai-connection=([0-9a-f]{64})").unwrap();
    let identities: Vec<_> = args
        .iter()
        .filter(|a| a.iter().any(|s| s == "-t"))
        .map(|a| marker.captures(a.last().unwrap()).unwrap()[1].to_owned())
        .collect();
    assert_eq!(identities.len(), 2);
    assert_ne!(identities[0], identities[1]);
    let patterns: Vec<_> = args
        .iter()
        .filter(|a| a.iter().any(|s| s == "pkill"))
        .map(|a| regex::Regex::new(a.last().unwrap().trim_matches('\'')).unwrap())
        .collect();
    assert_eq!(patterns.len(), 2);
    let commandlines:Vec<_>=identities.iter().map(|id|format!("/synthetic/candidate/many-ai-cli [many-ai-connection={id}] --trial-root /synthetic/candidate --trial-port 49001 serve --port 49001")).collect();
    for pattern in patterns {
        assert_eq!(
            commandlines.iter().filter(|s| pattern.is_match(s)).count(),
            1
        );
        assert!(!pattern.is_match("many-ai-cli serve --port 49001"));
    }
}

#[test]
fn launcher_registry_subprocess_helper() {
    let Some(root) = std::env::var_os("MANY_TEST_LAUNCHER_ROOT") else {
        return;
    };
    let Some(name) = std::env::var_os("MANY_TEST_LAUNCHER_PROFILE") else {
        panic!("missing synthetic helper profile");
    };
    let store = store_at(Path::new(&root));
    store
        .register_active(&name.to_string_lossy(), "http://127.0.0.1:49001/?token=abc")
        .unwrap();
}
#[test]
fn launcher_registry_cross_process_updates_do_not_lose_records() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("candidate");
    let store = store_at(&root);
    let mut children = Vec::new();
    for n in 0..6 {
        children.push(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "launcher_registry_subprocess_helper",
                    "--nocapture",
                ])
                .env("MANY_TEST_LAUNCHER_ROOT", &root)
                .env("MANY_TEST_LAUNCHER_PROFILE", format!("child-{n}"))
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "synthetic helper failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let records = store.active_snapshot().unwrap();
    assert_eq!(records.len(), 6);
    let names: std::collections::BTreeSet<_> = records.iter().map(|r| r.profile.clone()).collect();
    for n in 0..6 {
        assert!(names.contains(&format!("child-{n}")));
    }
    // Unregistering this process cannot delete other processes' records.
    store.unregister_all().unwrap();
    assert_eq!(store.active_snapshot().unwrap().len(), 6);
}

#[cfg(unix)]
#[tokio::test]
async fn launcher_manager_superseded_attempt_cannot_overwrite_or_remove_new_connection() {
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let slow = Profile {
        name: "slow".into(),
        host: "silent.example".into(),
        hub_port: 49001,
        ..ssh()
    };
    let fast = Profile {
        name: "fast".into(),
        hub_port: 49101,
        ..ssh()
    };
    let third = Profile {
        name: "third".into(),
        hub_port: 49201,
        ..ssh()
    };
    store
        .save_profiles(&ProfilesFile {
            profiles: Some(vec![slow, fast, third]),
            ..ProfilesFile::fresh()
        })
        .unwrap();
    let config = stub_config(&store, t.path(), "ready");
    let manager = ConnectionManager::new(store.clone(), config);
    manager.connect("slow").await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    manager.connect("fast").await.unwrap();
    for _ in 0..200 {
        if manager.owns("fast").await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(manager.owns("fast").await);
    assert!(!manager.owns("slow").await);
    assert_eq!(manager.status().await["status"], "connected");
    manager.connect("third").await.unwrap();
    for _ in 0..200 {
        if manager.owns("third").await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(manager.owns("fast").await);
    assert!(manager.owns("third").await);
    manager.connect("fast").await.unwrap();
    for _ in 0..200 {
        if manager.owns("fast").await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(manager.owns("fast").await);
    assert!(manager.owns("third").await);
    let records = store.active_snapshot().unwrap();
    assert_eq!(records.iter().filter(|r| r.profile == "fast").count(), 1);
    assert_eq!(records.iter().filter(|r| r.profile == "third").count(), 1);
    manager.close_all().await;
    assert!(store.active_snapshot().unwrap().is_empty());
}

#[tokio::test]
async fn launcher_registry_readiness_rejects_redirect_without_following_token() {
    use axum::{Router, routing::get};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let followed = Arc::new(AtomicUsize::new(0));
    let observed = followed.clone();
    let router = Router::new()
        .route(
            "/api/info",
            get(|| async { axum::response::Redirect::temporary("/elsewhere") }),
        )
        .route(
            "/elsewhere",
            get(move || {
                let observed = observed.clone();
                async move {
                    observed.fetch_add(1, Ordering::SeqCst);
                    "{}"
                }
            }),
        );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let url = format!("http://127.0.0.1:{port}/?token=abc123");
    assert!(!probe_hub(&url, Duration::from_secs(1)).await);
    store.register_active("redirected", &url).unwrap();
    assert!(store.active_pruned().await.unwrap().is_empty());
    assert_eq!(followed.load(Ordering::SeqCst), 0);
    server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn launcher_cli_caller_reuses_active_and_cleans_on_signal_cancellation() {
    use axum::{Router, routing::get};
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/api/info", get(|| async { "{}" })),
        )
        .await
        .unwrap();
    });
    let output = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = output.clone();
    let mut config = stub_config(&store, t.path(), "ready");
    config.output = Some(Arc::new(move |_, bytes| {
        sink.lock().unwrap().extend_from_slice(bytes)
    }));
    let mut profile = ssh();
    profile.hub_port = i64::from(port);
    let opens = Arc::new(std::sync::Mutex::new(Vec::new()));
    let first_opens = opens.clone();
    let cancel = Cancellation::default();
    let first_cancel = cancel.clone();
    let first_store = store.clone();
    let first_config = config.clone();
    let first_profile = profile.clone();
    let first = tokio::spawn(async move {
        connect_cli(
            &first_store,
            &first_config,
            first_profile,
            &first_cancel,
            &move |url| {
                first_opens.lock().unwrap().push(url.to_owned());
                Ok(())
            },
        )
        .await
    });
    for _ in 0..200 {
        if !store.active_snapshot().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(store.active_snapshot().unwrap().len(), 1);
    let second_opens = opens.clone();
    connect_cli(
        &store,
        &config,
        profile,
        &Cancellation::default(),
        &move |url| {
            second_opens.lock().unwrap().push(url.to_owned());
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_eq!(opens.lock().unwrap().len(), 2);
    cancel.cancel();
    first.await.unwrap().unwrap();
    assert!(store.active_snapshot().unwrap().is_empty());
    assert!(
        store.load_profiles().unwrap().last_used.is_empty(),
        "Go direct connect doesn't update last_used"
    );
    let output = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(output.contains("already connected — reusing"));
    server.abort();
}

#[tokio::test]
async fn launcher_ui_asset_head_range_and_conditional_match_servefile() {
    use many_ai_cli::hub::http::Request;
    let t = tempfile::tempdir().unwrap();
    let store = store_at(&t.path().join("candidate"));
    let manager = ConnectionManager::new(
        store.clone(),
        ConnectorConfig::new(store.paths.clone(), t.path().into()),
    );
    let server = UiServer::from_token(manager, "synthetic-ui".into(), 49001).unwrap();
    let mut request = Request {
        method: "GET".into(),
        path: "/".into(),
        query: "token=synthetic-ui".into(),
        host: "localhost:49001".into(),
        ..Request::default()
    };
    request.headers.push(("Range".into(), "bytes=0-3".into()));
    let response = server.handle(&request, &Cancellation::default()).await;
    assert_eq!(response.status, 206);
    assert_eq!(response.body, &many_ai_cli::assets::LAUNCHER_UI[..4]);
    request.method = "HEAD".into();
    let response = server.handle(&request, &Cancellation::default()).await;
    assert_eq!(response.status, 206);
    assert_eq!(response.headers["Content-Length"], "4");
    assert!(response.body.is_empty());
    request.method = "GET".into();
    request.headers = vec![("Range".into(), "bytes=0-3,8-10".into())];
    let response = server.handle(&request, &Cancellation::default()).await;
    assert_eq!(response.status, 206);
    assert!(response.headers["Content-Type"].starts_with("multipart/byteranges; boundary="));
    request.headers = vec![("Range".into(), "bytes=999999999-".into())];
    assert_eq!(
        server
            .handle(&request, &Cancellation::default())
            .await
            .status,
        416
    );
    request.headers = vec![("If-None-Match".into(), "*".into())];
    assert_eq!(
        server
            .handle(&request, &Cancellation::default())
            .await
            .status,
        304
    );
}
