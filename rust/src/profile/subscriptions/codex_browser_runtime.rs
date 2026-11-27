//! Refresh the package and plugin paths in an existing, bundled Windows Browser
//! Use server.
use super::codex_browser::{MARKETPLACE, PLUGINS};
use std::path::{Component, Path, PathBuf};
use toml_edit::{DocumentMut, Item, TableLike, value};

const SERVICES: &str = "NODE_REPL_TRUSTED_SERVICES";
const APP_VERSION: &str = "BROWSER_USE_CODEX_APP_VERSION";
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
    // The app package and the plugin are updated separately, so a failure of
    // one does not hold back the other.
    let outcomes = [
        repair(&mut config, installed::location),
        repair_services(&mut config),
    ];
    let mut failed = outcomes.contains(&Err(()));
    if outcomes.contains(&Ok(true)) {
        if profile
            .replace(CONFIG, config.to_string().as_bytes(), 0o600)
            .is_ok()
        {
            notice.synced.push(LABEL.into());
        } else {
            failed = true;
        }
    }
    if failed {
        notice.failed.push(LABEL.into());
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
    if !fields.next()?.is_empty() || fields.next()? != PUBLISHER_ID || fields.next().is_some() {
        return None;
    }
    Some((version.try_into().ok()?, arch))
}

/// The enabled `node_repl` server that Codex Desktop wrote, and the package
/// directory its command points into. Any other server is left alone.
fn bundled_server(config: &mut DocumentMut) -> Option<(&mut dyn TableLike, PathBuf)> {
    let server = config
        .get_mut("mcp_servers")
        .and_then(|item| item.get_mut("node_repl"))
        .and_then(Item::as_table_like_mut)?;
    if server.get("enabled").and_then(Item::as_bool) == Some(false) {
        return None;
    }
    let command = server
        .get("command")
        .and_then(Item::as_str)
        .map(Path::new)?;
    let root = command.ancestors().find(|ancestor| {
        ancestor
            .file_name()
            .and_then(|name| package_id(name.to_str()?))
            .is_some()
    })?;
    if !command.is_absolute()
        || root
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            != Some("WindowsApps")
        || !command.strip_prefix(root).is_ok_and(|relative| {
            relative
                .components()
                .eq(Path::new(NODE_REPL_PATH).components())
        })
    {
        return None;
    }
    let root = root.to_path_buf();
    Some((server, root))
}

/// `NODE_REPL_PATH` under a package root, with the platform's separators.
fn node_repl(root: &Path) -> PathBuf {
    root.join(Path::new(NODE_REPL_PATH).components().collect::<PathBuf>())
}

fn numeric_version(name: &str) -> Option<Vec<u64>> {
    name.split('.').map(|part| part.parse().ok()).collect()
}

/// A service file inside a bundled plugin that no longer exists.
struct MovedService {
    plugin: &'static str,
    old_version: String,
    new_version: String,
    new_path: String,
}

/// Looks for the newest installed version of the same plugin that has the same
/// file. `Ok(None)` means the path exists or is not a bundled plugin path.
fn moved_service(path: &str) -> Result<Option<MovedService>, ()> {
    if !Path::new(path).is_absolute() || Path::new(path).exists() {
        return Ok(None);
    }
    let parts: Vec<&str> = path.split(['/', '\\']).collect();
    let Some(marker) = parts
        .windows(3)
        .position(|names| names == ["plugins", "cache", MARKETPLACE])
    else {
        return Ok(None);
    };
    let Some(&[name, old_version, ref rest @ ..]) = parts.get(marker + 3..) else {
        return Ok(None);
    };
    let Some(plugin) = PLUGINS.iter().copied().find(|plugin| *plugin == name) else {
        return Ok(None);
    };
    if numeric_version(old_version).is_none()
        || rest.is_empty()
        || rest.iter().any(|part| matches!(*part, "" | "." | ".."))
    {
        return Ok(None);
    }
    // Every separator is one byte, so the offset follows from the parts.
    let start: usize = parts[..marker + 4].iter().map(|part| part.len() + 1).sum();
    let (head, tail) = (&path[..start], &path[start + old_version.len()..]);
    let (_, new_version) = std::fs::read_dir(head)
        .map_err(|_| ())?
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            let version = numeric_version(&name)?;
            Path::new(&format!("{head}{name}{tail}"))
                .is_file()
                .then_some((version, name))
        })
        .max()
        .ok_or(())?;
    Ok(Some(MovedService {
        plugin,
        old_version: old_version.to_owned(),
        new_path: format!("{head}{new_version}{tail}"),
        new_version,
    }))
}

