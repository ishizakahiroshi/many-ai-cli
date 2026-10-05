//! Windows-only first-initialize transport evidence, not the complete RPC fixture.
//! No provider or account is invoked; the original app-server tests remain authoritative.
#![cfg(all(test, windows))]

use crate::{
    application::{main_program::MainContext, subscriptions::cli::NativeSubscriptionCli},
    config::RuntimePaths,
    process::{
        ExitOutcome, InputSender, ManagedProcess, OutputStream, ProcessEvent, ProcessPlan, SpawnOptions,
    },
};
use serde::Serialize;
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

const INNER_TIMEOUT: Duration = Duration::from_secs(15);
const OUTER_TIMEOUT: Duration = Duration::from_secs(20);
// Every case creates these directories. The captured-runtime control redirects
// these home/profile keys in addition to the seven baseline keys and CODEX_HOME;
// it deliberately is not a byte-identical production environment.
// Both full seeds also redirect XDG_DATA_HOME and PSModuleAnalysisCachePath
// only when inherited. Those safety overlays do not alter the trial helper.
const SYNTHETIC_HOME_DIRS: &[(&str, &str)] = &[
    ("APPDATA", "app-data"),
    ("LOCALAPPDATA", "local-app-data"),
    ("XDG_CONFIG_HOME", "config"),
    ("XDG_CACHE_HOME", "cache"),
    ("TMPDIR", "tmp"),
    ("CLAUDE_CONFIG_DIR", "claude"),
    ("GROK_HOME", "grok"),
    ("COPILOT_CONFIG_DIR", "copilot"),
    ("CURSOR_CONFIG_DIR", "cursor"),
    ("OPENCODE_CONFIG_DIR", "opencode"),
    ("COMMAND_CODE_HOME", "command-code"),
];

#[derive(Clone, Copy, Serialize)]
struct Case {
    name: &'static str,
    compare_to: &'static str,
    pwsh: bool,
    via_cmd: bool,
    batch_stderr_redirect: bool,
    command_mode: bool,
    console_in: bool,
    explicit_flush: bool,
    no_window: bool,
    env_clear: bool,
    stderr_null: bool,
    ps_module_path: bool,
    local_app_data: bool,
    module_analysis_cache_path: bool,
    dotnet_exists: bool,
    dotnet_append: bool,
    restored_key: Option<&'static str>,
    captured_environment: bool,
    trial_environment: bool,
}

fn cases() -> [Case; 14] {
    let baseline = Case {
        name: "baseline",
        compare_to: "baseline",
        pwsh: false,
        via_cmd: true,
        batch_stderr_redirect: true,
        command_mode: false,
        console_in: false,
        explicit_flush: false,
        no_window: true,
        env_clear: true,
        stderr_null: true,
        ps_module_path: false,
        local_app_data: false,
        module_analysis_cache_path: false,
        dotnet_exists: false,
        dotnet_append: false,
        restored_key: None,
        captured_environment: false,
        trial_environment: false,
    };
    [
        baseline,
        Case {
            name: "windir",
            restored_key: Some("WINDIR"),
            ..baseline
        },
        Case {
            name: "system_drive",
            restored_key: Some("SystemDrive"),
            ..baseline
        },
        Case {
            name: "comspec",
            restored_key: Some("ComSpec"),
            ..baseline
        },
        Case {
            name: "program_files",
            restored_key: Some("ProgramFiles"),
            ..baseline
        },
        Case {
            name: "program_files_x86",
            restored_key: Some("ProgramFiles(x86)"),
            ..baseline
        },
        Case {
            name: "program_w6432",
            restored_key: Some("ProgramW6432"),
            ..baseline
        },
        Case {
            name: "app_data",
            restored_key: Some("APPDATA"),
            ..baseline
        },
        Case {
            name: "appdata_localappdata",
            compare_to: "app_data",
            restored_key: Some("APPDATA"),
            local_app_data: true,
            ..baseline
        },
        Case {
            name: "program_data",
            restored_key: Some("ProgramData"),
            ..baseline
        },
        Case {
            name: "all_users_profile",
            restored_key: Some("ALLUSERSPROFILE"),
            ..baseline
        },
        Case {
            name: "captured_environment",
            captured_environment: true,
            ..baseline
        },
        Case {
            name: "trial_environment",
            compare_to: "captured_environment",
            captured_environment: true,
            trial_environment: true,
            ..baseline
        },
        Case {
            name: "baseline_repeated",
            ..baseline
        },
    ]
}

