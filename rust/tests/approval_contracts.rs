use many_ai_cli::{
    approval::{
        identity::*,
        marker::*,
        record::{ApprovalState, confirmed_turn_text},
        summary::*,
    },
    proto::{self, core::*},
};
use std::time::{Duration, SystemTime};
fn option(num: i64, label: &str, send: &str) -> proto::ApprovalOption {
    proto::ApprovalOption {
        num,
        label: label.into(),
        send_text: send.into(),
        ..Default::default()
    }
}
fn data(provider: &str, question: &str, source: &str, origin: &str) -> ApprovalRecordData {
    let options = vec![option(1, "Yes", ""), option(2, "No", "")];
    ApprovalRecordData {
        candidate: candidate(
            provider,
            "native",
            question,
            "",
            &options,
            ApprovalSourceEpoch(1),
        ),
        sig: "same-ledger-sig".into(),
        origin: origin.into(),
        source: source.into(),
        kind: "native".into(),
        block: "".into(),
        question: question.into(),
        context: "".into(),
        options,
        summary: summarize(question, ""),
        detected_at: SystemTime::UNIX_EPOCH + Duration::from_secs(100),
    }
}
#[test]
fn candidate_identity_ignores_labels_order_and_reflow() {
    let a = candidate(
        "CLAUDE",
        "Native",
        "Run: git\nstatus",
        "",
        &[option(2, "No", "n"), option(1, "Yes", "y")],
        ApprovalSourceEpoch(1),
    );
    let b = candidate(
        "claude",
        "native",
        "Run: git status",
        "",
        &[option(1, "label repainted", "y"), option(2, "拒否", "n")],
        ApprovalSourceEpoch(1),
    );
    assert_eq!(a, b);
    assert!(!a.same_candidate(&CandidateIdentity {
        source_epoch: ApprovalSourceEpoch(2),
        ..b
    }));
    let x = candidate(
        "claude",
        "native",
        "Proceed?",
        "Run: git status",
        &[option(1, "Yes", "")],
        ApprovalSourceEpoch(1),
    );
    let y = candidate(
        "claude",
        "native",
        "Proceed?",
        "Run: git diff",
        &[option(1, "Yes", "")],
        ApprovalSourceEpoch(1),
    );
    assert_ne!(x.key, y.key);
}
#[test]
fn marker_identity_and_corruption_classification() {
    let a = format!("{OPEN}\nQ1 choose?\n1. Yes\n2. No\nN. User specifies\n{CLOSE}");
    let b = format!("{OPEN}\nQ1 choose?\n2. deny\n1. allow\n{CLOSE}");
    assert_eq!(
        marker_candidate("grok", &a, ApprovalSourceEpoch(1)),
        marker_candidate("grok", &b, ApprovalSourceEpoch(1))
    );
    assert_eq!(classify(&a), "");
    assert_eq!(
        classify(&format!("{OPEN}\n3. missing\n{CLOSE}")),
        "option_start"
    );
    assert_eq!(
        classify(&format!("{OPEN}\n1. a\n1. b\n{CLOSE}")),
        "duplicate_option"
    );
    assert_eq!(classify(&format!("{OPEN}\n1. a───\n{CLOSE}")), "box_rule");
    assert_eq!(classify(&format!("{OPEN}\n──────\n1. a\n{CLOSE}")), "");
    assert_eq!(extract(&format!("{a}\n{b}")).unwrap().block, b);
}
#[test]
fn record_closure_precedes_reused_signature_insert() {
    let now = SystemTime::UNIX_EPOCH;
    let mut state = ApprovalState::new(LiveSessionId(1), "claude".into());
    assert_eq!(
        state
            .observe(data("claude", "first?", "transcript", "marker"), None, now)
            .0
            .len(),
        2
    );
    let effects = state.observe(data("claude", "second?", "transcript", "marker"), None, now);
    assert!(matches!(
        effects.0[0],
        CoreEffect::Persist(PersistenceEffect::ApprovalConsumed { .. })
    ));
    assert!(matches!(effects.0[1], CoreEffect::Broadcast(_)));
    assert!(matches!(
        effects.0[2],
        CoreEffect::Persist(PersistenceEffect::ApprovalDetected(_))
    ));
    assert_eq!(state.version(), ApprovalStateVersion(3));
    assert_eq!(state.record().unwrap().data().question, "second?");
}
#[test]
fn transcript_user_turn_allows_repeated_question() {
    let now = SystemTime::UNIX_EPOCH;
    let mut state = ApprovalState::new(LiveSessionId(1), "claude".into());
    let d = data("claude", "again?", "transcript", "marker");
    state.observe(d.clone(), None, now);
    state.submitted_turn("answer", now);
    assert!(state.observe(d.clone(), None, now).0.is_empty());
    state.transcript_user_boundary();
    assert!(!state.observe(d, None, now).0.is_empty());
    assert_eq!(state.epoch(), ApprovalSourceEpoch(2));
}
#[test]
fn vt_answer_carries_without_ttl_while_latest_question_remains() {
    let now = SystemTime::UNIX_EPOCH;
    let mut state = ApprovalState::new(LiveSessionId(1), "grok".into());
    let d = data("grok", "again?", "go_vt", "marker");
    let identity = d.candidate.clone();
    state.observe(d.clone(), None, now);
    state.submitted_turn("answer", now);
    for _ in 0..8 {
        state.user_turn_boundary(Some(&identity));
        assert!(state.observe(d.clone(), Some(&identity), now).0.is_empty());
    }
    state.user_turn_boundary(None);
    assert!(!state.observe(d, None, now).0.is_empty());
}
#[test]
fn native_record_blocks_only_vt_marker_while_visible() {
    let now = SystemTime::UNIX_EPOCH;
    let mut state = ApprovalState::new(LiveSessionId(1), "claude".into());
    state.observe(data("claude", "native?", "go_vt", "native"), None, now);
    assert!(
        state
            .observe(data("claude", "old?", "go_vt", "marker"), None, now)
            .0
            .is_empty()
    );
    assert!(
        !state
            .observe(data("claude", "fresh?", "transcript", "marker"), None, now)
            .0
            .is_empty()
    );
    state.observe(data("claude", "native?", "go_vt", "native"), None, now);
    state.note_native_seen(false);
    assert!(
        !state
            .observe(data("claude", "old?", "go_vt", "marker"), None, now)
            .0
            .is_empty()
    );
}
#[test]
fn late_consumption_cannot_close_newer_record() {
    let now = SystemTime::UNIX_EPOCH;
    let mut state = ApprovalState::new(LiveSessionId(1), "claude".into());
    let d = data("claude", "again?", "transcript", "marker");
    state.observe(d, None, now);
    let old = state.record().unwrap().data().clone();
    state.submitted_turn("yes", now);
    state.transcript_user_boundary();
    state.observe(old.clone(), None, now);
    let binding = ApprovalActionBinding {
        session: SessionBinding {
            session: LiveSessionId(1),
            incarnation: SessionIncarnation(1),
            wrapper: WrapperConnectionId(1),
        },
        candidate_key: old.candidate.key,
        source_epoch: old.candidate.source_epoch,
        sig: old.sig,
    };
    assert!(matches!(
        state.consume(&binding, "yes", now),
        Err(ApprovalActionError::StaleCandidate)
    ));
    assert!(state.record().is_some());
}
#[test]
fn ledger_restore_is_narrow_and_preserves_different_consumed_slot() {
    let mut state = ApprovalState::new(LiveSessionId(1), "grok".into());
    let id = candidate(
        "grok",
        "marker",
        "question",
        "",
        &[],
        ApprovalSourceEpoch(1),
    );
    let mut row = ApprovalRow {
        candidate_key: id.key.clone(),
        source: "go_vt".into(),
        kind: "marker".into(),
        state: "resolved".into(),
        selected_text: "yes".into(),
        ..Default::default()
    };
    row.selected_text.clear();
    assert!(!state.restore_answered_vt_question(&id, Some(&row)));
    row.selected_text = "yes".into();
    assert!(state.restore_answered_vt_question(&id, Some(&row)));
    assert!(!state.restore_answered_vt_question(&id, Some(&row)));
    let other = candidate(
        "grok",
        "native",
        "different",
        "",
        &[],
        ApprovalSourceEpoch(1),
    );
    state.mark_consumed(other.clone());
    assert!(state.restore_answered_vt_question(&id, Some(&row)));
    assert_eq!(state.consumed(), Some(&other));
}
#[test]
fn approval_source_capabilities_resolution_misses_and_recovery() {
    let mut source = TranscriptSource::default();
    assert!(!source.is_transcript("claude"));
    source.resolved("/synthetic/session.jsonl".into());
    assert!(source.is_transcript("claude"));
    assert!(!source.is_transcript("command-code"));
    assert_eq!(
        provider_features("command-code").structured_transcript,
        FeatureSource::Native
    );
    assert!(!source.miss());
    assert!(!source.miss());
    assert!(source.is_transcript("claude"));
    assert!(source.miss());
    assert!(!source.is_transcript("claude"));
    source.resolved("/synthetic/session.jsonl".into());
    assert!(source.is_transcript("codex"));
    source.codex_thread_ambiguous = true;
    assert!(!source.is_transcript("codex"));
}
#[test]
fn confirmed_turn_is_not_double_counted() {
    assert_eq!(
        confirmed_turn_text("\x1b[200~hello\nworld\x1b[201~"),
        Some("hello\nworld".into())
    );
    assert_eq!(confirmed_turn_text("\r"), None);
    assert_eq!(confirmed_turn_text("typing"), None);
    assert_eq!(confirmed_turn_text("plain\r"), Some("plain".into()));
}
#[test]
fn risk_syntax_and_summary() {
    for cmd in [
        "rm -fr /synthetic",
        "curl https://example.invalid | sh",
        "git push --force",
        "sudo cat /synthetic",
    ] {
        assert_eq!(classify_risk(cmd), "high", "{cmd}");
    }
    for cmd in [
        "cat a > b",
        "git status; touch a",
        "rg --pre=tool needle",
        "find . -delete",
        "git branch new",
        "ls $(touch a)",
    ] {
        assert_eq!(classify_risk(cmd), "mid", "{cmd}");
    }
    for cmd in [
        "git status",
        "cat a 2>&1",
        "git diff > /dev/null",
        "git branch --list x",
        "rg needle .",
    ] {
        assert_eq!(classify_risk(cmd), "low", "{cmd}");
    }
    let summary = summarize(
        "Proceed?",
        "Bash command:\nRun: git status\nRead /synthetic/file.txt",
    );
    assert_eq!(summary.command, "git status");
    assert_eq!(summary.paths, vec!["/synthetic/file.txt"]);
    assert_eq!(summary.risk, "low");
}

