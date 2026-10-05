use super::*;
#[test]
fn source_scan_count_and_empty_history_null() {
    for (value, expected) in [
        ("", 100),
        ("bogus", 100),
        ("1tail", 1),
        (" +2 trailing", 2),
        ("-1", -1),
        ("0", 0),
        ("999999999999999999999999", 100),
    ] {
        assert_eq!(query_count(value), expected, "{value}");
    }
    let response = simulation(&Policy::default(), vec![], "1");
    let value: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"total":0,"matched":0,"items":null})
    );
}
#[test]
fn simulation_recomputes_persisted_policy_ignoring_old_decision_and_enabled_gate() {
    let policy = Policy::from_yaml(
        b"version: 1\nrules:\n - id: synthetic\n   command: '^git status$'\n   risk: [low]\n",
    );
    let candidate = |id: i64, command: &str| Candidate {
        at: String::new(),
        session_id: id,
        provider: "codex".into(),
        cwd: "/synthetic".into(),
        summary: crate::proto::ApprovalSummary {
            command: command.into(),
            risk: "low".into(),
            ..Default::default()
        },
        decision: crate::approval::policy::Decision {
            allowed: false,
            rule_id: String::new(),
            reason: "old".into(),
        },
    };
    let response = simulation(
        &policy,
        vec![candidate(1, "bad"), candidate(2, "git status")],
        "1tail",
    );
    let value: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(value["total"], 1);
    assert_eq!(value["matched"], 1);
    assert_eq!(value["items"][0]["session_id"], 2);
    assert_eq!(value["items"][0]["decision"]["rule_id"], "synthetic");
}

#[test]
fn source_fmt_sscanf_count_prefix_oracle() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/auto-approval-http/go-observations.json"
    ))
    .unwrap();
    for case in cases.as_array().unwrap() {
        assert_eq!(
            query_count(case["input"].as_str().unwrap()),
            case["n"].as_i64().unwrap() as isize,
            "{}",
            case["input"]
        );
    }
}
