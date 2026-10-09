use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Observation {
    pattern: String,
    #[serde(default)]
    values: Option<Vec<String>>,
    valid: bool,
    #[serde(default)]
    matches: Option<Vec<bool>>,
}
fn expected_surrogate_gap(pattern: &str) -> bool {
    if [
        r"^\x{d800}?$",
        r"^\x{d800}*$",
        r"^\x{d800}|a$",
        r"^a(?:)\x{d800}$",
        r"^(?:)\x{d800}$",
    ]
    .contains(&pattern)
    {
        return true;
    }
    ["", "(?i)", "(?s)", "(?i-i)"].into_iter().any(|prefix| {
        ["*", "+", "?", "{0}", "{2}", "{2,}"]
            .into_iter()
            .any(|quantifier| pattern == format!("{prefix}^(?:\\x{{d800}}){quantifier}$"))
    })
}
fn check(case: Observation) {
    let result = compile(&case.pattern);
    if expected_surrogate_gap(&case.pattern) {
        assert!(case.valid, "{}", case.pattern);
        assert_eq!(
            result.unwrap_err(),
            RegexIssue::Unsupported(SURROGATE_GAP),
            "{}",
            case.pattern
        );
        return;
    }
    assert_eq!(result.is_ok(), case.valid, "{}: {result:?}", case.pattern);
    if let Ok(re) = result {
        let values = case.values.unwrap_or_default();
        let matches = case.matches.unwrap_or_default();
        assert_eq!(values.len(), matches.len(), "{}", case.pattern);
        for (value, expected) in values.into_iter().zip(matches) {
            assert_eq!(
                re.is_match(&value),
                expected,
                "{} on {value:?}",
                case.pattern
            );
        }
    }
}

#[test]
fn pinned_go_unicode_property_fold_and_syntax_oracle() {
    let cases: Vec<Observation> = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/services/autoapproval-unicode/go_oracle.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 7028);
    assert_eq!(
        cases
            .iter()
            .filter(|case| expected_surrogate_gap(&case.pattern))
            .count(),
        29
    );
    for case in cases {
        check(case);
    }
}

#[test]
fn previous_go_regex_corpus_now_including_all_compatibility_gaps() {
    let value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/services/autoapproval/go_oracle.json"
    ))
    .unwrap();
    for case in value["regexes"].as_array().unwrap() {
        check(serde_json::from_value(case.clone()).unwrap());
    }
}

#[test]
fn generated_tables_are_sorted_complete_fold_cycles_and_scalar_safe() {
    assert_eq!(tables::PROPERTIES.len(), 158);
    for pair in tables::PROPERTIES.windows(2) {
        assert!(pair[0].0 < pair[1].0);
    }
    for pair in tables::ALIASES.windows(2) {
        assert!(pair[0].0 < pair[1].0);
    }
    for pair in tables::SIMPLE_FOLD.windows(2) {
        assert!(pair[0].0 < pair[1].0);
    }
    for &(name, normal, folded) in tables::PROPERTIES {
        for set in [normal, folded] {
            let mut normalized = set.to_vec();
            normalize(&mut normalized);
            assert_eq!(set, normalized, "{name}");
            for &(lo, hi) in set {
                assert!(lo <= hi && hi <= MAX_RUNE);
            }
        }
    }
    for &(rune, next) in tables::SIMPLE_FOLD {
        assert_ne!(rune, next);
        let mut value = next;
        let mut count = 0;
        while value != rune {
            assert!(count < 4, "broken fold orbit for {rune:x}");
            value = simple_fold(value);
            count += 1;
        }
    }
}

#[test]
fn unicode_16_and_17_assignments_and_folds_cannot_widen_authorization() {
    // Cyrillic Tje (16), CJK Extension I (15.1), and Extension J (17).
    for value in ['\u{1c89}', '\u{1c8a}', '\u{2ebf0}', '\u{323b0}'] {
        let value = value.to_string();
        assert!(!compile(r"^\p{Assigned}$").unwrap().is_match(&value));
        assert!(!compile(r"^\pL$").unwrap().is_match(&value));
        assert!(compile(r"^\p{Cn}$").unwrap().is_match(&value));
    }
    assert!(!compile(r"(?i)^\x{1c89}$").unwrap().is_match("\u{1c8a}"));
    assert!(compile(r"(?i)^\x{1c89}$").unwrap().is_match("\u{1c89}"));
    assert!(compile(r"^\p{Han}$").unwrap().is_match("\u{31350}")); // Extension H: 15.0.
    assert!(compile(r"(?i)^k$").unwrap().is_match("K"));
    assert!(compile(r"(?i)^s$").unwrap().is_match("ſ"));
}

#[test]
fn surrogate_atoms_preserve_empty_language_versus_empty_match() {
    for p in [r"^[\x{d800}-\x{dfff}]$", r"^\p{Cs}$"] {
        let re = compile(p).unwrap();
        for value in ["", "a", "�", "\u{d7ff}", "\u{e000}"] {
            assert!(!re.is_match(value), "{p}");
        }
    }
    assert!(compile(r"^[\x{d800}-\x{dfff}]?$").unwrap().is_match(""));
    assert!(compile(r"^[\x{d800}-\x{dfff}]*$").unwrap().is_match(""));
    assert!(!compile(r"^[\x{d800}-\x{dfff}]+$").unwrap().is_match(""));
    assert!(compile(r"^\x{d800}$").unwrap().is_match("�"));
    assert!(!compile(r"\x{d800}").unwrap().is_match("�"));
    assert_eq!(
        compile(r"^\x{d800}?$").unwrap_err(),
        RegexIssue::Unsupported(SURROGATE_GAP)
    );
    let re = compile(r"^[\x{d7ff}-\x{e000}]$").unwrap();
    assert!(re.is_match("\u{d7ff}"));
    assert!(re.is_match("\u{e000}"));
    assert!(!re.is_match("�"));
    assert!(compile(r"^[^\x{d800}-\x{dfff}]$").unwrap().is_match("�"));
    assert_eq!(compile(r"\x{110000}").unwrap_err(), RegexIssue::Invalid);
}

#[test]
fn scoped_flags_and_ascii_boundaries_do_not_leak() {
    assert!(compile(r"^(?i:λ)(?-i:λ)(?i:λ)$").unwrap().is_match("ΛλΛ"));
    assert!(!compile(r"^(?i:λ)(?-i:λ)(?i:λ)$").unwrap().is_match("ΛΛΛ"));
    assert!(!compile(r"^(?i:λ)λ$").unwrap().is_match("ΛΛ"));
    assert!(compile(r"(?i)\bλ\b").unwrap().is_match("aλa"));
    assert!(!compile(r"(?i)\bλ\b").unwrap().is_match(" λ "));
    assert!(compile(r"(?m:^λ$)").unwrap().is_match("x\nλ\ny"));
    assert!(!compile(r"(?m:^λ$)").unwrap().is_match("λ\r\n"));
    assert!(compile(r"(?s)^.$").unwrap().is_match("\n"));
    assert!(!compile(r"(?s:.)^.$").unwrap().is_match("\na"));
}

#[test]
fn property_names_follow_pinned_parser_acceptance_not_raw_unicode_maps() {
    assert!(compile(r"\p{ lower-case_letter }").is_ok());
    for p in [
        r"\p{Anatolian_Hieroglyphs}",
        r"\p{Old_Italic}",
        r"\p{Alphabetic}",
        r"\p{sc=Greek}",
        r"\p{Script=Greek}",
    ] {
        assert_eq!(compile(p).unwrap_err(), RegexIssue::Invalid, "{p}");
    }
}
