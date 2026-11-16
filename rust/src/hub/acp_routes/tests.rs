//! Route-level checks: the dashboard's proof gates every route, and the whole
//! flow runs over the scripted agent. Synthetic provider ids and folders only.
use super::*;
use crate::acp::mock_agent::MockConfig;
use crate::application::acp_sessions::tests::{Fixture, fixture, human};
use serde_json::Value;

fn request(method: &str, path: &str, body: Value) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        body: if body.is_null() {
            Vec::new()
        } else {
            serde_json::to_vec(&body).unwrap()
        },
        ..Default::default()
    }
}

fn with_query(mut request: Request, query: &str) -> Request {
    request.query = query.into();
    request
}

async fn call(http: &AcpHttp, request: Request) -> (u16, Value) {
    let response = http.handle_authenticated(&request, Some(human())).await;
    let value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
    (response.status, value)
}

fn http(f: &Fixture) -> Arc<AcpHttp> {
    AcpHttp::new(f.sessions.clone())
}

fn open_body(f: &Fixture) -> Value {
    json!({"provider": "synthetic-acp", "cwd": f.folder.path().to_str().unwrap()})
}

#[test]
fn only_the_acp_paths_are_routes_and_each_has_fixed_methods() {
    assert_eq!(methods("/api/acp/sessions"), Some(&["GET", "POST"][..]));
    assert_eq!(methods("/api/acp/sessions/3/events"), Some(&["GET"][..]));
    assert_eq!(
        methods("/api/acp/sessions/3/agent-sessions"),
        Some(&["GET"][..])
    );
    for action in ["prompt", "permission", "cancel", "end"] {
        assert_eq!(
            methods(&format!("/api/acp/sessions/3/{action}")),
            Some(&["POST"][..])
        );
    }
    for path in [
        "/api/acp",
        "/api/acp/sessions/x/events",
        "/api/acp/sessions/3/unknown",
        "/api/acp/sessions/3",
        "/api/acp/other",
        "/api/sessions/3/prompt",
    ] {
        assert!(!is_route(path), "{path}");
    }
    assert!(crate::hub::router::needs_owned_request("/api/acp/sessions"));
    assert!(crate::hub::router::needs_owned_request(
        "/api/acp/sessions/3/prompt"
    ));
}

#[tokio::test]
async fn without_the_dashboards_proof_every_route_is_refused() {
    let f = fixture(MockConfig::default());
    let http = http(&f);
    let session = f.open().await;
    let cases = [
        ("GET", "/api/acp/sessions".to_owned()),
        ("POST", "/api/acp/sessions".to_owned()),
        ("GET", format!("/api/acp/sessions/{}/events", session.id)),
        ("POST", format!("/api/acp/sessions/{}/prompt", session.id)),
        (
            "POST",
            format!("/api/acp/sessions/{}/permission", session.id),
        ),
        ("POST", format!("/api/acp/sessions/{}/cancel", session.id)),
        ("POST", format!("/api/acp/sessions/{}/end", session.id)),
        (
            "GET",
            format!("/api/acp/sessions/{}/agent-sessions", session.id),
        ),
    ];
    for (method, path) in cases {
        let response = http
            .handle_authenticated(&request(method, &path, open_body(&f)), None)
            .await;
        assert_eq!(response.status, 403, "{method} {path}");
    }
    // Nothing happened behind the refusals: no prompt, no second session.
    assert_eq!(f.sessions.list().len(), 1);
    assert_eq!(f.mock(0).methods(), ["initialize", "session/new"]);
    // A wrong method is a 405 before anything else.
    let response = http
        .handle_authenticated(
            &request("GET", "/api/acp/sessions/1/prompt", Value::Null),
            None,
        )
        .await;
    assert_eq!(response.status, 405);
}

