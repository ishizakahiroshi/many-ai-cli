//! Mirror of the frozen web/scripts/build.mjs input/output contract.
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

pub const WINDOWS_RUNTIME_NAMES: &[&str] = &[
    "vcomp140.dll",
    "msvcp140.dll",
    "vcruntime140.dll",
    "vcruntime140_1.dll",
];

/// Authenticode/source verification belongs to the VS-only preparation script.
/// Reject partial or invalid prepared inputs before embedding their exact bytes.
pub fn windows_runtime_files(
    root: &Path,
    required: bool,
) -> io::Result<Vec<(&'static str, PathBuf)>> {
    let mut files = Vec::new();
    if root
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(io::Error::other(
            "runtime payload directory must not be a symlink",
        ));
    }
    for &name in WINDOWS_RUNTIME_NAMES {
        let path = root.join(name);
        let metadata = match path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_file() {
            return Err(io::Error::other(format!(
                "runtime payload is not a regular file: {name}"
            )));
        }
        let bytes = fs::read(&path)?;
        let offset = bytes
            .get(0x3c..0x40)
            .map(|value| u32::from_le_bytes(value.try_into().unwrap()) as usize);
        let pe =
            offset.and_then(|offset| offset.checked_add(6).and_then(|end| bytes.get(offset..end)));
        if bytes.get(..2) != Some(b"MZ") || pe != Some(&b"PE\0\0\x64\x86"[..]) {
            return Err(io::Error::other(format!(
                "runtime payload must be an x64 PE DLL: {name}"
            )));
        }
        files.push((name, path));
    }
    if files.len() != WINDOWS_RUNTIME_NAMES.len() && (required || !files.is_empty()) {
        return Err(io::Error::other(
            "prepare all four verified VS runtime DLLs before the Windows candidate build",
        ));
    }
    Ok(files)
}
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

#[cfg(test)]
mod runtime_tests {
    use super::*;

    fn dll(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0; 70];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&machine.to_le_bytes());
        bytes
    }

    #[test]
    fn runtime_payload_empty_development_and_required_candidate_are_distinct() {
        let temp = tempfile::tempdir().unwrap();
        assert!(
            windows_runtime_files(temp.path(), false)
                .unwrap()
                .is_empty()
        );
        assert!(windows_runtime_files(temp.path(), true).is_err());
        fs::write(temp.path().join(WINDOWS_RUNTIME_NAMES[0]), dll(0x8664)).unwrap();
        assert!(windows_runtime_files(temp.path(), false).is_err());
        for name in WINDOWS_RUNTIME_NAMES {
            fs::write(temp.path().join(name), dll(0x8664)).unwrap();
        }
        let files = windows_runtime_files(temp.path(), true).unwrap();
        assert_eq!(files.len(), 4);
        assert_eq!(
            files.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            WINDOWS_RUNTIME_NAMES
        );
    }

    #[test]
    fn runtime_payload_rejects_wrong_architecture_and_truncated_pe() {
        let temp = tempfile::tempdir().unwrap();
        for name in WINDOWS_RUNTIME_NAMES {
            fs::write(temp.path().join(name), dll(0x8664)).unwrap();
        }
        let path = temp.path().join(WINDOWS_RUNTIME_NAMES[0]);
        fs::write(&path, dll(0x14c)).unwrap();
        assert!(windows_runtime_files(temp.path(), true).is_err());
        fs::write(&path, b"MZ").unwrap();
        assert!(windows_runtime_files(temp.path(), true).is_err());
        fs::write(&path, dll(0x8664)).unwrap();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(windows_runtime_files(temp.path(), true).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn runtime_payload_rejects_symlinked_dll() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("synthetic.dll");
        fs::write(&outside, dll(0x8664)).unwrap();
        std::os::unix::fs::symlink(&outside, temp.path().join(WINDOWS_RUNTIME_NAMES[0])).unwrap();
        assert!(windows_runtime_files(temp.path(), false).is_err());
    }
}
