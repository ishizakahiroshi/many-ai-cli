//! Synthetic tests only: fake Deskly/GitHub/launcher, loopback HTTP servers,
//! temporary journals. No real Deskly, GitHub, credential or AI process.
use super::github::{GithubPrHeadReader, PrHead, PrHeadError};
use super::launch::{initial_prompt, ordinary_request};
use super::mcp::{CallError, Endpoint, MachineKey, McpClient, ToolReply, parse_reply};
use super::wire::*;
use super::*;
use crate::proto::core::AuthEpoch;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

const ID: &str = "00000000-0000-4000-8000-000000000001";
const ATTEMPT: &str = "00000000-0000-4000-9000-000000000002";
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER_SHA: &str = "fedcba9876543210fedcba9876543210fedcba98";
const H64: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NUMBER: &str = "#P-20261030-001";
const SESSION: &str = "worker-observed-session";
const NOW: i64 = 1_793_318_400_000;
fn now() -> i64 {
    NOW
}

// ---------------------------------------------------------------- wire

fn discovery_json() -> Value {
    json!({"profile": DISCOVERY_PROFILE, "truncated": false, "work": [
        {"request_id": ID, "expected_revision": 3, "generation": 1,
         "authorization_epoch": 2, "session_id": SESSION}]})
}
#[test]
fn discovery_decodes_only_the_closed_v1_shape() {
    let decoded = decode_discovery(discovery_json()).unwrap();
    assert_eq!(decoded.work.len(), 1);
    assert_eq!(decoded.work[0].expected_revision, 3);
    let bad: Vec<Box<dyn Fn(&mut Value)>> = vec![
        Box::new(|v| v["profile"] = json!("pc_relay_discovery_v2")),
        Box::new(|v| v["cursor"] = json!("next")),
        Box::new(|v| v["work"][0]["repo"] = json!("o/r")),
        Box::new(|v| v["work"][0]["generation"] = json!(2)),
        Box::new(|v| v["work"][0]["expected_revision"] = json!(0)),
        Box::new(|v| v["work"][0]["expected_revision"] = json!(3.5)),
        Box::new(|v| v["work"][0]["authorization_epoch"] = json!(-1)),
        Box::new(|v| v["work"][0]["request_id"] = json!("ABCDEF00-0000-4000-8000-000000000001")),
        Box::new(|v| v["work"][0]["session_id"] = json!("")),
        Box::new(|v| v["work"][0]["session_id"] = json!("a\nb")),
        Box::new(|v| {
            let item = v["work"][0].clone();
            v["work"] = json!([item.clone(), item]);
        }),
        Box::new(|v| {
            let item = v["work"][0].clone();
            v["work"] = Value::Array(vec![item; MAX_DISCOVERY_ITEMS + 1]);
        }),
        Box::new(|v| v["truncated"] = json!("false")),
    ];
    for (index, mutate) in bad.iter().enumerate() {
        let mut value = discovery_json();
        mutate(&mut value);
        assert!(decode_discovery(value).is_err(), "case {index}");
    }
    let mut truncated = discovery_json();
    truncated["truncated"] = json!(true);
    assert!(decode_discovery(truncated).unwrap().truncated);
}
fn receipt_json(stage: &str) -> Value {
    json!({"request_id": ID, "number": NUMBER, "revision": 4, "generation": 1, "stage": stage})
}
fn confirmation_json() -> Value {
    let identity = json!({"principal_id": "p", "instance": "i", "key_id": "k"});
    json!({"request_id": ID, "number": NUMBER, "generation": 1, "worker": identity,
        "orchestrator": identity, "session_id": SESSION,
        "destination": {"workspace_id": "T1", "channel_id": "C1", "thread_mode": "root", "thread_ts": null},
        "body_digest": H64, "wire_digest": H64, "authority_expires_at": "2026-10-30T00:05:00.000Z"})
}
fn token_json() -> Value {
    json!({"expires_at": "2026-10-30T00:01:00.000Z", "signature": H64})
}
#[test]
fn previews_require_confirmation_only_for_handoff() {
    let claim =
        json!({"applied": false, "preview": receipt_json("wired"), "preview_token": token_json()});
    assert!(decode_preview(claim.clone(), ID, false).is_ok());
    assert!(decode_preview(claim, ID, true).is_err());
    let handoff = json!({"applied": false, "preview": receipt_json("sending"),
        "confirmation": confirmation_json(), "preview_token": token_json()});
    assert!(decode_preview(handoff.clone(), ID, true).is_ok());
    assert!(decode_preview(handoff.clone(), ID, false).is_err());
    let mut other = handoff.clone();
    other["preview"]["request_id"] = json!(ATTEMPT);
    assert!(decode_preview(other, ID, true).is_err());
    let mut bad_token = handoff;
    bad_token["preview_token"]["signature"] = json!("XYZ");
    assert!(decode_preview(bad_token, ID, true).is_err());
}
fn handoff_json() -> Value {
    let identity = json!({"principal_id": "p", "instance": "i", "key_id": "k"});
    json!({"applied": true, "outcome": "issued", "receipt": receipt_json("consumed_unknown"),
        "handoff": {"profile": HANDOFF_PROFILE, "request_id": ID, "number": NUMBER, "generation": 1,
            "authorization_epoch": 2, "attempt_id": ATTEMPT,
            "authority_id": "00000000-0000-4000-8000-000000000003",
            "consumption_id": "00000000-0000-4000-8000-000000000004",
            "wire_id": "00000000-0000-4000-8000-000000000005",
            "worker": identity, "orchestrator": identity, "session_id": SESSION,
            "issued_at": "2026-10-30T00:00:00.000Z", "expires_at": "2026-10-30T00:10:00.000Z",
            "wire": {"version": 1, "template_version": 1, "role": "worker", "transport": "slack_auto",
                "kind": "code", "ready_to_send": false, "automatic_start": false,
                "destination": {"workspace_id": "T1"}, "text": "PRIVATE-WIRE-TEXT",
                "wire_digest": H64, "marker": H64, "unfurl_links": false, "unfurl_media": false}}})
}
#[test]
fn issued_handoff_is_closed_and_drops_wire_text() {
    let HandoffApplied::Issued(issued) =
        decode_handoff_applied(handoff_json(), ID, ATTEMPT).unwrap()
    else {
        panic!("issued");
    };
    assert!(!format!("{issued:?}").contains("PRIVATE-WIRE-TEXT"));
    assert_eq!(issued.wire_digest, H64);
    let bad: Vec<Box<dyn Fn(&mut Value)>> = vec![
        Box::new(|v| v["handoff"]["attempt_id"] = json!(ID)),
        Box::new(|v| v["handoff"]["wire"]["automatic_start"] = json!(true)),
        Box::new(|v| v["handoff"]["wire"]["ready_to_send"] = json!(true)),
        Box::new(|v| v["handoff"]["wire"]["command"] = json!("rm")),
        Box::new(|v| v["handoff"]["expires_at"] = json!("2026-10-29T00:00:00.000Z")),
        Box::new(|v| v["receipt"]["stage"] = json!("wired")),
        Box::new(|v| v["handoff"]["profile"] = json!("other")),
        Box::new(|v| v["outcome"] = json!("spent")),
    ];
    for (index, mutate) in bad.iter().enumerate() {
        let mut value = handoff_json();
        mutate(&mut value);
        assert!(
            decode_handoff_applied(value, ID, ATTEMPT).is_err(),
            "case {index}"
        );
    }
    let spent =
        json!({"applied": true, "outcome": "spent", "receipt": receipt_json("consumed_unknown")});
    assert!(matches!(
        decode_handoff_applied(spent, ID, ATTEMPT),
        Ok(HandoffApplied::Spent(_))
    ));
}
#[test]
fn issued_validation_must_echo_the_issued_tuple() {
    let HandoffApplied::Issued(issued) =
        decode_handoff_applied(handoff_json(), ID, ATTEMPT).unwrap()
    else {
        panic!("issued");
    };
    let valid = json!({"profile": VALIDATION_PROFILE, "request_id": ID, "revision": 5, "generation": 1,
        "authorization_epoch": 2, "client_attempt_id": ATTEMPT,
        "authority_id": issued.authority_id, "consumption_id": issued.consumption_id,
        "wire_id": issued.wire_id, "wire_digest": H64, "session_id": SESSION,
        "validated_at": "2026-10-30T00:00:01.000Z", "valid_until": "2026-10-30T00:01:01.000Z"});
    assert!(decode_validation(valid.clone(), &issued).is_ok());
    for key in ["wire_digest", "session_id", "consumption_id"] {
        let mut value = valid.clone();
        value[key] = json!("00000000-0000-4000-8000-0000000000ff");
        assert!(decode_validation(value, &issued).is_err(), "{key}");
    }
}

