//! Read-only profile export; fixed cmd/many-ai-cli/main.go flags and native identity.
use crate::launcher::{ExportIdentity, ExportOptions};
pub const USAGE: &str = "Usage of profile-export:\n  -json\n    \toutput as JSON\n  -name string\n    \tprofile display name (default: hostname)\n  -host string\n    \tpublic host to advertise first (overrides auto-detected IP; e.g. for Docker)\n  -cwd string\n    \tremote working directory (default: current directory)\n  -hub-port int\n    \tfixed hub port (0 = auto-select)\n";
pub fn parse(args: &[String]) -> Result<Option<ExportOptions>, String> {
    let mut options = ExportOptions::default();
    let mut json = false;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "--" || arg == "-" || !arg.starts_with('-') {
            break;
        }
        let flag = arg
            .strip_prefix("--")
            .or_else(|| arg.strip_prefix('-'))
            .expect("flag");
        if flag.starts_with(['-', '=']) {
            return Err(format!("bad flag syntax: {arg}"));
        }
        let (name, inline) = flag
            .split_once('=')
            .map_or((flag, None), |(name, value)| (name, Some(value)));
        if matches!(name, "h" | "help") {
            return Ok(None);
        }
        if name == "json" {
            json = match inline.unwrap_or("true") {
                "1" | "t" | "T" | "true" | "TRUE" | "True" => true,
                "0" | "f" | "F" | "false" | "FALSE" | "False" => false,
                value => {
                    return Err(format!(
                        "invalid boolean value {} for -json: parse error",
                        crate::proto::go_quote::quote(value)
                    ));
                }
            };
        } else {
            if !matches!(name, "name" | "host" | "cwd" | "hub-port") {
                return Err(format!("flag provided but not defined: -{name}"));
            }
            let value = match inline {
                Some(value) => value,
                None => {
                    index += 1;
                    args.get(index)
                        .ok_or_else(|| format!("flag needs an argument: -{name}"))?
                }
            };
            match name {
                "name" => options.name = value.into(),
                "host" => options.public_host = value.into(),
                "cwd" => options.cwd = value.into(),
                "hub-port" => {
                    options.hub_port = parse_integer(value).ok_or_else(|| {
                        format!(
                            "invalid value {} for flag -hub-port: parse error",
                            crate::proto::go_quote::quote(value)
                        )
                    })?
                }
                _ => unreachable!(),
            }
        }
        index += 1;
    }
    if !json {
        return Err("profile-export currently supports only --json output".into());
    }
    Ok(Some(options))
}
fn parse_integer(value: &str) -> Option<i64> {
    let (negative, value) = if let Some(v) = value.strip_prefix('-') {
        (true, v)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    let (radix, digits, prefix) = if value.starts_with("0x") || value.starts_with("0X") {
        (16, &value[2..], true)
    } else if value.starts_with("0b") || value.starts_with("0B") {
        (2, &value[2..], true)
    } else if value.starts_with("0o") || value.starts_with("0O") {
        (8, &value[2..], true)
    } else if value.len() > 1 && value.starts_with('0') {
        (8, &value[1..], true)
    } else {
        (10, value, false)
    };
    let mut previous = prefix;
    for (index, character) in digits.chars().enumerate() {
        if character == '_' {
            if !previous || index + 1 == digits.len() {
                return None;
            }
            previous = false;
        } else if character.is_digit(radix) {
            previous = true;
        } else {
            return None;
        }
    }
    if !previous {
        return None;
    }
    let number = u64::from_str_radix(&digits.replace('_', ""), radix).ok()?;
    if negative && number == 1 << 63 {
        Some(i64::MIN)
    } else {
        i64::try_from(number)
            .ok()
            .map(|number| if negative { -number } else { number })
    }
}
pub fn write(
    args: &[String],
    identity: &ExportIdentity,
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
) -> Result<(), String> {
    let options = match parse(args) {
        Ok(Some(options)) => options,
        Ok(None) => {
            return stderr
                .write_all(USAGE.as_bytes())
                .map_err(|error| error.to_string());
        }
        Err(error) => {
            let _ = stderr.write_all(format!("{error}\n{USAGE}").as_bytes());
            return Err(error);
        }
    };
    let exported = crate::launcher::build_export_profile(&options, identity);
    let json = serde_json::to_string_pretty(&exported).map_err(|error| error.to_string())?;
    let escaped = json
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    stdout
        .write_all(escaped.as_bytes())
        .map_err(|error| error.to_string())?;
    stdout.write_all(b"\n").map_err(|error| error.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_only_export_uses_real_shared_profile_without_config_or_keys() {
        let args = [
            "--json",
            "--name=example",
            "--host=example.invalid",
            "--cwd=/synthetic/project",
            "--hub-port=0x_1234",
        ]
        .map(String::from);
        let identity = ExportIdentity {
            username: "synthetic".into(),
            hostname: "synthetic-host".into(),
            executable: "/synthetic/bin/many-ai-cli".into(),
            ..Default::default()
        };
        let mut output = Vec::new();
        let mut errors = Vec::new();
        write(&args, &identity, &mut output, &mut errors).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["profile"]["name"], "example");
        assert_eq!(value["profile"]["hub_port"], 4660);
        assert!(errors.is_empty());
        assert!(parse(&["--json=false".into()]).is_err());
        assert!(parse(&["--help".into()]).unwrap().is_none());
        assert_eq!(
            parse(&["--json".into(), "positional".into(), "--unknown".into()])
                .unwrap()
                .unwrap()
                .hub_port,
            0
        );
    }
}
