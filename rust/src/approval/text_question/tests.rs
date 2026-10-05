use super::*;
#[test]
fn pinned_go_pure_grammar_oracle_matches_every_field_and_reconstructed_line() {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("oracle_cases.json")).unwrap();
    let expected: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("oracle_result.json")).unwrap();
    assert_eq!(cases.len(), 32);
    assert_eq!(expected.len(), cases.len());
    for (case, expected) in cases.iter().zip(expected) {
        let name = case["Name"].as_str().unwrap();
        assert_eq!(expected["Name"], name);
        let lines = case["Lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let actual=detect(&lines).map(|q|serde_json::json!({"Kind":q.kind,"Block":q.block,"Sig":q.sig,"Question":q.question,"Options":q.options})).unwrap_or(serde_json::Value::Null);
        assert_eq!(actual, expected["Question"], "Go question oracle: {name}");
        assert_eq!(
            serde_json::json!(unglued_lines(&lines)),
            expected["Lines"],
            "Go reconstructed lines oracle: {name}"
        );
    }
}
fn lines(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).into()).collect()
}
fn nums(q: &TextQuestion) -> Vec<i64> {
    q.options.iter().map(|o| o.num).collect()
}
#[test]
fn plain_yesno_source_fixtures_and_tail_boundaries() {
    for text in [
        "Do you want to apply this patch? (Y:1/N:0)",
        "A拠点・B拠点・C拠点の3台で対象機能を無効化しますか？ （Y：1／N：0）",
    ] {
        let q = detect(&lines(&[text])).unwrap();
        assert_eq!(q.kind, "plain_yes_no");
        assert_eq!(nums(&q), [1, 0]);
        assert!(q.options[0].preserve_order);
        assert!(q.options[1].preserve_order);
        assert_eq!(q.sig, identity::digest(&identity::normalize(&q.block)));
    }
    assert!(detect(&lines(&["無効化しますか？ (Y:1/", "N:0)"])).is_some());
    for text in [
        "question? (Y:1/N:0)",
        "Question 123？ (Y:1/N:0)",
        "This is not a question (Y:1/N:0)",
        "Question? (Y:\u{a0}1/N:0)",
    ] {
        assert!(detect(&lines(&[text])).is_none());
    }
    let mut screen = lines(&["Apply? (Y:1/N:0)"]);
    screen.extend(vec![String::new(); 80]);
    assert!(detect(&screen).is_some());
    screen = lines(&["Apply? (Y:1/N:0)"]);
    screen.extend(vec!["ordinary output".into(); 20]);
    assert!(detect(&screen).is_none());
}
#[test]
fn sequential_source_sorting_roundtrip_duplicates_and_marker_exclusion() {
    let source = lines(&[
        "Q1: Choose branch",
        "  2. develop",
        "  1. main",
        "  N. User specifies",
        "Q2: Run tests",
        "  1. Yes",
        "  2. No",
        "  N. User specifies",
    ]);
    let q = detect(&source).unwrap();
    assert_eq!(q.kind, "sequential_choice");
    assert_eq!(nums(&q), [1, 2, 1, 2]);
    assert!(q.options.iter().all(|v| !v.is_current));
    let roundtrip = detect(&q.block.lines().map(Into::into).collect::<Vec<_>>()).unwrap();
    assert_eq!(roundtrip.block, q.block);
    assert_eq!(roundtrip.sig, q.sig);
    let mut repeated = source.clone();
    repeated.extend(source);
    assert_eq!(detect(&repeated).unwrap().block, q.block);
    repeated.push(marker::OPEN.into());
    assert!(detect(&repeated).is_none());
    assert!(detect(&lines(&["Q1: Only one", "1. a", "2. b"])).is_none());
}
#[test]
fn legacy_source_structure_and_glued_wrapped_choices() {
    let q = detect(&lines(&[
        "Q1 どちらで進めますか？",
        "1. 最小修正 (Recommended)",
        "2. 原因調査も行う",
        "N. User specifies",
    ]))
    .unwrap();
    assert_eq!(q.kind, "hub_choice");
    assert_eq!(q.question, "Q1 どちらで進めますか？");
    assert_eq!(nums(&q), [1, 2]);
    assert!(q.options[0].is_current);
    let q = detect(&lines(&[
        "Which option? 1. [A] 説明A (Recommended)2. [B] 説明B3. [C] 説明C N. User specifies",
    ]))
    .unwrap();
    assert_eq!(nums(&q), [1, 2, 3]);
    assert!(
        q.options
            .iter()
            .all(|v| !v.label.contains("User specifies") && v.send_text.is_empty())
    );
    let f = fallback(&lines(&[
        "1. C4を使え",
        "る状態に。PINは見送り）  (Recommended)",
        "2. PIN一式も実装",
        "3. docsだけ先に作る",
        "N. User specifies",
    ]))
    .unwrap();
    assert_eq!(
        f.options.iter().map(|v| v.num).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(f.options[0].label.contains("る状態に"));
    assert!(f.options[0].label.contains("Recommended"));
    assert!(
        detect(&lines(&[
            "1. explanation",
            "2. explanation",
            "Q1 どちらで進めますか？",
            "N. User specifies"
        ]))
        .is_none()
    );
    assert!(
        detect(&lines(&[
            "Implementation notes:",
            "1. Read config",
            "2. Update renderer"
        ]))
        .is_none()
    );
}
#[test]
fn fallback_source_native_picker_rejection_radio_and_nonconsecutive_guards() {
    assert!(fallback(&lines(&["1. first 3. third"])).is_none());
    assert!(
        fallback(&lines(&[
            "❯ 1. Choice",
            "2. Another",
            "3. Type something.",
            "4. Chat about this"
        ]))
        .is_none()
    );
    let native = fallback(&lines(&[
        "This command requires approval",
        "❯ 1. Yes",
        "2. Yes, and don't ask again",
        "3. No",
    ]))
    .unwrap();
    assert_eq!(native.options.len(), 3);
    let radio = fallback(&lines(&[
        "1 (•) Yes, always approve",
        "2 (○) Yes, proceed",
        "3 (○) No, reject",
        "1/3:select | Tab:next option",
    ]))
    .unwrap();
    assert!(radio.options[0].is_current);
    assert!(!radio.options[1].is_current);
    assert_eq!(radio.options[1].label, "Yes, proceed");
    let keys = fallback(&lines(&["1. Yes (y)", "2. No (esc)"])).unwrap();
    assert_eq!(keys.options[0].send_text, "y");
    assert_eq!(keys.options[1].send_text, "\x1b");
    assert!(fallback(&lines(&["1. a", "21. b"])).is_none());
}
#[test]
fn unglue_preserves_proper_lines_and_quote_literal_headings() {
    let proper = unglued_lines(&lines(&[
        "  Q1 進め方",
        "  1. [A] 説明A (Recommended)",
        "  2. [B] 説明B",
        "   N. User specifies",
    ]));
    assert_eq!(
        proper.iter().filter(|v| v.trim().starts_with("1.")).count(),
        1
    );
    assert_eq!(
        proper.iter().filter(|v| v.trim().starts_with("2.")).count(),
        1
    );
    assert_eq!(
        unglued_lines(&lines(&["literal [Q1] label"])),
        ["literal [Q1] label"]
    );
    assert_eq!(
        unglued_lines(&lines(&["firstQ12: promptQ2: other"])),
        ["first", "Q12: prompt", "Q2: other"]
    );
    assert_eq!(unglued_lines(&lines(&["Q123: literal"])), ["Q123: literal"]);
}
