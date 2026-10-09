//! Relay persistence uses the existing held-directory/private-write boundary.
use super::RelayFile;
use crate::{
    config::{Resource, RuntimePaths},
    files::safe_fs::Dir,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

const MAX_RELAY_BYTES: usize = 16 * 1024 * 1024;
pub type ProgressStamp = (u64, std::time::SystemTime);
pub type ProgressText = (String, ProgressStamp);
pub struct RelayStore {
    paths: RuntimePaths,
    root: PathBuf,
}
impl RelayStore {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            paths: paths.clone(),
            root: paths.resource(Resource::Orchestration),
        }
    }
    fn board_dir(&self, file: &RelayFile) -> io::Result<PathBuf> {
        let path = file.board_dir();
        if self.paths.is_trial() {
            crate::profile::subscriptions::check_path(&self.paths, &path)
                .map_err(io::Error::other)?;
            if !path.starts_with(&self.root) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "relay board outside orchestration root",
                ));
            }
        }
        Ok(path)
    }
    pub fn save(&self, file: &RelayFile) -> io::Result<()> {
        let path = self.board_dir(file)?;
        let mut persisted = file.clone();
        persisted.version = 1;
        let data = serde_json::to_vec_pretty(&persisted).map_err(io::Error::other)?;
        if self.paths.is_trial() {
            Dir::open_or_create_private_components(&path)?.replace("relay.json", &data, 0o600)
        } else {
            crate::config::private_io::write_atomic(&path.join("relay.json"), &data)
                .map_err(io::Error::other)
        }
    }
    pub fn load(&self) -> io::Result<(Vec<RelayFile>, Vec<String>)> {
        let mut files = Vec::new();
        let mut warnings = Vec::new();
        let root = match Dir::open(&self.root) {
            Ok(root) => root,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((files, warnings)),
            Err(e) => return Err(e),
        };
        for name in root.entries()? {
            let Ok(directory) = root.child_dir(&name, false) else {
                continue;
            };
            let bytes = match directory.read("relay.json", MAX_RELAY_BYTES) {
                Ok(data) => data,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => {
                    warnings.push(format!("relay record could not be read: {name}"));
                    continue;
                }
            };
            let mut file: RelayFile = match crate::proto::decode_wire(&bytes) {
                Ok(file) => file,
                Err(_) => {
                    warnings.push(format!("relay record skipped: {name}"));
                    continue;
                }
            };
            if file.orchestration_id.is_empty() {
                warnings.push(format!("relay record has no identity: {name}"));
                continue;
            }
            if file.board_path.is_empty() {
                file.board_path = directory
                    .path()
                    .join("board.md")
                    .to_string_lossy()
                    .into_owned();
            }
            if self.board_dir(&file).is_err() {
                warnings.push(format!("relay record outside selected root: {name}"));
                continue;
            }
            files.push(file);
        }
        files.sort_by(|a, b| a.orchestration_id.cmp(&b.orchestration_id));
        Ok((files, warnings))
    }
    pub fn read_progress(&self, file: &RelayFile, role: &str) -> io::Result<ProgressText> {
        self.read_progress_if_changed(file, role, None)?
            .ok_or_else(|| io::Error::other("progress read unexpectedly unchanged"))
    }
    pub fn read_progress_if_changed(
        &self,
        file: &RelayFile,
        role: &str,
        previous: Option<(u64, std::time::SystemTime)>,
    ) -> io::Result<Option<ProgressText>> {
        let path = self.board_dir(file)?;
        let name = format!("child-{}.md", file.progress_id(role));
        let directory = Dir::open(&path)?;
        let mut source = directory.open_file(&name, false)?;
        let metadata = source.metadata()?;
        let stamp = (metadata.len(), metadata.modified()?);
        if previous == Some(stamp) {
            return Ok(None);
        }
        use std::io::Read;
        let mut data = Vec::new();
        source
            .by_ref()
            .take((MAX_RELAY_BYTES + 1) as u64)
            .read_to_end(&mut data)?;
        if data.len() > MAX_RELAY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "relay progress exceeds read boundary",
            ));
        }
        Ok(Some((String::from_utf8_lossy(&data).into_owned(), stamp)))
    }
    pub fn review_exists(&self, file: &RelayFile, path: &Path) -> bool {
        if self.paths.is_trial() {
            if !path.starts_with(file.board_dir()) {
                return false;
            }
            let Some(parent) = path.parent() else {
                return false;
            };
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                return false;
            };
            Dir::open(parent)
                .and_then(|dir| dir.open_file(name, false))
                .is_ok()
        } else {
            fs::metadata(path).is_ok_and(|m| !m.is_dir())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, RuntimePaths, RelayStore, RelayFile) {
        let root = tempfile::tempdir().unwrap();
        let trial = root.path().join("trial");
        fs::create_dir(&trial).unwrap();
        let paths = RuntimePaths::trial(&trial, 49662, &root.path().join("installed")).unwrap();
        let store = RelayStore::new(&paths);
        let file = RelayFile {
            version: 1,
            orchestration_id: "r1-synthetic".into(),
            board_path: paths
                .resource(Resource::Orchestration)
                .join("r1-synthetic")
                .join("board.md")
                .to_string_lossy()
                .into_owned(),
            state: "stopped".into(),
            reason: "timeout".into(),
            ..Default::default()
        };
        (root, paths, store, file)
    }
    #[test]
    fn historical_record_uses_go_wire_unknown_versions_and_private_atomic_save() {
        let (_root, _paths, store, file) = fixture();
        let directory = Dir::open_or_create_private(&file.board_dir()).unwrap();
        directory.replace("relay.json",br#"{"VERSION":-9,"ORCHESTRATION_ID":"r1-synthetic","state":"stopped","reason":"timeout","roles":null,"events":null,"future":{"ignored":true}}"#,0o600).unwrap();
        let (loaded, warnings) = store.load().unwrap();
        assert!(warnings.is_empty());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].version, -9);
        assert_eq!(loaded[0].board_path, file.board_path);
        assert!(loaded[0].roles.is_empty());
        store.save(&loaded[0]).unwrap();
        let reread: serde_json::Value =
            serde_json::from_slice(&directory.read("relay.json", MAX_RELAY_BYTES).unwrap())
                .unwrap();
        assert_eq!(reread["version"], 1);
        assert!(reread.get("parent_started_at").is_none());
        assert!(reread.get("implementation_progress_id").is_none());
        assert!(reread.get("last_verdict").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                directory
                    .metadata("relay.json")
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn outside_board_record_is_skipped_without_writing_its_target() {
        let (root, _paths, store, mut file) = fixture();
        let directory = Dir::open_or_create_private(&file.board_dir()).unwrap();
        file.board_path = root
            .path()
            .join("outside")
            .join("board.md")
            .to_string_lossy()
            .into_owned();
        directory
            .replace("relay.json", &serde_json::to_vec(&file).unwrap(), 0o600)
            .unwrap();
        assert!(store.save(&file).is_err());
        let (loaded, warnings) = store.load().unwrap();
        assert!(loaded.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(!root.path().join("outside").exists());
    }
    #[test]
    fn progress_cap_plus_one_is_not_partial_completion_evidence() {
        let (_root, _paths, store, mut file) = fixture();
        file.implementation_session_id = 9;
        store.save(&file).unwrap();
        let directory = Dir::open(&file.board_dir()).unwrap();
        let mut text = vec![b'x'; MAX_RELAY_BYTES + 1];
        let marker = b"## DONE implementation\n";
        text[..marker.len()].copy_from_slice(marker);
        directory.replace("child-9.md", &text, 0o600).unwrap();
        assert!(
            store
                .read_progress(&file, super::super::IMPLEMENTATION)
                .is_err()
        );
    }
    #[cfg(unix)]
    #[test]
    fn progress_and_record_symlink_aliases_cannot_leave_selected_root() {
        use std::os::unix::fs::symlink;
        let (root, _paths, store, mut file) = fixture();
        file.implementation_session_id = 9;
        store.save(&file).unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, "## DONE implementation\n").unwrap();
        symlink(&outside, file.board_dir().join("child-9.md")).unwrap();
        assert!(
            store
                .read_progress(&file, super::super::IMPLEMENTATION)
                .is_err()
        );
        assert!(!store.review_exists(&file, &file.board_dir().join("child-9.md")));
        fs::remove_file(file.board_dir().join("relay.json")).unwrap();
        symlink(&outside, file.board_dir().join("relay.json")).unwrap();
        let (loaded, warnings) = store.load().unwrap();
        assert!(loaded.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            fs::read_to_string(&outside).unwrap(),
            "## DONE implementation\n"
        );
    }
}