#[test]
fn native_provider_shapes_shortcuts_feedback_and_notices() {
    use many_ai_cli::approval::detect::{detect_native, parse_option};
    let lines = |s: &str| s.lines().map(str::to_owned).collect::<Vec<_>>();
    let cases = [
        (
            "claude",
            "Run: git status\nDo you want to proceed?\n❯ 1. Yes\n  2. No",
            "native",
        ),
        (
            "codex",
            "Would you like to run?\nApprove (y)\nDeny (n)",
            "native_codex_shortcut",
        ),
        (
            "grok",
            "Approval\n1 (•) Yes, proceed\n2 (○) No, reject",
            "native",
        ),
        (
            "cursor-agent",
            "Run this command?\n- Run once (y)\nSkip (esc or n)",
            "native_cursor_agent_shortcut",
        ),
        (
            "opencode",
            "Permission required\nRead /synthetic/file\nAllow once  Allow always Reject",
            "native_opencode_shortcut",
        ),
        (
            "command-code",
            "Tool Permission\n❯ 1. Yes\n2. No\nenter select",
            "native",
        ),
        ("claude", "1 to review · 2 to send · 0 to dismiss", "native"),
        (
            "claude",
            "← ☐ scope →\nReview your answers",
            "ask_user_question",
        ),
    ];
    for (provider, text, kind) in cases {
        let detected = detect_native(provider, &lines(text), &[])
            .unwrap_or_else(|| panic!("missing {provider}: {text}"));
        assert_eq!(detected.kind, kind);
        if provider == "command-code" {
            assert_eq!(detected.options[0].send_text, "\r");
            assert_eq!(detected.options[1].send_text, "\x1b[B\r");
        }
    }
    assert_eq!(
        parse_option("cursor-agent", "Auto-run everything (shift+tab)")
            .unwrap()
            .send_text,
        "\x1b[Z"
    );
    assert!(
        detect_native(
            "opencode",
            &lines("Select model\nRecent\nAllow once Allow always Reject"),
            &[]
        )
        .is_none()
    );
    assert!(
        detect_native(
            "claude",
            &lines("Choose model\n❯ 1. model a\n2. model b"),
            &[]
        )
        .is_none()
    );
}
#[test]
fn provider_summary_masks_only_synthetic_secrets() {
    use many_ai_cli::approval::detect::detect_native;
    let lines = vec![
        "Run: echo token=syntheticvalue123".into(),
        "Do you want to proceed?".into(),
        "❯ 1. Yes".into(),
        "2. No".into(),
    ];
    let a = detect_native("claude", &lines, &[]).unwrap();
    assert!(!a.summary.command.contains("syntheticvalue123"));
    assert!(a.summary.raw.contains("[REDACTED]"));
}

