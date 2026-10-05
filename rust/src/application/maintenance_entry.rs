//! Thin native CLI composition over canonical maintenance owners.
use super::{main_program::MainContext, maintenance_cli};
use crate::{files::safe_fs::Dir, process::Cancellation};
use std::{io, path::PathBuf, sync::Arc};
pub fn stop_command(context: &MainContext) -> io::Result<maintenance_cli::stop::StopCommand> {
    let cfg = context.config.snapshot().map_err(io::Error::other)?.config;
    let directory = if context.paths.is_trial() {
        context.paths.resource(crate::config::Resource::Logs)
    } else {
        PathBuf::from(&cfg.hub.log_dir)
    };
    let logger = Dir::open_or_create_private(&directory)
        .and_then(|dir| crate::logging::RollingLog::new(Arc::new(dir), "hub.log"))
        .ok()
        .map(Arc::new);
    let log_config = cfg.log.clone();
    let command = maintenance_cli::stop::StopCommand {
        ledger: super::hub_runtime::RuntimeLedger::open(&context.paths)?,
        config: context.config.clone(),
        io: Arc::new(maintenance_cli::stop::NativeStopIo {
            paths: context.paths.clone(),
        }),
        log: Arc::new(move |message, pid, port| {
            if !log_config.enabled {
                return Ok(());
            }
            if let Some(logger) = &logger {
                logger.write(
                    &log_config,
                    format!(
                        "time={} level=INFO msg={} reason=cli_stop pid={pid} port={port}\n",
                        chrono::Local::now().to_rfc3339(),
                        crate::proto::go_quote::quote(message)
                    )
                    .as_bytes(),
                )?;
            }
            Ok(())
        }),
    };
    Ok(command)
}
pub async fn stop(context: &MainContext, cancel: &Cancellation) -> io::Result<()> {
    stop_command(context)?.run(cancel).await
}
pub fn parse_log_clean(args: &[String]) -> Result<(PathBuf, Option<PathBuf>), String> {
    let mut output = None;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if arg == "-" || !arg.starts_with('-') {
            break;
        }
        let flag = arg
            .strip_prefix("--")
            .or_else(|| arg.strip_prefix('-'))
            .unwrap();
        if flag.starts_with(['-', '=']) {
            return Err(format!("bad flag syntax: {arg}"));
        }
        let (name, value) = flag
            .split_once('=')
            .map_or((flag, None), |(n, v)| (n, Some(v)));
        if matches!(name, "help" | "h") {
            return Err("flag: help requested".into());
        }
        if name != "o" {
            return Err(format!("flag provided but not defined: -{name}"));
        }
        let value = match value {
            Some(v) => v,
            None => {
                index += 1;
                args.get(index).ok_or("flag needs an argument: -o")?
            }
        };
        output = (!value.is_empty()).then(|| PathBuf::from(value));
        index += 1;
    }
    if args.len().saturating_sub(index) != 1 {
        return Err("log-clean <session.jsonl> [-o transcript.txt]".into());
    }
    Ok((PathBuf::from(&args[index]), output))
}
pub fn log_clean(context: &MainContext, args: &[String]) -> Result<PathBuf, String> {
    let (input, output) = parse_log_clean(args)?;
    maintenance_cli::log_clean(&context.paths, &input, output.as_deref()).map_err(|e| e.to_string())
}
pub fn provider(context: &MainContext, args: &[String]) -> io::Result<String> {
    let store = Arc::new(
        crate::profile::store::ProviderRegistryStore::new(&context.paths, context.config.clone())
            .map_err(io::Error::other)?,
    );
    maintenance_cli::provider_command(&store, args)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn go_flag_parser_stops_before_input_and_preserves_output_defaults() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse_log_clean(&args(&["-o=chosen.txt", "input.jsonl"])).unwrap(),
            (
                Path::new("input.jsonl").into(),
                Some(Path::new("chosen.txt").into())
            )
        );
        assert_eq!(
            parse_log_clean(&args(&["--o=", "input.jsonl"])).unwrap().1,
            None
        );
        assert!(parse_log_clean(&args(&["input.jsonl", "-o", "chosen.txt"])).is_err());
        assert!(parse_log_clean(&args(&["-o"])).is_err());
    }
}