// ---------------------------------------------------------------- mcp

fn rpc(id: u64, result: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": result})).unwrap()
}
fn text_result(value: &Value, error: bool) -> Value {
    json!({"content": [{"type": "text", "text": value.to_string()}], "isError": error})
}
#[test]
fn reply_parser_accepts_json_or_single_sse_frame_and_refusals() {
    let data = json!({"ok": 1});
    assert_eq!(
        parse_reply(&rpc(7, text_result(&data, false)), 7),
        Ok(ToolReply::Data(data.clone()))
    );
    let sse = format!(
        "event: message\ndata: {}\n\n",
        String::from_utf8(rpc(7, text_result(&data, false))).unwrap()
    );
    assert_eq!(
        parse_reply(sse.as_bytes(), 7),
        Ok(ToolReply::Data(data.clone()))
    );
    let refusal = json!({"code": "development_runtime_refused", "retryable": false});
    assert_eq!(
        parse_reply(&rpc(7, text_result(&refusal, true)), 7),
        Ok(ToolReply::Refused("development_runtime_refused".into()))
    );
    let plain =
        json!({"content": [{"type": "text", "text": "Input validation error"}], "isError": true});
    assert_eq!(
        parse_reply(&rpc(7, plain), 7),
        Ok(ToolReply::Refused("refused".into()))
    );
    assert!(matches!(
        parse_reply(&rpc(8, text_result(&data, false)), 7),
        Err(CallError::Unknown(_))
    ));
    let two = json!({"content": [{"type": "text", "text": "{}"}, {"type": "text", "text": "{}"}]});
    assert!(matches!(
        parse_reply(&rpc(7, two), 7),
        Err(CallError::Unknown(_))
    ));
    let err =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -32602}})).unwrap();
    assert_eq!(parse_reply(&err, 7), Err(CallError::Rejected(200)));
}
#[test]
fn endpoint_and_key_rules_match_deskly_and_never_print_the_key() {
    assert!(Endpoint::parse("https://deskly.example/mcp/development-runtime").is_ok());
    assert!(Endpoint::parse("http://127.0.0.1:8787/mcp/development-runtime").is_ok());
    for bad in [
        "http://deskly.example/mcp/development-runtime",
        "https://deskly.example/mcp",
        "https://deskly.example/mcp/development-runtime?x=1",
        "https://deskly.example/mcp/development-runtime#f",
        "http://u:p@127.0.0.1/mcp/development-runtime",
    ] {
        assert!(Endpoint::parse(bad).is_err(), "{bad}");
    }
    assert!(MachineKey::new("short".into()).is_err());
    assert!(MachineKey::new("has space in it 0123456789".into()).is_err());
    let key = MachineKey::new("synthetic-test-only-key-0123456789".into()).unwrap();
    assert!(!format!("{key:?}").contains("synthetic"));
}

