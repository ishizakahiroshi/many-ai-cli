use super::*;

#[test]
fn login_using_a_replaced_pin_snapshot_cannot_issue_current_pin_authority() {
    let auth = AuthService::default();
    let old = Config {
        token: "synthetic".into(),
        remote_pin_hash: bcrypt::hash("123456", 4).unwrap(),
        auth_cookie_secret: "synthetic-secret".into(),
        ..Default::default()
    };
    let mut current = old.clone();
    current.remote_pin_hash = bcrypt::hash("654321", 4).unwrap();
    let mut request = Request {
        method: "POST".into(),
        path: "/api/auth/login".into(),
        host: "127.0.0.1:49200".into(),
        remote_addr: "203.0.113.1:1".into(),
        query: "token=synthetic".into(),
        headers: vec![
            ("User-Agent".into(), "Chrome/100 Windows".into()),
            ("Origin".into(), "http://127.0.0.1:49200".into()),
        ],
        body: br#"{"pin":"123456"}"#.to_vec(),
        ..Default::default()
    };
    // Simulate login's captured old config completing after a new PIN was
    // published, without a second signer or a forged nonce.
    let response = auth.login(&request, &old, 1000);
    assert_eq!(response.status, 200);
    request.headers.push((
        "Cookie".into(),
        response.cookies[0].split(';').next().unwrap().into(),
    ));
    assert!(auth.has_valid_pin(&request, &old, 1001));
    assert!(!auth.has_valid_pin(&request, &current, 1001));
    assert_eq!(
        auth.guard(&request, &current, 49200, &["POST"], 1001)
            .unwrap_err()
            .status,
        401
    );
    request.body = br#"{"pin":"654321"}"#.to_vec();
    let fresh = auth.login(&request, &current, 1002);
    assert_eq!(fresh.status, 200);
    request.headers.last_mut().unwrap().1 = fresh.cookies[0].split(';').next().unwrap().into();
    assert!(
        auth.guard(&request, &current, 49200, &["POST"], 1002)
            .is_ok()
    );
}
