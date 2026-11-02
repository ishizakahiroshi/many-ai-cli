//! Private argv transport for explicit interactive launch shells.
//!
//! Provider arguments stay in a private JSON file and are started natively by
//! this process. Only the runner path and payload path reach a shell command
//! line; neither is provider-controlled.
use crate::{
    config::{Resource, RuntimePaths},
    files::safe_fs::Dir,
    process::{self, execpath::ResolvedCommand, wrapper_startup::STARTUP_JOB_ENV},
    proto::core::SPAWN_PROOF_ENV,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::Command,
};

pub const INTERNAL_ARG: &str = "--many-ai-cli-internal-launch-payload";
pub const RUNNER_ENV: &str = "MANY_AI_CLI_INTERNAL_LAUNCH_RUNNER";
pub const PAYLOAD_ENV: &str = "MANY_AI_CLI_INTERNAL_LAUNCH_PAYLOAD";
pub const PROTECTED_ENV_NAMES_ENV: &str = "MANY_AI_CLI_INTERNAL_LAUNCH_PROTECTED_ENV_NAMES";
pub const CMD_RAW_BOOTSTRAP_ENV: &str = "MANY_AI_CLI_INTERNAL_CMD_RAW_BOOTSTRAP";
pub const CMD_RAW_BOOTSTRAP_VALUE: &str = "v1";

