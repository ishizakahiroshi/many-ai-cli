//! One-shot action selection from auto_approval.go / approval_action.go.
//! Selection consumes freshly detected options under the session input lane.
//! Persistent permission options are not one-shot approvals.
use crate::proto::ApprovalOption;

pub fn normalize_option_label(text: &str) -> String {
    let mut label = text
        .trim()
        .to_lowercase()
        .replace('’', "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for suffix in [
        " (y)",
        " (p)",
        " (n)",
        " (esc)",
        " (escape)",
        " (!)",
        " (?)",
        " (#)",
        " (recommended)",
    ] {
        if let Some(value) = label.strip_suffix(suffix) {
            label = value.trim().into();
            break;
        }
    }
    label
}
pub fn is_explicit_negative_label(text: &str) -> bool {
    let label = normalize_option_label(text);
    matches!(
        label.as_str(),
        "no" | "deny"
            | "deny once"
            | "reject"
            | "cancel"
            | "skip"
            | "abort"
            | "decline"
            | "do not allow"
            | "don't allow"
            | "do not run"
            | "don't run"
            | "拒否"
            | "許可しない"
            | "中止"
    ) || [
        "no ", "deny ", "reject ", "cancel ", "skip ", "abort ", "decline ",
    ]
    .iter()
    .any(|prefix| label.starts_with(prefix))
}
pub fn is_explicit_positive_label(text: &str) -> bool {
    let label = normalize_option_label(text);
    if is_explicit_negative_label(&label)
        || [
            "always",
            "don't ask",
            "dont ask",
            "all similar",
            "this session",
            "session",
        ]
        .iter()
        .any(|part| label.contains(part))
    {
        return false;
    }
    matches!(
        label.as_str(),
        "yes"
            | "yes, allow once"
            | "allow"
            | "allow once"
            | "approve"
            | "run"
            | "run command"
            | "run (once)"
            | "continue"
            | "proceed"
            | "yes, proceed"
            | "yes proceed"
            | "許可"
            | "実行"
            | "続行"
    )
}
/// Exact user action validation can inspect its live option without classifying
/// it as a safe one-shot operation. This does not grant permission by itself.
pub(crate) fn raw_option_input(option: &ApprovalOption) -> String {
    if !option.send_text.is_empty() {
        option.send_text.clone()
    } else if option.is_current {
        "\r".into()
    } else if option.num > 0 {
        format!("{}\r", option.num)
    } else {
        String::new()
    }
}
pub fn option_input(option: &ApprovalOption, positive: bool) -> String {
    if if positive {
        !is_explicit_positive_label(&option.label)
    } else {
        !is_explicit_negative_label(&option.label)
    } {
        return String::new();
    }
    raw_option_input(option)
}
pub fn approve_once_input(options: &[ApprovalOption]) -> String {
    options
        .iter()
        .map(|option| option_input(option, true))
        .find(|input| !input.is_empty())
        .unwrap_or_default()
}
pub fn reject_once_input(options: &[ApprovalOption]) -> String {
    options
        .iter()
        .map(|option| option_input(option, false))
        .find(|input| !input.is_empty())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_selection_matches_fixed_go_labels_options_and_focus() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/core/approval/selection-oracle-21d0bc7.json"
        ))
        .unwrap();
        let labels = corpus["labels"].as_array().unwrap();
        assert_eq!(labels.len(), 27);
        for case in labels {
            let label = case["label"].as_str().unwrap();
            assert_eq!(
                normalize_option_label(label),
                case["normalized"].as_str().unwrap(),
                "{label:?}"
            );
            assert_eq!(
                is_explicit_positive_label(label),
                case["positive"].as_bool().unwrap(),
                "{label:?}"
            );
            assert_eq!(
                is_explicit_negative_label(label),
                case["negative"].as_bool().unwrap(),
                "{label:?}"
            );
        }
        let cases = corpus["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 8);
        for (index, case) in cases.iter().enumerate() {
            let options: Vec<ApprovalOption> =
                serde_json::from_value(case["options"].clone()).unwrap();
            assert_eq!(
                approve_once_input(&options),
                case["approve"].as_str().unwrap(),
                "case {index}"
            );
            assert_eq!(
                reject_once_input(&options),
                case["reject"].as_str().unwrap(),
                "case {index}"
            );
        }
    }
}