#[derive(Default, Serialize)]
struct Stages {
    engine_available: bool,
    process_started: bool,
    write_requested: bool,
    write_ack: bool,
    flush_start: bool,
    flush_complete: bool,
    stdout_first: bool,
    matched_response: bool,
    operation_succeeded: bool,
    inner_timeout: bool,
    outer_timeout: bool,
    failure_stage: Option<&'static str>,
    error_kind: Option<String>,
    owned_wait_started: bool,
    owned_wait_complete: bool,
    owned_wait_outcome: Option<&'static str>,
    owned_exit_code: Option<i32>,
    owned_wait_error_kind: Option<String>,
    forced_pipes: Option<bool>,
    script_started: bool,
    before_read: bool,
    after_read: bool,
    read_null: Option<bool>,
    read_length: Option<u64>,
    before_test_path: bool,
    after_test_path: bool,
    first_line: bool,
    before_append: bool,
    after_append: bool,
    before_convert_from_json: bool,
    after_convert_from_json: bool,
    script_error: bool,
    script_error_kind: Option<&'static str>,
    stderr_file_nonempty: bool,
    batch_exit_code: Option<i32>,
    marker_error_kind: Option<String>,
    passed: bool,
}

#[derive(Serialize)]
struct CaseReport {
    case: Case,
    stages: Stages,
}

fn quoted_ps_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn pwsh_path() -> Option<PathBuf> {
    // Resolve before applying the deliberately restricted synthetic PATH.
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|path| path.join("pwsh.exe"))
        .chain(std::env::var_os("ProgramFiles").map(|path| {
            PathBuf::from(path)
                .join("PowerShell")
                .join("7")
                .join("pwsh.exe")
        }))
        .find(|path| path.is_file())
}

fn captured_environment() -> Vec<String> {
    std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.into_string().ok()?;
            if key.is_empty() || key.starts_with('=') {
                return None;
            }
            Some(format!("{key}={}", value.to_string_lossy()))
        })
        .collect()
}