/// Fixed script source. The provider executable and argv are read from the
/// private payload by the native runner; they are never interpolated here.
pub const POWERSHELL_BOOTSTRAP: &str = r#"
$__manyAiRunner = [System.Environment]::GetEnvironmentVariable('MANY_AI_CLI_INTERNAL_LAUNCH_RUNNER', 'Process')
$__manyAiPayloadPath = [System.Environment]::GetEnvironmentVariable('MANY_AI_CLI_INTERNAL_LAUNCH_PAYLOAD', 'Process')
try {
  $__manyAiPayload = ConvertFrom-Json -InputObject ([System.IO.File]::ReadAllText($__manyAiPayloadPath)) -AsHashtable -ErrorAction Stop
} catch {
  [Console]::Error.WriteLine('many-ai-cli: launch payload could not be read; AI was not started.')
  exit 1
}
$__manyAiBaseline = [System.Environment]::GetEnvironmentVariables('Process')
$__manyAiProtected = [System.Collections.Generic.Dictionary[string, object]]::new([System.StringComparer]::OrdinalIgnoreCase)
$__manyAiPrefixes = @('MANY_AI_CLI_', 'ANTHROPIC_', 'OPENAI_', 'CODEX_', 'CLAUDE_', 'GROK_', 'COPILOT_', 'CURSOR_', 'OPENCODE_', 'NVIDIA_')
foreach ($__manyAiName in $__manyAiBaseline.Keys) {
  foreach ($__manyAiPrefix in $__manyAiPrefixes) {
    if ($__manyAiName.StartsWith($__manyAiPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
      $__manyAiProtected[$__manyAiName] = $__manyAiBaseline[$__manyAiName]
      break
    }
  }
}
foreach ($__manyAiName in $__manyAiPayload.protected_environment_names) {
  if (-not $__manyAiProtected.ContainsKey([string]$__manyAiName)) {
    if ($__manyAiBaseline.Contains([string]$__manyAiName)) {
      $__manyAiProtected[[string]$__manyAiName] = $__manyAiBaseline[[string]$__manyAiName]
    } else {
      $__manyAiProtected[[string]$__manyAiName] = $null
    }
  }
}
# Preserve the console-host profile order even though -NoProfile prevents the
# automatic load. Capture paths before any profile can change $PROFILE.
$__manyAiProfiles = @(
  [string]$PROFILE.AllUsersAllHosts,
  [string]$PROFILE.AllUsersCurrentHost,
  [string]$PROFILE.CurrentUserAllHosts,
  [string]$PROFILE.CurrentUserCurrentHost
)
$__manyAiPriorErrorActionPreference = $ErrorActionPreference
try {
  foreach ($__manyAiProfilePath in $__manyAiProfiles) {
    $ErrorActionPreference = 'Stop'
    if (-not [string]::IsNullOrEmpty($__manyAiProfilePath) -and [System.IO.File]::Exists($__manyAiProfilePath)) {
      . $__manyAiProfilePath
    }
  }
} catch {
  [Console]::Error.WriteLine('many-ai-cli: PowerShell profile failed; AI was not started.')
  exit 1
} finally {
  $ErrorActionPreference = $__manyAiPriorErrorActionPreference
}
# Profiles may initialize PATH and ordinary custom variables. Restore account,
# route, permission, Hub, color and UTF-8 contracts; remove new internal/provider
# namespace variables so a profile cannot silently select another account.
$__manyAiCurrent = [System.Environment]::GetEnvironmentVariables('Process')
foreach ($__manyAiName in @($__manyAiCurrent.Keys)) {
  $__manyAiIsProtected = $false
  foreach ($__manyAiPrefix in $__manyAiPrefixes) {
    if ($__manyAiName.StartsWith($__manyAiPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
      $__manyAiIsProtected = $true
      break
    }
  }
  if ($__manyAiIsProtected -and -not $__manyAiProtected.ContainsKey($__manyAiName)) {
    [System.Environment]::SetEnvironmentVariable([string]$__manyAiName, $null, 'Process')
  }
}
foreach ($__manyAiName in $__manyAiProtected.Keys) {
  [System.Environment]::SetEnvironmentVariable([string]$__manyAiName, $__manyAiProtected[$__manyAiName], 'Process')
}
$__manyAiRunnerFailed = $false
$__manyAiExitCode = 1
$__manyAiPriorRunnerErrorActionPreference = $ErrorActionPreference
$__manyAiHasNativeErrorActionPreference = Test-Path Variable:PSNativeCommandUseErrorActionPreference
$__manyAiPriorNativeErrorActionPreference = $null
if ($__manyAiHasNativeErrorActionPreference) {
  $__manyAiPriorNativeErrorActionPreference = $PSNativeCommandUseErrorActionPreference
}
try {
  $ErrorActionPreference = 'Stop'
  # Keep nonzero native exit codes as ordinary status values, even on PowerShell
  # versions that can turn them into errors by preference.
  if ($__manyAiHasNativeErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $false
  }
  & $__manyAiRunner '--many-ai-cli-internal-launch-payload' $__manyAiPayloadPath
  $__manyAiExitCode = $LASTEXITCODE
  if ($null -eq $__manyAiExitCode) { $__manyAiExitCode = 1 }
} catch {
  [Console]::Error.WriteLine('many-ai-cli: native launch runner failed; AI was not started.')
  $__manyAiRunnerFailed = $true
} finally {
  $ErrorActionPreference = $__manyAiPriorRunnerErrorActionPreference
  if ($__manyAiHasNativeErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $__manyAiPriorNativeErrorActionPreference
  }
}
if ($__manyAiRunnerFailed) { exit 1 }
exit $__manyAiExitCode
"#;

#[derive(Serialize, Deserialize)]
struct LaunchPayload {
    executable: String,
    args: Vec<String>,
    cwd: PathBuf,
    utf8: bool,
    protected_environment_names: Vec<String>,
}

/// Owns a private payload file until the PTY session and its cancellation
/// cleanup have completed.
pub struct PreparedLaunchPayload {
    directory: Dir,
    name: String,
    path: PathBuf,
}
impl PreparedLaunchPayload {
    pub fn create(
        paths: &RuntimePaths,
        command: &ResolvedCommand,
        cwd: &Path,
        utf8: bool,
        environment: &[String],
    ) -> io::Result<Self> {
        if command.executable.is_empty()
            || command.executable.contains('\0')
            || command.args.iter().any(|arg| arg.contains('\0'))
            || !cwd.is_absolute()
        {
            return Err(invalid("invalid AI launch payload"));
        }
        let directory =
            Dir::open_or_create_private_components(&paths.resource(Resource::Temporary))?;
        sweep_stale(&directory);
        let name = format!(
            "ai-shell-launch-{}-{}.json",
            std::process::id(),
            process::random_token()?
        );
        let payload = LaunchPayload {
            executable: command.executable.clone(),
            args: command.args.clone(),
            cwd: cwd.to_owned(),
            utf8,
            protected_environment_names: protected_environment_names(environment),
        };
        let bytes =
            serde_json::to_vec(&payload).map_err(|_| invalid("invalid AI launch payload"))?;
        directory.create_new(&name, &bytes, 0o600)?;
        let path = directory.path().join(&name);
        Ok(Self {
            directory,
            name,
            path,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for PreparedLaunchPayload {
    fn drop(&mut self) {
        let _ = self.directory.remove_file(&self.name);
    }
}

pub(crate) struct BootstrapCommand {
    pub executable: OsString,
    pub args: Vec<OsString>,
    pub environment: Vec<(OsString, OsString)>,
}

pub(crate) fn build_bootstrap_command(
    mode: &str,
    shell_executable: &Path,
    runner_executable: &Path,
    payload_path: &Path,
) -> io::Result<BootstrapCommand> {
    if shell_executable.as_os_str().is_empty()
        || runner_executable.as_os_str().is_empty()
        || payload_path.as_os_str().is_empty()
    {
        return Err(invalid("AI launch shell bootstrap path is empty"));
    }
    match mode {
        "pwsh" => Ok(BootstrapCommand {
            executable: shell_executable.as_os_str().into(),
            args: vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-Command".into(),
                POWERSHELL_BOOTSTRAP.into(),
            ],
            environment: vec![
                (RUNNER_ENV.into(), runner_executable.as_os_str().into()),
                (PAYLOAD_ENV.into(), payload_path.as_os_str().into()),
            ],
        }),
        "cmd" => {
            let runner = cmd_path(runner_executable)?;
            let payload = cmd_path(payload_path)?;
            // /s removes only the outer quotes. The resulting command contains
            // fixed helper syntax and two validated private paths, never argv.
            let command = format!("\"\"{runner}\" {INTERNAL_ARG} \"{payload}\"\"");
            Ok(BootstrapCommand {
                executable: shell_executable.as_os_str().into(),
                args: vec![
                    "/d".into(),
                    "/v:off".into(),
                    "/s".into(),
                    "/c".into(),
                    command.into(),
                ],
                environment: vec![(CMD_RAW_BOOTSTRAP_ENV.into(), CMD_RAW_BOOTSTRAP_VALUE.into())],
            })
        }
        _ => Err(invalid("AI launch shell must be pwsh or cmd")),
    }
}

pub(super) fn validate_inner_cmd(command: &ResolvedCommand) -> io::Result<()> {
    if command.shell_shim.is_none() {
        return Ok(());
    }
    if command.args.len() < 2
        || command
            .args
            .first()
            .is_none_or(|arg| !arg.eq_ignore_ascii_case("/c"))
    {
        return Err(invalid(
            "unresolved cmd shim has an unsupported argument layout",
        ));
    }
    let unsafe_argument = |arg: &str| {
        arg.is_empty()
            || arg.chars().any(char::is_whitespace)
            || arg.ends_with('\\')
            || cmd_text_unsafe(arg)
    };
    if command
        .shell_shim
        .as_deref()
        .is_some_and(|shim| shim.chars().any(char::is_whitespace) || cmd_text_unsafe(shim))
        || command.args[1..].iter().any(|arg| unsafe_argument(arg))
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the selected provider still requires a .cmd shim with a space in its path or ambiguous/unsafe arguments; install a native CLI or use auto",
        ));
    }
    Ok(())
}

fn cmd_path(path: &Path) -> io::Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| invalid("cmd shell bootstrap path is not valid Unicode"))?;
    if value.is_empty() || cmd_text_unsafe(value) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "cmd launch requires a runner and private temporary path without cmd metacharacters",
        ));
    }
    Ok(value.into())
}
fn cmd_text_unsafe(value: &str) -> bool {
    value.chars().any(|ch| {
        matches!(
            ch,
            '\0' | '\r' | '\n' | '"' | '%' | '!' | '^' | '&' | '|' | '<' | '>' | '(' | ')'
        )
    })
}

