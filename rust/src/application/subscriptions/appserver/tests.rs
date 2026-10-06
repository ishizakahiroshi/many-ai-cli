use super::*;
#[test]
fn official_usage_requires_chatgpt_valid_window_and_preserves_zero_presence() {
    let account=br#"{"account":{"type":"chatgpt","email":"synthetic-secret@example.com","planType":"plus"}}"#;
    let usage=parse_usage(account,br#"{"rateLimits":{"primary":{"usedPercent":0,"windowDurationMins":300,"resetsAt":99},"credits":{"hasCredits":true,"unlimited":false,"balance":"12.5"}}}"#).unwrap();
    let value = serde_json::to_value(usage).unwrap();
    assert_eq!(value["primary"]["used_percent"].as_f64(), Some(0.0));
    assert_eq!(value["primary"]["remaining_percent"].as_f64(), Some(100.0));
    assert_eq!(value["credits_balance"], "12.5");
    assert!(!value.to_string().contains("synthetic-secret"));
    for limits in [
        r#"{"rateLimits":{"primary":{"windowDurationMins":300}}}"#,
        r#"{"rateLimits":{"primary":{"usedPercent":-1,"windowDurationMins":300}}}"#,
        r#"{"rateLimits":{"primary":{"usedPercent":101,"windowDurationMins":300}}}"#,
        r#"{"rateLimits":{"primary":{"usedPercent":1,"windowDurationMins":0}}}"#,
    ] {
        assert!(parse_usage(account, limits.as_bytes()).is_err());
    }
    assert!(parse_usage(br#"{"account":{"type":"apiKey"}}"#, br#"{}"#).is_err());
    let duplicate=parse_usage(account,br#"{"rateLimits":{"primary":{"usedPercent":7,"windowDurationMins":300},"primary":{"usedPercent":0}}}"#).unwrap();
    assert_eq!(duplicate.primary.unwrap().window_minutes, 300);
}
#[cfg(windows)]
async fn synthetic_account(kind: &str) -> (std::io::Result<CodexUsage>, String, bool, String) {
    use crate::config::RuntimePaths;
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49274, installed.path()).unwrap();
    let profile = root.path().join("profile");
    std::fs::create_dir(&profile).unwrap();
    let log = root.path().join("requests.txt");
    let script = root.path().join("rpc.ps1");
    let phases = root.path().join("rpc-phases.txt");
    let ps = format!(
        r#"$ErrorActionPreference = 'Stop'
$log = '{}'
$phaseLog = '{}'
$iteration = 0
$script:phaseCount = 0
function Record-Phase([int]$stage) {{
 try {{
  if ($script:phaseCount -ge 64) {{ return }}
  $script:phaseCount++
  $elapsed = [long](([Diagnostics.Stopwatch]::GetTimestamp() - $phaseStart) * 1000 / [Diagnostics.Stopwatch]::Frequency)
  [IO.File]::AppendAllText($phaseLog, ('{{0}} {{1}} {{2}}' -f $stage, $iteration, $elapsed) + [Environment]::NewLine)
 }} catch {{ }}
}}
$phaseStart = 0
try {{ $phaseStart = [Diagnostics.Stopwatch]::GetTimestamp() }} catch {{ }}
Record-Phase 0
Record-Phase 1
while ($null -ne ($line = [Console]::ReadLine())) {{
 $iteration++
 Record-Phase 2
 Record-Phase 3
 Add-Content -LiteralPath $log -Value $line
 Record-Phase 4
 Record-Phase 5
 $r = $line | ConvertFrom-Json
 Record-Phase 6
 switch ($r.method) {{
  'initialize' {{ Record-Phase 7; [Console]::WriteLine('{{"id":0,"result":{{}}}}'); Record-Phase 8 }}
  'initialized' {{ }}
  'account/read' {{ Record-Phase 7; [Console]::WriteLine('{{"id":1,"result":{{"account":{{"type":"{}","planType":"plus","email":"synthetic-only@example.com"}}}}}}'); Record-Phase 8 }}
  'account/rateLimits/read' {{ Record-Phase 7; [Console]::WriteLine('{{"id":2,"result":{{"rateLimits":{{"primary":{{"usedPercent":0,"windowDurationMins":300}}}}}}}}'); Record-Phase 8 }}
 }}
 Record-Phase 1
}}
Record-Phase 9
"#,
        log.display().to_string().replace("'", "''"),
        phases.display().to_string().replace("'", "''"),
        kind
    );
    std::fs::write(&script, ps).unwrap();
    let cmd = format!(
        r#"@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "{}"
exit /b %ERRORLEVEL%
"#,
        script.display()
    );
    let cmd = cmd.replace("\r\n", "\n").replace('\n', "\r\n");
    std::fs::write(root.path().join("codex.cmd"), cmd).unwrap();
    // Preserve the host runtime inputs, as MainContext does, while keeping all
    // provider lookup and trial home/profile/cache locations synthetic.
    let mut environment: Vec<String> = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.into_string().ok()?;
            if key.is_empty() || key.starts_with('=') {
                return None;
            }
            Some(format!("{key}={}", value.to_string_lossy()))
        })
        .collect();
    let mut overrides = vec![
        format!("PATH={}", root.path().display()),
        "PATHEXT=.CMD;.EXE".into(),
        format!("SystemRoot={}", std::env::var("SystemRoot").unwrap()),
        format!("USERPROFILE={}", root.path().display()),
        format!("HOME={}", root.path().display()),
        format!("TEMP={}", root.path().display()),
        format!("TMP={}", root.path().display()),
        format!("CODEX_HOME={}", profile.display()),
    ];
    std::fs::create_dir(root.path().join("data")).unwrap();
    for (key, destination) in [
        ("XDG_DATA_HOME", "data"),
        ("PSModuleAnalysisCachePath", "ModuleAnalysisCache"),
    ] {
        if environment.iter().any(|entry| {
            entry
                .split_once('=')
                .is_some_and(|(name, _)| name.eq_ignore_ascii_case(key))
        }) {
            overrides.push(format!("{key}={}", root.path().join(destination).display()));
        }
    }
    for entry in overrides {
        let (key, _) = entry.split_once('=').unwrap();
        environment.retain(|existing| {
            !existing
                .split_once('=')
                .is_some_and(|(name, _)| name.eq_ignore_ascii_case(key))
        });
        environment.push(entry);
    }
    let (_, environment) = crate::application::main_program::isolate_trial_environment(
        &paths,
        installed.path(),
        environment,
    )
    .unwrap();
    let cli = NativeSubscriptionCli::new(paths, root.path().into(), environment);
    let trace = phase_diagnostics::Trace::new();
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        phase_diagnostics::ACTIVE.scope(
            trace.clone(),
            cli.codex_usage(&profile, &TaskCancellation::default()),
        ),
    )
    .await;
    let outer_timeout = outcome.is_err();
    let result = outcome.unwrap_or_else(|_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "outer synthetic app-server timeout",
        ))
    });
    let result_summary = result
        .as_ref()
        .map(|_| "Ok".to_owned())
        .unwrap_or_else(|error| format!("Err({:?})", error.kind()));
    let requests = std::fs::read_to_string(log);
    let diagnostic = phase_diagnostics::summary(requests.as_deref().ok(), &phases, &trace);
    let case = match kind {
        "chatgpt" => "chatgpt",
        "apiKey" => "api_key",
        _ => "other",
    };
    crate::logging::write_diagnostic(&format!("case={case} {diagnostic}\n"));
    let requests = requests.unwrap_or_else(|error| {
        panic!(
            "synthetic app-server request log unavailable; usage result={result_summary}; log read kind={:?}; {}",
            error.kind(),
            diagnostic
        )
    });
    (result, requests, outer_timeout, diagnostic)
}
#[cfg(windows)]
#[tokio::test]
async fn native_interactive_rpc_waits_each_response_and_reaps_child() {
    let (result, requests, outer_timeout, diagnostic) = synthetic_account("chatgpt").await;
    assert!(
        !outer_timeout,
        "synthetic app-server outer timeout elapsed; {diagnostic}"
    );
    assert!(
        result.is_ok(),
        "native app-server RPC failed: {:?}; {diagnostic}",
        result.as_ref().err().map(std::io::Error::kind)
    );
    let methods = requests
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["method"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "account/read",
            "account/rateLimits/read"
        ]
    );
    assert!(
        !serde_json::to_string(&result.unwrap())
            .unwrap()
            .contains("synthetic-only")
    );
}
#[cfg(windows)]
#[tokio::test]
async fn api_key_account_stops_before_rate_limit_request() {
    let (result, requests, outer_timeout, diagnostic) = synthetic_account("apiKey").await;
    assert!(
        !outer_timeout,
        "API key synthetic app-server outer timeout elapsed; {diagnostic}"
    );
    assert_eq!(
        result.as_ref().err().map(std::io::Error::kind),
        Some(std::io::ErrorKind::Other),
        "API key account must fail with account rejection, not a timeout; {diagnostic}"
    );
    let methods = requests
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["method"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        ["initialize", "initialized", "account/read"],
        "API key request sequence differs"
    );
}
