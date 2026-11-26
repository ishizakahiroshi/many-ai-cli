//! Refresh the package paths in an existing, bundled Windows Browser Use server.
use std::path::{Component, Path, PathBuf};
use toml_edit::{DocumentMut, Item, value};

const PACKAGE_NAME: &str = "OpenAI.Codex";
const PUBLISHER_ID: &str = "2p2nqsd0c76g0";
const NODE_REPL_PATH: &str = "app/resources/cua_node/bin/node_repl.exe";
const ENV_PATHS: &[&str] = &[
    "CODEX_CLI_PATH",
    "NODE_REPL_NODE_PATH",
    "NODE_REPL_NODE_MODULE_DIRS",
    "NODE_REPL_TRUSTED_CODE_PATHS",
];

#[cfg(windows)]
pub(super) fn repair_profile(profile: &crate::files::safe_fs::Dir, notice: &mut super::SeedNotice) {
    const CONFIG: &str = "config.toml";
    const LABEL: &str = "Codex Browser Use runtime";
    let Ok(bytes) = profile.read(CONFIG, 8 * 1024 * 1024 + 1) else {
        return;
    };
    if bytes.len() > 8 * 1024 * 1024 {
        return;
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return;
    };
    let Ok(mut config) = text.parse::<DocumentMut>() else {
        return;
    };
    match repair(&mut config, installed::location) {
        Ok(false) => {}
        Ok(true) => {
            if profile
                .replace(CONFIG, config.to_string().as_bytes(), 0o600)
                .is_ok()
            {
                notice.synced.push(LABEL.into());
            } else {
                notice.failed.push(LABEL.into());
            }
        }
        Err(()) => notice.failed.push(LABEL.into()),
    }
}

/// Returns the numeric package version and architecture from a package full name.
fn package_id(name: &str) -> Option<([u16; 4], &str)> {
    let mut fields = name.split('_');
    if fields.next()? != PACKAGE_NAME {
        return None;
    }
    let version: Vec<u16> = fields
        .next()?
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let arch = fields.next()?;
    if fields.next()? != "" || fields.next()? != PUBLISHER_ID || fields.next().is_some() {
        return None;
    }
    Some((version.try_into().ok()?, arch))
}