fn fixture(
    case: Case,
    seed: &[String],
    stages: &mut Stages,
) -> io::Result<(tempfile::TempDir, tempfile::TempDir, ProcessPlan)> {
    let root = tempfile::tempdir()?;
    let installed = tempfile::tempdir()?;
    let paths = RuntimePaths::trial(root.path(), 49274, installed.path())?;
    let system_root =
        std::env::var_os("SystemRoot").ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    let engine = if case.pwsh {
        pwsh_path().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?
    } else {
        PathBuf::from(&system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe")
    };
    stages.engine_available = engine.is_file();
    if !stages.engine_available {
        return Err(io::Error::from(io::ErrorKind::NotFound));
    }
    let profile = root.path().join("profile");
    std::fs::create_dir(&profile)?;
    for (_, directory) in SYNTHETIC_HOME_DIRS {
        std::fs::create_dir(root.path().join(directory))?;
    }
    std::fs::create_dir(root.path().join("data"))?;
    let local_app_data = root.path().join("local-app-data");
    let app_data = root.path().join("app-data");
    let script = root.path().join("rpc.ps1");
    let reader = if case.console_in {
        "[Console]::In.ReadLine()"
    } else {
        "[Console]::ReadLine()"
    };
    let exists = if case.dotnet_exists {
        "[System.IO.File]::Exists($firstLine)"
    } else {
        "Test-Path -LiteralPath $firstLine"
    };
    let append = if case.dotnet_append {
        "[System.IO.File]::AppendAllText($log, ($line + [System.Environment]::NewLine))"
    } else {
        "Add-Content -LiteralPath $log -Value $line"
    };
    let script_text = format!(
        "$ErrorActionPreference = 'Stop'\n$root = '{}'\n",
        quoted_ps_path(root.path())
    ) + &r#"$started = [System.IO.Path]::Combine($root, 'script-started.txt')
$beforeRead = [System.IO.Path]::Combine($root, 'before-read.txt')
$afterRead = [System.IO.Path]::Combine($root, 'after-read.txt')
$readResult = [System.IO.Path]::Combine($root, 'read-result.json')
$beforeTestPath = [System.IO.Path]::Combine($root, 'before-test-path.txt')
$afterTestPath = [System.IO.Path]::Combine($root, 'after-test-path.txt')
$firstLine = [System.IO.Path]::Combine($root, 'first-line.txt')
$beforeAppend = [System.IO.Path]::Combine($root, 'before-append.txt')
$afterAppend = [System.IO.Path]::Combine($root, 'after-append.txt')
$beforeConvert = [System.IO.Path]::Combine($root, 'before-convert-from-json.txt')
$afterConvert = [System.IO.Path]::Combine($root, 'after-convert-from-json.txt')
$scriptError = [System.IO.Path]::Combine($root, 'script-error.txt')
$log = [System.IO.Path]::Combine($root, 'requests.txt')
[System.IO.File]::WriteAllText($started, 'true')
try {
 [System.IO.File]::WriteAllText($beforeRead, 'true')
 $line = @READ@
 [System.IO.File]::WriteAllText($afterRead, 'true')
 $isNull = ($null -eq $line).ToString().ToLowerInvariant()
 $length = if ($null -eq $line) { 'null' } else { $line.Length.ToString([System.Globalization.CultureInfo]::InvariantCulture) }
 [System.IO.File]::WriteAllText($readResult, ('{"null":' + $isNull + ',"length":' + $length + '}'))
 while ($null -ne $line) {
  [System.IO.File]::WriteAllText($beforeTestPath, 'true')
  $firstLineExists = @EXISTS@
  [System.IO.File]::WriteAllText($afterTestPath, 'true')
  if (-not $firstLineExists) { [System.IO.File]::WriteAllText($firstLine, 'received') }
  [System.IO.File]::WriteAllText($beforeAppend, 'true')
  @APPEND@
  [System.IO.File]::WriteAllText($afterAppend, 'true')
  [System.IO.File]::WriteAllText($beforeConvert, 'true')
  $r = $line | ConvertFrom-Json
  [System.IO.File]::WriteAllText($afterConvert, 'true')
  switch ($r.method) {
   'initialize' { [Console]::WriteLine('{"id":0,"result":{}}') }
  }
  $line = @READ@
 }
} catch {
 [System.IO.File]::WriteAllText($scriptError, $_.Exception.GetBaseException().GetType().Name)
 throw
}
"#
    .replace("@READ@", reader)
    .replace("@EXISTS@", exists)
    .replace("@APPEND@", append);
    std::fs::write(&script, script_text)?;

    let engine_in_batch = if case.pwsh {
        engine.to_string_lossy().into_owned()
    } else {
        r"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe".into()
    };
    let invocation = if case.command_mode {
        format!("-Command \"& '{}'\"", quoted_ps_path(&script))
    } else {
        format!("-File \"{}\"", script.display())
    };
    let redirect = if case.batch_stderr_redirect {
        format!(" 2>>\"{}\"", root.path().join("stderr.txt").display())
    } else {
        String::new()
    };
    let batch = format!(
        "@echo off\n\"{engine_in_batch}\" -NoLogo -NoProfile -ExecutionPolicy Bypass {invocation}{redirect}\nset \"exit_code=%ERRORLEVEL%\"\n>\"{}\" echo %exit_code%\nexit /b %exit_code%\n",
        root.path().join("exit-status.txt").display()
    );
    let cmd = root.path().join("codex.cmd");
    std::fs::write(&cmd, batch.replace('\n', "\r\n"))?;

    // Match synthetic_account and NativeSubscriptionCli::command at d9f892a.
    let base_environment = [
        format!("PATH={}", root.path().display()),
        "PATHEXT=.CMD;.EXE".into(),
        format!("SystemRoot={}", system_root.to_string_lossy()),
        format!("USERPROFILE={}", root.path().display()),
        format!("HOME={}", root.path().display()),
        format!("TEMP={}", root.path().display()),
        format!("TMP={}", root.path().display()),
    ];
    let mut environment = if case.captured_environment {
        seed.to_vec()
    } else {
        Vec::new()
    };
    let mut overrides = Vec::from(base_environment);
    overrides.push(format!("CODEX_HOME={}", profile.display()));
    if case.captured_environment {
        for (key, destination) in [
            ("XDG_DATA_HOME", "data"),
            ("PSModuleAnalysisCachePath", "ModuleAnalysisCache"),
        ] {
            if seed.iter().any(|entry| {
                entry
                    .split_once('=')
                    .is_some_and(|(name, _)| name.eq_ignore_ascii_case(key))
            }) {
                overrides.push(format!("{key}={}", root.path().join(destination).display()));
            }
        }
    }
    if case.captured_environment && !case.trial_environment {
        overrides.extend(SYNTHETIC_HOME_DIRS.iter().map(|(key, directory)| {
            format!("{key}={}", root.path().join(directory).display())
        }));
    }
    if let Some(key) = case.restored_key {
        let value = if key == "APPDATA" {
            app_data.to_string_lossy().into_owned()
        } else {
            seed.iter()
                .rev()
                .find_map(|entry| {
                    entry
                        .split_once('=')
                        .filter(|(name, _)| name.eq_ignore_ascii_case(key))
                        .map(|(_, value)| value.to_owned())
                })
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?
        };
        overrides.push(format!("{key}={value}"));
    }
    if case.ps_module_path {
        let modules = PathBuf::from(&system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("Modules");
        if !modules.is_dir() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        overrides.push(format!("PSModulePath={}", modules.display()));
    }
    if case.local_app_data {
        overrides.push(format!("LOCALAPPDATA={}", local_app_data.display()));
    }
    if case.module_analysis_cache_path {
        overrides.push(format!(
            "PSModuleAnalysisCachePath={}",
            root.path().join("ModuleAnalysisCache").display()
        ));
    }
    for entry in overrides {
        let (key, _) = entry
            .split_once('=')
            .expect("fixed diagnostic environment override");
        environment.retain(|existing| {
            !existing
                .split_once('=')
                .is_some_and(|(name, _)| name.eq_ignore_ascii_case(key))
        });
        environment.push(entry);
    }
    if case.trial_environment {
        // Start from the same captured seed plus baseline/CODEX_HOME and the
        // conditional XDG_DATA_HOME/PSModuleAnalysisCachePath safety overlays.
        // The production isolation helper supplies this case's home overrides.
        environment = MainContext::trial_environment_for_diagnostics(
            &paths,
            installed.path(),
            environment,
        )?;
    }
    let cli = NativeSubscriptionCli::new(paths, root.path().into(), environment);
    let mut plan = cli.command("codex", &profile, Duration::ZERO)?;
    let (executable, args) = if case.via_cmd {
        let (executable, args) = crate::application::subscriptions::cli::vendor_command(
            &cli.environment,
            cli.lookup("codex")?,
            vec!["app-server".into(), "--stdio".into()],
        );
        (PathBuf::from(executable), args)
    } else {
        // Compare only with the no-redirection control: stderr remains null
        // for both children, unlike the original batch's explicit reopening.
        (
            engine,
            vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                script.into_os_string(),
            ],
        )
    };
    plan.executable = executable;
    plan.args = args;
    Ok((root, installed, plan))
}

async fn initialize(
    input: &InputSender,
    events: &mut tokio::sync::broadcast::Receiver<ProcessEvent>,
    case: Case,
    stages: &mut Stages,
) -> io::Result<()> {
    let mut request = serde_json::to_vec(&serde_json::json!({
        "method": "initialize", "id": 0,
        "params": {"clientInfo": {"name": "many_ai_cli", "title": "many-ai-cli", "version": "1"}}
    }))
    .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    request.push(b'\n');
    stages.write_requested = true;
    input.write(request).await?;
    stages.write_ack = true;
    if case.explicit_flush {
        stages.flush_start = true;
        input.flush_for_diagnostics().await?;
        stages.flush_complete = true;
    }
    let mut buffer = Vec::new();
    loop {
        while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
            if end >= 4 * 1024 * 1024 {
                return Err(io::Error::from(io::ErrorKind::InvalidData));
            }
            let line: Vec<u8> = buffer.drain(..=end).collect();
            if let Ok(reply) = serde_json::from_slice::<serde_json::Value>(&line)
                && reply["id"].as_i64() == Some(0)
            {
                if reply.get("error").is_some() || !reply["result"].is_object() {
                    return Err(io::Error::from(io::ErrorKind::InvalidData));
                }
                stages.matched_response = true;
                return Ok(());
            }
        }
        if buffer.len() >= 4 * 1024 * 1024 {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        match events.recv().await {
            Ok(ProcessEvent::Started { .. }) => stages.process_started = true,
            Ok(ProcessEvent::Output {
                stream: OutputStream::Stdout,
                bytes,
            }) => {
                stages.stdout_first |= !bytes.is_empty();
                buffer.extend(bytes);
            }
            Ok(_) => {}
            Err(_) => return Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }
}

fn collect_markers(root: &Path, stages: &mut Stages) {
    stages.script_started = root.join("script-started.txt").is_file();
    stages.before_read = root.join("before-read.txt").is_file();
    stages.after_read = root.join("after-read.txt").is_file();
    stages.before_test_path = root.join("before-test-path.txt").is_file();
    stages.after_test_path = root.join("after-test-path.txt").is_file();
    stages.first_line = root.join("first-line.txt").is_file();
    stages.before_append = root.join("before-append.txt").is_file();
    stages.after_append = root.join("after-append.txt").is_file();
    stages.before_convert_from_json = root.join("before-convert-from-json.txt").is_file();
    stages.after_convert_from_json = root.join("after-convert-from-json.txt").is_file();
    stages.script_error = root.join("script-error.txt").is_file();
    stages.script_error_kind = std::fs::read_to_string(root.join("script-error.txt"))
        .ok()
        .map(|kind| match kind.trim() {
            "IOException" => "IOException",
            "InvalidOperationException" => "InvalidOperationException",
            "ArgumentException" => "ArgumentException",
            "UnauthorizedAccessException" => "UnauthorizedAccessException",
            "MethodInvocationException" => "MethodInvocationException",
            "RuntimeException" => "RuntimeException",
            _ => "Other",
        });
    stages.stderr_file_nonempty =
        std::fs::metadata(root.join("stderr.txt")).is_ok_and(|metadata| metadata.len() > 0);
    stages.batch_exit_code = std::fs::read_to_string(root.join("exit-status.txt"))
        .ok()
        .and_then(|value| value.trim().parse().ok());
    if stages.after_read {
        let marker = std::fs::read(root.join("read-result.json")).and_then(|bytes| {
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
        });
        match marker {
            Ok(marker) => {
                stages.read_null = marker["null"].as_bool();
                stages.read_length = marker["length"].as_u64();
            }
            Err(error) => stages.marker_error_kind = Some(format!("{:?}", error.kind())),
        }
    }
}

async fn run_case(case: Case, seed: &[String]) -> CaseReport {
    let mut stages = Stages::default();
    let (root, _installed, plan) = match fixture(case, seed, &mut stages) {
        Ok(fixture) => fixture,
        Err(error) => {
            stages.failure_stage = Some("fixture");
            stages.error_kind = Some(format!("{:?}", error.kind()));
            return CaseReport { case, stages };
        }
    };
    let (mut child, input, mut events) = ManagedProcess::spawn_interactive_owned_with_options(
        plan,
        1024,
        SpawnOptions {
            no_window: case.no_window,
            env_clear: case.env_clear,
            stderr_null: case.stderr_null,
            ..Default::default()
        },
    );
    let bounded = tokio::time::timeout(OUTER_TIMEOUT, async {
        // The first action is the request write, never a readiness wait or sleep.
        match tokio::time::timeout(
            INNER_TIMEOUT,
            initialize(&input, &mut events, case, &mut stages),
        )
        .await
        {
            Ok(Ok(())) => stages.operation_succeeded = true,
            Ok(Err(error)) => {
                stages.failure_stage = Some("initialize");
                stages.error_kind = Some(format!("{:?}", error.kind()));
            }
            Err(_) => {
                stages.inner_timeout = true;
                stages.failure_stage = Some("initialize");
                stages.error_kind = Some("TimedOut".into());
            }
        }
        child.close();
        stages.owned_wait_started = true;
        match child.wait().await {
            Ok(output) => {
                stages.owned_wait_complete = true;
                stages.forced_pipes = Some(output.pipes_forced_closed);
                stages.stdout_first |= !output.stdout.is_empty();
                stages.owned_wait_outcome = Some(match output.outcome {
                    ExitOutcome::Exited { code, .. } => {
                        stages.owned_exit_code = code;
                        "exited"
                    }
                    ExitOutcome::Cancelled => "cancelled",
                    ExitOutcome::TimedOut => "timed_out",
                });
            }
            Err(error) => {
                stages.owned_wait_complete = true;
                stages.owned_wait_outcome = Some("error");
                stages.owned_wait_error_kind = Some(format!("{:?}", error.kind()));
            }
        }
    })
    .await;
    if bounded.is_err() {
        stages.outer_timeout = true;
        stages.failure_stage = Some("outer");
        stages.error_kind = Some("TimedOut".into());
        stages.owned_wait_outcome = Some("outer_timeout");
    }
    child.close();
    drop(child); // Also aborts the owned task if the outer bound elapsed.
    while let Ok(event) = events.try_recv() {
        if matches!(event, ProcessEvent::Started { .. }) {
            stages.process_started = true;
        }
    }
    collect_markers(root.path(), &mut stages);
    let reaped = stages.owned_wait_outcome == Some("cancelled")
        || (stages.owned_wait_outcome == Some("exited") && stages.owned_exit_code == Some(0));
    stages.passed = stages.operation_succeeded
        && !stages.inner_timeout
        && !stages.outer_timeout
        && stages.owned_wait_complete
        && reaped
        && stages.forced_pipes == Some(false);
    CaseReport { case, stages }
}

fn write_artifact(report: &[u8]) -> io::Result<()> {
    let Some(directory) = std::env::var_os("MANY_AI_TEST_DIAGNOSTIC_DIR") else {
        return Ok(());
    };
    if directory.is_empty() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let directory = PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join("windows-appserver-stdio.json"), report)
}

#[tokio::test]
async fn windows_stdio_one_factor_diagnostics_require_baseline_success() {
    let seed = captured_environment();
    let mut reports = Vec::new();
    for case in cases() {
        reports.push(run_case(case, &seed).await);
    }
    let baseline_passed =
        reports[0].stages.passed && reports.last().is_some_and(|report| report.stages.passed);
    // Only fixed labels, booleans, numeric stages and ErrorKind names serialize.
    // Never include paths, environment values, output text or request contents.
    let report = serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": 1,
        "scope": "first_initialize_transport",
        "captured_environment_scope": "captured_runtime_with_synthetic_home_profile_overlay",
        "operation_timeout_seconds": INNER_TIMEOUT.as_secs(),
        "outer_timeout_seconds": OUTER_TIMEOUT.as_secs(),
        "cases": reports,
    }))
    .expect("fixed diagnostic schema serializes");
    let artifact = write_artifact(report.as_bytes());
    assert!(baseline_passed, "Windows stdio baseline failed; {report}");
    assert!(
        artifact.is_ok(),
        "diagnostic artifact write failed: {:?}",
        artifact.err().map(|error| error.kind())
    );
}
