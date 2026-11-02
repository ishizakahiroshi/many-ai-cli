//! Supplemental negative cases use task-owned sockets and the actual core.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct CountingObserver(AtomicUsize);
impl OrderedEventObserver for CountingObserver {
    fn observe<'a>(&'a self, _: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

async fn register_wrapper(f: &Fixture, pid: i64) -> (Client, serde_json::Value, SessionBinding) {
    let (mut client, status) = Client::connect(f.port, "").await;
    assert!(status.starts_with("HTTP/1.1 101"));
    client
        .json(serde_json::json!({"type":"register","provider":"shell",
            "cwd":f._root.path(),"pid":pid,"token":"synthetic-websocket-token"}))
        .await;
    let ack = client.until("registered").await;
    let binding = f
        .core
        .details(LiveSessionId(ack["session_id"].as_i64().unwrap()))
        .unwrap()
        .binding;
    (client, ack, binding)
}

async fn ui_client(f: &Fixture) -> Client {
    let (mut client, status) = Client::connect(f.port, "").await;
    assert!(status.starts_with("HTTP/1.1 101"));
    client
        .json(serde_json::json!({"role":"ui","token":"synthetic-websocket-token"}))
        .await;
    client.until("approval_snapshot").await;
    client
}

async fn wait_for_permits(f: &Fixture, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while f.websocket.preauth.available_permits() != count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn upgrade_host_and_remote_origin_rejections_emit_no_business_effects() {
    let observer = Arc::new(CountingObserver::default());
    let f = fixture_with_observer(observer.clone()).await;
    let revision = f.services.config.snapshot().unwrap().revision;
    let (_, status) = Client::connect_host(f.port, "rejected.example", "").await;
    assert!(status.starts_with("HTTP/1.1 403"), "{status}");
    let (_, status) = Client::connect(f.port, "X-Forwarded-Proto: https\r\n").await;
    assert!(status.starts_with("HTTP/1.1 403"), "{status}");
    assert_eq!(observer.0.load(Ordering::SeqCst), 0);
    assert!(f.core.snapshots().is_empty());
    assert_eq!(f.services.config.snapshot().unwrap().revision, revision);
    assert_eq!(f.websocket.preauth.available_permits(), MAX_PREAUTH_SOCKETS);
    let (_wrapper, _, _) = register_wrapper(&f, 401).await;
    assert_eq!(f.core.snapshots().len(), 1);
    assert!(observer.0.load(Ordering::SeqCst) > 0);
    f.stop().await;
}

#[tokio::test]
async fn complete_json_without_token_and_real_preauth_capacity_never_register_sessions() {
    let observer = Arc::new(CountingObserver::default());
    let f = fixture_with_observer(observer.clone()).await;
    for first in [
        serde_json::json!({"role":"ui"}),
        serde_json::json!({"type":"register","provider":"shell","pid":402}),
    ] {
        let (mut peer, status) = Client::connect(f.port, "").await;
        assert!(status.starts_with("HTTP/1.1 101"));
        peer.json(first).await;
        assert!(peer.frame().await.is_none());
        wait_for_permits(&f, MAX_PREAUTH_SOCKETS).await;
        assert!(f.core.snapshots().is_empty());
        assert_eq!(observer.0.load(Ordering::SeqCst), 0);
    }
    let mut peers = Vec::new();
    for _ in 0..MAX_PREAUTH_SOCKETS {
        let (peer, status) = Client::connect(f.port, "").await;
        assert!(status.starts_with("HTTP/1.1 101"));
        peers.push(peer);
    }
    assert_eq!(f.websocket.preauth.available_permits(), 0);
    let (_, status) = Client::connect(f.port, "").await;
    assert!(status.starts_with("HTTP/1.1 429"), "{status}");
    assert!(f.core.snapshots().is_empty());
    assert_eq!(observer.0.load(Ordering::SeqCst), 0);
    drop(peers.pop().unwrap());
    wait_for_permits(&f, 1).await;
    let (_wrapper, _, _) = register_wrapper(&f, 403).await;
    wait_for_permits(&f, 1).await;
    assert_eq!(f.core.snapshots().len(), 1);
    drop(peers);
    wait_for_permits(&f, MAX_PREAUTH_SOCKETS).await;
    f.stop().await;
}

async fn control_only_until_closed(peer: Client, send_ping: bool) -> (usize, usize) {
    let (mut reader, mut writer) = peer.0.into_split();
    let incoming = async {
        let mut pongs = 0;
        loop {
            let first = match reader.read_u8().await {
                Ok(first) => first,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                    ) =>
                {
                    return pongs;
                }
                Err(error) => panic!("preauthentication socket read: {error}"),
            };
            let second = reader.read_u8().await.unwrap();
            assert_eq!(second & 0x80, 0, "server frames must not be masked");
            let opcode = first & 0x0f;
            assert!(
                matches!(opcode, 8 | 10),
                "an unauthenticated peer must not receive application frames"
            );
            assert!(
                second & 0x7f <= 125,
                "control frames cannot use extended lengths"
            );
            let mut payload = vec![0; (second & 0x7f) as usize];
            reader.read_exact(&mut payload).await.unwrap();
            if opcode == 8 {
                return pongs;
            }
            assert!(
                payload.is_empty(),
                "the synthetic Ping has an empty payload"
            );
            pongs += 1;
        }
    };
    tokio::pin!(incoming);
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    let mut sent = 0;
    let mut sending = send_ping;
    loop {
        tokio::select! {
            pongs = &mut incoming => return (sent, pongs),
            _ = interval.tick(), if sending => {
                // FIN+Ping, client mask, empty payload and a deterministic
                // framing-only mask. No JSON authentication is ever sent.
                if writer.write_all(&[0x89, 0x80, 1, 2, 3, 4]).await.is_ok() {
                    sent += 1;
                } else {
                    // The read side still proves the server closed; a failed
                    // write by itself is not the deadline assertion.
                    sending = false;
                }
            }
        }
    }
}

#[tokio::test]
async fn real_silent_and_ping_only_peers_hit_the_absolute_first_frame_deadline_and_release_capacity()
 {
    let observer = Arc::new(CountingObserver::default());
    let f = fixture_with_observer(observer.clone()).await;
    let (silent, silent_status) = Client::connect(f.port, "").await;
    assert!(silent_status.starts_with("HTTP/1.1 101"));
    let silent_started = tokio::time::Instant::now();
    let (control, control_status) = Client::connect(f.port, "").await;
    assert!(control_status.starts_with("HTTP/1.1 101"));
    let control_started = tokio::time::Instant::now();
    assert_eq!(
        f.websocket.preauth.available_permits(),
        MAX_PREAUTH_SOCKETS - 2
    );
    let silent_wait = async {
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            control_only_until_closed(silent, false),
        )
        .await
        .expect("the silent peer must close at its first-frame deadline");
        (result, silent_started.elapsed())
    };
    let control_wait = async {
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            control_only_until_closed(control, true),
        )
        .await
        .expect("continuous Ping frames must not extend the first-frame deadline");
        (result, control_started.elapsed())
    };
    let (((silent_sent, silent_pongs), silent_elapsed), ((sent, pongs), control_elapsed)) =
        tokio::join!(silent_wait, control_wait);
    assert_eq!((silent_sent, silent_pongs), (0, 0));
    assert!(
        sent >= 3,
        "the control peer must actively send repeated Pings"
    );
    assert!(
        pongs >= 1,
        "the server must accept control framing before timeout"
    );
    for elapsed in [silent_elapsed, control_elapsed] {
        assert!(
            elapsed >= PREAUTH_TIMEOUT.saturating_sub(Duration::from_secs(1)),
            "early framing rejection must not masquerade as an authentication timeout"
        );
        assert!(elapsed < Duration::from_secs(15));
    }
    wait_for_permits(&f, MAX_PREAUTH_SOCKETS).await;
    assert!(f.core.snapshots().is_empty());
    assert_eq!(observer.0.load(Ordering::SeqCst), 0);
    let (_wrapper, _, _) = register_wrapper(&f, 407).await;
    wait_for_permits(&f, MAX_PREAUTH_SOCKETS).await;
    assert_eq!(f.core.snapshots().len(), 1);
    assert!(observer.0.load(Ordering::SeqCst) > 0);
    f.stop().await;
}

