use super::*;
#[test]
fn route_decoder_respects_source_nullable_update_and_nonnullable_add() {
    let request = |path: &str, body: &str| Request {
        method: "POST".into(),
        path: path.into(),
        body: body.as_bytes().to_vec(),
        ..Default::default()
    };
    let add = decode(
        &request(
            "/api/subscriptions",
            r#"{"name":"first","name":null,"provider":"codex","unknown":{"bad":true}}"#,
        ),
        "",
    )
    .unwrap();
    assert_eq!(add.name.as_deref(), Some("first"));
    let update = decode(
        &request(
            "/api/subscriptions/update",
            r#"{"name":"first","name":null,"enabled":false,"enabled":null}"#,
        ),
        "update",
    )
    .unwrap();
    assert!(update.name.is_none());
    assert!(update.enabled.is_none());
    assert!(decode(&request("/api/subscriptions", r#"{"name":42}"#), "").is_err());
    let remove = decode(
        &request(
            "/api/subscriptions/remove",
            r#"{"provider":"codex","id":"p1","name":42,"delete_credentials":true}"#,
        ),
        "remove",
    )
    .unwrap();
    assert!(remove.delete_credentials);
}
#[test]
fn subscription_routes_have_exact_method_groups() {
    assert_eq!(methods("/api/subscriptions"), Some(&["GET", "POST"][..]));
    assert_eq!(
        methods("/api/subscription-usage/probe"),
        Some(&["POST", "DELETE"][..])
    );
    assert_eq!(methods("/api/subscriptions/login"), Some(&["POST"][..]));
    assert!(methods("/api/subscription-usage/missing").is_none());
}
