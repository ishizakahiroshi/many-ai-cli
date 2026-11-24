//! Read-only seed metadata and key-name drift. Never returns credential values.
pub use super::seed::{DiagnosticEntry, diagnostic_entries, sync_drift};
#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::seed::SettingsFormat;
    use std::collections::BTreeSet;
    #[test]
    fn toml_and_json_drift_compare_meaning_and_never_return_values_or_write() {
        let t = tempfile::tempdir().unwrap();
        let source = t.path().join("source.toml");
        let destination = t.path().join("profile.toml");
        std::fs::write(
            &source,
            "model = 'gpt-5.1-codex' # comment\nmodel_reasoning_effort = 'high'\nunknown = 'secret-value'\nhide_agent_reasoning = true\n",
        )
        .unwrap();
        std::fs::write(
            &destination,
            "model = 'gpt-5.1-codex'\nmodel_reasoning_effort = 'low'\n",
        )
        .unwrap();
        let original = std::fs::read(&destination).unwrap();
        let entry = DiagnosticEntry {
            format: Some(SettingsFormat::Codex),
            trial_root: None,
            source: source.clone(),
            dest: "config.toml",
            label: "fixture".into(),
            mirror: false,
            sync_owned: Some(BTreeSet::from(["owned".into()])),
        };
        let (added, changed) = sync_drift(&entry, &destination).unwrap();
        assert_eq!(added, vec!["hide_agent_reasoning"]);
        assert_eq!(changed, vec!["model_reasoning_effort"]);
        assert!(!format!("{added:?}{changed:?}").contains("secret-value"));
        assert_eq!(std::fs::read(&destination).unwrap(), original);
        std::fs::write(
            &source,
            r#"{"theme":"dark","env":{"arbitrary":"never-return"},"voiceEnabled":{"secret":"never-return"}}"#,
        )
        .unwrap();
        std::fs::write(&destination, r#"{"theme":"light"}"#).unwrap();
        let entry = DiagnosticEntry {
            format: Some(SettingsFormat::Claude),
            dest: "settings.json",
            ..entry
        };
        assert_eq!(
            sync_drift(&entry, &destination).unwrap(),
            (vec![], vec!["theme".into()])
        );
        std::fs::write(&destination, b"not JSON").unwrap();
        assert!(sync_drift(&entry, &destination).is_err());
    }
    #[test]
    fn diagnostic_trial_reader_rejects_outside_and_parent_paths_without_opening_them() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("runtime");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(
            t.path().join("private.json"),
            br#"{"token":"must-not-be-returned"}"#,
        )
        .unwrap();
        std::fs::write(root.join("safe.json"), br#"{"setting":true}"#).unwrap();
        let entry = DiagnosticEntry {
            format: None,
            trial_root: Some(root.clone()),
            source: root.join("safe.json"),
            dest: "safe.json",
            label: "fixture".into(),
            mirror: false,
            sync_owned: Some(BTreeSet::new()),
        };
        assert_eq!(entry.read(&entry.source).unwrap(), br#"{"setting":true}"#);
        assert!(entry.read(&t.path().join("private.json")).is_err());
        assert!(entry.read(&root.join("../private.json")).is_err());
    }
}