#[derive(Default)]
struct Server {
    key: String,
    mode: Mutex<String>,
    hits: Mutex<Vec<(String, Value)>>,
    redirected: Mutex<usize>,
}
async fn serve(state: Arc<Server>) -> String {
    use axum::{Router, extract::State, http::HeaderMap, response::IntoResponse, routing::post};
    async fn mcp_handler(
        State(s): State<Arc<Server>>,
        headers: HeaderMap,
        body: String,
    ) -> axum::response::Response {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != format!("Bearer {}", s.key) {
            return (axum::http::StatusCode::UNAUTHORIZED, "{}").into_response();
        }
        let frame: Value = serde_json::from_str(&body).unwrap();
        let id = frame["id"].as_u64().unwrap();
        let name = frame["params"]["name"].as_str().unwrap().to_owned();
        s.hits
            .lock()
            .unwrap()
            .push((name, frame["params"]["arguments"].clone()));
        match s.mode.lock().unwrap().as_str() {
            "500" => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "{}").into_response(),
            "redirect" => (
                axum::http::StatusCode::TEMPORARY_REDIRECT,
                [("location", "/elsewhere")],
            )
                .into_response(),
            _ => {
                let sse = format!(
                    "event: message\ndata: {}\n\n",
                    String::from_utf8(rpc(id, text_result(&discovery_json(), false))).unwrap()
                );
                ([("content-type", "text/event-stream")], sse).into_response()
            }
        }
    }
    async fn elsewhere(State(s): State<Arc<Server>>) -> &'static str {
        *s.redirected.lock().unwrap() += 1;
        "{}"
    }
    let app = Router::new()
        .route(mcp::MCP_PATH, post(mcp_handler))
        .route("/elsewhere", post(elsewhere))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}{}", mcp::MCP_PATH)
}
const TEST_KEY: &str = "synthetic-test-only-key-0123456789";
#[tokio::test]
async fn authenticated_mcp_discovery_round_trip_and_failure_classes() {
    let state = Arc::new(Server {
        key: TEST_KEY.into(),
        ..Default::default()
    });
    let url = serve(state.clone()).await;
    let client = McpClient::new(
        Endpoint::parse(&url).unwrap(),
        MachineKey::new(TEST_KEY.into()).unwrap(),
    )
    .unwrap();
    let discovery = DesklyRelay::discover(&client).await.unwrap();
    assert_eq!(discovery.work[0].request_id, ID);
    assert_eq!(
        state.hits.lock().unwrap()[0],
        (mcp::DISCOVER_TOOL.to_owned(), json!({}))
    );

    let wrong = McpClient::new(
        Endpoint::parse(&url).unwrap(),
        MachineKey::new("synthetic-wrong-key-0123456789".into()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        wrong.call_tool(mcp::DISCOVER_TOOL, json!({})).await,
        Err(CallError::Unauthorized)
    );

    *state.mode.lock().unwrap() = "500".into();
    assert!(matches!(
        client.call_tool(mcp::CLAIM_TOOL, json!({})).await,
        Err(CallError::Unknown(_))
    ));
    // A 5xx on a mutating step is "unknown", never "refused".
    let token: PreviewToken = serde_json::from_value(token_json()).unwrap();
    assert!(matches!(
        DesklyRelay::claim_apply(&client, ID, 3, SESSION, &token).await,
        mcp::Step::Unknown(_)
    ));

    *state.mode.lock().unwrap() = "redirect".into();
    assert!(
        client
            .call_tool(mcp::DISCOVER_TOOL, json!({}))
            .await
            .is_err()
    );
    assert_eq!(
        *state.redirected.lock().unwrap(),
        0,
        "bearer must not follow redirects"
    );

    let closed = McpClient::new(
        Endpoint::parse("http://127.0.0.1:9/mcp/development-runtime").unwrap(),
        MachineKey::new(TEST_KEY.into()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        closed.call_tool(mcp::DISCOVER_TOOL, json!({})).await,
        Err(CallError::NotDelivered)
    );
}

// ---------------------------------------------------------------- github

#[tokio::test]
async fn github_reader_reports_head_and_refuses_bad_shapes() {
    use axum::{Router, extract::Path, routing::get};
    async fn pull(
        Path((owner, name, number)): Path<(String, String, u64)>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if number == 404 {
            return (axum::http::StatusCode::NOT_FOUND, "{}").into_response();
        }
        let sha = if number == 2 { "NOT-A-SHA" } else { SHA };
        axum::Json(json!({"state": "open", "head": {"sha": sha, "ref": "x"},
            "base": {"repo": {"full_name": format!("{owner}/{name}")}}, "title": "ignored"}))
        .into_response()
    }
    let app = Router::new().route("/repos/{owner}/{name}/pulls/{number}", get(pull));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let reader = GithubPrHeadReader::with_base(&format!("http://{addr}/")).unwrap();
    let repo = RepoName::parse("example/repo").unwrap();
    let head = github::PrHeadReader::head(&reader, &repo, 51)
        .await
        .unwrap();
    assert_eq!(
        head,
        PrHead {
            sha: SHA.into(),
            open: true,
            base_repo: "example/repo".into()
        }
    );
    assert_eq!(
        github::PrHeadReader::head(&reader, &repo, 404).await,
        Err(PrHeadError::NotVisible)
    );
    assert_eq!(
        github::PrHeadReader::head(&reader, &repo, 2).await,
        Err(PrHeadError::Malformed)
    );
    for bad in ["example", "a/b/c", "../x", "a/..", "a b/c", "a/"] {
        assert!(RepoName::parse(bad).is_err(), "{bad}");
    }
}

// ---------------------------------------------------------------- launch

fn launch_request(provider: ReadOnlyProvider) -> ReadOnlyLaunch {
    ReadOnlyLaunch {
        client_request_id: format!("deskly-{ATTEMPT}"),
        provider,
        cwd: std::env::temp_dir(),
        repo: RepoName::parse("example/repo").unwrap(),
        pull_request: 51,
        head_sha: SHA.into(),
    }
}
#[test]
fn launch_request_uses_only_validated_read_only_flags() {
    let claude = ordinary_request(&launch_request(ReadOnlyProvider::Claude));
    assert_eq!(
        (
            claude.provider.as_str(),
            claude.permission_mode.as_str(),
            claude.sandbox.as_str()
        ),
        ("claude", "plan", "")
    );
    let codex = ordinary_request(&launch_request(ReadOnlyProvider::Codex));
    assert_eq!(
        (
            codex.provider.as_str(),
            codex.sandbox.as_str(),
            codex.ask_for_approval.as_str()
        ),
        ("codex", "read-only", "untrusted")
    );
    for body in [&claude, &codex] {
        assert!(crate::proto::core::valid_client_request_id(
            &body.client_request_id
        ));
        assert_eq!(body.isolate_worktree, Some(false));
        assert!(!body.orchestration && !body.risk_confirmed);
    }
    let prompt = initial_prompt(&launch_request(ReadOnlyProvider::Claude));
    assert!(prompt.contains(SHA) && prompt.contains("example/repo") && prompt.contains("#51"));
}
#[test]
fn intake_never_mints_a_hub_confirmation_proof() {
    for source in [
        include_str!("../deskly_intake.rs"),
        include_str!("mcp.rs"),
        include_str!("launch.rs"),
        include_str!("journal.rs"),
        include_str!("github.rs"),
        include_str!("wire.rs"),
    ] {
        assert!(!source.contains("after_server_authentication"));
        assert!(!source.contains("after_server_verification"));
    }
}
#[test]
fn scope_is_exactly_read() {
    assert_eq!(Scope::from_capabilities(&["read"]), Some(Scope::ReadOnly));
    for bad in [
        &[][..],
        &["write"],
        &["read", "write"],
        &["read", "read"],
        &["READ"],
    ] {
        assert_eq!(Scope::from_capabilities(bad), None);
    }
}

// ---------------------------------------------------------------- journal

fn record(attempt: &str) -> journal::Record {
    journal::Record {
        request_id: ID.into(),
        generation: 1,
        attempt_id: attempt.into(),
        state: State::OwnerPending,
        reason: String::new(),
        revision: 3,
        authorization_epoch: 2,
        session_id: SESSION.into(),
        repo: "example/repo".into(),
        pull_request: 51,
        head_sha: SHA.into(),
        client_request_id: String::new(),
    }
}
#[test]
fn journal_reserves_once_and_restart_holds_every_in_flight_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("intake.sqlite");
    let (journal, held) = Journal::open(&path).unwrap();
    assert_eq!(held, 0);
    assert!(journal.reserve(&record(ATTEMPT), NOW).unwrap());
    assert!(
        !journal.reserve(&record(ID), NOW).unwrap(),
        "same tuple, new attempt"
    );
    assert!(
        !journal
            .transition(ID, 1, State::Claimed, State::Launched, "", NOW)
            .unwrap()
    );
    assert!(
        journal
            .transition(ID, 1, State::OwnerPending, State::Approved, "", NOW)
            .unwrap()
    );
    assert!(
        journal
            .transition(ID, 1, State::Approved, State::ClaimSent, "", NOW)
            .unwrap()
    );
    drop(journal);
    let (journal, held) = Journal::open(&path).unwrap();
    assert_eq!(held, 1);
    let row = journal.get(ID, 1).unwrap().unwrap();
    assert_eq!(
        (row.state, row.reason.as_str()),
        (State::Held, "restart_during_claim_sent")
    );
    assert_eq!(row.attempt_id, ATTEMPT);
    assert!(
        !journal.reserve(&record(ID), NOW).unwrap(),
        "held is never new again"
    );
    let id = journal::new_attempt_id().unwrap();
    assert!(valid_uuid(&id) && id.as_bytes()[14] == b'4');
}

// ---------------------------------------------------------------- coordinator

struct FakeDeskly {
    discovery: RefCell<Result<Discovery, String>>,
    claim_preview: RefCell<mcp::Step<Preview>>,
    claim_apply: RefCell<mcp::Step<Receipt>>,
    handoff_preview: RefCell<mcp::Step<Preview>>,
    handoff_apply: RefCell<mcp::Step<HandoffApplied>>,
    validate: RefCell<mcp::Step<i64>>,
    calls: RefCell<Vec<&'static str>>,
    handoff_attempt: RefCell<String>,
}
fn receipt(stage: Stage) -> Receipt {
    serde_json::from_value(receipt_json(match stage {
        Stage::Wired => "wired",
        Stage::Sending => "sending",
        _ => "consumed_unknown",
    }))
    .unwrap()
}
fn preview(confirmation: bool) -> Preview {
    Preview {
        receipt: receipt(Stage::Wired),
        confirmation: confirmation.then(|| serde_json::from_value(confirmation_json()).unwrap()),
        token: serde_json::from_value(token_json()).unwrap(),
    }
}
fn issued() -> wire::IssuedHandoff {
    let HandoffApplied::Issued(mut issued) =
        decode_handoff_applied(handoff_json(), ID, ATTEMPT).unwrap()
    else {
        unreachable!()
    };
    issued.expires_at_ms = NOW + 60_000;
    issued
}
impl Default for FakeDeskly {
    fn default() -> Self {
        Self {
            discovery: RefCell::new(Ok(decode_discovery(discovery_json()).unwrap())),
            claim_preview: RefCell::new(mcp::Step::Done(preview(false))),
            claim_apply: RefCell::new(mcp::Step::Done(receipt(Stage::Sending))),
            handoff_preview: RefCell::new(mcp::Step::Done(preview(true))),
            handoff_apply: RefCell::new(mcp::Step::Done(HandoffApplied::Issued(issued()))),
            validate: RefCell::new(mcp::Step::Done(NOW + 30_000)),
            calls: RefCell::default(),
            handoff_attempt: RefCell::default(),
        }
    }
}
impl DesklyRelay for FakeDeskly {
    async fn discover(&self) -> Result<Discovery, String> {
        self.calls.borrow_mut().push("discover");
        self.discovery.borrow().clone()
    }
    async fn claim_preview(&self, _: &str, _: u64, _: &str) -> mcp::Step<Preview> {
        self.calls.borrow_mut().push("claim_preview");
        self.claim_preview.borrow().clone()
    }
    async fn claim_apply(&self, _: &str, _: u64, _: &str, _: &PreviewToken) -> mcp::Step<Receipt> {
        self.calls.borrow_mut().push("claim_apply");
        self.claim_apply.borrow().clone()
    }
    async fn handoff_preview(&self, _: &str, _: u64, attempt: &str) -> mcp::Step<Preview> {
        self.calls.borrow_mut().push("handoff_preview");
        *self.handoff_attempt.borrow_mut() = attempt.to_owned();
        self.handoff_preview.borrow().clone()
    }
    async fn handoff_apply(
        &self,
        _: &str,
        _: u64,
        _: &str,
        _: &PreviewToken,
    ) -> mcp::Step<HandoffApplied> {
        self.calls.borrow_mut().push("handoff_apply");
        self.handoff_apply.borrow().clone()
    }
    async fn validate_issued(&self, _: u64, _: &wire::IssuedHandoff) -> mcp::Step<i64> {
        self.calls.borrow_mut().push("validate");
        self.validate.borrow().clone()
    }
}
struct FakeTargets(RefCell<TargetResolution>);
impl TargetSource for FakeTargets {
    async fn resolve(&self, _: &Candidate) -> TargetResolution {
        self.0.borrow().clone()
    }
}
struct FakeHeads(RefCell<Result<PrHead, PrHeadError>>);
impl PrHeadReader for FakeHeads {
    async fn head(&self, _: &RepoName, _: u64) -> Result<PrHead, PrHeadError> {
        self.0.borrow().clone()
    }
}
struct FakeLauncher {
    epoch: AuthEpoch,
    outcome: RefCell<Result<(), LaunchError>>,
    launches: RefCell<Vec<ReadOnlyLaunch>>,
}
impl ReadOnlyLauncher for FakeLauncher {
    fn auth_epoch(&self) -> AuthEpoch {
        self.epoch
    }
    async fn launch(
        &self,
        request: &ReadOnlyLaunch,
        owner: &VerifiedConfirmationRequest,
    ) -> Result<(), LaunchError> {
        assert_eq!(owner.auth_epoch(), self.epoch);
        self.launches.borrow_mut().push(request.clone());
        self.outcome.borrow().clone()
    }
}
fn target() -> ApprovedTarget {
    ApprovedTarget {
        request_id: ID.into(),
        expected_revision: 3,
        authorization_epoch: 2,
        repo: RepoName::parse("example/repo").unwrap(),
        pull_request: 51,
        head_sha: SHA.into(),
        scope: Scope::ReadOnly,
        expires_at_ms: NOW + 600_000,
    }
}
fn open_head() -> PrHead {
    PrHead {
        sha: SHA.into(),
        open: true,
        base_repo: "example/repo".into(),
    }
}
struct Rig {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    c: IntakeCoordinator<FakeDeskly, FakeTargets, FakeHeads>,
    launcher: FakeLauncher,
}
fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("intake.sqlite");
    let (journal, _) = Journal::open(&path).unwrap();
    let mut checkouts = BTreeMap::new();
    checkouts.insert(
        RepoName::parse("example/repo").unwrap(),
        std::env::temp_dir(),
    );
    Rig {
        _dir: dir,
        path,
        c: IntakeCoordinator {
            deskly: FakeDeskly::default(),
            targets: FakeTargets(RefCell::new(TargetResolution::Bound(target()))),
            heads: FakeHeads(RefCell::new(Ok(open_head()))),
            journal,
            policy: LocalPolicy {
                checkouts,
                provider: ReadOnlyProvider::Claude,
            },
            clock: now,
        },
        launcher: FakeLauncher {
            epoch: AuthEpoch(7),
            outcome: RefCell::new(Ok(())),
            launches: RefCell::default(),
        },
    }
}
fn proof(epoch: u64) -> VerifiedConfirmationRequest {
    VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(epoch))
}
async fn approve(r: &Rig, sha: &str, epoch: u64) -> ApproveOutcome {
    let verified = proof(epoch);
    r.c.approve(
        OwnerApproval {
            request_id: ID,
            generation: 1,
            head_sha: sha,
            verified: &verified,
        },
        &r.launcher,
    )
    .await
}
fn state(r: &Rig) -> (State, String) {
    let row = r.c.journal.get(ID, 1).unwrap().unwrap();
    (row.state, row.reason)
}
fn calls(r: &Rig) -> Vec<&'static str> {
    r.c.deskly.calls.borrow().clone()
}

