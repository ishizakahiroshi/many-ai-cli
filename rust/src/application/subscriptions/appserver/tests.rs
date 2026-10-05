use super::*;
#[cfg(windows)]
#[path = "stdio_diagnostics.rs"]
mod stdio_diagnostics;
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
async fn synthetic_account(kind: &str) -> (std::io::Result<CodexUsage>, String, String, bool) {
    use crate::config::RuntimePaths;
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49274, installed.path()).unwrap();
    let profile = root.path().join("profile");
    std::fs::create_dir(&profile).unwrap();
    let log = root.path().join("requests.txt");
    let started = root.path().join("script-started.txt");
    let first_line = root.path().join("first-line.txt");
    let script_error = root.path().join("script-error.txt");
    let stderr = root.path().join("stderr.txt");
    let exit_status = root.path().join("exit-status.txt");
    let script = root.path().join("rpc.ps1");
    let ps = format!(
        r#"$ErrorActionPreference = 'Stop'
$log = '{}'
$started = '{}'
$firstLine = '{}'
$scriptError = '{}'
[System.IO.File]::WriteAllText($started, 'started')
try {{
while ($null -ne ($line = [Console]::ReadLine())) {{
 if (-not (Test-Path -LiteralPath $firstLine)) {{ [System.IO.File]::WriteAllText($firstLine, 'received') }}
 Add-Content -LiteralPath $log -Value $line
 $r = $line | ConvertFrom-Json
 switch ($r.method) {{
  'initialize' {{ [Console]::WriteLine('{{"id":0,"result":{{}}}}') }}
  'initialized' {{ }}
  'account/read' {{ [Console]::WriteLine('{{"id":1,"result":{{"account":{{"type":"{}","planType":"plus","email":"synthetic-only@example.com"}}}}}}') }}
  'account/rateLimits/read' {{ [Console]::WriteLine('{{"id":2,"result":{{"rateLimits":{{"primary":{{"usedPercent":0,"windowDurationMins":300}}}}}}}}') }}
 }}
}}
}} catch {{
 [System.IO.File]::WriteAllText($scriptError, $_.Exception.GetType().FullName)
 throw
}}
"#,
        log.display().to_string().replace("'", "''"),
        started.display().to_string().replace("'", "''"),
        first_line.display().to_string().replace("'", "''"),
        script_error.display().to_string().replace("'", "''"),
        kind
    );
    std::fs::write(&script, ps).unwrap();
    let cmd = format!(
        r#"@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "{}" 2>>"{}"
set "exit_code=%ERRORLEVEL%"
>"{}" echo %exit_code%
exit /b %exit_code%
"#,
        script.display(),
        stderr.display(),
        exit_status.display()
    );
    let cmd = cmd.replace("\r\n", "\n").replace('\n', "\r\n");
    std::fs::write(root.path().join("codex.cmd"), cmd).unwrap();
    let environment = vec![
        format!("PATH={}", root.path().display()),
        "PATHEXT=.CMD;.EXE".into(),
        format!("SystemRoot={}", std::env::var("SystemRoot").unwrap()),
        format!("USERPROFILE={}", root.path().display()),
        format!("HOME={}", root.path().display()),
        format!("TEMP={}", root.path().display()),
        format!("TMP={}", root.path().display()),
    ];
    let cli = NativeSubscriptionCli::new(paths, root.path().into(), environment);
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        cli.codex_usage(&profile, &TaskCancellation::default()),
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
    let fixture_root = root.path().display().to_string();
    let system_root = std::env::var("SystemRoot").unwrap();
    let diagnostic_file = |path: &std::path::Path| {
        let mut bytes = Vec::new();
        let read = std::fs::File::open(path).and_then(|file| {
            use std::io::Read as _;
            file.take(2048).read_to_end(&mut bytes)
        });
        read.map(|_| {
            String::from_utf8_lossy(&bytes)
                .replace(&fixture_root, "<fixture>")
                .replace(&system_root, "<SystemRoot>")
                .trim()
                .to_owned()
        })
        .unwrap_or_else(|error| format!("<missing:{:?}>", error.kind()))
    };
    let diagnostics = format!(
        "outer_timeout={outer_timeout}, script_started={}, first_line={}, script_error={}, stderr={}, exit_status={}",
        diagnostic_file(&started),
        diagnostic_file(&first_line),
        diagnostic_file(&script_error),
        diagnostic_file(&stderr),
        diagnostic_file(&exit_status)
    );
    let requests = std::fs::read_to_string(log).unwrap_or_else(|error| {
        panic!(
            "synthetic app-server request log unavailable; usage result={result_summary}; log read kind={:?}; {diagnostics}",
            error.kind()
        )
    });
    (result, requests, diagnostics, outer_timeout)
}
#[cfg(windows)]
#[tokio::test]
async fn native_interactive_rpc_waits_each_response_and_reaps_child() {
    let (result, requests, diagnostics, outer_timeout) = synthetic_account("chatgpt").await;
    assert!(
        !outer_timeout,
        "synthetic app-server outer timeout elapsed; {diagnostics}"
    );
    assert!(
        result.is_ok(),
        "native app-server RPC failed; {diagnostics}"
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
    let (result, requests, diagnostics, outer_timeout) = synthetic_account("apiKey").await;
    assert!(
        !outer_timeout,
        "API key synthetic app-server outer timeout elapsed; {diagnostics}"
    );
    assert_eq!(
        result.as_ref().err().map(std::io::Error::kind),
        Some(std::io::ErrorKind::Other),
        "API key account must fail with account rejection, not a timeout; {diagnostics}"
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
        "API key request sequence differs; {diagnostics}"
    );
}
