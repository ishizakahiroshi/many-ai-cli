use super::*;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Deserialize)]
struct Time {
    seconds: i64,
    nanos: u32,
}
impl Time {
    fn value(self) -> Timestamp {
        Timestamp::from_unix(self.seconds, self.nanos).unwrap()
    }
}
#[derive(Deserialize)]
struct MarkerInput {
    block: String,
    sig: String,
}
#[derive(Deserialize)]
struct Step {
    marker: Option<MarkerInput>,
    at: Time,
    source: String,
    #[serde(default)]
    reset: bool,
    #[serde(default)]
    warm: bool,
}
#[derive(Deserialize)]
struct Expected {
    result: bool,
    reason: String,
    sig: String,
    at: Time,
    trace: Vec<String>,
    warnings: Vec<Value>,
    broadcasts: Vec<Value>,
    record: String,
}
#[derive(Deserialize)]
struct Row {
    input: Step,
    expected: Expected,
}
#[derive(Deserialize)]
struct Input {
    name: String,
    initial_sig: String,
    initial_at: Time,
    missing: bool,
    logger: bool,
    open_result: bool,
}
#[derive(Deserialize)]
struct Scenario {
    input: Input,
    steps: Vec<Row>,
}
#[derive(Deserialize)]
struct LegacyInput {
    key: String,
    consumed_sig: String,
    candidate_sig: String,
    at: Time,
    now: Time,
}
#[derive(Deserialize)]
struct LegacyRow {
    input: LegacyInput,
    expected: bool,
}
#[derive(Deserialize)]
struct Corpus {
    baseline: String,
    scenarios: Vec<Scenario>,
    legacy: Vec<LegacyRow>,
    source_census: Vec<String>,
    probe_sink: Value,
}
fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/core/marker-suppression/go.json"
    ))
    .unwrap()
}
fn marker(sig: &str) -> Marker {
    Marker {
        block: "3. Missing first option\n".into(),
        sig: sig.into(),
    }
}
fn at(seconds: i64, nanos: u32) -> Timestamp {
    Timestamp::from_unix(seconds, nanos).unwrap()
}

#[test]
fn pinned_go_state_and_effect_eligibility_order() {
    let corpus = corpus();
    assert_eq!(corpus.baseline, "21d0bc7935a2c4696fb89ccff2e324157a528c2d");
    assert_eq!(corpus.scenarios.len(), 87);
    let mut steps = 0;
    for scenario in corpus.scenarios {
        let input = scenario.input;
        let mut state = MarkerSuppressionState {
            suppressed_sig: input.initial_sig,
            suppressed_at: input.initial_at.value(),
        };
        for row in scenario.steps {
            steps += 1;
            let step = row.input;
            if step.reset {
                state.reset_history();
            }
            if step.warm {
                state = state.clone();
            }
            let candidate = step.marker.map(|m| Marker {
                sig: m.sig,
                block: m.block,
            });
            let mut trace = Vec::<String>::new();
            let mut warnings = Vec::new();
            let mut broadcasts = Vec::new();
            let mut result = false;
            // The fixed method's missing-session guard belongs to its caller,
            // not another registry inside this state machine.
            if input.missing {
                assert!(!row.expected.reason.is_empty());
                trace.extend(["lock".into(), "unlock".into()]);
            } else {
                match state.evaluate(candidate.as_ref(), step.at.value()) {
                    MarkerSuppressionDecision::Empty => {
                        assert!(row.expected.reason.is_empty());
                    }
                    MarkerSuppressionDecision::Valid => {
                        assert!(row.expected.reason.is_empty());
                        trace.push("open-question-entry".into());
                        result = input.open_result;
                    }
                    MarkerSuppressionDecision::Suppressed {
                        reason,
                        new_signature,
                        notify,
                        lines,
                    } => {
                        assert_eq!(reason, row.expected.reason, "{}", input.name);
                        trace.push("lock".into());
                        if notify {
                            trace.push("probe-call-site:approval-corrupt-snapshot".into());
                        }
                        trace.push("unlock".into());
                        let candidate = candidate.as_ref().unwrap();
                        if new_signature && input.logger {
                            trace.push("warn".into());
                            warnings.push(json!({
                                "message": "approval marker suppressed: corrupt block",
                                "session_id": 7,
                                "provider": "synthetic",
                                "reason": reason,
                                "sig": &candidate.sig[..candidate.sig.len().min(10)],
                                "lines": lines,
                            }));
                        }
                        if notify {
                            trace.push("probe-call-site:approval-corrupt-dump".into());
                            trace.push("broadcast".into());
                            broadcasts.push(json!({
                                "type": "approval_marker_suppressed",
                                "session_id": 7,
                                "provider": "synthetic",
                                "approval_sig": candidate.sig,
                                "approval_source": step.source,
                                "reason": reason,
                                "detected_at": crate::proto::time::format_go_rfc3339_layout(
                                    step.at.value(), 0, false).unwrap(),
                            }));
                        }
                    }
                }
            }
            assert_eq!(state.suppressed_sig(), row.expected.sig, "{}", input.name);
            assert_eq!(
                state.suppressed_at(),
                row.expected.at.value(),
                "{}",
                input.name
            );
            assert_eq!(trace, row.expected.trace, "{}", input.name);
            assert_eq!(warnings, row.expected.warnings, "{}", input.name);
            assert_eq!(broadcasts, row.expected.broadcasts, "{}", input.name);
            assert_eq!(result, row.expected.result, "{}", input.name);
            // The source method must not pass corrupt blocks into record logic.
            assert_eq!(row.expected.record, "immutable-record-sentinel");
        }
    }
    assert_eq!(steps, 108);
}

