//! Go launcher-profiles.yaml DTOs. Loading is deliberately separate from validation.
use crate::proto::wire::{Field, GoWire, Schema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub distro: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub host: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub user: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub ssh_port: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub identity_file: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token_command: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub binary: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub hub_port: i64,
}
fn is_zero(n: &i64) -> bool {
    *n == 0
}
fn absent_profiles(p: &Option<Vec<Profile>>) -> bool {
    p.as_ref().is_none_or(Vec::is_empty)
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfilesFile {
    pub version: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub last_used: String,
    #[serde(skip_serializing_if = "absent_profiles")]
    pub profiles: Option<Vec<Profile>>,
}
impl ProfilesFile {
    pub fn fresh() -> Self {
        Self {
            version: 1,
            ..Self::default()
        }
    }
    pub fn list(&self) -> &[Profile] {
        self.profiles.as_deref().unwrap_or_default()
    }
    pub fn normalize(&mut self) {
        for profile in self.profiles.iter_mut().flatten() {
            profile.normalize();
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for (i, p) in self.list().iter().enumerate() {
            let idx = i + 1;
            if p.name.is_empty() {
                return Err(format!("profile[{idx}]: name is required"));
            }
            if !seen.insert(&p.name) {
                return Err(format!("profile[{idx}]: duplicate name {}", quote(&p.name)));
            }
            p.validate_at(idx)?;
        }
        Ok(())
    }
    pub fn select(&self, name: &str, last: bool) -> Result<Profile, String> {
        let name = if !name.is_empty() {
            name
        } else if last {
            if self.last_used.is_empty() {
                return Err("no last-used profile recorded in launcher-profiles.yaml".into());
            }
            &self.last_used
        } else {
            return Err("SelectProfile requires a profile name or --last".into());
        };
        self.list()
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "profile {} not found in launcher-profiles.yaml",
                    quote(name)
                )
            })
    }
}
impl Profile {
    pub fn normalize(&mut self) {
        if self.kind == "ssh"
            && let Some((user, host)) = self.host.split_once('@')
        {
            let (user, host) = (user.to_owned(), host.to_owned());
            self.user = user;
            self.host = host;
        }
    }
    pub fn binary(&self) -> &str {
        if self.binary.is_empty() {
            "many-ai-cli"
        } else {
            &self.binary
        }
    }
    pub fn mode(&self) -> &str {
        if self.mode.is_empty() {
            "serve"
        } else {
            &self.mode
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        ProfilesFile {
            profiles: Some(vec![self.clone()]),
            ..ProfilesFile::fresh()
        }
        .validate()
    }
    fn validate_at(&self, idx: usize) -> Result<(), String> {
        let prefix = format!("profile[{idx}] {}:", quote(&self.name));
        let err = |detail: String| Err(format!("{prefix} {detail}"));
        match self.kind.as_str() {
            "wsl" => {
                if !self.mode.is_empty() {
                    return err("mode is not applicable to wsl profiles".into());
                }
                validate_port(self.hub_port, false, "hub_port", &prefix)?;
                for (field, value) in [
                    ("distro", &self.distro),
                    ("cwd", &self.cwd),
                    ("binary", &self.binary),
                ] {
                    reject_dash(field, value, &prefix)?;
                }
                for (field, value) in [("distro", &self.distro), ("binary", &self.binary)] {
                    if value.chars().any(space_or_control) {
                        return err(format!(
                            "{field} must not contain whitespace or control characters"
                        ));
                    }
                }
                if self.cwd.chars().any(control) {
                    return err("cwd must not contain control characters".into());
                }
            }
            "ssh" => {
                let mut check = self.clone();
                check.normalize();
                if check.host.trim().is_empty() {
                    return err("host is required".into());
                }
                for (field, value) in [
                    ("host", &check.host),
                    ("user", &check.user),
                    ("binary", &check.binary),
                    ("identity_file", &check.identity_file),
                    ("cwd", &check.cwd),
                ] {
                    reject_dash(field, value, &prefix)?;
                }
                for (field, value) in [("host", &check.host), ("user", &check.user)] {
                    if value.chars().any(space_or_control) {
                        return err(format!(
                            "{field} must not contain whitespace or control characters"
                        ));
                    }
                }
                if check.cwd.chars().any(control) {
                    return err("cwd must not contain control characters".into());
                }
                match self.mode() {
                    "serve" => validate_port(self.hub_port, false, "hub_port", &prefix)?,
                    "tunnel" => {
                        validate_port(self.hub_port, true, "hub_port", &prefix)?;
                        if self.token_command.trim().is_empty() {
                            return err("token_command is required for tunnel mode".into());
                        }
                    }
                    mode => {
                        return err(format!(
                            "unknown mode {} (must be serve or tunnel)",
                            quote(mode)
                        ));
                    }
                }
                validate_port(self.ssh_port, false, "ssh_port", &prefix)?;
            }
            "" => return err("type is required".into()),
            kind => return err(format!("unknown type {} (must be wsl or ssh)", quote(kind))),
        }
        Ok(())
    }
}
fn validate_port(port: i64, required: bool, field: &str, prefix: &str) -> Result<(), String> {
    if port == 0 {
        if required {
            return Err(format!("{prefix} {field} must be 1–65535 (got 0)"));
        }
    } else if !(1..=65535).contains(&port) {
        return Err(format!(
            "{prefix} {field} out of range (got {port}, must be 1–65535)"
        ));
    }
    Ok(())
}
fn reject_dash(field: &str, value: &str, prefix: &str) -> Result<(), String> {
    if value.starts_with('-') {
        Err(format!(
            "{prefix} {field} must not start with '-' (got {})",
            quote(value)
        ))
    } else {
        Ok(())
    }
}
pub(crate) fn control(c: char) -> bool {
    c < '\u{20}' || c == '\u{7f}'
}
pub(crate) fn space_or_control(c: char) -> bool {
    c.is_whitespace() || control(c)
}
/// Go %q uses printable Unicode and Go escapes, rather than Rust's \u{...} syntax.
pub fn quote(value: &str) -> String {
    static PRINTABLE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let printable = PRINTABLE.get_or_init(|| {
        regex::Regex::new(r"\A[\pL\pM\pN\pP\pS ]\z").expect("constant Go printable Unicode classes")
    });
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{b}' => out.push_str("\\v"),
            '\u{c}' => out.push_str("\\f"),
            c if c < '\u{20}' || c == '\u{7f}' => out.push_str(&format!("\\x{:02x}", c as u32)),
            c if !printable.is_match(c.encode_utf8(&mut [0u8; 4])) => {
                if c as u32 <= 0xffff {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                } else {
                    out.push_str(&format!("\\U{:08x}", c as u32));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
pub fn unique_profile_name(existing: &[Profile], name: &str) -> String {
    let name = name.trim();
    let name = if name.is_empty() { "remote" } else { name };
    if !existing.iter().any(|p| p.name == name) {
        return name.into();
    }
    (2..)
        .map(|i| format!("{name}-{i}"))
        .find(|n| !existing.iter().any(|p| p.name == *n))
        .unwrap()
}

macro_rules! fields { ($($name:literal => $kind:literal),* $(,)?) => { &[$(Field { name: $name, kind: $kind }),*] }; }
pub static SCHEMAS: &[Schema] = &[
    Schema {
        name: "LauncherProfilesRequest",
        fields: fields!("profiles"=>"[]LauncherProfile"),
    },
    Schema {
        name: "LauncherProfile",
        fields: fields!("name"=>"string", "type"=>"string", "distro"=>"string", "mode"=>"string", "host"=>"string", "user"=>"string", "ssh_port"=>"int", "identity_file"=>"string", "token_command"=>"string", "binary"=>"string", "cwd"=>"string", "hub_port"=>"int"),
    },
    Schema {
        name: "LauncherProfilesFile",
        fields: fields!("version"=>"int", "last_used"=>"string", "profiles"=>"[]LauncherProfile"),
    },
    Schema {
        name: "LauncherExportedProfile",
        fields: fields!("profile"=>"LauncherProfile", "host_candidates"=>"[]string"),
    },
    Schema {
        name: "LauncherConnectRequest",
        fields: fields!("name"=>"string"),
    },
    Schema {
        name: "LauncherDisconnectRequest",
        fields: fields!("name"=>"string", "mode"=>"string"),
    },
    Schema {
        name: "LauncherFetchParams",
        fields: fields!("host"=>"string", "user"=>"string", "ssh_port"=>"int", "identity_file"=>"string", "binary"=>"string"),
    },
    Schema {
        name: "LauncherActiveConnection",
        fields: fields!("profile"=>"string", "pid"=>"int", "hub_url"=>"string", "started_at"=>"time.Time"),
    },
    Schema {
        name: "LauncherActiveFile",
        fields: fields!("version"=>"int", "connections"=>"[]LauncherActiveConnection"),
    },
    Schema {
        name: "LauncherConnectLock",
        fields: fields!("profile"=>"string", "pid"=>"int", "started_at"=>"time.Time"),
    },
];
impl GoWire for Profile {
    const GO_TYPE: &'static str = "LauncherProfile";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for ProfilesFile {
    const GO_TYPE: &'static str = "LauncherProfilesFile";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}

macro_rules! yaml_fields { ($($name:literal => $kind:literal),* $(,)?) => { &[$(crate::config::YamlField { name: $name, kind: $kind }),*] }; }
pub static YAML_SCHEMAS: &[crate::config::YamlSchema] = &[
    crate::config::YamlSchema {
        name: "LauncherProfile",
        fields: yaml_fields!("name"=>"string", "type"=>"string", "distro"=>"string", "mode"=>"string", "host"=>"string", "user"=>"string", "ssh_port"=>"int", "identity_file"=>"string", "token_command"=>"string", "binary"=>"string", "cwd"=>"string", "hub_port"=>"int"),
    },
    crate::config::YamlSchema {
        name: "LauncherProfilesFile",
        fields: yaml_fields!("version"=>"int", "last_used"=>"string", "profiles"=>"[]LauncherProfile"),
    },
    crate::config::YamlSchema {
        name: "LauncherExportedProfile",
        fields: yaml_fields!("profile"=>"LauncherProfile", "host_candidates"=>"[]string"),
    },
    crate::config::YamlSchema {
        name: "LauncherConnectRequest",
        fields: yaml_fields!("name"=>"string"),
    },
    crate::config::YamlSchema {
        name: "LauncherDisconnectRequest",
        fields: yaml_fields!("name"=>"string", "mode"=>"string"),
    },
    crate::config::YamlSchema {
        name: "LauncherFetchParams",
        fields: yaml_fields!("host"=>"string", "user"=>"string", "ssh_port"=>"int", "identity_file"=>"string", "binary"=>"string"),
    },
    crate::config::YamlSchema {
        name: "LauncherActiveConnection",
        fields: yaml_fields!("profile"=>"string", "pid"=>"int", "hub_url"=>"string", "started_at"=>"time.Time"),
    },
    crate::config::YamlSchema {
        name: "LauncherActiveFile",
        fields: yaml_fields!("version"=>"int", "connections"=>"[]LauncherActiveConnection"),
    },
    crate::config::YamlSchema {
        name: "LauncherConnectLock",
        fields: yaml_fields!("profile"=>"string", "pid"=>"int", "started_at"=>"time.Time"),
    },
];
