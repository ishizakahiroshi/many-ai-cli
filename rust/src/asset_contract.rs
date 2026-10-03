//! Mirror of the frozen web/scripts/build.mjs input/output contract.
use std::{collections::BTreeSet, fs, io, path::Path};
const STATIC_ENTRIES: &[&str] = &[
    "index.html",
    "styles.css",
    "styles",
    "vendor",
    "icons",
    "i18n",
    "icon.svg",
    "manifest.webmanifest",
];
fn files(root: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(io::Error::other("asset symlinks are not allowed"));
        }
        if kind.is_dir() {
            files(root, &entry.path(), out)?;
        } else if kind.is_file() {
            out.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(io::Error::other)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}
pub fn expected_web_assets(source: &Path) -> io::Result<BTreeSet<String>> {
    let mut expected = BTreeSet::new();
    for name in STATIC_ENTRIES {
        let path = source.join(name);
        if path.is_dir() {
            let mut found = vec![];
            files(source, &path, &mut found)?;
            expected.extend(found);
        } else if path.is_file() {
            expected.insert((*name).into());
        } else {
            return Err(io::Error::other(format!("missing Web source asset {name}")));
        }
    }
    let mut all = vec![];
    files(source, source, &mut all)?;
    for name in all {
        if name.split('/').any(|p| p == "vendor")
            || name.ends_with(".d.ts")
            || (name.starts_with("debug/") && name != "debug/probe.ts")
        {
            continue;
        }
        if name.ends_with(".ts") || name.ends_with(".js") {
            let mut path = std::path::PathBuf::from(name);
            path.set_extension("js");
            expected.insert(path.to_string_lossy().replace('\\', "/"));
        }
    }
    expected.insert("debug/index.js".into());
    expected.insert(".src-hash".into());
    Ok(expected)
}
pub fn validate_web_assets(source: &Path, dist: &Path) -> io::Result<()> {
    for name in expected_web_assets(source)? {
        if !dist.join(&name).is_file() {
            return Err(io::Error::other(format!(
                "missing generated Web asset {name}; run the locked Bun build"
            )));
        }
    }
    for name in [
        "index.html",
        "app-entry.js",
        "app.js",
        "styles.css",
        "vendor/xterm.min.js",
        "sw.js",
        "whisper-recorder-worklet.js",
    ] {
        if fs::metadata(dist.join(name))?.len() == 0 {
            return Err(io::Error::other(format!(
                "generated Web asset {name} is empty"
            )));
        }
    }
    let source_hash = fs::read_to_string(dist.join(".src-hash"))?;
    if source_hash.len() != 12 || !source_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::other("invalid generated Web source identity"));
    }
    Ok(())
}