fn protected_environment_names(environment: &[String]) -> Vec<String> {
    let exact = [
        "COMSPEC",
        "PATHEXT",
        "TERM",
        "COLORTERM",
        "FORCE_COLOR",
        "NO_COLOR",
        "CLICOLOR",
        "CLICOLOR_FORCE",
        "TERM_PROGRAM",
        "WT_SESSION",
        "LANG",
        "LC_ALL",
        "PYTHONIOENCODING",
        "PYTHONUTF8",
        "CODEX_HOME",
        "CLAUDE_CONFIG_DIR",
        "GROK_HOME",
        "COPILOT_CONFIG_DIR",
        "CURSOR_CONFIG_DIR",
        "OPENCODE_CONFIG_DIR",
        "COMMAND_CODE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "HOME",
        "USERPROFILE",
    ];
    let mut names = BTreeSet::new();
    for name in exact {
        names.insert(name.to_owned());
    }
    for entry in environment {
        if let Some((name, _)) = entry.split_once('=')
            && is_protected_name(name)
        {
            names.insert(name.to_ascii_uppercase());
        }
    }
    if let Some(serialized) = environment.iter().rev().find_map(|entry| {
        entry
            .split_once('=')
            .filter(|(name, _)| name.eq_ignore_ascii_case(PROTECTED_ENV_NAMES_ENV))
            .map(|(_, value)| value)
    }) && let Ok(selected) = serde_json::from_str::<Vec<String>>(serialized)
    {
        for name in selected {
            if valid_environment_name(&name) {
                names.insert(name.to_ascii_uppercase());
            }
        }
    }
    names.into_iter().collect()
}
fn valid_environment_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['=', '\0'])
}
fn is_protected_name(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "MANY_AI_CLI_",
        "ANTHROPIC_",
        "OPENAI_",
        "CODEX_",
        "CLAUDE_",
        "GROK_",
        "COPILOT_",
        "CURSOR_",
        "OPENCODE_",
        "NVIDIA_",
    ];
    name.eq_ignore_ascii_case("COMSPEC")
        || name.eq_ignore_ascii_case("PATHEXT")
        || name.eq_ignore_ascii_case("TERM")
        || name.eq_ignore_ascii_case("COLORTERM")
        || name.eq_ignore_ascii_case("FORCE_COLOR")
        || name.eq_ignore_ascii_case("NO_COLOR")
        || name.eq_ignore_ascii_case("CLICOLOR")
        || name.eq_ignore_ascii_case("CLICOLOR_FORCE")
        || name.eq_ignore_ascii_case("TERM_PROGRAM")
        || name.eq_ignore_ascii_case("WT_SESSION")
        || name.eq_ignore_ascii_case("LANG")
        || name.eq_ignore_ascii_case("LC_ALL")
        || name.eq_ignore_ascii_case("PYTHONIOENCODING")
        || name.eq_ignore_ascii_case("PYTHONUTF8")
        || name.eq_ignore_ascii_case("CODEX_HOME")
        || name.eq_ignore_ascii_case("CLAUDE_CONFIG_DIR")
        || name.eq_ignore_ascii_case("GROK_HOME")
        || name.eq_ignore_ascii_case("COPILOT_CONFIG_DIR")
        || name.eq_ignore_ascii_case("CURSOR_CONFIG_DIR")
        || name.eq_ignore_ascii_case("OPENCODE_CONFIG_DIR")
        || name.eq_ignore_ascii_case("COMMAND_CODE_HOME")
        || name.eq_ignore_ascii_case("XDG_CONFIG_HOME")
        || name.eq_ignore_ascii_case("XDG_DATA_HOME")
        || name.eq_ignore_ascii_case("APPDATA")
        || name.eq_ignore_ascii_case("LOCALAPPDATA")
        || name.eq_ignore_ascii_case("HOME")
        || name.eq_ignore_ascii_case("USERPROFILE")
        || PREFIXES
            .iter()
            .any(|prefix| name.to_ascii_uppercase().starts_with(prefix))
}

