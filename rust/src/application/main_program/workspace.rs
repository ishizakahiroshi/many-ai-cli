use crate::{
    files::WorkspaceReads,
    orchestration::handoff::{HandoffStore, RecordKey},
    routine::memo::MemoManager,
};
use std::{path::Path, sync::Arc};
pub(super) struct Workspace {
    pub memos: Arc<MemoManager>,
    pub handoff: HandoffStore,
}
impl WorkspaceReads for Workspace {
    fn memo_mentions(&self) -> Vec<(String, String)> {
        self.memos.mentions()
    }
    fn recorded_handoff_note(&self, path: &Path, canonical: &Path) -> bool {
        let Some(name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix('s'))
        else {
            return false;
        };
        let (id, manual) = if let Some(id) = name.strip_suffix(".manual.note.md") {
            (id, true)
        } else if let Some(id) = name.strip_suffix(".note.md") {
            (id, false)
        } else {
            return false;
        };
        // `s20261116-6.note.md` for dated records, `s6.note.md` for older ones.
        let Some(key) = RecordKey::parse(id) else {
            return false;
        };
        let expected = if manual {
            self.handoff.manual_note_path_for(&key)
        } else {
            self.handoff.note_path_for(&key)
        };
        let Ok(expected) = expected else {
            return false;
        };
        if expected != path {
            return false;
        }
        let Ok(root) = self.handoff.directory().canonicalize() else {
            return false;
        };
        if !canonical.starts_with(&root) {
            return false;
        }
        self.handoff
            .read_session(key)
            .is_ok_and(|records| records.iter().any(|record| Path::new(&record.note) == path))
    }
}
