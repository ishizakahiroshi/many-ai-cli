use many_ai_cli::proto::time::Timestamp;
use many_ai_cli::{
    application::hub_runtime::{RuntimeData, RuntimeLedger, probe_hub_info},
    config::RuntimePaths,
    proto::decode_wire,
};
use serde_json::Value;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[test]
fn runtime_wire_matches_fixed_go_decoder() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/application/runtime/go.json")).unwrap();
    for case in cases {
        let decoded = decode_wire::<RuntimeData>(case["input"].as_str().unwrap().as_bytes());
        assert_eq!(
            decoded.is_err(),
            case["error"].as_bool().unwrap(),
            "{}",
            case["input"]
        );
        if let Ok(data) = decoded {
            assert_eq!(serde_json::to_value(&data).unwrap(), case["value"]);
            assert_eq!(
                data.pid > 0 && data.port > 0,
                case["valid"].as_bool().unwrap()
            );
        }
    }
}
#[test]
fn configured_probe_wins_and_dead_pid_cannot_authenticate_fallback() {
    let home = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::production(home.path()).unwrap();
    let ledger = RuntimeLedger::open(&paths).unwrap();
    ledger.write(48889, 42, Timestamp::UNIX_EPOCH).unwrap();
    let seen = Mutex::new(Vec::new());
    assert_eq!(
        ledger.running_port_with(
            48888,
            |_| panic!("configured success must precede PID"),
            |port| {
                seen.lock().unwrap().push(port);
                true
            }
        ),
        Some(48888)
    );
    assert_eq!(*seen.lock().unwrap(), [48888]);
    assert_eq!(
        ledger.running_port_with(48888, |pid| pid == 42, |port| port == 48889),
        Some(48889)
    );
    assert_eq!(ledger.running_port_with(48888, |_| false, |_| false), None);
    assert!(ledger.read().unwrap().is_none());
    ledger.write(48889, 43, Timestamp::UNIX_EPOCH).unwrap();
    ledger.remove_if_pid(42).unwrap();
    assert_eq!(ledger.read().unwrap().unwrap().pid, 43);
}
#[test]
fn trial_never_probes_installed_or_other_trial_ports() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49123, installed.path()).unwrap();
    let ledger = RuntimeLedger::open(&paths).unwrap();
    ledger.write(49123, 42, Timestamp::UNIX_EPOCH).unwrap();
    let seen = Mutex::new(Vec::new());
    assert_eq!(
        ledger.running_port_with(
            47777,
            |_| true,
            |port| {
                seen.lock().unwrap().push(port);
                true
            }
        ),
        Some(49123)
    );
    assert_eq!(*seen.lock().unwrap(), [49123]);
    assert!(ledger.write(49124, 42, Timestamp::UNIX_EPOCH).is_err());
}
#[cfg(unix)]
#[test]
fn held_runtime_directory_survives_replacement_and_private_helpers_refuse_links() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("trial");
    let moved = base.path().join("moved");
    let outside = base.path().join("outside");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let paths = RuntimePaths::trial(&root, 49123, &base.path().join("installed")).unwrap();
    let ledger = RuntimeLedger::open(&paths).unwrap();
    ledger.write(49123, 42, Timestamp::UNIX_EPOCH).unwrap();
    std::fs::rename(&root, &moved).unwrap();
    symlink(&outside, &root).unwrap();
    ledger.write(49123, 43, Timestamp::UNIX_EPOCH).unwrap();
    assert_eq!(ledger.read().unwrap().unwrap().pid, 43);
    assert!(!outside.join("hub-runtime.json").exists());
    assert_eq!(
        std::fs::metadata(moved.join("hub-runtime.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    ledger.remove_if_pid(43).unwrap();
    assert!(!moved.join("hub-runtime.json").exists());
}
#[test]
fn runtime_writers_serialize_across_independent_handles() {
    let home = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::production(home.path()).unwrap();
    let first = Arc::new(RuntimeLedger::open(&paths).unwrap());
    let second = Arc::new(RuntimeLedger::open(&paths).unwrap());
    let a = first.clone();
    let b = second.clone();
    let one = std::thread::spawn(move || {
        for _ in 0..20 {
            a.write(48888, 42, Timestamp::UNIX_EPOCH).unwrap();
        }
    });
    let two = std::thread::spawn(move || {
        for _ in 0..20 {
            b.write(48889, 43, Timestamp::UNIX_EPOCH).unwrap();
        }
    });
    one.join().unwrap();
    two.join().unwrap();
    let value = first.read().unwrap().unwrap();
    assert!([42, 43].contains(&value.pid));
}
#[tokio::test]
async fn authenticated_loopback_probe_does_not_follow_redirects() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let foreign = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let other = foreign.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 2048];
        let n = socket.read(&mut bytes).await.unwrap();
        assert!(
            String::from_utf8_lossy(&bytes[..n])
                .starts_with("GET /api/info?token=synthetic+runtime+token HTTP/1.1")
        );
        socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{other}/must-not-receive\r\nContent-Length: 0\r\n\r\n").as_bytes()).await.unwrap();
    });
    assert!(!probe_hub_info(port, "synthetic runtime token").await);
    server.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(40), foreign.accept())
            .await
            .is_err()
    );
}