/// Codex Desktop removes the old plugin version on update, while the profile
/// keeps naming its service file in `NODE_REPL_TRUSTED_SERVICES`.
fn repair_services(config: &mut DocumentMut) -> Result<bool, ()> {
    let Some((server, _)) = bundled_server(config) else {
        return Ok(false);
    };
    let Some(env) = server.get_mut("env").and_then(Item::as_table_like_mut) else {
        return Ok(false);
    };
    let Some(text) = env.get(SERVICES).and_then(Item::as_str) else {
        return Ok(false);
    };
    let services: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(text).map_err(|_| ())?;
    let mut updated = text.to_owned();
    let mut app_version = None;
    for path in services.values().filter_map(serde_json::Value::as_str) {
        let Some(moved) = moved_service(path)? else {
            continue;
        };
        // Replace only this string, keeping the rest of the text as written.
        let old = serde_json::to_string(path).map_err(|_| ())?;
        if !updated.contains(&old) {
            return Err(());
        }
        let new = serde_json::to_string(&moved.new_path).map_err(|_| ())?;
        updated = updated.replace(&old, &new);
        if moved.plugin == "browser" {
            app_version = Some((moved.old_version, moved.new_version));
        }
    }
    if updated == text {
        return Ok(false);
    }
    env.insert(SERVICES, value(updated));
    // Codex Desktop writes the Browser plugin version here as well.
    if let Some((old, new)) = app_version
        && env.get(APP_VERSION).and_then(Item::as_str) == Some(old.as_str())
    {
        env.insert(APP_VERSION, value(new));
    }
    Ok(true)
}

fn repair(
    config: &mut DocumentMut,
    installed_location: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<bool, ()> {
    let Some((server, old_root)) = bundled_server(config) else {
        return Ok(false);
    };
    let command_exists = server
        .get("command")
        .and_then(Item::as_str)
        .is_some_and(|command| Path::new(command).is_file());
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
    let command = node_repl(&current);
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
        server["command"] = value(node_repl(root).to_str().unwrap());
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

    /// A server whose app package is current but whose Browser plugin moved.
    fn services_config(temp: &Path, plugin_version: &str) -> (DocumentMut, PathBuf) {
        let package = temp.join("WindowsApps").join(CURRENT);
        make_file(&package.join(NODE_REPL_PATH));
        make_file(&package.join(NODE));
        fs::create_dir_all(package.join(MODULES)).unwrap();
        let plugin = temp.join("plugins/cache/openai-bundled/browser");
        let service = plugin.join(plugin_version).join("scripts/service.mjs");
        let mut settings = config(&package);
        let env = &mut settings["mcp_servers"]["node_repl"]["env"];
        env[SERVICES] = value(
            serde_json::json!({"browser": service.to_str().unwrap(), "sky": "@oai/sky/service"})
                .to_string(),
        );
        env[APP_VERSION] = value(plugin_version);
        (settings, plugin)
    }

    #[test]
    fn browser_runtime_moves_a_removed_service_to_the_newest_plugin_version() {
        let temp = tempfile::tempdir().unwrap();
        let (mut settings, plugin) = services_config(temp.path(), "1.2.8");
        make_file(&plugin.join("1.2.9/scripts/service.mjs"));
        make_file(&plugin.join("1.2.10/scripts/service.mjs"));
        // Newer, but without the service file.
        fs::create_dir_all(plugin.join("1.3.0/scripts")).unwrap();
        // Same order as the launch: the app package is already current.
        assert_eq!(repair(&mut settings, |_| panic!("current")), Ok(false));
        assert_eq!(repair_services(&mut settings), Ok(true));
        let (expected, _) = services_config(temp.path(), "1.2.10");
        assert_eq!(settings.to_string(), expected.to_string());
        assert_eq!(repair_services(&mut settings), Ok(false));
    }

    #[test]
    fn browser_runtime_keeps_a_service_without_a_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let (mut settings, plugin) = services_config(temp.path(), "1.2.8");
        let before = settings.to_string();
        assert_eq!(repair_services(&mut settings), Err(()));
        fs::create_dir_all(plugin.join("1.2.10")).unwrap();
        assert_eq!(repair_services(&mut settings), Err(()));
        assert_eq!(settings.to_string(), before);
    }

    #[test]
    fn browser_runtime_leaves_services_outside_the_bundled_plugins_alone() {
        let temp = tempfile::tempdir().unwrap();
        let (mut settings, plugin) = services_config(temp.path(), "1.2.8");
        make_file(&plugin.join("1.2.10/scripts/service.mjs"));
        settings["mcp_servers"]["node_repl"]["env"][SERVICES] = value(
            serde_json::json!({"mine": temp.path().join("gone.mjs").to_str().unwrap()}).to_string(),
        );
        assert_eq!(repair_services(&mut settings), Ok(false));
        let (mut custom, _) = services_config(temp.path(), "1.2.8");
        custom["mcp_servers"]["node_repl"]["command"] = value("custom-server");
        assert_eq!(repair_services(&mut custom), Ok(false));
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