fn repair(
    config: &mut DocumentMut,
    installed_location: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<bool, ()> {
    let Some(server) = config
        .get_mut("mcp_servers")
        .and_then(|item| item.get_mut("node_repl"))
        .and_then(Item::as_table_like_mut)
    else {
        return Ok(false);
    };
    if server.get("enabled").and_then(Item::as_bool) == Some(false) {
        return Ok(false);
    }
    let Some(command) = server.get("command").and_then(Item::as_str).map(Path::new) else {
        return Ok(false);
    };
    let Some(old_root) = command.ancestors().find(|ancestor| {
        ancestor
            .file_name()
            .and_then(|name| package_id(name.to_str()?))
            .is_some()
    }) else {
        return Ok(false);
    };
    if !command.is_absolute()
        || old_root
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            != Some("WindowsApps")
        || !command.strip_prefix(old_root).is_ok_and(|relative| {
            relative
                .components()
                .eq(Path::new(NODE_REPL_PATH).components())
        })
    {
        return Ok(false);
    }
    let old_root = old_root.to_path_buf();
    let command_exists = command.is_file();
    let Some(env) = server.get_mut("env").and_then(Item::as_table_like_mut) else {
        return if command_exists { Ok(false) } else { Err(()) };
    };
    let has_missing_path = ENV_PATHS.iter().any(|key| {
        env.get(key).and_then(Item::as_str).is_some_and(|entry| {
            entry
                .split(';')
                .map(Path::new)
                .any(|path| path.starts_with(&old_root) && !path.exists())
        })
    });
    if command_exists && !has_missing_path {
        return Ok(false);
    }

    let (_, architecture) =
        package_id(old_root.file_name().and_then(|n| n.to_str()).ok_or(())?).ok_or(())?;
    let current = installed_location(architecture).ok_or(())?;
    let command = current.join(NODE_REPL_PATH);
    if !command.is_file() {
        return Err(());
    }
    let command = command.to_str().ok_or(())?.to_owned();
    let mut updates = Vec::new();
    for key in ENV_PATHS {
        let Some(entry) = env.get(key).and_then(Item::as_str) else {
            continue;
        };
        let paths = entry
            .split(';')
            .map(|entry| {
                let Ok(relative) = Path::new(entry).strip_prefix(&old_root) else {
                    return Ok(entry.to_owned());
                };
                if !relative
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
                {
                    return Err(());
                }
                let target = current.join(relative);
                let valid = match *key {
                    "CODEX_CLI_PATH" | "NODE_REPL_NODE_PATH" => target.is_file(),
                    _ => target.is_dir(),
                };
                if !valid {
                    return Err(());
                }
                Ok(target.to_str().ok_or(())?.to_owned())
            })
            .collect::<Result<Vec<_>, ()>>()?;
        let updated = paths.join(";");
        if updated != entry {
            updates.push((*key, updated));
        }
    }
    // Validate every replacement before changing the parsed config.
    for (key, updated) in updates {
        env.insert(key, value(updated));
    }
    server.insert("command", value(command));
    Ok(true)
}

#[cfg(windows)]
mod installed {
    use super::{PACKAGE_NAME, PUBLISHER_ID, package_id};
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf, ptr::null_mut};
    use windows_sys::Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
        Storage::Packaging::Appx::{GetPackagePathByFullName, GetPackagesByPackageFamily},
    };

    pub(super) fn location(architecture: &str) -> Option<PathBuf> {
        let family: Vec<u16> = format!("{PACKAGE_NAME}_{PUBLISHER_ID}\0")
            .encode_utf16()
            .collect();
        let mut count = 0;
        let mut length = 0;
        // A null first call requests the buffer sizes from Windows.
        let status = unsafe {
            GetPackagesByPackageFamily(
                family.as_ptr(),
                &mut count,
                null_mut(),
                &mut length,
                null_mut(),
            )
        };
        if status != ERROR_INSUFFICIENT_BUFFER || count == 0 || length == 0 {
            return None;
        }
        let mut names = vec![null_mut(); count as usize];
        let mut buffer = vec![0u16; length as usize];
        let status = unsafe {
            GetPackagesByPackageFamily(
                family.as_ptr(),
                &mut count,
                names.as_mut_ptr(),
                &mut length,
                buffer.as_mut_ptr(),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let (_, name) = names
            .iter()
            .take(count as usize)
            .filter_map(|name| {
                // The returned pointers address strings within the supplied buffer.
                let bytes = name.addr().checked_sub(buffer.as_ptr().addr())?;
                let tail = buffer.get(bytes / size_of::<u16>()..)?;
                let end = tail.iter().position(|char| *char == 0)?;
                let name_string = String::from_utf16(&tail[..end]).ok()?;
                let (version, arch) = package_id(&name_string)?;
                (arch == architecture).then_some((version, *name))
            })
            .max_by_key(|(version, _)| *version)?;
        let mut length = 0;
        if unsafe { GetPackagePathByFullName(name, &mut length, null_mut()) }
            != ERROR_INSUFFICIENT_BUFFER
        {
            return None;
        }
        let mut path = vec![0u16; length as usize];
        if unsafe { GetPackagePathByFullName(name, &mut length, path.as_mut_ptr()) }
            != ERROR_SUCCESS
        {
            return None;
        }
        let end = path.iter().position(|char| *char == 0)?;
        Some(PathBuf::from(OsString::from_wide(&path[..end])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const OLD: &str = "OpenAI.Codex_1.2.8.0_x64__2p2nqsd0c76g0";
    const CURRENT: &str = "OpenAI.Codex_1.2.10.0_x64__2p2nqsd0c76g0";
    const NODE: &str = "app/resources/cua_node/bin/node.exe";
    const MODULES: &str = "app/resources/cua_node/bin/node_modules";

    fn config(root: &Path) -> DocumentMut {
        let mut config = DocumentMut::new();
        let server = &mut config["mcp_servers"]["node_repl"];
        server["command"] = value(root.join(NODE_REPL_PATH).to_str().unwrap());
        server["env"]["NODE_REPL_NODE_PATH"] = value(root.join(NODE).to_str().unwrap());
        server["env"]["NODE_REPL_TRUSTED_CODE_PATHS"] =
            value(format!("external;{}", root.join(MODULES).display()));
        config["mcp_servers"]["other"]["command"] = value("custom-server");
        config
    }

    fn make_file(file: &Path) {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, []).unwrap();
    }

    #[test]
    fn browser_runtime_refreshes_the_removed_package_and_preserves_other_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("WindowsApps");
        let old = root.join(OLD);
        let current = root.join(CURRENT);
        make_file(&current.join(NODE_REPL_PATH));
        make_file(&current.join(NODE));
        fs::create_dir_all(current.join(MODULES)).unwrap();
        let mut settings = config(&old);
        assert_eq!(
            repair(&mut settings, |architecture| {
                assert_eq!(architecture, "x64");
                Some(current.clone())
            }),
            Ok(true)
        );
        assert_eq!(settings.to_string(), config(&current).to_string());
        assert_eq!(
            repair(&mut settings, |_| panic!("already current")),
            Ok(false)
        );
    }

    #[test]
    fn browser_runtime_rejects_an_incomplete_package_without_changing_config() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("WindowsApps");
        let mut settings = config(&root.join(OLD));
        let before = settings.to_string();
        let current = root.join(CURRENT);
        make_file(&current.join(NODE_REPL_PATH));
        assert_eq!(repair(&mut settings, |_| Some(current)), Err(()));
        assert_eq!(settings.to_string(), before);
    }

    #[test]
    fn browser_runtime_leaves_custom_and_disabled_servers_alone() {
        let temp = tempfile::tempdir().unwrap();
        let mut custom = config(temp.path());
        assert_eq!(repair(&mut custom, |_| panic!("custom")), Ok(false));
        let mut disabled = config(&temp.path().join("WindowsApps").join(OLD));
        disabled["mcp_servers"]["node_repl"]["enabled"] = value(false);
        assert_eq!(repair(&mut disabled, |_| panic!("disabled")), Ok(false));
    }
}
