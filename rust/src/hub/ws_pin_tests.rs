//! Real loopback WS peers; all config, journals and providers are synthetic.
use super::*;
use crate::config::Resource;

const TOKEN: &str = "synthetic-websocket-token";
const UA: &str = "Chrome/100 Windows";

fn configure_pin(f: &Fixture) {
    let mut snapshot = f.services.config.snapshot().unwrap();
    snapshot.config.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
    snapshot.config.auth_cookie_secret = "synthetic-pin-secret".into();
    f.services
        .config
        .persist(snapshot.revision, snapshot.config)
        .unwrap();
}
async fn post(
    f: &Fixture,
    path: &str,
    body: serde_json::Value,
    cookie: Option<&str>,
    remote: bool,
) -> reqwest::Response {
    let mut request = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://127.0.0.1:{}{path}?token={TOKEN}", f.port))
        .header("Origin", format!("http://127.0.0.1:{}", f.port))
        .header("User-Agent", UA)
        .json(&body);
    if remote {
        request = request.header("X-Forwarded-For", "203.0.113.9");
    }
    if let Some(cookie) = cookie {
        request = request.header("Cookie", cookie);
    }
    request.send().await.unwrap()
}
async fn login(f: &Fixture, pin: &str) -> String {
    let response = post(
        f,
        "/api/auth/login",
        serde_json::json!({"pin":pin}),
        None,
        true,
    )
    .await;
    assert_eq!(response.status(), 200);
    response
        .headers()
        .get_all("Set-Cookie")
        .iter()
        .map(|value| value.to_str().unwrap())
        .find(|value| value.starts_with("MANY_AI_CLI_pin="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .into()
}
async fn ui(f: &Fixture, cookie: Option<&str>, remote: bool) -> Client {
    let mut headers = format!(
        "Origin: http://127.0.0.1:{}\r\nUser-Agent: {UA}\r\n",
        f.port
    );
    if remote {
        headers.push_str("X-Forwarded-For: 203.0.113.9\r\n");
    }
    if let Some(cookie) = cookie {
        headers.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    let (mut client, status) = Client::connect(f.port, &headers).await;
    assert!(status.starts_with("HTTP/1.1 101"));
    client
        .json(serde_json::json!({"role":"ui","token":TOKEN}))
        .await;
    client
}
async fn attached(f: &Fixture, cookie: Option<&str>, remote: bool) -> Client {
    let mut client = ui(f, cookie, remote).await;
    client.until("approval_snapshot").await;
    client
}
async fn wrapper(f: &Fixture) -> (Client, i64) {
    let (mut client, _) = Client::connect(f.port, "").await;
    client.json(serde_json::json!({"type":"register","provider":"shell","cwd":f._root.path(),"pid":123,"token":TOKEN})).await;
    let id = client.until("registered").await["session_id"]
        .as_i64()
        .unwrap();
    (client, id)
}
async fn input(wrapper: &mut Client, ui: &mut Client, id: i64, text: &str) {
    use base64::Engine as _;
    ui.json(serde_json::json!({"type":"pty_input","session_id":id,"text":text}))
        .await;
    let frame = wrapper.until("pty_input").await;
    assert_eq!(
        frame["data"],
        base64::engine::general_purpose::STANDARD.encode(text)
    );
    wrapper
        .json(serde_json::json!({"type":"pty_input_ack","input_seq":frame["input_seq"]}))
        .await;
}

#[tokio::test]
async fn remote_pin_logout_revokes_its_cookie_ui_and_keeps_other_cookie_local_ui_and_wrapper() {
    let f = fixture().await;
    configure_pin(&f);
    let (mut wrapper, id) = wrapper(&f).await;
    let first = login(&f, "123456").await;
    let second = login(&f, "123456").await;
    let mut revoked = attached(&f, Some(&first), true).await;
    let mut other = attached(&f, Some(&second), true).await;
    let mut local = attached(&f, None, false).await;
    input(&mut wrapper, &mut revoked, id, "before logout").await;
    let response = post(
        &f,
        "/api/auth/logout",
        serde_json::json!({}),
        Some(&first),
        true,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert!(
        revoked.frame().await.is_none(),
        "logged-out UI remained attached"
    );
    assert!(ui(&f, Some(&first), true).await.frame().await.is_none());
    input(&mut wrapper, &mut other, id, "other nonce survives").await;
    input(&mut wrapper, &mut local, id, "local survives").await;
    assert!(f.core.details(LiveSessionId(id)).unwrap().connected);
    f.stop().await;
}

#[tokio::test]
async fn remote_pin_change_revokes_old_uis_only_after_successful_persistence() {
    let f = fixture().await;
    configure_pin(&f);
    let (mut wrapper, id) = wrapper(&f).await;
    let first = login(&f, "123456").await;
    let second = login(&f, "123456").await;
    let mut one = attached(&f, Some(&first), true).await;
    let mut two = attached(&f, Some(&second), true).await;
    let mut local = attached(&f, None, false).await;
    let destination = f.services.paths.resource(Resource::Config);
    let saved = destination.with_extension("preserved-synthetic");
    std::fs::rename(&destination, &saved).unwrap();
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("keep"), b"synthetic obstruction").unwrap();
    let before = f.services.config.snapshot().unwrap();
    assert_eq!(
        post(
            &f,
            "/api/auth/set-pin",
            serde_json::json!({"pin":"654321"}),
            None,
            false
        )
        .await
        .status(),
        500
    );
    assert_eq!(
        f.services.config.snapshot().unwrap().revision,
        before.revision
    );
    input(&mut wrapper, &mut one, id, "failed save preserves one").await;
    input(&mut wrapper, &mut two, id, "failed save preserves two").await;
    std::fs::remove_file(destination.join("keep")).unwrap();
    std::fs::remove_dir(&destination).unwrap();
    std::fs::rename(saved, &destination).unwrap();
    assert_eq!(
        post(
            &f,
            "/api/auth/set-pin",
            serde_json::json!({"pin":"654321"}),
            None,
            false
        )
        .await
        .status(),
        200
    );
    assert!(
        one.frame().await.is_none(),
        "old remote UI survived PIN change"
    );
    assert!(two.frame().await.is_none());
    assert!(ui(&f, Some(&first), true).await.frame().await.is_none());
    let fresh = login(&f, "654321").await;
    let mut replacement = attached(&f, Some(&fresh), true).await;
    input(&mut wrapper, &mut replacement, id, "new PIN works").await;
    input(&mut wrapper, &mut local, id, "local survives PIN change").await;
    assert!(f.core.details(LiveSessionId(id)).unwrap().connected);
    f.stop().await;
}

#[tokio::test]
async fn remote_pin_enable_and_clear_close_preexisting_remote_ui_but_keep_local_ui() {
    let f = fixture().await;
    let (mut wrapper, id) = wrapper(&f).await;
    let mut no_pin = attached(&f, None, true).await;
    let mut local = attached(&f, None, false).await;
    assert_eq!(
        post(
            &f,
            "/api/auth/set-pin",
            serde_json::json!({"pin":"123456"}),
            None,
            false
        )
        .await
        .status(),
        200
    );
    assert!(no_pin.frame().await.is_none());
    let cookie = login(&f, "123456").await;
    let mut pinned = attached(&f, Some(&cookie), true).await;
    assert_eq!(
        post(
            &f,
            "/api/auth/set-pin",
            serde_json::json!({"clear":true}),
            None,
            false
        )
        .await
        .status(),
        200
    );
    assert!(pinned.frame().await.is_none());
    let mut fresh = attached(&f, None, true).await;
    input(&mut wrapper, &mut fresh, id, "new unpinned remote UI").await;
    input(&mut wrapper, &mut local, id, "local survives clear").await;
    f.stop().await;
}

#[tokio::test]
async fn remote_pin_expiry_closes_idle_ui_and_rejects_expired_cookie_without_stopping_wrapper() {
    let f = fixture().await;
    configure_pin(&f);
    let (mut wrapper, id) = wrapper(&f).await;
    let mut local = attached(&f, None, false).await;
    let expires = Timestamp::now().unix_seconds() + 2;
    let login = Request {
        method: "POST".into(),
        path: "/api/auth/login".into(),
        host: format!("127.0.0.1:{}", f.port),
        remote_addr: "127.0.0.1:1".into(),
        query: format!("token={TOKEN}"),
        headers: vec![
            ("Origin".into(), format!("http://127.0.0.1:{}", f.port)),
            ("X-Forwarded-For".into(), "203.0.113.9".into()),
            ("User-Agent".into(), UA.into()),
        ],
        body: br#"{"pin":"123456"}"#.to_vec(),
        ..Default::default()
    };
    // Inject only the login wall clock. The real WS server and its idle timer
    // still use native time, with an actual valid nonce that expires shortly.
    let response = f.services.handle(&login, expires - 12 * 60 * 60).response;
    assert_eq!(response.status, 200);
    let cookie = response
        .cookies
        .iter()
        .find(|value| value.starts_with("MANY_AI_CLI_pin="))
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let mut remote = attached(&f, Some(cookie), true).await;
    assert!(
        remote
            .frame_with_timeout(Duration::from_secs(5))
            .await
            .is_none(),
        "idle expired UI remained attached"
    );
    assert!(ui(&f, Some(cookie), true).await.frame().await.is_none());
    input(&mut wrapper, &mut local, id, "local after remote expiry").await;
    assert!(f.core.details(LiveSessionId(id)).unwrap().connected);
    f.stop().await;
}

#[tokio::test]
async fn remote_pin_logout_waits_for_accepted_paste_and_rejects_new_input() {
    let gate = Arc::new(GateObserver {
        reached: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let f = fixture_with_observer(gate.clone()).await;
    configure_pin(&f);
    let (mut wrapper, id) = wrapper(&f).await;
    let cookie = login(&f, "123456").await;
    let mut remote = attached(&f, Some(&cookie), true).await;
    remote.json(serde_json::json!({"type":"pty_input","session_id":id,"text":"\u{1b}[200~accepted\u{1b}[201~\r"})).await;
    tokio::time::timeout(Duration::from_secs(2), gate.reached.notified())
        .await
        .unwrap();
    let request = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!(
            "http://127.0.0.1:{}/api/auth/logout?token={TOKEN}",
            f.port
        ))
        .header("Origin", format!("http://127.0.0.1:{}", f.port))
        .header("User-Agent", UA)
        .header("X-Forwarded-For", "203.0.113.9")
        .header("Cookie", cookie.clone());
    let check = Request {
        remote_addr: "127.0.0.1:1".into(),
        headers: vec![
            ("User-Agent".into(), UA.into()),
            ("X-Forwarded-For".into(), "203.0.113.9".into()),
            ("Cookie".into(), cookie),
        ],
        ..Default::default()
    };
    assert!(f.services.auth.has_valid_pin(
        &check,
        &f.services.config.snapshot().unwrap().config,
        Timestamp::now().unix_seconds(),
    ));
    let logout = tokio::spawn(async move { request.send().await.unwrap().status() });
    tokio::time::timeout(Duration::from_secs(2), async {
        while f.services.auth.has_valid_pin(
            &check,
            &f.services.config.snapshot().unwrap().config,
            Timestamp::now().unix_seconds(),
        ) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        !logout.is_finished(),
        "logout acknowledged before accepted input drained"
    );
    remote
        .json(serde_json::json!({"type":"pty_input","session_id":id,"text":"must not send"}))
        .await;
    gate.release.notify_one();
    let paste = wrapper.until("pty_input").await;
    assert_ne!(paste["data"], "DQ==");
    wrapper
        .json(serde_json::json!({"type":"pty_input_ack","input_seq":paste["input_seq"]}))
        .await;
    let enter = wrapper.until("pty_input").await;
    assert_eq!(enter["data"], "DQ==");
    wrapper
        .json(serde_json::json!({"type":"pty_input_ack","input_seq":enter["input_seq"]}))
        .await;
    // The synthetic wrapper emits no output, so the accepted submit's existing
    // policy sends exactly one confirmation Enter. This is accepted work too.
    let retry = wrapper.until("pty_input").await;
    assert_eq!(retry["data"], "DQ==");
    wrapper
        .json(serde_json::json!({"type":"pty_input_ack","input_seq":retry["input_seq"]}))
        .await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), logout)
            .await
            .unwrap()
            .unwrap(),
        200
    );
    assert!(remote.frame().await.is_none());
    let mut local = attached(&f, None, false).await;
    input(&mut wrapper, &mut local, id, "only expected next input").await;
    assert!(f.core.details(LiveSessionId(id)).unwrap().connected);
    f.stop().await;
}
