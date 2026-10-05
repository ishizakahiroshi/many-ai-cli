//! Board files retain ordinary Go OpenFile append semantics, including hardlinks.
//! This owns filesystem serialization only, never sessions or admission state.
use super::{ChildLaunchError, safe_token};
use crate::{
    config::{Resource, RuntimePaths},
    proto::{
        core::SessionSnapshot,
        time::{self, Timestamp},
    },
};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

pub struct BoardStore {
    root: PathBuf,
    append: Mutex<()>,
}
impl BoardStore {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            root: paths.resource(Resource::Orchestration),
            append: Mutex::new(()),
        }
    }
    pub fn path(&self, orchestration: &str) -> PathBuf {
        self.root.join(safe_token(orchestration)).join("board.md")
    }
    pub fn ensure(
        &self,
        orchestration: &str,
        parent: &SessionSnapshot,
        initial_prompt: &str,
        now: Timestamp,
    ) -> Result<PathBuf, ChildLaunchError> {
        let path = self.path(orchestration);
        private_dirs(path.parent().expect("board has a parent"))
            .map_err(ChildLaunchError::board)?;
        // Source only creates when Stat reports NotExist. Other Stat failures
        // do not replace a pre-existing board or truncate an ordinary hardlink.
        if matches!(fs::metadata(&path), Err(e) if e.kind() == io::ErrorKind::NotFound) {
            let date = timestamp(now).map_err(ChildLaunchError::board)?;
            let content = format!(
                "# Orchestration {orchestration}\n\n- conductor: session #{} provider={} model={}\n- purpose: {}\n\n## conductor {date}\nCreated board. Children must append progress sections and finish with `## DONE <role> session=<child_id>`.\n",
                parent.id.0,
                parent.provider,
                parent.model,
                initial_prompt.trim(),
            );
            private_options()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .and_then(|mut file| file.write_all(content.as_bytes()))
                .map_err(ChildLaunchError::board)?;
        }
        Ok(path)
    }
    pub fn append(&self, path: &Path, role: &str, text: &str, now: Timestamp) -> io::Result<()> {
        let _guard = self
            .append
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut file = private_options().create(true).append(true).open(path)?;
        writeln!(file, "\n## {role} {}\n{text}", timestamp(now)?)
    }
}
fn timestamp(now: Timestamp) -> io::Result<String> {
    time::format_rfc3339(now).map_err(io::Error::other)
}
pub(super) fn private_dirs(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}
fn private_options() -> OpenOptions {
    let options = OpenOptions::new();
    #[cfg(unix)]
    let options = {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = options;
        options.mode(0o600);
        options
    };
    options
}