#[test]
fn repeated_signature_never_reissues_even_after_valid_or_empty_marker() {
    let mut state = MarkerSuppressionState::default();
    state.evaluate(Some(&marker("same")), at(100, 0));
    let previous = state.clone();
    assert_eq!(
        state.evaluate(None, at(200, 0)),
        MarkerSuppressionDecision::Empty
    );
    let valid = Marker {
        block: "1. First\n2. Other".into(),
        sig: "valid".into(),
    };
    assert_eq!(
        state.evaluate(Some(&valid), at(201, 0)),
        MarkerSuppressionDecision::Valid
    );
    assert_eq!(state, previous);
    assert_eq!(
        state.evaluate(Some(&marker("same")), at(1000, 0)),
        MarkerSuppressionDecision::Suppressed {
            reason: "option_start",
            new_signature: false,
            notify: false,
            lines: 2,
        }
    );
    assert_eq!(state, previous);
}

#[test]
fn signature_changes_warn_without_moving_throttled_notice_time() {
    let mut state = MarkerSuppressionState::default();
    state.evaluate(Some(&marker("a")), at(100, 1));
    for (sig, now) in [("b", at(110, 0)), ("a", at(130, 0))] {
        assert_eq!(
            state.evaluate(Some(&marker(sig)), now),
            MarkerSuppressionDecision::Suppressed {
                reason: "option_start",
                new_signature: true,
                notify: false,
                lines: 2,
            }
        );
        assert_eq!(state.suppressed_at(), at(100, 1));
    }
    assert!(matches!(
        state.evaluate(Some(&marker("b")), at(130, 1)),
        MarkerSuppressionDecision::Suppressed { notify: true, .. }
    ));
    assert_eq!(state.suppressed_at(), at(130, 1));
}

#[test]
fn exact_zero_time_and_negative_elapsed_preserve_go_semantics() {
    let mut state = MarkerSuppressionState::default();
    for sig in ["a", "b"] {
        assert!(matches!(
            state.evaluate(Some(&marker(sig)), GO_ZERO),
            MarkerSuppressionDecision::Suppressed { notify: true, .. }
        ));
        assert_eq!(state.suppressed_at(), GO_ZERO);
    }
    assert!(matches!(
        state.evaluate(Some(&marker("c")), GO_ZERO - Duration::from_nanos(1)),
        MarkerSuppressionDecision::Suppressed { notify: true, .. }
    ));
    assert!(matches!(
        state.evaluate(Some(&marker("d")), GO_ZERO - Duration::from_secs(2)),
        MarkerSuppressionDecision::Suppressed { notify: false, .. }
    ));
}

