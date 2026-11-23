use super::*;
#[derive(serde::Deserialize)]
struct Input {
    provider: String,
    cwd: String,
    lines: Vec<String>,
}
#[test]
fn pinned_go_banner_corpus() {
    let input: Vec<Input> = serde_json::from_str(include_str!("input.json")).unwrap();
    let expected: Vec<DetectedModel> = serde_json::from_str(include_str!("go.json")).unwrap();
    assert_eq!(input.len(), expected.len());
    assert_eq!(input.len(), 30);
    for (i, (input, want)) in input.iter().zip(expected).enumerate() {
        assert_eq!(
            banner(&input.provider, &input.cwd, &input.lines),
            want,
            "case {i}"
        );
    }
}
#[test]
fn change_requires_unsplit_raw_trigger_and_preserves_source_effort_case() {
    assert_eq!(
        change(
            "claude",
            b"Set model to ",
            "Set model to Opus 5 with high effort · Claude Pro"
        ),
        Some(detected("Opus 5", "high"))
    );
    assert_eq!(
        change("claude", b"model to Opus 5", "Set model to Opus 5"),
        None
    );
    assert_eq!(
        change(
            "codex",
            b"Model changed to ",
            "Model changed to gpt-5.3\rignored"
        ),
        Some(detected("gpt-5.3", ""))
    );
    assert_eq!(change("claude", b"Set model to ", "Set model to \t"), None);
    assert_eq!(change("shell", b"Set model to ", "Set model to Opus"), None);
}
#[test]
fn a_declared_change_pattern_reports_the_model_of_a_registered_provider() {
    let pattern = r"Now using ([^\r\n]+)";
    assert_eq!(
        change_with(pattern, false, b"Now using ", "Now using demo-2\r\n"),
        Some(detected("demo-2", ""))
    );
    assert_eq!(
        change_with(
            pattern,
            true,
            b"Now using ",
            "Now using demo-2 with high effort\r\n"
        ),
        Some(detected("demo-2", "high"))
    );
    // The raw bytes must hold the literal text before the capture group.
    assert_eq!(
        change_with(pattern, false, b"other", "Now using demo-2"),
        None
    );
    // A pattern that is not a regular expression never matches (it is refused
    // when a definition is accepted, so this is only a guard).
    assert_eq!(change_with("(", false, b"", "Now using demo-2"), None);
}
