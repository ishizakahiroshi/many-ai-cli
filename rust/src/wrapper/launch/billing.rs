//! Fail-closed authentication and private helper settings for Claude headless.
//! No resolver process is spawned, including during configuration inspection.
use super::LaunchPlan;
use crate::wrapper::hooks::SessionHooks;
use crate::{
    config::{
        RuntimePaths,
        headless_billing::{self, BillingError},
    },
    files::safe_fs::Dir,
    process::{self, Cancellation, ExitOutcome, ProcessPlan, SpawnOptions},
    proto::Message,
};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

pub struct BillingContext<'a> {
    pub paths: &'a RuntimePaths,
    pub home_dir: &'a Path,
    pub cwd: &'a Path,
    pub environment: &'a [String],
    pub launch: &'a LaunchPlan,
    pub registered: &'a Message,
    pub executable: &'a Path,
    pub hub_port: u16,
    pub hub_token: &'a str,
}
/// Keep the real launch on the same settings/auth roots as `auth status`.
/// Probe commands intentionally omit inference flags, so root, policy, hosted
/// auth and alternate-execution selectors must fail closed before either probe.
/// Debug selectors are also forbidden: raw provider logs are not a safe output
/// channel. Resume/continue can relocate cwd after the probe, so they are also
/// refused in both modes (and cannot import prior cost into an API budget).
pub fn validate_provider_args(args: &[String], api: bool) -> io::Result<()> {
    if args.iter().any(|arg| {
        let name = arg.split('=').next().unwrap_or(arg);
        matches!(
            name,
            "--settings"
                | "--setting-sources"
                | "--managed-settings"
                | "--project-config-root"
                | "--client-data-url"
                | "--worktree"
                | "--deep-link-cwd-b64"
                | "--desktop"
                | "--sdk-url"
                | "--teleport"
                | "--cloud"
                | "--remote"
                | "--attach-serve"
                | "--remote-control"
                | "--rc"
                | "--bg"
                | "--background"
                | "--tmux"
                | "--resume"
                | "--continue"
                | "--fork-session"
                | "--api-key"
                | "--apiKeyHelper"
                | "--max-budget-usd"
                | "--bare"
                | "--debug"
                | "--debug-file"
                | "--debug-to-stderr"
        ) || conflicting_short_option(arg)
            || api
                && matches!(
                    name,
                    "--session-id" | "--add-dir" | "--mcp-config" | "--plugin-dir"
                )
    }) {
        return Err(BillingError::AuthConflict.into());
    }
    Ok(())
}
/// Commander accepts attached short values and clusters of boolean options.
/// Recognize only the reviewed 2.1.295 short grammar; stop at a value-taking
/// safe option so a label like `-nwork` is not mistaken for `-w`.
fn conflicting_short_option(arg: &str) -> bool {
    let Some(short) = arg
        .strip_prefix('-')
        .filter(|short| !short.starts_with('-'))
    else {
        return false;
    };
    for option in short.chars() {
        match option {
            'd' | 'w' | 'r' | 'c' => return true,
            'n' => return false,
            'p' | 'v' | 'V' | 'h' => {}
            _ => return false,
        }
    }
    false
}
/// These reviewed environment switches can supply a different hosted auth
/// source or enable provider-owned debug output independently of CLI flags.
fn conflicting_launch_environment(environment: &[String]) -> bool {
    environment
        .iter()
        .filter_map(|entry| entry.split_once('='))
        .any(|(key, _)| {
            matches!(
                key.to_ascii_uppercase().as_str(),
                "CLAUDE_CODE_CLIENT_DATA_URL"
                    | "DEBUG"
                    | "DEBUG_SDK"
                    | "DEBUG_AUTH"
                    | "DEBUG_CLAUDE_AGENT_SDK"
                    | "CLAUDE_DEBUG"
                    | "NODE_DEBUG"
                    | "CLAUDE_CODE_DEBUG_LOG_LEVEL"
                    | "CLAUDE_CODE_DEBUG_LOGS_DIR"
            )
        })
}
fn env<'a>(entries: &'a [String], name: &str) -> Option<&'a str> {
    entries.iter().rev().find_map(|entry| {
        entry
            .split_once('=')
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    })
}
fn inspect_file(paths: &RuntimePaths, path: &Path, managed: bool) -> io::Result<()> {
    if paths.is_trial() {
        paths
            .relative_to_selected_root(path)
            .map_err(|_| io::Error::from(BillingError::AuthConflict))?;
    }
    let parent = path.parent().ok_or(BillingError::AuthConflict)?;
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or(BillingError::AuthConflict)?;
    let directory = match Dir::open(parent) {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(BillingError::AuthConflict.into()),
    };
    let bytes = match directory.read(name, 128 * 1024) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(BillingError::AuthConflict.into()),
    };
    // Even innocuous-looking managed files can contain policy outside the
    // public auth schema. Never guess which of those policies wins.
    if managed {
        return Err(BillingError::AuthConflict.into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| io::Error::from(BillingError::AuthConflict))?;
    if headless_billing::auth_settings_conflict(&value)
        || value
            .get("env")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|env| {
                // The initial process root was selected and inspected above.
                // A settings-loaded root would redirect later CLI reads after
                // that inspection, so it cannot inherit the same authorization.
                env.keys()
                    .any(|key| key.eq_ignore_ascii_case("CLAUDE_CONFIG_DIR"))
                    || conflicting_launch_environment(
                        &env.keys().map(|key| format!("{key}=")).collect::<Vec<_>>(),
                    )
            })
    {
        return Err(BillingError::AuthConflict.into());
    }
    Ok(())
}
fn inspect_managed_directory(paths: &RuntimePaths, root: &Path) -> io::Result<()> {
    inspect_file(paths, &root.join("managed-settings.json"), true)?;
    let drop_ins = root.join("managed-settings.d");
    if paths.is_trial() {
        paths
            .relative_to_selected_root(&drop_ins)
            .map_err(|_| io::Error::from(BillingError::AuthConflict))?;
    }
    match Dir::open(&drop_ins) {
        Ok(directory)
            if directory
                .entries()
                .map_err(|_| io::Error::from(BillingError::AuthConflict))?
                .is_empty() =>
        {
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        _ => Err(BillingError::AuthConflict.into()),
    }
}
pub fn check_auth_files(
    paths: &RuntimePaths,
    home: &Path,
    cwd: &Path,
    environment: &[String],
) -> io::Result<()> {
    if environment
        .iter()
        .any(|entry| entry.starts_with("WSL_DISTRO_NAME=") || entry.starts_with("WSL_INTEROP="))
    {
        // WSL may inherit Windows managed policy. Its cross-OS policy source
        // cannot be verified through this local filesystem inspection.
        return Err(BillingError::AuthConflict.into());
    }
    if headless_billing::auth_environment_conflicts(environment)
        || conflicting_launch_environment(environment)
    {
        return Err(BillingError::AuthConflict.into());
    }
    let config = match env(environment, "CLAUDE_CONFIG_DIR") {
        Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
        Some(_) => return Err(BillingError::AuthConflict.into()),
        None => home.join(".claude"),
    };
    for name in ["settings.json", "settings.local.json"] {
        inspect_file(paths, &config.join(name), false)?;
    }
    inspect_managed_directory(paths, &config)?;
    for name in [
        "managed-settings.json",
        "managed-settings-cache.json",
        "remote-settings.json",
    ] {
        inspect_file(paths, &config.join(name), true)?;
    }
    // Claude can discover a project above cwd; inspect each possible settings
    // scope up to the selected home/root. No settings are overwritten.
    for ancestor in cwd.ancestors() {
        if paths.is_trial() && !ancestor.starts_with(paths.root()) {
            break;
        }
        for name in ["settings.json", "settings.local.json"] {
            inspect_file(paths, &ancestor.join(".claude").join(name), false)?;
        }
        if ancestor == home || ancestor == paths.root() {
            break;
        }
    }
    if !paths.is_trial() {
        #[cfg(target_os = "linux")]
        inspect_managed_directory(paths, Path::new("/etc/claude-code"))?;
        #[cfg(target_os = "macos")]
        {
            inspect_managed_directory(paths, Path::new("/Library/Application Support/ClaudeCode"))?;
            // MDM managed settings are not represented by the JSON file.
            inspect_file(
                paths,
                Path::new("/Library/Managed Preferences/com.anthropic.claudecode.plist"),
                true,
            )?;
            // The CLI also reads per-user MDM files. Conservatively reject any
            // such managed source without guessing the current account name.
            let directory = match Dir::open(Path::new("/Library/Managed Preferences")) {
                Ok(directory) => Some(directory),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(_) => return Err(BillingError::AuthConflict.into()),
            };
            if let Some(directory) = directory {
                for entry in directory
                    .entries()
                    .map_err(|_| io::Error::from(BillingError::AuthConflict))?
                {
                    if directory
                        .metadata(&entry)
                        .map_err(|_| io::Error::from(BillingError::AuthConflict))?
                        .is_dir()
                    {
                        inspect_file(
                            paths,
                            &directory
                                .path()
                                .join(entry)
                                .join("com.anthropic.claudecode.plist"),
                            true,
                        )?;
                    }
                }
            }
        }
        #[cfg(windows)]
        {
            inspect_managed_directory(paths, Path::new(r"C:\Program Files\ClaudeCode"))?;
            if managed_registry_present()? {
                return Err(BillingError::AuthConflict.into());
            }
        }
    }
    Ok(())
}
#[cfg(windows)]
fn managed_registry_present() -> io::Result<bool> {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, RegCloseKey,
            RegOpenKeyExW,
        },
    };
    let key: Vec<u16> = "SOFTWARE\\Policies\\ClaudeCode"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let mut handle = std::ptr::null_mut();
        let result = unsafe {
            RegOpenKeyExW(
                root,
                key.as_ptr(),
                0,
                KEY_READ | KEY_WOW64_64KEY,
                &mut handle,
            )
        };
        if result == ERROR_SUCCESS {
            unsafe { RegCloseKey(handle) };
            return Ok(true);
        }
        if result != ERROR_FILE_NOT_FOUND {
            return Err(BillingError::AuthConflict.into());
        }
    }
    Ok(false)
}
async fn probe(plan: &ProcessPlan, cancel: &Cancellation) -> io::Result<Vec<u8>> {
    let mut plan = plan.clone();
    plan.stdin.clear();
    plan.timeout = Duration::from_secs(10);
    plan.output_cap = 16 * 1024;
    plan.pipe_drain_timeout = Duration::from_secs(1);
    let output = process::run_capped_with_options(
        &plan,
        cancel,
        SpawnOptions {
            stderr_null: true,
            ..Default::default()
        },
    )
    .await
    .map_err(|_| io::Error::from(BillingError::AuthUnverified))?;
    if output.outcome
        != (ExitOutcome::Exited {
            code: Some(0),
            signal: None,
        })
        || output.stdout_truncated
        || output.pipes_forced_closed
    {
        return Err(BillingError::AuthUnverified.into());
    }
    Ok(output.stdout)
}
/// Exact versions require a separately recorded compatibility review. No
/// semver ranges or help-output heuristics silently enable a newer CLI.
pub fn verified_version(bytes: &[u8]) -> bool {
    let Ok(version) = std::str::from_utf8(bytes) else {
        return false;
    };
    const VERIFIED: &[&str] = &["2.1.295"];
    VERIFIED
        .iter()
        .any(|v| version.trim() == format!("{v} (Claude Code)"))
}
pub async fn prepare(
    context: BillingContext<'_>,
    hooks: &mut SessionHooks,
    args: &mut Vec<String>,
    version_probe: Option<&ProcessPlan>,
    auth_probe: Option<&ProcessPlan>,
    cancel: &Cancellation,
) -> io::Result<()> {
    if context.launch.provider != "claude" {
        return Ok(());
    }
    if context.launch.custom_argv.is_some() {
        return Err(BillingError::AuthConflict.into());
    }
    // Validate the final assembled flags too: registry/headless definitions
    // must not smuggle a second settings source past wrapper flag parsing.
    let mut checked = args.clone();
    if !context.launch.agent_session_id.is_empty()
        && let Some(index) = checked.windows(2).position(|pair| {
            pair[0] == "--session-id" && pair[1] == context.launch.agent_session_id
        })
    {
        checked.drain(index..index + 2);
    }
    validate_provider_args(&checked, context.launch.billing.is_api())?;
    check_auth_files(
        context.paths,
        context.home_dir,
        context.cwd,
        context.environment,
    )?;
    let version = probe(
        version_probe.ok_or(BillingError::UnsupportedVersion)?,
        cancel,
    )
    .await?;
    if !verified_version(&version) {
        return Err(BillingError::UnsupportedVersion.into());
    }
    if context.launch.billing.is_api() {
        let recipe = headless_billing::recipe(context.paths, &context.launch.billing)?;
        args.extend(hooks.claude_billing_settings(
            context.paths,
            context.registered,
            context.executable,
            context.hub_port,
            context.hub_token,
            cfg!(windows),
            &recipe,
        )?);
        args.extend([
            "--bare".into(),
            "--max-budget-usd".into(),
            context
                .launch
                .billing
                .max_budget_usd
                .ok_or(BillingError::InvalidSelection)?
                .to_string(),
        ]);
    } else {
        let status = probe(auth_probe.ok_or(BillingError::AuthUnverified)?, cancel).await?;
        if !headless_billing::verified_plan_auth(&status) {
            return Err(BillingError::AuthUnverified.into());
        }
    }
    if !args.iter().any(|arg| arg == "--include-partial-messages") {
        args.push("--include-partial-messages".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conflicting_flags_and_unknown_versions_fail_closed() {
        for flag in [
            "--settings=synthetic",
            "--resume=synthetic",
            "-c",
            "--debug-file=synthetic",
        ] {
            assert!(validate_provider_args(&[flag.into()], true).is_err());
        }
        assert!(validate_provider_args(&["--model".into(), "sonnet".into()], true).is_ok());
        assert!(!verified_version(b"99.99.99 (Claude Code)"));
    }
    #[test]
    fn alternate_auth_roots_and_debug_forms_are_rejected_before_probing() {
        for api in [false, true] {
            for flag in [
                "--project-config-root",
                "--managed-settings",
                "--settings",
                "--setting-sources",
                "--client-data-url",
                "--worktree",
                "--deep-link-cwd-b64",
                "--desktop",
                "--sdk-url",
                "--teleport",
                "--cloud",
                "--remote",
                "--attach-serve",
                "--remote-control",
                "--rc",
                "--bg",
                "--background",
                "--tmux",
                "--resume",
                "--continue",
                "--fork-session",
                "--debug",
                "--debug-file",
                "--debug-to-stderr",
            ] {
                for args in [
                    vec![flag.into(), "synthetic".into()],
                    vec![format!("{flag}=synthetic")],
                ] {
                    let error = validate_provider_args(&args, api).unwrap_err();
                    assert_eq!(
                        error.to_string(),
                        BillingError::AuthConflict.to_string(),
                        "{flag}"
                    );
                }
            }
            for flag in [
                "-d",
                "-dapi",
                "-d=api",
                "-d2e",
                "-d2e=true",
                "-pd",
                "-pdapi",
                "-cd2e",
                "-w",
                "-wother",
                "-w=other",
                "-pwother",
                "-r",
                "-rprior",
                "-r=prior",
                "-prprior",
                "-c",
                "-cp",
                "-pc",
            ] {
                assert!(
                    validate_provider_args(&[flag.into()], api).is_err(),
                    "{flag}"
                );
            }
            for args in [
                vec!["--model", "sonnet", "--effort", "high"],
                vec!["-p"],
                vec!["-nwork-debug"],
                vec!["--name=work-debug"],
            ] {
                assert!(
                    validate_provider_args(
                        &args.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                        api
                    )
                    .is_ok()
                );
            }
        }
    }
    #[test]
    fn hosted_auth_and_debug_environment_cannot_bypass_flag_guard() {
        for name in [
            "CLAUDE_CODE_CLIENT_DATA_URL",
            "DEBUG",
            "DEBUG_SDK",
            "DEBUG_AUTH",
            "DEBUG_CLAUDE_AGENT_SDK",
            "CLAUDE_DEBUG",
            "NODE_DEBUG",
            "CLAUDE_CODE_DEBUG_LOGS_DIR",
            "CLAUDE_CODE_DEBUG_LOG_LEVEL",
        ] {
            assert!(conflicting_launch_environment(&[format!(
                "{name}=synthetic"
            )]));
            assert!(conflicting_launch_environment(&[format!(
                "{}=",
                name.to_ascii_lowercase()
            )]));
        }
        assert!(!conflicting_launch_environment(&[
            "CLAUDE_CONFIG_DIR=/synthetic/known".into(),
            "TERM=xterm".into()
        ]));
    }
    #[test]
    fn settings_inspection_rejects_helpers_and_managed_overrides_without_running_commands() {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49001, installed.path()).unwrap();
        let home = root.path().join("home");
        let cwd = root.path().join("project");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir(&cwd).unwrap();
        assert!(check_auth_files(&paths, &home, &cwd, &[]).is_ok());
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"apiKeyHelper":"synthetic-do-not-execute"}"#,
        )
        .unwrap();
        assert!(check_auth_files(&paths, &home, &cwd, &[]).is_err());
        std::fs::write(home.join(".claude/settings.json"), "{}").unwrap();
        for key in [
            "CLAUDE_CODE_CLIENT_DATA_URL",
            "DEBUG",
            "DEBUG_SDK",
            "CLAUDE_CODE_DEBUG_LOGS_DIR",
        ] {
            assert!(check_auth_files(&paths, &home, &cwd, &[format!("{key}=synthetic")]).is_err());
            std::fs::write(
                home.join(".claude/settings.json"),
                serde_json::to_vec(&serde_json::json!({"env": {key: "synthetic"}})).unwrap(),
            )
            .unwrap();
            assert!(check_auth_files(&paths, &home, &cwd, &[]).is_err());
        }
        std::fs::write(home.join(".claude/settings.json"), "{}").unwrap();
        std::fs::write(home.join(".claude/managed-settings.json"), "{}").unwrap();
        assert!(check_auth_files(&paths, &home, &cwd, &[]).is_err());
    }
    #[test]
    fn initial_config_root_is_supported_but_nested_root_override_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49001, installed.path()).unwrap();
        let home = root.path().join("home");
        let cwd = root.path().join("project");
        let selected = root.path().join("selected-profile");
        for directory in [&home, &cwd, &selected] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let environment = vec![format!("CLAUDE_CONFIG_DIR={}", selected.display())];
        assert!(check_auth_files(&paths, &home, &cwd, &environment).is_ok());
        for key in ["CLAUDE_CONFIG_DIR", "claude_config_dir"] {
            std::fs::write(
                selected.join("settings.json"),
                serde_json::to_vec(&serde_json::json!({
                    "env": {key: root.path().join("uninspected-profile").to_string_lossy()},
                }))
                .unwrap(),
            )
            .unwrap();
            let error = check_auth_files(&paths, &home, &cwd, &environment).unwrap_err();
            assert_eq!(error.to_string(), BillingError::AuthConflict.to_string());
        }
    }
    #[cfg(unix)]
    fn fake_probe(cwd: &Path, output: &str) -> ProcessPlan {
        ProcessPlan {
            executable: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "printf '%s' \"$1\"; printf '%s' synthetic-stderr >&2".into(),
                "synthetic".into(),
                output.into(),
            ],
            cwd: cwd.into(),
            env: Default::default(),
            stdin: Vec::new(),
            timeout: Duration::from_secs(1),
            output_cap: 100,
            pipe_drain_timeout: Duration::from_secs(1),
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn api_prepare_merges_owned_settings_without_invoking_resolver_or_auth_probe() {
        use crate::{
            config::{Config, Resource, private_io},
            wrapper::launch::{self, WrapperArgs},
        };
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49001, installed.path()).unwrap();
        let home = root.path().join("home");
        let cwd = root.path().join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir(&cwd).unwrap();
        private_io::write_atomic(&paths.resource(Resource::HeadlessBilling), br#"{"version":1,"profiles":[{"id":"synthetic","display_name":"Synthetic","provider":"claude","command":"synthetic-resolver-must-never-run"}]}"#).unwrap();
        let selected = headless_billing::resolve(
            &paths,
            headless_billing::validate_selection(
                "claude",
                "headless",
                "api-key",
                "synthetic",
                Some(0.25),
                "",
            )
            .unwrap(),
        )
        .unwrap();
        let plan = launch::prepare_launch(
            &Config::default(),
            "claude",
            WrapperArgs {
                headless: true,
                billing_mode: "api-key".into(),
                billing_profile: "synthetic".into(),
                billing_revision: selected.revision,
                max_budget_usd: "0.25".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let registered = Message {
            session_id: 9,
            auto: true,
            orchestration_id: "synthetic".into(),
            token_statusbar: true,
            ..Default::default()
        };
        let mut hooks = SessionHooks::default();
        let mut args = plan.provider_args.clone();
        let version = fake_probe(&cwd, "2.1.295 (Claude Code)");
        prepare(
            BillingContext {
                paths: &paths,
                home_dir: &home,
                cwd: &cwd,
                environment: &[],
                launch: &plan,
                registered: &registered,
                executable: Path::new("/synthetic/many-ai-cli"),
                hub_port: 49001,
                hub_token: "synthetic-hub-token",
            },
            &mut hooks,
            &mut args,
            Some(&version),
            None,
            &Cancellation::default(),
        )
        .await
        .unwrap();
        assert_eq!(args.iter().filter(|v| *v == "--settings").count(), 1);
        assert_eq!(args.iter().filter(|v| *v == "--bare").count(), 1);
        assert!(
            !args
                .iter()
                .any(|arg| arg.contains("resolver-must-never-run"))
        );
        let index = args.iter().position(|v| v == "--settings").unwrap();
        let file = PathBuf::from(&args[index + 1]);
        let settings: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(
            settings["apiKeyHelper"],
            "synthetic-resolver-must-never-run"
        );
        assert!(settings.get("statusLine").is_some());
        assert_eq!(settings["crossSessionInbound"], "accept");
        hooks.finish().unwrap();
        assert!(!file.exists());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn plan_probes_keep_model_and_effort_without_alternate_auth_roots() {
        use crate::{
            config::Config,
            wrapper::launch::{self, WrapperArgs},
        };
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49001, installed.path()).unwrap();
        let home = root.path().join("home");
        let cwd = root.path().join("project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir(&cwd).unwrap();
        let launch = launch::prepare_launch(
            &Config::default(),
            "claude",
            WrapperArgs {
                headless: true,
                model: "sonnet".into(),
                effort: "high".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let registered = Message::default();
        let version = fake_probe(&cwd, "2.1.295 (Claude Code)");
        let auth = fake_probe(
            &cwd,
            r#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"pro","apiProvider":"firstParty"}"#,
        );
        let mut hooks = SessionHooks::default();
        let mut args = launch.provider_args.clone();
        let original_args = args.clone();
        prepare(
            BillingContext {
                paths: &paths,
                home_dir: &home,
                cwd: &cwd,
                environment: &[],
                launch: &launch,
                registered: &registered,
                executable: Path::new("/synthetic/many-ai-cli"),
                hub_port: 49001,
                hub_token: "synthetic-hub-token",
            },
            &mut hooks,
            &mut args,
            Some(&version),
            Some(&auth),
            &Cancellation::default(),
        )
        .await
        .unwrap();
        assert_eq!(&args[..original_args.len()], original_args);
        assert_eq!(args.iter().filter(|arg| *arg == "--model").count(), 1);
        assert_eq!(args.iter().filter(|arg| *arg == "--effort").count(), 1);
        assert_eq!(
            args.last().map(String::as_str),
            Some("--include-partial-messages")
        );
        hooks.finish().unwrap();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn alternate_project_helper_in_final_argv_is_refused_without_running_probes() {
        use crate::{
            config::Config,
            wrapper::launch::{self, WrapperArgs},
        };
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49001, installed.path()).unwrap();
        let home = root.path().join("home");
        let cwd = root.path().join("ordinary-project");
        let other = root.path().join("other-project");
        for directory in [&home, &cwd, &other.join(".claude")] {
            std::fs::create_dir_all(directory).unwrap();
        }
        std::fs::write(
            other.join(".claude/settings.json"),
            br#"{"apiKeyHelper":"synthetic-resolver-must-not-run"}"#,
        )
        .unwrap();
        // This is exactly why a cwd-only scan cannot authorize an override.
        assert!(check_auth_files(&paths, &home, &cwd, &[]).is_ok());
        let marker = root.path().join("probe-ran");
        let mut probe = fake_probe(&cwd, "2.1.295 (Claude Code)");
        probe.args = vec![
            "-c".into(),
            r#"printf unsafe > "$1""#.into(),
            "synthetic".into(),
            marker.as_os_str().to_owned(),
        ];
        let registered = Message::default();
        for api in [false, true] {
            let mut plan = launch::prepare_launch(
                &Config::default(),
                "claude",
                WrapperArgs {
                    headless: true,
                    ..Default::default()
                },
            )
            .unwrap();
            if api {
                plan.billing.mode = headless_billing::API_KEY.into();
            }
            for args in [
                vec![
                    "--project-config-root".into(),
                    other.to_string_lossy().into_owned(),
                ],
                vec![format!("--project-config-root={}", other.display())],
                vec![
                    "--managed-settings".into(),
                    "{\"apiKeyHelper\":\"synthetic-resolver-must-not-run\"}".into(),
                ],
                vec!["-dapi".into()],
                vec!["-d2e".into()],
                vec!["--debug-to-stderr".into()],
                vec!["--resume=synthetic-worktree-session".into()],
                vec!["-rsynthetic-worktree-session".into()],
                vec!["--continue".into()],
                vec!["-pc".into()],
            ] {
                let mut hooks = SessionHooks::default();
                let mut assembled = plan.provider_args.clone();
                assembled.extend(args);
                let error = prepare(
                    BillingContext {
                        paths: &paths,
                        home_dir: &home,
                        cwd: &cwd,
                        environment: &[],
                        launch: &plan,
                        registered: &registered,
                        executable: Path::new("/synthetic/many-ai-cli"),
                        hub_port: 49001,
                        hub_token: "synthetic-hub-token",
                    },
                    &mut hooks,
                    &mut assembled,
                    Some(&probe),
                    Some(&probe),
                    &Cancellation::default(),
                )
                .await
                .unwrap_err();
                assert_eq!(error.to_string(), BillingError::AuthConflict.to_string());
                assert!(!marker.exists());
                hooks.finish().unwrap();
            }
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn probe_captures_only_bounded_stdout_and_uses_fixed_failure() {
        let root = tempfile::tempdir().unwrap();
        let plan = fake_probe(root.path(), "synthetic-stdout");
        let result = probe(&plan, &Cancellation::default()).await.unwrap();
        assert_eq!(result, b"synthetic-stdout");
        let mut failed = plan;
        failed.args = vec!["-c".into(), "printf synthetic-secret >&2; exit 1".into()];
        let error = probe(&failed, &Cancellation::default()).await.unwrap_err();
        assert_eq!(error.to_string(), BillingError::AuthUnverified.to_string());
        assert!(!error.to_string().contains("synthetic-secret"));
    }
}