fn payload_owner(name: &str) -> Option<i64> {
    let rest = name
        .strip_prefix("ai-shell-launch-")?
        .strip_suffix(".json")?;
    let (pid, token) = rest.split_once('-')?;
    (token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| pid.parse::<i64>().ok())?
}
fn sweep_stale(directory: &Dir) {
    let Ok(names) = directory.entries() else {
        return;
    };
    for name in names {
        let Some(pid) = payload_owner(&name).filter(|pid| *pid > 0) else {
            continue;
        };
        if process::pid_alive(pid) {
            continue;
        }
        let _ = directory.remove_file(&name);
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub fn is_internal_invocation(args: &[OsString]) -> bool {
    args.first()
        .is_some_and(|arg| arg == std::ffi::OsStr::new(INTERNAL_ARG))
}

/// Remove crashed launch payloads during the next normal application startup,
/// including startup when the user has since selected `auto`.
pub fn cleanup_stale_payloads(paths: &RuntimePaths) {
    if let Ok(directory) =
        Dir::open_or_create_private_components(&paths.resource(Resource::Temporary))
    {
        sweep_stale(&directory);
    }
}

/// Run only the private internal helper path. This branch is dispatched by the
/// binary before config loading, Hub startup, or startup-Job adoption.
pub fn run_internal_invocation(args: &[OsString]) -> i32 {
    if !is_internal_invocation(args) || args.len() != 2 {
        eprintln!("many-ai-cli: invalid internal launch invocation");
        return 2;
    }
    run_payload(Path::new(&args[1])).unwrap_or_else(|error| {
        eprintln!("many-ai-cli: {error}");
        1
    })
}

fn run_payload(path: &Path) -> io::Result<i32> {
    let bytes = std::fs::read(path).map_err(|_| invalid("AI launch payload could not be read"))?;
    let payload: LaunchPayload =
        serde_json::from_slice(&bytes).map_err(|_| invalid("AI launch payload is invalid"))?;
    if payload.executable.is_empty()
        || payload.executable.contains('\0')
        || payload.args.iter().any(|arg| arg.contains('\0'))
        || !payload.cwd.is_absolute()
    {
        return Err(invalid("AI launch payload is invalid"));
    }
    #[cfg(windows)]
    let _ctrl_handler = CtrlHandlerGuard::install()?;
    #[cfg(windows)]
    if payload.utf8 {
        use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};
        // The wrapper's UTF-8 hook runs before the shell profile. Repeat it
        // here so a profile cannot reset the console code pages for the CLI.
        unsafe {
            SetConsoleCP(65001);
            SetConsoleOutputCP(65001);
        }
    }
    let status = Command::new(&payload.executable)
        .args(&payload.args)
        .current_dir(&payload.cwd)
        .env_remove(RUNNER_ENV)
        .env_remove(PAYLOAD_ENV)
        .env_remove(CMD_RAW_BOOTSTRAP_ENV)
        .env_remove(SPAWN_PROOF_ENV)
        .env_remove(STARTUP_JOB_ENV)
        .env_remove(PROTECTED_ENV_NAMES_ENV)
        .status()
        .map_err(|_| io::Error::other("AI launch executable could not be started"))?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(windows)]
struct CtrlHandlerGuard;
#[cfg(windows)]
impl CtrlHandlerGuard {
    fn install() -> io::Result<Self> {
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
        // Install a process-local handler, not the inherited ignore-Ctrl+C
        // flag. The provider is a separate console process and keeps its own
        // handler table, so Ctrl+C reaches the provider while this waiter stays
        // alive to observe and return its exit status.
        if unsafe { SetConsoleCtrlHandler(Some(keep_runner_alive), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self)
    }
}
#[cfg(windows)]
impl Drop for CtrlHandlerGuard {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
        unsafe {
            SetConsoleCtrlHandler(Some(keep_runner_alive), 0);
        }
    }
}
#[cfg(windows)]
unsafe extern "system" fn keep_runner_alive(signal: u32) -> windows_sys::core::BOOL {
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};
    i32::from(matches!(signal, CTRL_C_EVENT | CTRL_BREAK_EVENT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::RuntimePaths, process::execpath::ResolvedCommand};

    fn paths(root: &Path) -> RuntimePaths {
        let runtime = root.join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        RuntimePaths::trial(&runtime, 49439, &root.join("installed")).unwrap()
    }

    #[test]
    fn private_payload_round_trips_argv_as_data_and_cleans_up() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        let argv = [
            "".into(),
            " has space 日本語 ".into(),
            "quote\"and-trailing-slash\\".into(),
            "first\nsecond & | < > % ! ^ $ ;".into(),
        ];
        let command = ResolvedCommand {
            executable: r"C:\synthetic\cli.exe".into(),
            args: argv.to_vec(),
            shell_shim: None,
        };
        let cwd = root.path().join("仕事 with space");
        std::fs::create_dir(&cwd).unwrap();
        let payload = PreparedLaunchPayload::create(
            &paths,
            &command,
            &cwd,
            true,
            &[
                "CODEX_HOME=C:\\synthetic\\account".into(),
                "OPENAI_API_KEY=synthetic-route-key".into(),
                "MANY_AI_CLI_SESSION_ID=17".into(),
                "MY_PROFILE_SETTING=kept".into(),
            ],
        )
        .unwrap();
        let decoded: LaunchPayload =
            serde_json::from_slice(&std::fs::read(payload.path()).unwrap()).unwrap();
        assert_eq!(decoded.executable, command.executable);
        assert_eq!(decoded.args, argv);
        assert_eq!(decoded.cwd, cwd);
        assert!(decoded.utf8);
        assert!(
            decoded
                .protected_environment_names
                .contains(&"CODEX_HOME".into())
        );
        assert!(
            decoded
                .protected_environment_names
                .contains(&"OPENAI_API_KEY".into())
        );
        assert!(
            decoded
                .protected_environment_names
                .contains(&"MANY_AI_CLI_SESSION_ID".into())
        );
        assert!(
            !decoded
                .protected_environment_names
                .contains(&"MY_PROFILE_SETTING".into())
        );
        let payload_path = payload.path().to_owned();
        drop(payload);
        assert!(!payload_path.exists());
    }

    #[test]
    fn selected_route_names_are_protected_without_protecting_unrelated_profile_values() {
        let environment = vec![
            "PATH=C:\\synthetic\\path".into(),
            "AWS_REGION=synthetic-region".into(),
            "SYNTHETIC_ACCOUNT_HOME=C:\\synthetic\\selected".into(),
            format!(
                "{PROTECTED_ENV_NAMES_ENV}={}",
                serde_json::to_string(&["SYNTHETIC_ACCOUNT_HOME"]).unwrap()
            ),
        ];
        let names = protected_environment_names(&environment);
        assert!(names.iter().any(|name| name == "SYNTHETIC_ACCOUNT_HOME"));
        assert!(
            names
                .iter()
                .any(|name| name == "MANY_AI_CLI_INTERNAL_LAUNCH_PROTECTED_ENV_NAMES")
        );
        assert!(!names.iter().any(|name| name == "PATH"));
        assert!(!names.iter().any(|name| name == "AWS_REGION"));
    }

    #[test]
    fn shell_bootstrap_sources_only_fixed_code_and_cmd_paths_are_checked() {
        let runner = Path::new(r"C:\Program Files\Many 日本語\many-ai-cli.exe");
        let payload = Path::new(r"C:\synthetic-home\App Data\ai-shell-launch.json");
        let pwsh = build_bootstrap_command(
            "pwsh",
            Path::new(r"C:\PowerShell\pwsh.exe"),
            runner,
            payload,
        )
        .unwrap();
        assert_eq!(pwsh.args[0], "-NoLogo");
        assert_eq!(pwsh.args[1], "-NoProfile");
        assert_eq!(pwsh.args[2], "-Command");
        assert_eq!(pwsh.args[3], POWERSHELL_BOOTSTRAP);
        assert!(
            !pwsh
                .args
                .iter()
                .any(|arg| arg.to_string_lossy().contains("& | < > % ! ^ $ ;"))
        );
        assert_eq!(pwsh.environment.len(), 2);

        let cmd = build_bootstrap_command(
            "cmd",
            Path::new(r"C:\Windows\System32\cmd.exe"),
            runner,
            payload,
        )
        .unwrap();
        let command = cmd.args.last().unwrap().to_string_lossy();
        assert!(command.contains("many-ai-cli.exe"));
        assert!(command.contains("ai-shell-launch.json"));
        assert!(!command.contains("& | < > % ! ^ $ ;"));
        assert_eq!(cmd.environment.len(), 1);
        assert_eq!(
            cmd.environment[0].0,
            std::ffi::OsStr::new(CMD_RAW_BOOTSTRAP_ENV)
        );
        assert_eq!(
            cmd.environment[0].1,
            std::ffi::OsStr::new(CMD_RAW_BOOTSTRAP_VALUE)
        );

        assert!(
            build_bootstrap_command(
                "cmd",
                Path::new(r"C:\Windows\System32\cmd.exe"),
                Path::new(r"C:\Program Files\100%\many-ai-cli.exe"),
                payload,
            )
            .is_err()
        );
        assert!(POWERSHELL_BOOTSTRAP.contains("$PROFILE.AllUsersAllHosts"));
        assert!(POWERSHELL_BOOTSTRAP.contains("$PROFILE.AllUsersCurrentHost"));
        assert!(POWERSHELL_BOOTSTRAP.contains("$PROFILE.CurrentUserAllHosts"));
        assert!(POWERSHELL_BOOTSTRAP.contains("$PROFILE.CurrentUserCurrentHost"));
    }

    #[test]
    fn explicit_shell_rejects_unsafe_inner_cmd_shim_values_but_auto_is_unchanged() {
        let unsafe_cmd = ResolvedCommand {
            executable: r"C:\Windows\System32\cmd.exe".into(),
            args: vec![
                "/c".into(),
                r"C:\npm\claude.cmd".into(),
                "prompt & calc".into(),
            ],
            shell_shim: Some(r"C:\npm\claude.cmd".into()),
        };
        assert!(validate_inner_cmd(&unsafe_cmd).is_err());
        let safe_cmd = ResolvedCommand {
            args: vec![
                "/c".into(),
                r"C:\npm\claude.cmd".into(),
                "--model".into(),
                "synthetic".into(),
            ],
            shell_shim: Some(r"C:\npm\claude.cmd".into()),
            ..unsafe_cmd.clone()
        };
        assert!(validate_inner_cmd(&safe_cmd).is_ok());
        let spaced_shim = ResolvedCommand {
            executable: r"C:\Windows\System32\cmd.exe".into(),
            args: vec![
                "/c".into(),
                r"C:\synthetic home\cli.cmd".into(),
                "--model".into(),
            ],
            shell_shim: Some(r"C:\synthetic home\cli.cmd".into()),
        };
        assert_eq!(
            validate_inner_cmd(&spaced_shim).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        for ambiguous in ["", "argument with spaces", "C:\\synthetic\\tail\\"] {
            let command = ResolvedCommand {
                executable: r"C:\Windows\System32\cmd.exe".into(),
                args: vec!["/c".into(), r"C:\npm\cli.cmd".into(), ambiguous.into()],
                shell_shim: Some(r"C:\npm\cli.cmd".into()),
            };
            assert_eq!(
                validate_inner_cmd(&command).unwrap_err().kind(),
                io::ErrorKind::Unsupported
            );
        }
        let auto = ResolvedCommand {
            args: vec![
                "/c".into(),
                r"C:\npm\claude.cmd".into(),
                "prompt & calc".into(),
            ],
            ..unsafe_cmd
        };
        assert!(validate_inner_cmd(&auto).is_err());
        // Auto does not call this validator; existing launch construction stays
        // unchanged. The helper's error is deliberately opt-in for explicit mode.
    }

    #[test]
    fn internal_invocation_is_unambiguous_and_payload_names_are_owned() {
        assert!(is_internal_invocation(&[
            INTERNAL_ARG.into(),
            "fixture".into()
        ]));
        assert!(!is_internal_invocation(&["claude".into()]));
        assert_eq!(
            payload_owner(
                "ai-shell-launch-42-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.json"
            ),
            Some(42)
        );
        assert_eq!(
            payload_owner(
                "ai-shell-launch-not-a-pid-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.json"
            ),
            None
        );
        assert_eq!(payload_owner("unrelated.json"), None);
    }

    #[test]
    fn normal_startup_cleanup_removes_payloads_owned_by_dead_processes() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        let directory =
            Dir::open_or_create_private_components(&paths.resource(Resource::Temporary)).unwrap();
        let dead_pid = i64::from(std::process::id()) + 1_000_000_000;
        let stale_name = format!(
            "ai-shell-launch-{dead_pid}-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.json"
        );
        directory
            .create_new(&stale_name, b"synthetic", 0o600)
            .unwrap();
        assert!(directory.metadata(&stale_name).is_ok());
        cleanup_stale_payloads(&paths);
        assert!(directory.metadata(&stale_name).is_err());
    }

    #[test]
    fn native_runner_forwards_synthetic_argv_and_exit_code() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        let output = root.path().join("captured argv.json");
        let executable = std::env::current_exe().unwrap();
        let mut args = vec![
            "--exact".into(),
            "wrapper::launch_payload::tests::synthetic_child_probe".into(),
            "--nocapture".into(),
            "--".into(),
            output.to_string_lossy().into_owned(),
        ];
        let expected = vec![
            String::new(),
            "日本語 with spaces".into(),
            "quote\"and-trailing-slash\\".into(),
            "line one\nline two & | < > % ! ^ $ ;".into(),
            "synthetic-exit-23".into(),
        ];
        args.extend(expected.clone());
        let command = ResolvedCommand {
            executable: executable.to_string_lossy().into_owned(),
            args,
            shell_shim: None,
        };
        let cwd = root.path().to_path_buf();
        let payload = PreparedLaunchPayload::create(&paths, &command, &cwd, false, &[]).unwrap();
        assert_eq!(run_payload(payload.path()).unwrap(), 23);
        let captured: Vec<String> =
            serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
        assert_eq!(captured, expected);
    }

    #[test]
    fn synthetic_child_probe() {
        let args = std::env::args().collect::<Vec<_>>();
        let Some(separator) = args.iter().position(|arg| arg == "--") else {
            return;
        };
        let Some(output) = args.get(separator + 1) else {
            return;
        };
        let values = &args[separator + 2..];
        std::fs::write(output, serde_json::to_vec(values).unwrap()).unwrap();
        if values
            .last()
            .is_some_and(|value| value == "synthetic-exit-23")
        {
            std::process::exit(23);
        }
    }

    #[cfg(windows)]
    #[test]
    fn synthetic_profiles_restore_selected_route_and_keep_path_custom_values() {
        let root = tempfile::tempdir().unwrap();
        let profile_paths = (0..4)
            .map(|index| root.path().join(format!("synthetic-profile-{index}.ps1")))
            .collect::<Vec<_>>();
        for (index, path) in profile_paths.iter().enumerate() {
            let script = format!(
                "$env:SYNTHETIC_ACCOUNT_HOME = 'profile-value'\n$env:PATH = [System.Environment]::GetEnvironmentVariable('PATH', 'Process') + ';synthetic-profile-segment'\n$env:SYNTHETIC_PROFILE_CUSTOM = 'profile-kept'\n$env:MANY_AI_CLI_PROFILE_INJECTED = 'remove-me'\n$env:SYNTHETIC_ORDER = [string]$env:SYNTHETIC_ORDER + '{}'\n",
                index + 1
            );
            std::fs::write(path, script).unwrap();
        }
        let payload_path = root.path().join("synthetic-payload.json");
        let payload = LaunchPayload {
            executable: "synthetic-provider-never-started".into(),
            args: vec![],
            cwd: root.path().to_owned(),
            utf8: false,
            protected_environment_names: vec!["SYNTHETIC_ACCOUNT_HOME".into()],
        };
        std::fs::write(&payload_path, serde_json::to_vec(&payload).unwrap()).unwrap();
        let capture_path = root.path().join("synthetic-environment.json");
        let profile_list = profile_paths
            .iter()
            .map(|path| ps_literal(&path.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(",\n  ");
        let mut script = POWERSHELL_BOOTSTRAP.to_owned();
        let profile_start = script.find("$__manyAiProfiles = @(").unwrap();
        let profile_end =
            profile_start + script[profile_start..].find("\n)").unwrap() + "\n)".len();
        script.replace_range(
            profile_start..profile_end,
            &format!("$__manyAiProfiles = @(\n  {profile_list}\n)"),
        );
        let runner_start = script.find("$__manyAiRunnerFailed = $false").unwrap();
        script.truncate(runner_start);
        script.push_str(&format!(
            "$__manyAiCapture = @{{ account = [System.Environment]::GetEnvironmentVariable('SYNTHETIC_ACCOUNT_HOME', 'Process'); path = [System.Environment]::GetEnvironmentVariable('PATH', 'Process'); custom = [System.Environment]::GetEnvironmentVariable('SYNTHETIC_PROFILE_CUSTOM', 'Process'); order = [System.Environment]::GetEnvironmentVariable('SYNTHETIC_ORDER', 'Process'); injected = [System.Environment]::GetEnvironmentVariable('MANY_AI_CLI_PROFILE_INJECTED', 'Process') }}\n[System.IO.File]::WriteAllText({}, (ConvertTo-Json -InputObject $__manyAiCapture -Compress))\nexit 23\n",
            ps_literal(&capture_path.to_string_lossy())
        ));

        let environment = std::env::vars()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();
        let resolver = crate::process::execpath::Resolver::new(
            crate::process::execpath::Platform::native(),
            &environment,
            root.path(),
            &crate::process::execpath::NativeFs,
        );
        let pwsh = match resolver.resolve_ai_launch_shell("pwsh") {
            Ok(path) => path,
            Err(_) => {
                eprintln!("synthetic PowerShell environment test skipped: pwsh.exe is unavailable");
                return;
            }
        };
        let mut command = Command::new(pwsh);
        command
            .args(["-NoLogo", "-NoProfile", "-Command"])
            .arg(script)
            .current_dir(root.path())
            .env(RUNNER_ENV, "synthetic-runner-never-started")
            .env(PAYLOAD_ENV, &payload_path)
            .env(PROTECTED_ENV_NAMES_ENV, "[\"SYNTHETIC_ACCOUNT_HOME\"]")
            .env("SYNTHETIC_ACCOUNT_HOME", "selected-account")
            .env("SYNTHETIC_PROFILE_CUSTOM", "before-profile")
            .env("PATH", "C:\\SyntheticOriginalPath");
        let status = command.status().unwrap();
        assert_eq!(status.code(), Some(23));
        let captured: serde_json::Value =
            serde_json::from_slice(&std::fs::read(capture_path).unwrap()).unwrap();
        assert_eq!(captured["account"], "selected-account");
        assert!(
            captured["path"]
                .as_str()
                .unwrap()
                .contains("synthetic-profile-segment")
        );
        assert_eq!(captured["custom"], "profile-kept");
        assert_eq!(captured["order"], "1234");
        assert!(captured["injected"].is_null() || captured["injected"] == "");
    }

    #[cfg(windows)]
    #[test]
    fn missing_native_runner_fails_even_if_profile_sets_last_exit_code_zero() {
        let root = tempfile::tempdir().unwrap();
        let profile_paths = (0..4)
            .map(|index| {
                root.path()
                    .join(format!("synthetic-missing-runner-profile-{index}.ps1"))
            })
            .collect::<Vec<_>>();
        for path in &profile_paths {
            std::fs::write(path, "$global:LASTEXITCODE = 0\n").unwrap();
        }
        let payload_path = root.path().join("synthetic-missing-runner-payload.json");
        let payload = LaunchPayload {
            executable: "synthetic-provider-never-started".into(),
            args: vec![],
            cwd: root.path().to_owned(),
            utf8: false,
            protected_environment_names: vec![],
        };
        std::fs::write(&payload_path, serde_json::to_vec(&payload).unwrap()).unwrap();
        let profile_list = profile_paths
            .iter()
            .map(|path| ps_literal(&path.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(",\n  ");
        let mut script = POWERSHELL_BOOTSTRAP.to_owned();
        let profile_start = script.find("$__manyAiProfiles = @(").unwrap();
        let profile_end =
            profile_start + script[profile_start..].find("\n)").unwrap() + "\n)".len();
        script.replace_range(
            profile_start..profile_end,
            &format!("$__manyAiProfiles = @(\n  {profile_list}\n)"),
        );

        let resolver_environment = std::env::vars()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>();
        let resolver = crate::process::execpath::Resolver::new(
            crate::process::execpath::Platform::native(),
            &resolver_environment,
            root.path(),
            &crate::process::execpath::NativeFs,
        );
        let pwsh = match resolver.resolve_ai_launch_shell("pwsh") {
            Ok(path) => path,
            Err(_) => {
                eprintln!(
                    "synthetic PowerShell missing-runner test skipped: pwsh.exe is unavailable"
                );
                return;
            }
        };
        let mut command = Command::new(pwsh);
        command
            .args(["-NoLogo", "-NoProfile", "-Command"])
            .arg(script)
            .current_dir(root.path())
            .env(RUNNER_ENV, root.path().join("missing synthetic runner.exe"))
            .env(PAYLOAD_ENV, &payload_path)
            .env(PROTECTED_ENV_NAMES_ENV, "[]");
        let status = command.status().unwrap();
        assert_eq!(status.code(), Some(1));
    }

    #[cfg(windows)]
    fn ps_literal(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }
}