#[tokio::test]
async fn owner_approved_read_only_launch_runs_once_through_existing_authority() {
    let r = rig();
    let report = r.c.poll_once().await;
    assert_eq!(report.pending_owner.len(), 1);
    assert_eq!(state(&r).0, State::OwnerPending);
    assert_eq!(calls(&r), ["discover"], "poll never claims");
    let ApproveOutcome::Launched { client_request_id } = approve(&r, SHA, 7).await else {
        panic!("launched");
    };
    assert_eq!(
        calls(&r),
        [
            "discover",
            "claim_preview",
            "claim_apply",
            "handoff_preview",
            "handoff_apply",
            "validate"
        ]
    );
    let row = r.c.journal.get(ID, 1).unwrap().unwrap();
    assert_eq!(row.state, State::Launched);
    assert_eq!(row.client_request_id, client_request_id);
    assert_eq!(*r.c.deskly.handoff_attempt.borrow(), row.attempt_id);
    let launches = r.launcher.launches.borrow();
    assert_eq!(launches.len(), 1);
    assert_eq!(
        (launches[0].provider, launches[0].head_sha.as_str()),
        (ReadOnlyProvider::Claude, SHA)
    );
    drop(launches);
    // Duplicate poll and duplicate approval do nothing.
    let again = r.c.poll_once().await;
    assert_eq!((again.pending_owner.len(), again.already_known), (0, 1));
    assert_eq!(
        approve(&r, SHA, 7).await,
        ApproveOutcome::NotApplied("not pending")
    );
    assert_eq!(r.launcher.launches.borrow().len(), 1);
}
#[tokio::test]
async fn advisory_discovery_without_bound_target_records_and_sends_nothing() {
    let r = rig();
    *r.c.targets.0.borrow_mut() = NoTargetContract
        .resolve(&decode_discovery(discovery_json()).unwrap().work[0])
        .await;
    let report = r.c.poll_once().await;
    assert_eq!((report.unbound, report.pending_owner.len()), (1, 0));
    assert!(r.c.journal.get(ID, 1).unwrap().is_none());
    assert_eq!(calls(&r), ["discover"]);
}
#[tokio::test]
async fn binding_scope_policy_and_live_head_are_checked_before_recording() {
    let cases: Vec<(&str, Box<dyn Fn(&Rig)>)> = vec![
        (
            "revision",
            Box::new(|r| {
                if let TargetResolution::Bound(t) = &mut *r.c.targets.0.borrow_mut() {
                    t.expected_revision = 4
                }
            }),
        ),
        (
            "epoch",
            Box::new(|r| {
                if let TargetResolution::Bound(t) = &mut *r.c.targets.0.borrow_mut() {
                    t.authorization_epoch = 9
                }
            }),
        ),
        (
            "request",
            Box::new(|r| {
                if let TargetResolution::Bound(t) = &mut *r.c.targets.0.borrow_mut() {
                    t.request_id = ATTEMPT.into()
                }
            }),
        ),
        (
            "expired",
            Box::new(|r| {
                if let TargetResolution::Bound(t) = &mut *r.c.targets.0.borrow_mut() {
                    t.expires_at_ms = NOW
                }
            }),
        ),
        (
            "repo not allowed",
            Box::new(|r| {
                if let TargetResolution::Bound(t) = &mut *r.c.targets.0.borrow_mut() {
                    t.repo = RepoName::parse("other/repo").unwrap()
                }
            }),
        ),
        (
            "bad sha",
            Box::new(|r| {
                if let TargetResolution::Bound(t) = &mut *r.c.targets.0.borrow_mut() {
                    t.head_sha = "ABC".into()
                }
            }),
        ),
        (
            "head moved",
            Box::new(|r| r.c.heads.0.borrow_mut().as_mut().unwrap().sha = OTHER_SHA.into()),
        ),
        (
            "closed",
            Box::new(|r| r.c.heads.0.borrow_mut().as_mut().unwrap().open = false),
        ),
        (
            "other base",
            Box::new(|r| r.c.heads.0.borrow_mut().as_mut().unwrap().base_repo = "fork/repo".into()),
        ),
        (
            "not visible",
            Box::new(|r| *r.c.heads.0.borrow_mut() = Err(PrHeadError::NotVisible)),
        ),
    ];
    for (name, mutate) in cases {
        let r = rig();
        mutate(&r);
        let report = r.c.poll_once().await;
        assert_eq!(report.rejected.len(), 1, "{name}");
        assert!(r.c.journal.get(ID, 1).unwrap().is_none(), "{name}");
        assert_eq!(calls(&r), ["discover"], "{name}");
    }
}
#[tokio::test]
async fn truncated_or_failed_discovery_takes_no_action() {
    let r = rig();
    r.c.deskly
        .discovery
        .borrow_mut()
        .as_mut()
        .unwrap()
        .truncated = true;
    assert!(r.c.poll_once().await.truncated);
    *r.c.deskly.discovery.borrow_mut() = Err("Unauthorized".into());
    assert!(r.c.poll_once().await.discovery_error.is_some());
    assert!(r.c.journal.get(ID, 1).unwrap().is_none());
}
#[tokio::test]
async fn unknown_claim_holds_and_is_never_retried() {
    let r = rig();
    r.c.poll_once().await;
    *r.c.deskly.claim_apply.borrow_mut() = mcp::Step::Unknown("transport");
    assert_eq!(
        approve(&r, SHA, 7).await,
        ApproveOutcome::Held("claim_unknown:transport".into())
    );
    *r.c.deskly.claim_apply.borrow_mut() = mcp::Step::Done(receipt(Stage::Sending));
    assert_eq!(r.c.poll_once().await.already_known, 1);
    assert_eq!(
        approve(&r, SHA, 7).await,
        ApproveOutcome::NotApplied("not pending")
    );
    assert_eq!(calls(&r).iter().filter(|c| **c == "claim_apply").count(), 1);
    assert!(r.launcher.launches.borrow().is_empty());
}
#[tokio::test]
async fn spent_unknown_or_refused_handoff_holds_without_launch() {
    for (step, reason) in [
        (
            mcp::Step::Done(HandoffApplied::Spent(receipt(Stage::Sending))),
            "handoff_spent",
        ),
        (
            mcp::Step::Unknown("server status"),
            "handoff_unknown:server status",
        ),
        (
            mcp::Step::Refused("development_runtime_refused".into()),
            "handoff_refused:development_runtime_refused",
        ),
    ] {
        let r = rig();
        r.c.poll_once().await;
        *r.c.deskly.handoff_apply.borrow_mut() = step;
        assert_eq!(
            approve(&r, SHA, 7).await,
            ApproveOutcome::Held(reason.into())
        );
        assert_eq!(state(&r), (State::Held, reason.to_owned()));
        assert!(r.launcher.launches.borrow().is_empty());
        assert!(!calls(&r).contains(&"validate"));
    }
}
#[tokio::test]
async fn refused_claim_is_terminal_and_skips_handoff() {
    let r = rig();
    r.c.poll_once().await;
    *r.c.deskly.claim_apply.borrow_mut() = mcp::Step::Refused("development_runtime_refused".into());
    assert!(matches!(
        approve(&r, SHA, 7).await,
        ApproveOutcome::Refused(_)
    ));
    assert_eq!(state(&r).0, State::Refused);
    assert!(!calls(&r).contains(&"handoff_preview"));
}
#[tokio::test]
async fn handoff_for_a_different_session_or_bad_validation_holds() {
    let r = rig();
    r.c.poll_once().await;
    if let mcp::Step::Done(p) = &mut *r.c.deskly.handoff_preview.borrow_mut() {
        p.confirmation.as_mut().unwrap().session_id = "another-session".into();
    }
    assert_eq!(
        approve(&r, SHA, 7).await,
        ApproveOutcome::Held("handoff session mismatch".into())
    );
    assert!(!calls(&r).contains(&"handoff_apply"));

    let r = rig();
    r.c.poll_once().await;
    *r.c.deskly.validate.borrow_mut() = mcp::Step::Done(NOW - 1);
    assert_eq!(
        approve(&r, SHA, 7).await,
        ApproveOutcome::Held("issued validation expired".into())
    );
    assert!(r.launcher.launches.borrow().is_empty());
}
#[tokio::test]
async fn owner_approval_is_bound_to_hub_auth_epoch_shown_sha_and_live_head() {
    let r = rig();
    r.c.poll_once().await;
    assert_eq!(
        approve(&r, SHA, 6).await,
        ApproveOutcome::NotApplied("hub authentication expired")
    );
    assert_eq!(
        approve(&r, OTHER_SHA, 7).await,
        ApproveOutcome::NotApplied("approved tuple differs")
    );
    assert_eq!(state(&r).0, State::OwnerPending);
    assert_eq!(calls(&r), ["discover"]);
    // The PR moved after the owner saw it: refuse before any claim.
    r.c.heads.0.borrow_mut().as_mut().unwrap().sha = OTHER_SHA.into();
    assert_eq!(
        approve(&r, SHA, 7).await,
        ApproveOutcome::Refused("pr head moved".into())
    );
    assert_eq!(calls(&r), ["discover"]);
}
#[tokio::test]
async fn declined_requests_never_launch() {
    let r = rig();
    r.c.poll_once().await;
    assert!(r.c.decline(ID, 1));
    assert_eq!(
        approve(&r, SHA, 7).await,
        ApproveOutcome::NotApplied("not pending")
    );
    assert_eq!(state(&r).0, State::OwnerDeclined);
}
#[tokio::test]
async fn unknown_launch_holds_and_restart_keeps_it_held() {
    let r = rig();
    r.c.poll_once().await;
    *r.launcher.outcome.borrow_mut() = Err(LaunchError::Unknown("spawn_error".into()));
    assert_eq!(
        approve(&r, SHA, 7).await,
        ApproveOutcome::Held("launch_unknown:spawn_error".into())
    );
    let path = r.path.clone();
    // Simulate a crash after the launch intent but before any reply.
    r.c.journal
        .transition(ID, 1, State::Held, State::LaunchSent, "", NOW)
        .unwrap();
    let (reopened, held) = Journal::open(&path).unwrap();
    assert_eq!(held, 1);
    let row = reopened.get(ID, 1).unwrap().unwrap();
    assert_eq!(
        (row.state, row.reason.as_str()),
        (State::Held, "restart_during_launch_sent")
    );
}