#[tokio::test]
async fn forged_wrapper_session_ids_cannot_redirect_output_ack_or_end_to_another_session() {
    let f = fixture().await;
    let (mut first, _, a) = register_wrapper(&f, 404).await;
    let (mut second, _, b) = register_wrapper(&f, 405).await;
    let mut ui = ui_client(&f).await;
    // Enable ACK capability synchronously so each queue retains its first send.
    f.core.acknowledge(a, InputSeq(0));
    f.core.acknowledge(b, InputSeq(0));
    for binding in [a, b] {
        ui.json(
            serde_json::json!({"type":"pty_input","session_id":binding.session.0,"text":"queued"}),
        )
        .await;
    }
    let a_input = first.until("pty_input").await;
    let b_input = second.until("pty_input").await;
    let b_before = f.core.details(b.session).unwrap().snapshot;
    first
        .json(serde_json::json!({"type":"pty_input_ack","session_id":b.session.0,"input_seq":a_input["input_seq"]}))
        .await;
    first
        .json(serde_json::json!({"type":"pty_data","session_id":b.session.0,"data":[65,45,111,110,108,121]}))
        .await;
    // Same socket ordering makes this output a barrier after the forged ACK.
    let output = ui.until("pty_data").await;
    assert_eq!(output["session_id"], a.session.0);
    assert_eq!(output["data"], "QS1vbmx5");
    assert_eq!(
        f.core
            .acknowledge(a, InputSeq(a_input["input_seq"].as_i64().unwrap())),
        AckDisposition::CapabilityObserved
    );
    assert_eq!(
        f.core
            .acknowledge(b, InputSeq(b_input["input_seq"].as_i64().unwrap())),
        AckDisposition::Removed,
        "forged ACK must leave the other session's inflight entry intact"
    );
    assert!(f.core.details(b.session).unwrap().snapshot == b_before);
    first
        .json(serde_json::json!({"type":"session_end","session_id":b.session.0,"state":"completed","exit_code":7,"reason":"synthetic first only"}))
        .await;
    // The engine's explicit completion event is the barrier; a declared end
    // need not publish a wire session_end until transport disconnect.
    tokio::time::timeout(Duration::from_secs(2), async {
        while f.core.details(a.session).unwrap().snapshot.state != "completed" {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(f.core.details(b.session).unwrap().connected);
    assert!(f.core.details(b.session).unwrap().snapshot == b_before);
    ui.json(serde_json::json!({"type":"pty_input","session_id":b.session.0,"text":"survives"}))
        .await;
    assert_eq!(second.until("pty_input").await["data"], "c3Vydml2ZXM=");
    f.stop().await;
}

#[tokio::test]
async fn reattached_wrapper_rejects_stale_core_output_ack_and_disconnect_without_losing_queue() {
    let f = fixture().await;
    let (mut old, ack, old_binding) = register_wrapper(&f, 406).await;
    let mut ui = ui_client(&f).await;
    f.core.acknowledge(old_binding, InputSeq(0));
    ui.json(serde_json::json!({"type":"pty_input","session_id":old_binding.session.0,"text":"retained"}))
        .await;
    let original = old.until("pty_input").await;
    let (mut replacement, _) = Client::connect(f.port, "").await;
    replacement
        .json(
            serde_json::json!({"type":"reattach","session_id":old_binding.session.0,
            "provider":"shell","cwd":f._root.path(),"pid":406,
            "started_at":ack["started_at"],"token":"synthetic-websocket-token"}),
        )
        .await;
    assert_eq!(replacement.frame().await.unwrap()["type"], "reattach_ack");
    let replay = replacement.until("pty_input").await;
    assert_eq!(replay["input_seq"], original["input_seq"]);
    assert_eq!(replay["data"], original["data"]);
    assert!(old.frame().await.is_none());
    let current = f.core.details(old_binding.session).unwrap();
    assert_ne!(current.binding.wrapper, old_binding.wrapper);
    assert!(matches!(
        f.core.observe_output(
            old_binding,
            OutputChunk {
                bytes: b"stale-output".to_vec(),
                total_pty_bytes: 12
            },
            Timestamp::now()
        ),
        Err(SessionError::StaleBinding)
    ));
    let seq = InputSeq(replay["input_seq"].as_i64().unwrap());
    assert_eq!(
        f.core.acknowledge(old_binding, seq),
        AckDisposition::WrongConnection
    );
    f.effects
        .apply(f.core.disconnected(old_binding, Timestamp::now()).unwrap())
        .await
        .unwrap();
    let after = f.core.details(old_binding.session).unwrap();
    assert_eq!(after.binding, current.binding);
    assert_eq!(after.last_output_at, current.last_output_at);
    assert!(after.snapshot == current.snapshot);
    assert!(after.connected);
    assert_eq!(
        f.core.acknowledge(current.binding, seq),
        AckDisposition::Removed
    );
    replacement
        .json(serde_json::json!({"type":"pty_data","data":[102,114,101,115,104]}))
        .await;
    let output = ui.until("pty_data").await;
    assert_eq!(output["session_id"], current.binding.session.0);
    assert_eq!(output["data"], "ZnJlc2g=");
    ui.json(serde_json::json!({"type":"pty_input","session_id":current.binding.session.0,"text":"still writable"}))
        .await;
    assert_eq!(
        replacement.until("pty_input").await["data"],
        "c3RpbGwgd3JpdGFibGU="
    );
    f.stop().await;
}
