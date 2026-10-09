//! Source CLI maintenance operations with explicit runtime and filesystem owners.
mod provider;
pub mod stop;
pub mod transcript;
use crate::{config::RuntimePaths, files::safe_fs::Dir};
pub use provider::provider_command;
use std::{
    io::{self, BufReader},
    path::{Path, PathBuf},
};
fn pinned_parent(paths: &RuntimePaths, path: &Path) -> io::Result<(Dir, String)> {
    if paths.is_trial()
        && path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "trial log-clean parent traversal refused",
        ));
    }
    let path = std::path::absolute(path)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("invalid file name"))?
        .to_owned();
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("invalid file parent"))?;
    if !paths.is_trial() {
        return Ok((Dir::open(parent)?, name));
    }
    let relative = relative_parent(paths.root(), parent)?;
    let mut dir = Dir::open(paths.root())?;
    for part in relative.components() {
        let std::path::Component::Normal(part) = part else {
            return Err(io::Error::other("invalid log-clean component"));
        };
        dir = dir.child_dir(
            part.to_str()
                .ok_or_else(|| io::Error::other("invalid log-clean component"))?,
            false,
        )?;
    }
    Ok((dir, name))
}
fn relative_parent(root: &Path, parent: &Path) -> io::Result<PathBuf> {
    if let Ok(relative) = parent.strip_prefix(root) {
        return Ok(relative.into());
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut components = root.components();
        let mut ordinary = PathBuf::new();
        if let Some(Component::Prefix(prefix)) = components.next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(drive) => ordinary.push(format!("{}:", char::from(drive))),
                Prefix::VerbatimUNC(server, share) => {
                    ordinary.push(Path::new(r"\\").join(server).join(share))
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "unsupported trial root prefix",
                    ));
                }
            }
            for component in components {
                ordinary.push(component.as_os_str());
            }
            if let Ok(relative) = parent.strip_prefix(ordinary) {
                return Ok(relative.into());
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "trial log-clean path escapes runtime",
    ))
}
pub fn log_clean(paths: &RuntimePaths, input: &Path, output: Option<&Path>) -> io::Result<PathBuf> {
    let output = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| input.with_extension("txt"));
    let (input_dir, input_name) = pinned_parent(paths, input)?;
    let input = input_dir.open_file(&input_name, false)?;
    let (output_dir, output_name) = pinned_parent(paths, &output)?;
    let mut out = output_dir.open_write_or_create(&output_name, 0o600)?;
    out.set_len(0)?;
    transcript::write_transcript(&mut BufReader::new(input), &mut out)?;
    Ok(output)
}
#[cfg(test)]
mod tests;
