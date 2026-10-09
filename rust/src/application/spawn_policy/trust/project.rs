use crate::{config::RuntimePaths, profile::subscriptions::check_path};
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};
pub(super) struct Plan {
    pub target: PathBuf,
    pub write_key: String,
    pub lookup: Vec<String>,
    pub guard: Vec<PathBuf>,
}
pub(super) fn unsupported(path: &Path) -> bool {
    cfg!(windows)
        && (path.to_string_lossy().starts_with("\\\\") || path.to_string_lossy().starts_with("//"))
}
pub(super) fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            value => out.push(value.as_os_str()),
        }
    }
    out
}
fn canonical(path: &Path) -> io::Result<PathBuf> {
    let path = path.canonicalize()?;
    #[cfg(windows)]
    {
        Ok(PathBuf::from(simplify_verbatim(&path.to_string_lossy())))
    }
    #[cfg(not(windows))]
    {
        Ok(path)
    }
}
#[cfg(any(windows, test))]
pub(super) fn simplify_verbatim(path: &str) -> &str {
    let Some(rest) = path.strip_prefix(r"\\?\") else {
        return path;
    };
    if rest.len() < 3
        || !rest.as_bytes()[0].is_ascii_alphabetic()
        || &rest.as_bytes()[1..3] != b":\\"
        || path.len() > 260
    {
        return path;
    }
    for part in rest[3..].split('\\').filter(|p| !p.is_empty()) {
        if part.len() > 255
            || part.ends_with([' ', '.'])
            || part.bytes().any(|b| b < 32 || b"<>:\"/\\|?*".contains(&b))
        {
            return path;
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "AUX" | "NUL" | "PRN" | "CON")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return path;
        }
    }
    rest
}
fn canonical_or_clean(path: &Path) -> PathBuf {
    canonical(path).unwrap_or_else(|_| clean(path))
}
pub(super) fn normalize(key: &str) -> String {
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key.into()
    }
}
pub(super) fn comparable(key: &str) -> PathBuf {
    let key = if cfg!(windows) {
        key.strip_prefix("\\\\?\\")
            .filter(|s| s.as_bytes().get(1) == Some(&b':'))
            .unwrap_or(key)
    } else {
        key
    };
    PathBuf::from(normalize(&clean(Path::new(key)).to_string_lossy()))
}
pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    normalize(&clean(a).to_string_lossy()) == normalize(&clean(b).to_string_lossy())
}
fn claude_key(path: &Path) -> String {
    let s = clean(path).to_string_lossy().into_owned();
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s
    }
}
fn codex_key(path: &Path) -> String {
    normalize(&canonical_or_clean(path).to_string_lossy())
}
fn push_unique<T: PartialEq>(list: &mut Vec<T>, value: T) {
    if !list.contains(&value) {
        list.push(value);
    }
}
fn lookup_keys(path: &Path) -> Vec<String> {
    let mut keys = vec![codex_key(path)];
    push_unique(&mut keys, normalize(&clean(path).to_string_lossy()));
    keys
}
fn permitted(paths: &RuntimePaths, path: &Path) -> bool {
    check_path(paths, path).is_ok()
}
fn exists(paths: &RuntimePaths, path: &Path) -> bool {
    permitted(paths, path) && path.exists()
}
fn metadata(paths: &RuntimePaths, path: &Path) -> Option<fs::Metadata> {
    permitted(paths, path)
        .then(|| fs::metadata(path).ok())
        .flatten()
}
fn read_metadata(paths: &RuntimePaths, path: &Path) -> Option<Vec<u8>> {
    if !permitted(paths, path) || fs::symlink_metadata(path).ok()?.file_type().is_symlink() {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return None;
    }
    let mut bytes = vec![];
    file.take(64 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= 64 * 1024).then_some(bytes)
}
fn text(paths: &RuntimePaths, path: &Path) -> Option<String> {
    String::from_utf8(read_metadata(paths, path)?)
        .ok()
        .map(|s| s.trim().into())
}
fn gitdir(paths: &RuntimePaths, path: &Path) -> Option<PathBuf> {
    let contents = text(paths, path)?;
    let target = contents.strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    Some(clean(&path.parent()?.join(target)))
}
fn nearest(paths: &RuntimePaths, base: &Path) -> Option<PathBuf> {
    for p in base.ancestors() {
        if !permitted(paths, p) {
            break;
        }
        if exists(paths, &p.join(".git")) {
            return Some(p.into());
        }
    }
    None
}
fn codex_root(paths: &RuntimePaths, cwd: &Path) -> Option<PathBuf> {
    let mut base = if cwd.is_dir() {
        cwd.to_path_buf()
    } else {
        cwd.parent()?.to_path_buf()
    };
    let root = loop {
        let candidate = nearest(paths, &base)?;
        let marker = candidate.join(".git");
        let meta = metadata(paths, &marker)?;
        if !meta.is_dir() || exists(paths, &marker.join("HEAD")) {
            break candidate;
        }
        base = candidate.parent()?.into();
    };
    let marker = root.join(".git");
    if metadata(paths, &marker)?.is_dir() {
        return Some(root);
    }
    let git = gitdir(paths, &marker)?;
    if !metadata(paths, &git)?.is_dir() || fs::symlink_metadata(&git).ok()?.file_type().is_symlink()
    {
        return None;
    }
    let real_git = canonical(&git).ok()?;
    let worktrees = real_git.parent()?;
    if worktrees.file_name()? != "worktrees" {
        return None;
    }
    let common = worktrees.parent()?;
    let backlink = text(paths, &real_git.join("gitdir"))?;
    if backlink.is_empty() {
        return None;
    }
    let worktree_marker = clean(&real_git.join(backlink));
    if worktree_marker.file_name()? != ".git" {
        return None;
    }
    let common_ref = text(paths, &real_git.join("commondir"))?;
    if common_ref.is_empty() {
        return None;
    }
    let linked = clean(&real_git.join(common_ref));
    if !permitted(paths, &linked) || !permitted(paths, &worktree_marker) {
        return None;
    }
    if canonical(worktree_marker.parent()?).ok()? != canonical(&root).ok()?
        || canonical(&linked).ok()? != common
    {
        return None;
    }
    let main = git.parent()?.parent()?.parent()?;
    let main_marker = main.join(".git");
    let main_git = if metadata(paths, &main_marker)?.is_dir() {
        main_marker
    } else {
        gitdir(paths, &main_marker)?
    };
    if !permitted(paths, &main_git) || canonical(&main_git).ok()? != common {
        return None;
    }
    Some(main.into())
}
fn claude_root(paths: &RuntimePaths, root: &Path) -> PathBuf {
    let Some(contents) = text(paths, &root.join(".git")) else {
        return root.into();
    };
    let Some(reference) = contents
        .strip_prefix("gitdir:")
        .map(str::trim)
        .filter(|s| !s.is_empty() && !unsupported(Path::new(s)))
    else {
        return root.into();
    };
    let git = clean(&root.join(reference));
    let Some(common_ref) =
        text(paths, &git.join("commondir")).filter(|s| !s.is_empty() && !unsupported(Path::new(s)))
    else {
        return root.into();
    };
    let common = clean(&git.join(common_ref));
    if git.parent() != Some(common.join("worktrees").as_path()) {
        return root.into();
    }
    let Some(back) =
        text(paths, &git.join("gitdir")).filter(|s| !s.is_empty() && !unsupported(Path::new(s)))
    else {
        return root.into();
    };
    let back = clean(&git.join(back));
    if !permitted(paths, &back) || !permitted(paths, &common) {
        return root.into();
    }
    let (Ok(real_back), Ok(real_root)) = (canonical(&back), canonical(root)) else {
        return root.into();
    };
    if real_back != real_root.join(".git") {
        return root.into();
    }
    if common.file_name().is_some_and(|s| s == ".git") {
        return common.parent().unwrap_or(root).into();
    }
    if exists(paths, &common.join(".git")) {
        root.into()
    } else {
        common
    }
}
pub(super) fn plan(provider: &str, cwd: &Path, paths: &RuntimePaths) -> io::Result<Plan> {
    let cwd = clean(cwd);
    if provider == "claude" {
        let cwd = if cfg!(windows) {
            cwd
        } else {
            canonical_or_clean(&cwd)
        };
        let root = cwd
            .ancestors()
            .take_while(|p| permitted(paths, p))
            .find(|p| metadata(paths, &p.join(".git")).is_some_and(|m| m.is_file() || m.is_dir()))
            .map(Path::to_path_buf);
        let target = root
            .as_ref()
            .map(|root| claude_root(paths, root))
            .unwrap_or_else(|| cwd.clone());
        check_path(paths, &target)?;
        let mut lookup = vec![claude_key(&target)];
        for parent in cwd.ancestors() {
            push_unique(&mut lookup, claude_key(parent));
            if root.as_deref() == Some(parent) {
                break;
            }
        }
        return Ok(Plan {
            write_key: claude_key(&target),
            target,
            lookup,
            guard: vec![],
        });
    }
    let target = codex_root(paths, &cwd).unwrap_or_else(|| cwd.clone());
    check_path(paths, &target)?;
    let mut lookup = lookup_keys(&cwd);
    if !same_path(&target, &cwd) {
        for key in lookup_keys(&target) {
            push_unique(&mut lookup, key);
        }
    }
    let mut guard = vec![];
    for p in [
        &cwd,
        &canonical_or_clean(&cwd),
        &target,
        &canonical_or_clean(&target),
    ] {
        push_unique(&mut guard, comparable(&p.to_string_lossy()));
    }
    Ok(Plan {
        write_key: codex_key(&target),
        target,
        lookup,
        guard,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_verbatim_key_simplification_keeps_unsafe_components() {
        use super::simplify_verbatim;
        assert_eq!(
            simplify_verbatim(r"\\?\C:\Work\Project"),
            r"C:\Work\Project"
        );
        for path in [
            r"\\?\UNC\server\share",
            r"\\?\C:\CON.txt",
            r"\\?\C:\trailing.",
            r"\\?\C:\trailing ",
            r"\\?\C:\bad?name",
            r"\\?\C:\LPT1",
            r"\\?\C:\COM9",
        ] {
            assert_eq!(simplify_verbatim(path), path);
        }
        let long = format!(r"\\?\C:\{}", "a".repeat(260));
        assert_eq!(simplify_verbatim(&long), long);
    }
}
