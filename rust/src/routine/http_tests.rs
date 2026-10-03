use super::*;
use crate::files::safe_fs::Dir;
use std::time::Duration;
use std::time::UNIX_EPOCH;
fn at() -> SystemTime {
    UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789)
}
fn fixture() -> (tempfile::TempDir, RoutineHttp) {
    let root = tempfile::tempdir().unwrap();
    let store = Arc::new(RoutineStore::open(Arc::new(
        Dir::open(root.path()).unwrap(),
    )));
    let http = RoutineHttp {
        store,
        runner: None,
        home: root.path().into(),
    };
    (root, http)
}
fn req(method: &str, path: &str, body: &str) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        body: body.as_bytes().into(),
        ..Default::default()
    }
}
fn decode(response: Response, status: u16) -> serde_json::Value {
    assert_eq!(
        response.status,
        status,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    serde_json::from_slice(&response.body).unwrap()
}
fn body(root: &std::path::Path) -> String {
    let cwd = root.join("synthetic-project");
    std::fs::create_dir_all(&cwd).unwrap();
    json!({"name":"Routine","cwd":cwd,"provider":"codex","prompt":"Read the synthetic fixture."})
        .to_string()
}
#[test]
fn routine_http_create_get_update_delete_and_first_value_decoder() {
    let (root, http) = fixture();
    assert_eq!(
        decode(
            http.handle_authenticated(&req("GET", "/api/routines", ""), at())
                .unwrap(),
            200
        ),
        json!({"routines":[]})
    );
    let created = decode(
        http.handle_authenticated(
            &req(
                "POST",
                "/api/routines",
                &(body(root.path()) + " {\"trailing\":true}"),
            ),
            at(),
        )
        .unwrap(),
        200,
    );
    let mut item = created["routine"].clone();
    let id = item["id"].as_str().unwrap().to_owned();
    let path = format!("/api/routines/{id}");
    assert_eq!(item["completion_mode"], "native_or_marker");
    assert_eq!(item["schedule"], json!({"kind":"manual"}));
    assert!(item.get("next_run_at").is_none());
    item["name"] = "Changed".into();
    let changed = decode(
        http.handle_authenticated(
            &req("PUT", &path, &item.to_string()),
            at() + Duration::from_secs(1),
        )
        .unwrap(),
        200,
    );
    assert_eq!(changed["routine"]["created_at"], item["created_at"]);
    let stale = decode(
        http.handle_authenticated(&req("PUT", &path, &item.to_string()), at())
            .unwrap(),
        409,
    );
    assert_eq!(stale["error"], "routine_changed");
    assert_eq!(
        decode(
            http.handle_authenticated(&req("DELETE", &path, "ignored body"), at())
                .unwrap(),
            200
        ),
        json!({"ok":true})
    );
    assert_eq!(
        decode(
            http.handle_authenticated(&req("GET", &path, ""), at())
                .unwrap(),
            404
        )["detail"],
        "routine not found"
    );
}
#[test]
fn routine_http_route_method_and_readiness_precedence() {
    let (root, http) = fixture();
    for (method, path, status, code) in [
        ("PATCH", "/api/routines", 405, "method_not_allowed"),
        ("GET", "/api/routines/id/runs", 404, "not_found"),
        ("POST", "/api/routines/id", 405, "method_not_allowed"),
        ("PUT", "/api/routines", 405, "method_not_allowed"),
        ("GET", "/api/routine-runs/missing", 404, "not_found"),
        ("POST", "/api/routine-runs", 405, "method_not_allowed"),
    ] {
        assert_eq!(
            decode(
                http.handle_authenticated(&req(method, path, "{}"), at())
                    .unwrap(),
                status
            )["error"],
            code
        );
    }
    std::fs::write(root.path().join("routines.json"), b"broken").unwrap();
    let http = RoutineHttp {
        store: Arc::new(RoutineStore::open(Arc::new(
            Dir::open(root.path()).unwrap(),
        ))),
        runner: None,
        home: root.path().into(),
    };
    assert_eq!(
        decode(
            http.handle_authenticated(&req("GET", "/api/routines/id/runs", ""), at())
                .unwrap(),
            503
        )["error"],
        "routine_store_unavailable"
    );
    assert_eq!(
        decode(
            http.handle_authenticated(&req("PATCH", "/api/routines", ""), at())
                .unwrap(),
            405
        )["error"],
        "method_not_allowed"
    );
}
#[test]
fn routine_http_manual_start_validates_request_then_exposes_missing_launcher() {
    let (root, http) = fixture();
    let created = decode(
        http.handle_authenticated(&req("POST", "/api/routines", &body(root.path())), at())
            .unwrap(),
        200,
    );
    let path = format!(
        "/api/routines/{}/runs",
        created["routine"]["id"].as_str().unwrap()
    );
    for body in [
        "{}",
        "{\"request_id\":null}",
        "{\"request_id\":\"  \"}",
        "{\"request_id\":4}",
    ] {
        assert_eq!(
            decode(
                http.handle_authenticated(&req("POST", &path, body), at())
                    .unwrap(),
                400
            )["error"],
            "bad_request"
        );
    }
    assert_eq!(
        decode(
            http.handle_authenticated(
                &req(
                    "POST",
                    &path,
                    "{\"REQUEST_ID\":\"synthetic-click\",\"unknown\":true}"
                ),
                at()
            )
            .unwrap(),
            503
        )["error"],
        "routine_launcher_unavailable"
    );
    assert!(http.store.runs("").unwrap().is_empty());
}
#[test]
fn routine_http_history_filter_is_reverse_and_keeps_terminal_records_after_delete() {
    let (root, http) = fixture();
    let created = decode(
        http.handle_authenticated(&req("POST", "/api/routines", &body(root.path())), at())
            .unwrap(),
        200,
    );
    let id = created["routine"]["id"].as_str().unwrap();
    for n in 0..3 {
        let run = http
            .store
            .admit(id, &format!("click{n}"), "manual", at())
            .unwrap()
            .run;
        http.store.launched(&run.id, Err(()), "hub", at()).unwrap();
    }
    http.store.delete(id).unwrap();
    let mut request = req("GET", "/api/routine-runs", "");
    request.query = format!("routine_id={id}");
    let runs = decode(http.handle_authenticated(&request, at()).unwrap(), 200);
    assert_eq!(runs["runs"][0]["request_id"], "click2");
    assert_eq!(runs["runs"].as_array().unwrap().len(), 3);
    request.query = "routine_id=other".into();
    assert_eq!(
        decode(http.handle_authenticated(&request, at()).unwrap(), 200),
        json!({"runs":[]})
    );
}
