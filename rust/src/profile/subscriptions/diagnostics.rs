//! Read-only seed metadata and key-name drift. Never returns credential values.
pub use super::seed::{DiagnosticEntry, diagnostic_entries, sync_drift};
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    #[test]
    fn toml_and_json_drift_compare_meaning_and_never_return_values_or_write() {
        let t = tempfile::tempdir().unwrap();
        let source = t.path().join("source.toml");
        let destination = t.path().join("profile.toml");
        std::fs::write(
            &source,
            "policy = 'secret-value' # comment\ninteger = 1\nowned = 'left'\nadded = true\n",
        )
        .unwrap();
        std::fs::write(
            &destination,
            "policy=\"secret-value\"\ninteger = 1.0\nowned = 'right'\n",
        )
        .unwrap();
        let original = std::fs::read(&destination).unwrap();
        let entry = DiagnosticEntry {
            trial_root: None,
            source: source.clone(),
            dest: "config.toml",
            label: "fixture".into(),
            mirror: false,
            sync_owned: Some(BTreeSet::from(["owned".into()])),
        };
        let (added, changed) = sync_drift(&entry, &destination).unwrap();
        assert_eq!(added, vec!["added"]);
        assert_eq!(changed, vec!["integer"]);
        assert!(!format!("{added:?}{changed:?}").contains("secret-value"));
        assert_eq!(std::fs::read(&destination).unwrap(), original);
        std::fs::write(
            &source,
            r#"{"policy":{"count":1,"secret":"never-return"},"owned":"left"}"#,
        )
        .unwrap();
        std::fs::write(
            &destination,
            r#"{"owned":"right","policy":{"secret":"never-return","count":1.0}}"#,
        )
        .unwrap();
        let entry = DiagnosticEntry {
            dest: "settings.json",
            ..entry
        };
        assert_eq!(sync_drift(&entry, &destination).unwrap(), (vec![], vec![]));
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