#[test]
fn warm_reattach_preserves_and_history_reset_clears_both_fields() {
    let mut state = MarkerSuppressionState::default();
    state.evaluate(Some(&marker("a")), at(100, 0));
    let mut replacement = state.clone();
    assert_eq!(replacement, state);
    assert!(matches!(
        replacement.evaluate(Some(&marker("a")), at(200, 0)),
        MarkerSuppressionDecision::Suppressed { notify: false, .. }
    ));
    replacement.reset_history();
    assert_eq!(replacement, MarkerSuppressionState::default());
    assert!(matches!(
        replacement.evaluate(Some(&marker("a")), at(101, 0)),
        MarkerSuppressionDecision::Suppressed { notify: true, .. }
    ));
}

#[test]
fn go_display_layout_retains_extended_years_and_unusual_zone_offsets() {
    use crate::proto::time::{format_go_rfc3339_layout, format_with_offset, from_utc};

    // Observed with pinned Go 1.26.8 using time_layout_oracle.go. Go Format is
    // less restrictive than RFC3339 parsing/serialization at these boundaries.
    for (year, text) in [
        (-1, "-0001"),
        (0, "0000"),
        (1, "0001"),
        (9999, "9999"),
        (10000, "10000"),
    ] {
        let date = chrono::NaiveDate::from_ymd_opt(year, 1, 1)
            .unwrap()
            .and_hms_nano_opt(0, 0, 0, 123_400_000)
            .unwrap()
            .and_utc();
        let stamp = from_utc(date).unwrap();
        assert_eq!(
            format_go_rfc3339_layout(stamp, 0, false).unwrap(),
            format!("{text}-01-01T00:00:00Z")
        );
        assert_eq!(
            format_go_rfc3339_layout(stamp, 0, true).unwrap(),
            format!("{text}-01-01T00:00:00.1234Z")
        );
        if !(0..=9999).contains(&year) {
            assert!(format_with_offset(stamp, 0, false).is_err());
        }
    }
    for (offset, text) in [
        (-86400, "1969-12-31T00:00:00.000000001-24:00"),
        (-30, "1969-12-31T23:59:30.000000001+00:00"),
        (30, "1970-01-01T00:00:30.000000001+00:00"),
        (86400, "1970-01-02T00:00:00.000000001+24:00"),
    ] {
        assert_eq!(
            format_go_rfc3339_layout(at(0, 1), offset, true).unwrap(),
            text
        );
    }
    assert!(format_go_rfc3339_layout(at(i64::MAX, 0), 0, false).is_err());
}

#[test]
fn hand_seeded_legacy_predicate_is_evidence_not_another_consumed_owner() {
    let corpus = corpus();
    assert_eq!(corpus.legacy.len(), 48);
    for row in corpus.legacy {
        let input = row.input;
        // Test-only expression reproduces the source branch to expose its
        // negative-time and strict TTL behavior. No production state stores it.
        let legacy = input.key.is_empty()
            && input.consumed_sig == input.candidate_sig
            && input.now.value().unix_nanos() - input.at.value().unix_nanos() < 10_000_000_000;
        assert_eq!(legacy, row.expected);
        if !input.key.is_empty() {
            assert!(!row.expected);
        }
    }
    assert!(corpus.source_census.iter().any(|line| line.contains(
        "approval_native.go:63:\tlegacyConsumed := ses.approvalConsumedCandidateKey == \"\""
    )));
    assert_eq!(corpus.probe_sink["status"], "removed");
    assert_eq!(corpus.probe_sink["removedOn"], "2026-09-07");
}
