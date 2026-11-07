//! Production boards retain Go OpenFile append semantics, including hardlinks.
//! Trial boards use held directories and refuse path/symlink escapes. Hardlink
//! identity remains source-compatible; this is not hardlink containment.
//! This owns filesystem serialization only, never sessions or admission state.
use super::{ChildLaunchError, safe_token};
use crate::{
    config::{Resource, RuntimePaths},
    files::safe_fs::Dir,
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
    trial: bool,
    append: Mutex<()>,
}
impl BoardStore {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            root: paths.resource(Resource::Orchestration),
            trial: paths.is_trial(),
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
        if self.trial {
            let directory =
                Dir::open_or_create_private_components(path.parent().expect("board has a parent"))
                    .map_err(ChildLaunchError::board)?;
            match directory.open_file("board.md", false) {
                Ok(_) => return Ok(path),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(ChildLaunchError::board(error)),
            }
            let content = board_content(orchestration, parent, initial_prompt, now)
                .map_err(ChildLaunchError::board)?;
            match directory.create_new("board.md", content.as_bytes(), 0o600) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    // A concurrent creator may win; accept only its regular
                    // held-directory file, without truncating or following it.
                    directory
                        .open_file("board.md", false)
                        .map_err(ChildLaunchError::board)?;
                }
                Err(error) => return Err(ChildLaunchError::board(error)),
            }
            return Ok(path);
        }
        private_dirs(path.parent().expect("board has a parent"))
            .map_err(ChildLaunchError::board)?;
        // Source only creates when Stat reports NotExist. Other Stat failures
        // do not replace a pre-existing board or truncate an ordinary hardlink.
        if matches!(fs::metadata(&path), Err(e) if e.kind() == io::ErrorKind::NotFound) {
            let content = board_content(orchestration, parent, initial_prompt, now)
                .map_err(ChildLaunchError::board)?;
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
        let mut file = if self.trial {
            if !path.starts_with(&self.root) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "board is outside the trial orchestration root",
                ));
            }
            let parent = path
                .parent()
                .ok_or_else(|| io::Error::other("board has no parent"))?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| io::Error::other("board has no UTF-8 filename"))?;
            // Dir's component walk rejects lexical traversal and directory
            // aliases; the append itself stays attached to its held inode.
            Dir::open(parent)?.open_append(name)?
        } else {
            private_options().create(true).append(true).open(path)?
        };
        writeln!(file, "\n## {role} {}\n{text}", timestamp(now)?)
    }
}
fn board_content(
    orchestration: &str,
    parent: &SessionSnapshot,
    initial_prompt: &str,
    now: Timestamp,
) -> io::Result<String> {
    let date = timestamp(now)?;
    Ok(format!(
        "# Orchestration {orchestration}\n\n- conductor: session #{} provider={} model={}\n- purpose: {}\n\n## conductor {date}\nCreated board. Children must append progress sections and finish with `## DONE <role> session=<child_id>`.\n",
        parent.id.0,
        parent.provider,
        parent.model,
        initial_prompt.trim(),
    ))
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
/// Creates the folders of `relative` below `base` one level at a time and
/// refuses any that is a link or junction, so a link committed to a
/// repository cannot move a worktree outside it (U28). Links above `base`,
/// such as a home folder on another drive, are the user's own and still
/// resolve. Returns `base` joined with `relative`.
pub(crate) fn plain_dirs_below(base: &Path, relative: &Path) -> io::Result<PathBuf> {
    let refused = |path: &Path, reason: &str| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{}: {reason}", path.display()),
        )
    };
    let mut path = base.to_path_buf();
    let mut below = PathBuf::new();
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => continue,
            std::path::Component::Normal(name) => {
                path.push(name);
                below.push(name);
            }
            _ => return Err(refused(relative, "not a plain relative folder")),
        }
        #[cfg(unix)]
        let created = {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(&path)
        };
        #[cfg(not(unix))]
        let created = fs::create_dir(&path);
        match created {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        // Not followed: a link (or a junction on Windows) is refused here.
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(refused(&path, "a link or not a folder"));
        }
    }
    if fs::canonicalize(&path)? != fs::canonicalize(base)?.join(&below) {
        return Err(refused(&path, "resolves outside the folder it was made in"));
    }
    Ok(path)
}
/// A folder link for tests: a symlink, or on Windows a junction, which needs
/// no privilege.
#[cfg(test)]
pub(crate) fn link_dir_for_test(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .status()
            .unwrap();
        assert!(status.success(), "synthetic junction creation failed");
    }
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
