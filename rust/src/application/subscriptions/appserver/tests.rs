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
async fn synthetic_account(kind: &str) -> (std::io::Result<CodexUsage>, String) {
    use crate::config::RuntimePaths;
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49274, installed.path()).unwrap();
    let profile = root.path().join("profile");
    std::fs::create_dir(&profile).unwrap();
    let log = root.path().join("requests.txt");
    let script = root.path().join("rpc.ps1");
    let ps = format!(
        r#"$ErrorActionPreference = 'Stop'
$log = '{}'
while ($null -ne ($line = [Console]::ReadLine())) {{
 Add-Content -LiteralPath $log -Value $line
 $r = $line | ConvertFrom-Json
 switch ($r.method) {{
  'initialize' {{ [Console]::WriteLine('{{"id":0,"result":{{}}}}') }}
  'initialized' {{ }}
  'account/read' {{ [Console]::WriteLine('{{"id":1,"result":{{"account":{{"type":"{}","planType":"plus","email":"synthetic-only@example.com"}}}}}}') }}
  'account/rateLimits/read' {{ [Console]::WriteLine('{{"id":2,"result":{{"rateLimits":{{"primary":{{"usedPercent":0,"windowDurationMins":300}}}}}}}}') }}
 }}
}}
"#,
        log.display().to_string().replace("'", "''"),
        kind
    );
    std::fs::write(&script, ps).unwrap();
    let cmd = format!(
        r#"@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "{}"
"#,
        script.display()
    );
    std::fs::write(root.path().join("codex.cmd"), cmd.replace('\n', "\r\n")).unwrap();
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
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        cli.codex_usage(&profile, &TaskCancellation::default()),
    )
    .await
    .expect("bounded synthetic app-server receipt");
    let requests = std::fs::read_to_string(log).unwrap();
    (result, requests)
}
#[cfg(windows)]
#[tokio::test]
async fn native_interactive_rpc_waits_each_response_and_reaps_child() {
    let (result, requests) = synthetic_account("chatgpt").await;
    assert!(result.is_ok());
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
    let (result, requests) = synthetic_account("apiKey").await;
    assert!(result.is_err());
    assert!(requests.contains("account/read"));
    assert!(!requests.contains("account/rateLimits/read"));
}
