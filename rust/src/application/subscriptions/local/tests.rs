use super::*;
use crate::config::RuntimePaths;
fn fixture() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49300, installed.path()).unwrap();
    let profile = root.path().join("profile");
    std::fs::create_dir_all(profile.join("sessions")).unwrap();
    (root, installed, paths, profile)
}
fn event(limits: &str) -> String {
    format!(
        r#"{{"timestamp":"2026-10-04T12:34:56.123+09:00","payload":{{"type":"token_count","rate_limits":{limits}}}}}"#
    )
}
#[test]
fn codex_latest_explicit_null_clears_but_absent_and_malformed_do_not_resurrect_older_limits() {
    let (_root, _installed, paths, profile) = fixture();
    let path = profile.join("sessions/rollout-a.jsonl");
    let old = event(
        r#"{"primary":{"used_percent":12.5,"window_minutes":300,"resets_at":100},"credits":{"has_credits":true,"balance":"synthetic-balance"}}"#,
    );
    std::fs::write(&path, format!("{old}\n{}\n", event("null"))).unwrap();
    let usage = read_profile(&paths, "codex", &profile).unwrap();
    assert!(usage.codex.unwrap().primary.is_none());
    assert_eq!(usage.observed_display, "2026-10-04T12:34:56+09:00");
    for latest in [
        r#"{"payload":{"type":"token_count"}}"#,
        r#"{"payload":{"type":"token_count","rate_limits":false}}"#,
        r#"{"payload":{"type":"token_count"},"broken": "#,
    ] {
        std::fs::write(&path, format!("{old}\n{latest}\n")).unwrap();
        assert!(read_profile(&paths, "codex", &profile).is_none());
    }
}
#[test]
fn codex_zero_presence_duplicate_payload_merge_and_large_reverse_chunk_preserve_typed_shape() {
    let (_root, _installed, paths, profile) = fixture();
    let path = profile.join("sessions/rollout-a.jsonl");
    let valid = r#"{"payload":{"type":"token_count"},"payload":{"rate_limits":{"primary":{"used_percent":0,"window_minutes":300},"secondary":{"used_percent":250},"credits":{"has_credits":false,"unlimited":true,"balance":"synthetic"}}}}"#;
    std::fs::write(&path, format!("{}\n{valid}\n", " ".repeat(70 * 1024))).unwrap();
    let usage = read_profile(&paths, "codex", &profile)
        .unwrap()
        .codex
        .unwrap();
    assert_eq!(usage.primary.unwrap().remaining_percent, 100.0);
    assert_eq!(usage.secondary.unwrap().used_percent, 100.0);
    assert!(usage.credits.unwrap().unlimited);
    assert!(usage.credits_balance.is_empty());
}
#[test]
fn newest_rollout_tie_uses_path_and_grok_skips_malformed_without_reading_auth() {
    let (_root, _installed, paths, profile) = fixture();
    let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
    for (name, used) in [("rollout-a.jsonl", 10), ("rollout-z.jsonl", 80)] {
        let path = profile.join("sessions").join(name);
        std::fs::write(
            &path,
            event(&format!(r#"{{"primary":{{"used_percent":{used}}}}}"#)),
        )
        .unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(at))
            .unwrap();
    }
    assert_eq!(
        read_profile(&paths, "codex", &profile)
            .unwrap()
            .codex
            .unwrap()
            .primary
            .unwrap()
            .used_percent,
        80.0
    );
    std::fs::create_dir(profile.join("auth.json")).unwrap();
    std::fs::create_dir(profile.join("logs")).unwrap();
    std::fs::write(profile.join("logs/unified.jsonl"),b"{\"ts\":\"2026-10-04T12:00:00Z\",\"msg\":\"billing: fetched credits config\",\"ctx\":{\"config\":{\"creditUsagePercent\":-2,\"currentPeriod\":{\"type\":\"weekly\"}}}}\nmalformed\n").unwrap();
    let usage = read_profile(&paths, "grok", &profile)
        .unwrap()
        .grok
        .unwrap();
    assert_eq!(usage.used_percent, 0.0);
    assert_eq!(usage.remaining_percent, 100.0);
    assert_eq!(usage.period_type, "weekly");
    assert!(read_profile(&paths, "opencode", &profile).is_none());
}
