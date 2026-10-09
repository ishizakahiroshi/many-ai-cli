use super::*;
fn request(ip: &str, ua: &str) -> Request {
    Request {
        remote_addr: ip.into(),
        host: "tailnet.example".into(),
        headers: vec![("User-Agent".into(), ua.into())],
        ..Default::default()
    }
}
fn at(seconds: i64) -> Timestamp {
    Timestamp::from_unix(seconds, 0).unwrap()
}
#[test]
fn source_brand_order_and_minor_version_dedup() {
    let first = request("203.0.113.7:4012", "Windows Chrome/123 Safari/10 Edg/123");
    let mut next = first.clone();
    next.remote_addr = "203.0.113.7:5011".into();
    next.headers[0].1 = "Windows Chrome/124 Safari/11 Edg/124".into();
    assert_eq!(user_agent_bucket(first.header("User-Agent")), "edg/windows");
    assert_eq!(request_device_hash(&first), request_device_hash(&next));
    assert_eq!(request_device_hash(&first).len(), 16);
    let devices = Mutex::default();
    assert!(admit(&devices, &first, "page", at(0)).is_some());
    assert!(admit(&devices, &next, "page", at(1)).is_none());
    assert!(admit(&devices, &first, "ws", at(2)).is_none());
    assert_eq!(
        devices.lock().unwrap().values().next().copied(),
        Some(at(2))
    );
}
#[test]
fn source_loopback_proxy_identity_and_ttl_boundary() {
    let devices = Mutex::default();
    let mut local = request("127.0.0.1:4321", "curl/8");
    local.host = "localhost:1234".into();
    assert!(admit(&devices, &local, "page", at(0)).is_none());
    local
        .headers
        .push(("Tailscale-User-Login".into(), " Alice@Example.Com ".into()));
    let first = request_device_hash(&local);
    assert!(admit(&devices, &local, "page", at(0)).is_some());
    local.headers[1].1 = "alice@example.com".into();
    assert_eq!(first, request_device_hash(&local));
    assert!(admit(&devices, &local, "ws", at(86400)).is_none());
    assert!(admit(&devices, &local, "ws", at(172801)).is_some());
    local.headers[1].1 = "bob@example.com".into();
    assert_ne!(first, request_device_hash(&local));
}
#[test]
fn source_bounded_oldest_device_and_utf8_byte_slice() {
    let devices = Mutex::default();
    for i in 0..256 {
        assert!(
            admit(
                &devices,
                &request(&format!("192.0.2.{i}:123"), "curl/8"),
                "page",
                at(i)
            )
            .is_some()
        );
    }
    let first = request("192.0.2.0:123", "curl/8");
    assert!(
        admit(
            &devices,
            &request("198.51.100.0:123", "curl/8"),
            "page",
            at(256)
        )
        .is_some()
    );
    assert_eq!(devices.lock().unwrap().len(), 256);
    assert!(
        !devices
            .lock()
            .unwrap()
            .contains_key(&request_device_hash(&first))
    );
    let ua = format!("{}あ", "x".repeat(79));
    let (_, body) = admit(
        &devices,
        &request("203.0.113.0:123", &ua),
        "pin_login",
        at(257),
    )
    .unwrap();
    assert!(body.contains(&format!("{}�", "x".repeat(79))));
    assert!(body.ends_with("via pin_login"));
}