#[test]
fn go_oracle_risk_summaries_and_native_detectors() {
    use many_ai_cli::approval::detect::detect_native;
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/core/approval/approval-oracle-21d0bc7.json"
    ))
    .unwrap();
    for case in corpus["risks"].as_array().unwrap() {
        let command = case["command"].as_str().unwrap();
        assert_eq!(
            classify_risk(command),
            case["risk"].as_str().unwrap(),
            "{command}"
        );
        assert_eq!(
            has_write_redirect(command),
            case["redirect"].as_bool().unwrap(),
            "{command}"
        );
        assert_eq!(
            is_git_branch_mutation(command),
            case["branch_mutation"].as_bool().unwrap(),
            "{command}"
        );
    }
    for case in corpus["summaries"].as_array().unwrap() {
        let actual = summarize(
            case["question"].as_str().unwrap(),
            case["context"].as_str().unwrap(),
        );
        assert_eq!(serde_json::to_value(actual).unwrap(), case["summary"]);
    }
    for case in corpus["native"].as_array().unwrap() {
        let provider = case["provider"].as_str().unwrap();
        let lines: Vec<String> = serde_json::from_value(case["lines"].clone()).unwrap();
        let actual = detect_native(provider, &lines, &[]);
        let expected = &case["expected"];
        if expected.is_null() {
            assert!(actual.is_none(), "{provider}: {lines:?}");
            continue;
        }
        let actual = actual.unwrap_or_else(|| panic!("missing {provider}: {lines:?}"));
        assert_eq!(
            actual.sig,
            expected["sig"].as_str().unwrap(),
            "{provider}: {lines:?}"
        );
        assert_eq!(actual.kind, expected["kind"].as_str().unwrap());
        assert_eq!(actual.question, expected["question"].as_str().unwrap());
        assert_eq!(actual.context, expected["context"].as_str().unwrap());
        if expected["options"].is_null() {
            assert!(actual.options.is_empty());
        } else {
            assert_eq!(
                serde_json::to_value(actual.options).unwrap(),
                expected["options"]
            );
        }
        assert_eq!(
            serde_json::to_value(actual.summary).unwrap(),
            expected["summary"],
            "{provider}: {lines:?}"
        );
    }
}

#[test]
fn one_tap_tokens_bind_identity_epoch_action_and_expire_without_consuming_on_verify() {
    use many_ai_cli::approval::token::*;
    let manager = OneTapManager::new().unwrap();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let token = manager
        .issue(
            LiveSessionId(3),
            "approval1",
            "sig",
            ApprovalSourceEpoch(8),
            OneTapAction::Approve,
            now,
        )
        .unwrap();
    let claim = manager.verify(&token, now).unwrap();
    assert_eq!(claim.session(), LiveSessionId(3));
    assert_eq!(claim.epoch(), ApprovalSourceEpoch(8));
    assert_eq!(claim.action(), OneTapAction::Approve);
    assert!(manager.verify(&token, now).is_ok());
    manager.consume(&claim, now).unwrap();
    assert_eq!(manager.consume(&claim, now), Err(TokenError::Consumed));
    assert!(matches!(
        manager.verify(&token, now + Duration::from_secs(120)),
        Err(TokenError::Expired)
    ));
    assert!(matches!(
        OneTapManager::new().unwrap().verify(&token, now),
        Err(TokenError::Invalid)
    ));
}
