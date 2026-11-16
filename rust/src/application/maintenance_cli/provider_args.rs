//! Argument parsing for `many-ai-cli provider add|list|remove|test`.
//! Pure functions: nothing here touches the Hub, the registry or the disk.

/// Every value-taking option accepts `--name value` and `--name=value`. The
/// value is always the next token, even when it starts with `-` (a provider's
/// own flags such as `-p` are exactly what `--arg` / `--headless-arg` carry).
struct Parsed {
    values: Vec<(String, String)>,
    positional: Vec<String>,
    help: bool,
}
fn split(args: &[String], value_options: &[&str]) -> Result<Parsed, String> {
    let mut parsed = Parsed {
        values: vec![],
        positional: vec![],
        help: false,
    };
    let mut index = 0;
    while let Some(token) = args.get(index) {
        index += 1;
        if token == "--help" || token == "-h" {
            parsed.help = true;
        } else if let Some(option) = token.strip_prefix("--") {
            let (name, inline) = match option.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (option, None),
            };
            if !value_options.contains(&name) {
                return Err(format!("unknown option --{name}"));
            }
            let value = match inline {
                Some(value) => value,
                None => {
                    let value = args
                        .get(index)
                        .ok_or_else(|| format!("option --{name} needs a value"))?;
                    index += 1;
                    value.clone()
                }
            };
            parsed.values.push((name.to_owned(), value));
        } else {
            parsed.positional.push(token.clone());
        }
    }
    Ok(parsed)
}
impl Parsed {
    fn one(&self, name: &str) -> Result<Option<&str>, String> {
        let mut found = self.values.iter().filter(|(key, _)| key == name);
        let first = found.next().map(|(_, value)| value.as_str());
        if found.next().is_some() {
            return Err(format!("option --{name} was given more than once"));
        }
        Ok(first)
    }
    fn many(&self, name: &str) -> Vec<String> {
        self.values
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct AddArgs {
    pub id: String,
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub headless_args: Vec<String>,
    /// Some when any `--headless-*` / `--prompt-via` option was given.
    pub headless: Option<(String, String)>,
    pub description: String,
    pub approval_options: Vec<String>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Parse<T> {
    Help,
    Args(T),
}
pub fn parse_add(args: &[String]) -> Result<Parse<AddArgs>, String> {
    let parsed = split(
        args,
        &[
            "id",
            "label",
            "command",
            "arg",
            "headless-arg",
            "headless-format",
            "prompt-via",
            "description",
            "approval-option",
        ],
    )?;
    if parsed.help {
        return Ok(Parse::Help);
    }
    if let Some(extra) = parsed.positional.first() {
        return Err(format!(
            "unexpected argument {extra:?}; use --id, --command ..."
        ));
    }
    let required = |name: &str| -> Result<String, String> {
        parsed
            .one(name)?
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("--{name} is required"))
    };
    let id = required("id")?;
    let command = required("command")?;
    let label = parsed
        .one("label")?
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| id.clone());
    let headless_args = parsed.many("headless-arg");
    let format = parsed.one("headless-format")?;
    let via = parsed.one("prompt-via")?;
    if let Some(via) = via
        && !matches!(via, "arg" | "stdin")
    {
        return Err(format!("--prompt-via must be arg or stdin, not {via:?}"));
    }
    if let Some(format) = format
        && !matches!(format, "text" | "claude-stream-json")
    {
        return Err(format!(
            "--headless-format must be text or claude-stream-json, not {format:?}"
        ));
    }
    let headless = (!headless_args.is_empty() || format.is_some() || via.is_some()).then(|| {
        (
            format.unwrap_or("text").to_owned(),
            via.unwrap_or("arg").to_owned(),
        )
    });
    let approval_options = parsed.many("approval-option");
    for option in &approval_options {
        if option.trim().is_empty() || option.contains(['`', '\n', '\r']) {
            return Err(
                "--approval-option must be a non-empty single line without a backtick".into(),
            );
        }
    }
    Ok(Parse::Args(AddArgs {
        id,
        label,
        command,
        args: parsed.many("arg"),
        headless_args,
        headless,
        description: parsed.one("description")?.unwrap_or("").trim().to_owned(),
        approval_options,
    }))
}

pub fn parse_remove(args: &[String]) -> Result<Parse<String>, String> {
    let parsed = split(args, &[])?;
    if parsed.help {
        return Ok(Parse::Help);
    }
    match parsed.positional.as_slice() {
        [id] => Ok(Parse::Args(id.clone())),
        _ => Err("provider remove <id>".into()),
    }
}

pub const DEFAULT_TEST_PROMPT: &str = "1+1 の答えを数字だけで返して";
pub const DEFAULT_TEST_TIMEOUT_SECS: u64 = 120;
#[derive(Debug, PartialEq, Eq)]
pub struct TestArgs {
    pub id: String,
    pub prompt: String,
    pub timeout_secs: u64,
}
pub fn parse_test(args: &[String]) -> Result<Parse<TestArgs>, String> {
    let parsed = split(args, &["prompt", "timeout"])?;
    if parsed.help {
        return Ok(Parse::Help);
    }
    let [id] = parsed.positional.as_slice() else {
        return Err("provider test <id> [--prompt <text>] [--timeout <sec>]".into());
    };
    let timeout_secs = match parsed.one("timeout")? {
        None => DEFAULT_TEST_TIMEOUT_SECS,
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|seconds| (1..=3600).contains(seconds))
            .ok_or_else(|| format!("--timeout must be 1 to 3600 seconds, not {value:?}"))?,
    };
    let prompt = parsed
        .one("prompt")?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_TEST_PROMPT)
        .to_owned();
    Ok(Parse::Args(TestArgs {
        id: id.clone(),
        prompt,
        timeout_secs,
    }))
}

