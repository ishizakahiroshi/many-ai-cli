//! Doctor CLI shares the production diagnostic actor and explicit authority.
use super::{diagnostics::*, main_program::MainContext};
use crate::process::Cancellation;
use std::{io, sync::Arc};
pub const USAGE: &str = "Usage of doctor:\n  -json\n    \toutput as JSON\n";
pub fn parse(args: &[String]) -> Result<Option<bool>, String> {
    let mut json = false;
    for arg in args {
        if arg == "--" || arg == "-" || !arg.starts_with('-') {
            break;
        }
        let flag = arg
            .strip_prefix("--")
            .or_else(|| arg.strip_prefix('-'))
            .unwrap();
        if flag.starts_with(['-', '=']) {
            return Err(format!("bad flag syntax: {arg}"));
        }
        let (name, value) = flag.split_once('=').unwrap_or((flag, "true"));
        if matches!(name, "help" | "h") {
            return Ok(None);
        }
        if name != "json" {
            return Err(format!("flag provided but not defined: -{name}"));
        }
        json = match value {
            "1" | "t" | "T" | "true" | "TRUE" | "True" => true,
            "0" | "f" | "F" | "false" | "FALSE" | "False" => false,
            _ => {
                return Err(format!(
                    "invalid boolean value {} for -json: parse error",
                    crate::proto::go_quote::quote(value)
                ));
            }
        };
    }
    Ok(Some(json))
}
/// The doctor report when the settings file cannot be read as a whole: the
/// other checks all need the settings, so this one finding is the report.
pub fn unreadable_config_report(unreadable: &crate::config::ConfigUnreadable) -> DoctorReport {
    let fix = format!(
        "Correct the file at the position shown, then run `{}`; or run `{} --yes` to start over from the defaults (the current file is saved to {} first)",
        crate::config::CHECK_COMMAND,
        "many-ai-cli config reset",
        unreadable.backups.display()
    );
    let message = format!(
        "The settings file cannot be read: {} ({})",
        unreadable.summary_en(),
        unreadable.path.display()
    );
    DoctorReport {
        checks: vec![Check::new(
            "config",
            "FAIL",
            report::redact(&message),
            &report::redact(&fix),
        )],
    }
}
pub fn run_unreadable_config(
    args: &[String],
    unreadable: &crate::config::ConfigUnreadable,
    output: &mut dyn io::Write,
    errors: &mut dyn io::Write,
) -> Result<(), String> {
    let Some(json) = parse(args)? else {
        errors
            .write_all(USAGE.as_bytes())
            .map_err(|e| e.to_string())?;
        return Err("flag: help requested".into());
    };
    write_report(unreadable_config_report(unreadable), json, output)
}
fn write_report(
    report: DoctorReport,
    json: bool,
    output: &mut dyn io::Write,
) -> Result<(), String> {
    if json {
        let json = serde_json::to_string(&report)
            .map_err(|e| e.to_string())?
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026")
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029");
        writeln!(output, "{json}").map_err(|e| e.to_string())?;
    } else {
        for check in report.checks {
            writeln!(
                output,
                "[{}] {}: {}",
                check.level, check.name, check.message
            )
            .map_err(|e| e.to_string())?;
            if !check.fix.is_empty() {
                writeln!(output, "      -> {}", check.fix).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}
pub async fn run(
    context: &MainContext,
    args: &[String],
    cancel: &Cancellation,
    output: &mut dyn io::Write,
    errors: &mut dyn io::Write,
) -> Result<(), String> {
    let Some(json) = parse(args)? else {
        errors
            .write_all(USAGE.as_bytes())
            .map_err(|e| e.to_string())?;
        return Err("flag: help requested".into());
    };
    let catalog = Arc::new(
        super::main_program::model_catalog::native::NativeCatalogIo::new(
            context.paths.clone(),
            context.environment.clone(),
            context.cwd.clone(),
            Arc::new(super::main_program::model_cache::LocalModelCache::default()),
        ),
    );
    let doctor = Diagnostics::new(DiagnosticsDependencies {
        config: context.config.clone(),
        paths: context.paths.clone(),
        cwd: context.cwd.clone(),
        home: context.vendor_home.clone(),
        environment: context.environment.clone(),
        platform: std::env::consts::OS.into(),
        io: Arc::new(NativeDiagnosticIo {
            paths: context.paths.clone(),
            cwd: context.cwd.clone(),
            environment: context.environment.clone(),
        }),
        subscription_cli: Arc::new(super::subscriptions::cli::NativeSubscriptionCli::new(
            context.paths.clone(),
            context.cwd.clone(),
            context.environment.clone(),
        )),
        nvidia_key_configured: Arc::new(move || {
            use super::main_program::model_catalog::CatalogIo;
            catalog.nvidia_key().map(|key| !key.is_empty())
        }),
    });
    let report = doctor
        .run(cancel)
        .await
        .map_err(|_| "doctor was cancelled or unavailable".to_string())?;
    write_report(report, json, output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn go_bool_flags_stop_at_first_positional_and_do_not_consume_next_argument() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse(&args(&["-json", "false", "--invalid"])).unwrap(),
            Some(true)
        );
        assert_eq!(parse(&args(&["--json=FALSE"])).unwrap(), Some(false));
        assert_eq!(parse(&args(&["--", "--invalid"])).unwrap(), Some(false));
        assert_eq!(parse(&args(&["-h"])).unwrap(), None);
        assert!(parse(&args(&["--json=other"])).is_err());
        assert!(parse(&args(&["---json"])).is_err());
    }
}
