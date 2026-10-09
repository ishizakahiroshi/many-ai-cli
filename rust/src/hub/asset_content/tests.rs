use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    name: String,
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    compare_content: bool,
}
#[derive(Deserialize)]
struct ObservedResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body_base64: String,
}
#[derive(Deserialize)]
struct Observation {
    name: String,
    file_server: ObservedResponse,
    serve_content: Option<ObservedResponse>,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    content_base64: String,
}
const FIXTURE_ASSETS: &[(&str, &[u8])] = &[
    (
        "app/index.html",
        include_bytes!("../../../tests/fixtures/services/asset-content/data/app/index.html"),
    ),
    (
        "app/module.js",
        include_bytes!("../../../tests/fixtures/services/asset-content/data/app/module.js"),
    ),
    (
        "asset.txt",
        include_bytes!("../../../tests/fixtures/services/asset-content/data/asset.txt"),
    ),
    (
        "binary.png",
        include_bytes!("../../../tests/fixtures/services/asset-content/data/binary.png"),
    ),
    (
        "empty.txt",
        include_bytes!("../../../tests/fixtures/services/asset-content/data/empty.txt"),
    ),
    (
        "empty-dir/.gitkeep",
        include_bytes!("../../../tests/fixtures/services/asset-content/data/empty-dir/.gitkeep"),
    ),
];
fn compare(mut actual: Response, expected: &ObservedResponse, name: &str) {
    let boundary = actual
        .headers
        .get("Content-Type")
        .and_then(|content_type| content_type.strip_prefix("multipart/byteranges; boundary="))
        .map(str::to_owned);
    if let Some(boundary) = boundary {
        assert_eq!(boundary.len(), 60, "{name}");
        assert!(
            boundary
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "{name}"
        );
        let normalized = "b".repeat(60);
        actual.headers.insert(
            "Content-Type".into(),
            format!("multipart/byteranges; boundary={normalized}"),
        );
        // Preserve arbitrary binary payloads; replace only exact boundary bytes.
        let mut body = vec![];
        let mut position = 0;
        while position < actual.body.len() {
            if actual.body[position..].starts_with(boundary.as_bytes()) {
                body.extend_from_slice(normalized.as_bytes());
                position += boundary.len();
            } else {
                body.push(actual.body[position]);
                position += 1;
            }
        }
        actual.body = body;
    }
    assert_eq!(actual.status, expected.status, "{name}: status");
    assert_eq!(actual.headers, expected.headers, "{name}: headers");
    assert_eq!(
        actual.body,
        STANDARD.decode(&expected.body_base64).unwrap(),
        "{name}: body"
    );
}

#[test]
fn all_recovered_cases_match_pinned_go_file_server_and_serve_content() {
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/services/asset-content/cases.json"
    ))
    .unwrap();
    let observations: Vec<Observation> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/services/asset-content/go_observations.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 398);
    assert_eq!(cases.len(), observations.len());
    let mut compared_content = 0;
    for (case, observed) in cases.into_iter().zip(observations) {
        assert_eq!(case.name, observed.name);
        let (path, query) = case.path.split_once('?').unwrap_or((&case.path, ""));
        let request = Request {
            method: case.method,
            path: path.into(),
            query: query.into(),
            headers: case.headers,
            ..Default::default()
        };
        compare(
            file_server(&request, FIXTURE_ASSETS),
            &observed.file_server,
            &format!("{} FileServer", case.name),
        );
        if case.compare_content {
            compared_content += 1;
            let content = STANDARD.decode(observed.content_base64).unwrap();
            compare(
                serve_content(&request, &observed.content_type, &content),
                &observed.serve_content.unwrap(),
                &format!("{} ServeContent", case.name),
            );
        } else {
            assert!(observed.serve_content.is_none());
        }
    }
    assert_eq!(compared_content, 368);
}

#[test]
fn static_routes_retain_security_headers_without_api_auth_guards() {
    let request = Request {
        method: "POST".into(),
        path: "/app-entry.js".into(),
        host: "untrusted.invalid".into(),
        headers: vec![("If-Match".into(), "\"not-an-asset-etag\"".into())],
        ..Default::default()
    };
    let response = super::super::static_asset(&request);
    assert_eq!(response.status, 412);
    assert!(response.headers.contains_key("Content-Security-Policy"));
    assert!(response.cookies.is_empty());
    assert!(!response.headers.contains_key("Allow"));
}
