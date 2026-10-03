use super::memo::*;
use crate::proto::time::{Timestamp, UNIX_EPOCH};
use crate::{
    files::safe_fs::Dir,
    hub::http::{Request, Response},
};
use std::{sync::Arc, time::Duration};
fn manager() -> (tempfile::TempDir, MemoManager) {
    let dir = tempfile::tempdir().unwrap();
    let root = Arc::new(Dir::open(dir.path()).unwrap());
    let m = MemoManager::open(root);
    (dir, m)
}
fn request(method: &str, path: &str, body: &str) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        body: body.as_bytes().into(),
        ..Default::default()
    }
}
fn value(r: Response) -> serde_json::Value {
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    serde_json::from_slice(&r.body).unwrap()
}
fn now() -> Timestamp {
    UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789)
}
#[test]
fn memo_create_patch_delete_persists_and_project_is_server_derived() {
    let (dir, m) = manager();
    let created = value(m.handle_memos(
        &request(
            "POST",
            "/api/memos",
            r#"{"text":"  hello 日本語  ","session_id":7,"project":"/spoofed"}"#,
        ),
        |id| {
            if id == 7 {
                "/synthetic/project".into()
            } else {
                String::new()
            }
        },
        now(),
    ));
    let memo = &created["memo"];
    assert_eq!(memo["text"], "hello 日本語");
    assert_eq!(memo["project"], "/synthetic/project");
    assert_eq!(memo["created_at"], "2023-11-14T22:13:20.123456789Z");
    let id = memo["id"].as_str().unwrap();
    let path = format!("/api/memos/{id}");
    let patched = value(m.handle_memos(
        &request("PATCH", &path, r#"{"done":true,"text":null}"#),
        |_| panic!("patch cannot resolve project"),
        now(),
    ));
    assert_eq!(patched["memo"]["done"], true);
    assert_eq!(patched["memo"]["text"], "hello 日本語");
    assert!(patched["memo"]["done_at"].is_string());
    let reloaded = MemoManager::open(Arc::new(Dir::open(dir.path()).unwrap()));
    assert_eq!(
        reloaded.mentions(),
        vec![("/synthetic/project".into(), "hello 日本語".into())]
    );
    value(reloaded.handle_memos(&request("DELETE", &path, ""), |_| String::new(), now()));
    let reopened = MemoManager::open(Arc::new(Dir::open(dir.path()).unwrap()));
    assert_eq!(
        value(reopened.handle_memos(&request("GET", "/api/memos", ""), |_| String::new(), now()))["memos"],
        serde_json::json!([])
    );
}
#[test]
fn memo_failed_write_does_not_publish_or_destroy() {
    let (dir, m) = manager();
    let created = value(m.handle_memos(
        &request("POST", "/api/memos", r#"{"text":"original"}"#),
        |_| String::new(),
        now(),
    ));
    let before = std::fs::read(dir.path().join("memos.json")).unwrap();
    let fail = MemoManager::failed_writes(Arc::new(Dir::open(dir.path()).unwrap()));
    let id = created["memo"]["id"].as_str().unwrap();
    let r = fail.handle_memos(
        &request(
            "PATCH",
            &format!("/api/memos/{id}"),
            r#"{"text":"changed"}"#,
        ),
        |_| String::new(),
        now(),
    );
    assert_eq!(r.status, 500);
    assert_eq!(fail.mentions()[0].1, "original");
    assert_eq!(
        std::fs::read(dir.path().join("memos.json")).unwrap(),
        before
    );
}
#[test]
fn memo_corrupt_store_fail_closed_without_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = b"{broken synthetic store";
    std::fs::write(dir.path().join("memos.json"), bytes).unwrap();
    let m = MemoManager::open(Arc::new(Dir::open(dir.path()).unwrap()));
    assert!(m.mentions().is_empty());
    assert_eq!(
        m.handle_memos(
            &request("POST", "/api/memos", r#"{"text":"new"}"#),
            |_| String::new(),
            now()
        )
        .status,
        503
    );
    assert_eq!(std::fs::read(dir.path().join("memos.json")).unwrap(), bytes);
    assert_eq!(m.clean_orphan_images(now()), 0);
}
#[test]
fn memo_limits_count_bytes_and_pointer_patch_semantics() {
    let (_dir, m) = manager();
    let body = serde_json::json!({"text":"日".repeat(1334)}).to_string();
    assert_eq!(
        m.handle_memos(
            &request("POST", "/api/memos", &body),
            |_| String::new(),
            now()
        )
        .status,
        400
    );
    let c = value(m.handle_memos(
        &request("POST", "/api/memos", r#"{"text":"text"} {"second":true}"#),
        |_| String::new(),
        now(),
    ));
    let path = format!("/api/memos/{}", c["memo"]["id"].as_str().unwrap());
    assert_eq!(
        m.handle_memos(
            &request("PATCH", &path, r#"{"text":"  "}"#),
            |_| String::new(),
            now()
        )
        .status,
        400
    );
    let unchanged = value(m.handle_memos(
        &request("PATCH", &path, r#"{"text":null,"done":null}"#),
        |_| String::new(),
        now(),
    ));
    assert_eq!(unchanged["memo"]["text"], "text");
}
#[test]
fn memo_images_sniff_content_not_header_and_delete_after_save() {
    let (dir, m) = manager();
    let mut upload = request("POST", "/api/memo-images", "");
    upload.body = b"\x89PNG\r\n\x1a\nsynthetic".to_vec();
    upload
        .headers
        .push(("Content-Type".into(), "image/svg+xml".into()));
    let image = value(m.handle_images(&upload))["image"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(image.ends_with(".png"));
    let body = serde_json::json!({"text":"","images":[image.clone()]}).to_string();
    let memo = value(m.handle_memos(
        &request("POST", "/api/memos", &body),
        |_| String::new(),
        now(),
    ));
    assert_eq!(
        m.handle_memos(
            &request("POST", "/api/memos", &body),
            |_| String::new(),
            now()
        )
        .status,
        400
    );
    let get = m.handle_images(&request("GET", &format!("/api/memo-images/{image}"), ""));
    assert_eq!(get.body, upload.body);
    assert_eq!(get.headers["Content-Type"], "image/png");
    let path = format!("/api/memos/{}", memo["memo"]["id"].as_str().unwrap());
    let failing = MemoManager::failed_writes(Arc::new(Dir::open(dir.path()).unwrap()));
    assert_eq!(
        failing
            .handle_memos(&request("DELETE", &path, ""), |_| String::new(), now())
            .status,
        500
    );
    assert!(dir.path().join("memo-images").join(&image).exists());
    value(m.handle_memos(&request("DELETE", &path, ""), |_| String::new(), now()));
    assert!(!dir.path().join("memo-images").join(image).exists());
}
#[test]
fn memo_images_basename_limits_svg_and_orphan_cleanup() {
    let (dir, m) = manager();
    for name in [
        "../../secrets",
        "A0000000000000000000000000000000.png",
        "00000000000000000000000000000000.svg",
    ] {
        assert!(!valid_image_name(name));
    }
    assert_eq!(
        m.handle_images(&request("POST", "/api/memo-images", "<svg>bad</svg>"))
            .status,
        415
    );
    let mut big = request("POST", "/api/memo-images", "");
    big.body = vec![0; IMAGE_LIMIT + 1];
    assert_eq!(m.handle_images(&big).status, 413);
    let mut upload = request("POST", "/api/memo-images", "");
    upload.body = b"GIF89asynth".to_vec();
    let image = value(m.handle_images(&upload))["image"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        m.clean_orphan_images(Timestamp::now() + Duration::from_secs(90000)),
        1
    );
    assert!(!dir.path().join("memo-images").join(image).exists());
}

#[test]
fn routine_schedule_matches_223_fixed_go_civil_time_cases() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/services/schedules.json")).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 223);
    for case in cases.as_array().unwrap() {
        let schedule: super::schedule::Schedule =
            serde_json::from_value(case["schedule"].clone()).unwrap();
        let after = crate::proto::time::parse_rfc3339(case["after"].as_str().unwrap()).unwrap();
        let result = super::schedule::next(&schedule, after);
        if let Some(error) = case["error"].as_str() {
            assert_eq!(result.unwrap_err(), error, "{case}");
        } else {
            let value = result
                .unwrap()
                .map(|time| crate::proto::time::format_with_offset(time, 0, true).unwrap())
                .unwrap_or_else(|| "0001-01-01T00:00:00Z".into());
            assert_eq!(value, case["next"].as_str().unwrap(), "{case}");
        }
    }
}