#[tokio::test]
async fn the_dashboard_can_open_prompt_answer_and_end_a_session() {
    let f = fixture(MockConfig::default());
    let http = http(&f);
    let (status, opened) = call(&http, request("POST", "/api/acp/sessions", open_body(&f))).await;
    assert_eq!(status, 200, "{opened}");
    let id = opened["id"].as_u64().unwrap();
    let base = format!("/api/acp/sessions/{id}");

    let (status, _) = call(
        &http,
        request(
            "POST",
            &format!("{base}/prompt"),
            json!({"text": "permission"}),
        ),
    )
    .await;
    assert_eq!(status, 202);

    // Wait for the stored permission request, then answer it.
    let mut after = 0;
    let key = loop {
        let (status, events) = call(
            &http,
            with_query(
                request("GET", &format!("{base}/events"), Value::Null),
                &format!("after={after}&wait_ms=500"),
            ),
        )
        .await;
        assert_eq!(status, 200);
        after = events["last_seq"].as_u64().unwrap();
        if let Some(pending) = events["pending_permissions"]
            .as_array()
            .and_then(|p| p.first())
        {
            break pending["request_key"].as_u64().unwrap();
        }
    };
    let (status, _) = call(
        &http,
        request(
            "POST",
            &format!("{base}/permission"),
            json!({"request_key": key, "option_id": "reject-once"}),
        ),
    )
    .await;
    assert_eq!(status, 200);
    // Both an option and a cancel, or neither, is a bad request.
    let (status, _) = call(
        &http,
        request(
            "POST",
            &format!("{base}/permission"),
            json!({"request_key": key}),
        ),
    )
    .await;
    assert_eq!(status, 400);

    let (status, events) = loop {
        let (status, events) = call(
            &http,
            with_query(
                request("GET", &format!("{base}/events"), Value::Null),
                &format!("after={after}&wait_ms=500"),
            ),
        )
        .await;
        after = events["last_seq"].as_u64().unwrap();
        if events["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "turn_end")
        {
            break (status, events);
        }
    };
    assert_eq!(status, 200);
    assert_eq!(events["session"]["phase"], "idle");

    let (status, list) = call(&http, request("GET", "/api/acp/sessions", Value::Null)).await;
    assert_eq!(status, 200);
    assert_eq!(list["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(list["saved"].as_array().unwrap().len(), 1);

    let (status, ended) = call(&http, request("POST", &format!("{base}/end"), Value::Null)).await;
    assert_eq!(status, 200);
    assert_eq!(ended["phase"], "ended");
    let (status, error) = call(
        &http,
        request("POST", &format!("{base}/prompt"), json!({"text": "plain"})),
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(error["error"], "session_ended");
}

#[tokio::test]
async fn refusals_have_their_own_statuses() {
    let f = fixture(MockConfig::default());
    let http = http(&f);
    let (status, body) = call(
        &http,
        request(
            "POST",
            "/api/acp/sessions",
            json!({"provider": "claude", "cwd": f.folder.path().to_str().unwrap()}),
        ),
    )
    .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("not_acp_provider"))
    );
    let (status, body) = call(
        &http,
        request("POST", "/api/acp/sessions/99/prompt", json!({"text": "x"})),
    )
    .await;
    assert_eq!((status, body["error"].as_str()), (404, Some("not_found")));
    let (status, _) = call(
        &http,
        request(
            "POST",
            "/api/acp/sessions",
            json!({"provider": "synthetic-acp", "cwd": "rel"}),
        ),
    )
    .await;
    assert_eq!(status, 400);
    let (status, _) = call(
        &http,
        request("POST", "/api/acp/sessions/1/prompt", Value::Null),
    )
    .await;
    // An empty prompt is refused before the session is even looked up.
    assert_eq!(status, 400);
}

#[tokio::test]
async fn a_missing_sign_in_is_a_409_with_the_official_procedure_pointer() {
    let f = fixture(MockConfig {
        auth_required: true,
        ..Default::default()
    });
    let http = http(&f);
    let (status, body) = call(&http, request("POST", "/api/acp/sessions", open_body(&f))).await;
    assert_eq!(status, 409);
    assert_eq!(body["error"], "auth_required");
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .contains("official procedure")
    );
    assert_eq!(body["auth_methods"].as_array().unwrap().len(), 2);
}