pub fn parse_list(args: &[String]) -> Result<Parse<()>, String> {
    let parsed = split(args, &[])?;
    if parsed.help {
        return Ok(Parse::Help);
    }
    if parsed.positional.is_empty() {
        Ok(Parse::Args(()))
    } else {
        Err("provider list takes no arguments".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }
    #[test]
    fn add_takes_option_values_that_look_like_flags_and_defaults_the_headless_pair() {
        let Parse::Args(add) = parse_add(&strings(&[
            "--id",
            "fixture-cli",
            "--label=Fixture CLI",
            "--command",
            "fixture-agent",
            "--arg",
            "--interactive",
            "--headless-arg",
            "-p",
            "--approval-option",
            "Allow once",
        ]))
        .unwrap() else {
            panic!("expected arguments")
        };
        assert_eq!(add.label, "Fixture CLI");
        assert_eq!(add.args, ["--interactive"]);
        assert_eq!(add.headless_args, ["-p"]);
        assert_eq!(add.headless, Some(("text".into(), "arg".into())));
        assert_eq!(add.approval_options, ["Allow once"]);
    }
    #[test]
    fn add_without_headless_options_has_no_headless_and_label_falls_back_to_id() {
        let Parse::Args(add) = parse_add(&strings(&[
            "--id",
            "fixture-cli",
            "--command",
            "fixture-agent",
        ]))
        .unwrap() else {
            panic!("expected arguments")
        };
        assert_eq!(add.headless, None);
        assert_eq!(add.label, "fixture-cli");
    }
    #[test]
    fn add_refuses_missing_unknown_repeated_and_invalid_options() {
        let refused = |values: &[&str]| parse_add(&strings(values)).unwrap_err();
        assert!(refused(&["--command", "x"]).contains("--id is required"));
        assert!(refused(&["--id", "a"]).contains("--command is required"));
        assert!(
            refused(&["--id", "a", "--command", "x", "--bogus", "1"]).contains("unknown option")
        );
        assert!(refused(&["--id", "a", "--id", "b", "--command", "x"]).contains("more than once"));
        assert!(
            refused(&["--id", "a", "--command", "x", "--prompt-via", "pipe"])
                .contains("arg or stdin")
        );
        assert!(
            refused(&["--id", "a", "--command", "x", "--headless-format", "json"])
                .contains("claude-stream-json")
        );
        assert!(refused(&["--id", "a", "--command", "x", "--arg"]).contains("needs a value"));
        assert!(
            refused(&["--id", "a", "--command", "x", "--approval-option", "a`b"])
                .contains("backtick")
        );
        assert!(refused(&["--id", "a", "--command", "x", "stray"]).contains("unexpected"));
    }
    #[test]
    fn help_is_recognised_before_required_options() {
        assert_eq!(parse_add(&strings(&["--help"])).unwrap(), Parse::Help);
        assert_eq!(parse_test(&strings(&["-h"])).unwrap(), Parse::Help);
        assert_eq!(parse_remove(&strings(&["--help"])).unwrap(), Parse::Help);
        assert_eq!(parse_list(&strings(&["--help"])).unwrap(), Parse::Help);
    }
    #[test]
    fn test_defaults_and_bounds() {
        let Parse::Args(test) = parse_test(&strings(&["fixture-cli"])).unwrap() else {
            panic!("expected arguments")
        };
        assert_eq!(test.prompt, DEFAULT_TEST_PROMPT);
        assert_eq!(test.timeout_secs, 120);
        assert!(parse_test(&strings(&["a", "--timeout", "0"])).is_err());
        assert!(parse_test(&strings(&["a", "--timeout", "9999"])).is_err());
        assert!(parse_test(&strings(&[])).is_err());
        assert!(parse_test(&strings(&["a", "b"])).is_err());
    }
    #[test]
    fn remove_and_list_shapes() {
        assert_eq!(
            parse_remove(&strings(&["fixture-cli"])).unwrap(),
            Parse::Args("fixture-cli".into())
        );
        assert!(parse_remove(&strings(&[])).is_err());
        assert!(parse_list(&strings(&["extra"])).is_err());
    }
}
