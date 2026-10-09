//! Go flag lexical contract shared by source CLI clients.
use std::collections::BTreeMap;
#[derive(Clone, Copy)]
pub enum Kind {
    String,
    Bool,
    Int,
}
pub struct Flags {
    pub values: BTreeMap<String, String>,
    pub positional: Vec<String>,
    pub help: bool,
}
impl Flags {
    pub fn value(&self, key: &str) -> &str {
        self.values.get(key).map(String::as_str).unwrap_or("")
    }
    pub fn boolean(&self, key: &str) -> bool {
        self.value(key) == "true"
    }
    pub fn integer(&self, key: &str) -> i64 {
        self.value(key).parse().unwrap_or(0)
    }
}
pub fn parse(args: &[String], spec: &[(&str, Kind)]) -> Result<Flags, String> {
    let mut result = Flags {
        values: BTreeMap::new(),
        positional: vec![],
        help: false,
    };
    let mut at = 0;
    while at < args.len() {
        let arg = &args[at];
        if arg == "--" {
            result.positional = args[at + 1..].to_vec();
            return Ok(result);
        }
        if arg == "-" || !arg.starts_with('-') {
            result.positional = args[at..].to_vec();
            return Ok(result);
        }
        let flag = arg
            .strip_prefix("--")
            .or_else(|| arg.strip_prefix('-'))
            .unwrap();
        if flag.starts_with(['-', '=']) {
            return Err(format!("bad flag syntax: {arg}"));
        }
        let (name, inline) = flag
            .split_once('=')
            .map_or((flag, None), |(name, value)| (name, Some(value)));
        if matches!(name, "h" | "help") {
            result.help = true;
            return Ok(result);
        }
        let kind = spec
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, kind)| *kind)
            .ok_or_else(|| format!("flag provided but not defined: -{name}"))?;
        let value = if matches!(kind, Kind::Bool) {
            match inline.unwrap_or("true") {
                "1" | "t" | "T" | "true" | "TRUE" | "True" => "true",
                "0" | "f" | "F" | "false" | "FALSE" | "False" => "false",
                _ => return Err(format!("invalid boolean value for -{name}")),
            }
            .to_owned()
        } else {
            let value = if let Some(value) = inline {
                value.to_owned()
            } else {
                at += 1;
                args.get(at)
                    .ok_or_else(|| format!("flag needs an argument: -{name}"))?
                    .clone()
            };
            if matches!(kind, Kind::Int) && value.parse::<i64>().is_err() {
                return Err(format!("invalid integer value for -{name}"));
            }
            value
        };
        result.values.insert(name.to_owned(), value);
        at += 1;
    }
    Ok(result)
}
