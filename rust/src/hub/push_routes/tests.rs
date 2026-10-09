use super::*;
fn request(method: &str, path: &str, body: &str) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        body: body.as_bytes().to_vec(),
        ..Default::default()
    }
}
#[test]
fn disabled_status_shape_and_unavailable_precede_json_decoder() {
    let http = PushHttp::new(None);
    let now = Timestamp::now();
    let status = http
        .handle_authenticated(&request("GET", "/api/push/status", "ignored"), now)
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"supported":false,"public_key":"","subscriptions":0})
    );
    assert_eq!(
        http.handle_authenticated(&request("POST", "/api/push/subscriptions", "invalid"), now)
            .unwrap()
            .status,
        503
    );
    assert_eq!(
        http.handle_authenticated(&request("PUT", "/api/push/subscriptions", "invalid"), now)
            .unwrap()
            .status,
        405
    );
    assert_eq!(
        http.handle_authenticated(
            &request("GET", "/api/push/vapid-public-key", "ignored"),
            now
        )
        .unwrap()
        .status,
        503
    );
}
#[test]
fn source_nested_keys_merge_duplicate_null_and_type_errors() {
    let body=decode_json::<Body>(&request("POST","/api/push/subscriptions",r#"{"endpoint":"https://example.invalid","keys":{"auth":"first","p256dh":"key"},"keys":{"auth":null}}"#)).unwrap();
    assert_eq!(body.keys.auth, "first");
    assert_eq!(body.keys.p256dh, "key");
    assert!(
        decode_json::<Body>(&request(
            "POST",
            "/api/push/subscriptions",
            r#"{"keys":{"auth":42}}"#
        ))
        .is_err()
    );
}
