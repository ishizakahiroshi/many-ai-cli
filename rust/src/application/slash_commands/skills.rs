use super::*;
use crate::config::{YamlField, YamlSchema, decode_yaml_schema};
use std::{fs, io::Read, path::Path};
const SCHEMAS: &[YamlSchema] = &[YamlSchema {
    name: "skillFrontmatter",
    fields: &[
        YamlField {
            name: "name",
            kind: "string",
        },
        YamlField {
            name: "description",
            kind: "string",
        },
        YamlField {
            name: "user-invokable",
            kind: "*bool",
        },
    ],
}];
#[derive(Default, Deserialize)]
#[serde(default)]
struct Frontmatter {
    name: String,
    description: String,
    #[serde(rename = "user-invokable")]
    user_invokable: Option<bool>,
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b':'))
}
fn read(provider: &str, path: &Path) -> Option<SlashCmd> {
    let mut body = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut body)
        .ok()?;
    let mut normalized = Vec::new();
    let mut offset = 0;
    while offset < body.len() {
        let byte = body[offset];
        if byte == b'\r' {
            normalized.push(b'\n');
            offset += if body.get(offset + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
        } else {
            normalized.push(byte);
            offset += 1;
        }
    }
    let rest = normalized.strip_prefix(b"---\n")?;
    let end = rest.windows(4).position(|bytes| bytes == b"\n---")?;
    // yaml.v3 rejects invalid UTF-8 in frontmatter; body bytes are ignored.
    let raw = std::str::from_utf8(&rest[..end]).ok()?;
    let value = decode_yaml_schema(raw, "skillFrontmatter", SCHEMAS).ok()?;
    let meta: Frontmatter = if value.is_null() {
        Frontmatter::default()
    } else {
        serde_json::from_value(value).ok()?
    };
    if meta.user_invokable == Some(false) {
        return None;
    }
    let name = if meta.name.trim().is_empty() {
        path.parent()?.file_name()?.to_string_lossy().into_owned()
    } else {
        meta.name.trim().to_owned()
    };
    if !safe_name(&name) {
        return None;
    }
    let desc = markdown::clean(&meta.description);
    Some(SlashCmd {
        cmd: format!("{}{name}", skill_prefix(provider)),
        desc: if desc.is_empty() {
            "Skill".into()
        } else {
            format!("Skill. {desc}")
        },
        kind: "skill".into(),
        name,
        path: path.to_owned(),
    })
}
/// The character that starts a skill invocation, declared per provider
/// (`slash_commands.skill_prefix`; `/` when nothing is declared).
fn skill_prefix(provider: &str) -> String {
    crate::profile::declared::declared(provider)
        .and_then(|d| d.slash_commands)
        .map(|s| s.skill_prefix)
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".into())
}
fn env_get<'a>(environment: &'a [String], key: &str) -> &'a str {
    environment
        .iter()
        .find_map(|entry| {
            entry
                .split_once('=')
                .filter(|(name, _)| {
                    if cfg!(windows) {
                        name.eq_ignore_ascii_case(key)
                    } else {
                        *name == key
                    }
                })
                .map(|(_, value)| value.trim())
        })
        .unwrap_or("")
}
pub(super) fn discover(
    provider: &str,
    ctx: &SearchContext,
    environment: &[String],
    home: Option<&Path>,
    paths: &RuntimePaths,
) -> Vec<SlashCmd> {
    if !matches!(provider, "claude" | "codex" | "devin-cli") {
        return vec![];
    }
    let home = if !ctx.home_dir.trim().is_empty() {
        PathBuf::from(ctx.home_dir.trim())
    } else {
        match home {
            Some(home) => home.to_owned(),
            None => return vec![],
        }
    };
    let mut roots = Vec::new();
    if provider == "devin-cli" {
        // `devin skills paths` の実測値: global は APPDATA/devin・APPDATA/cognition・
        // ~/.agents、project は cwd/.devin・.cognition・.agents。
        let config_root = {
            let app_data = env_get(environment, "APPDATA");
            if cfg!(windows) {
                if app_data.is_empty() {
                    home.join("AppData").join("Roaming")
                } else {
                    PathBuf::from(app_data)
                }
            } else {
                home.join(".config")
            }
        };
        roots.push(config_root.join("devin").join("skills"));
        roots.push(config_root.join("cognition").join("skills"));
        roots.push(home.join(".agents").join("skills"));
        let cwd = ctx.cwd.trim();
        if !cwd.is_empty() {
            for name in [".devin", ".cognition", ".agents"] {
                roots.push(PathBuf::from(cwd).join(name).join("skills"));
            }
        }
    } else {
        let (explicit, key, suffix, children) = if provider == "codex" {
            (
                &ctx.codex_home,
                "CODEX_HOME",
                ".codex",
                ["skills", "plugins/cache"],
            )
        } else {
            (
                &ctx.claude_dir,
                "CLAUDE_CONFIG_DIR",
                ".claude",
                ["skills", "plugins"],
            )
        };
        let from_env = env_get(environment, key);
        let vendor = if !explicit.trim().is_empty() {
            PathBuf::from(explicit.trim())
        } else if !from_env.is_empty() {
            PathBuf::from(from_env)
        } else {
            home.join(suffix)
        };
        for suffix in children {
            roots.push(vendor.join(suffix));
        }
    }
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        if !seen.insert(root.clone())
            || !root.is_dir()
            || fs::symlink_metadata(&root).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            continue;
        }
        if paths.is_trial() && sources::under_root(&root, paths.root()).is_err() {
            continue;
        }
        walk(provider, &root, &mut out, paths);
    }
    out
}
fn walk(provider: &str, root: &Path, out: &mut Vec<SlashCmd>, paths: &RuntimePaths) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().into_owned());
    for entry in entries {
        if out.len() >= 500 {
            return;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            if !matches!(
                entry.file_name().to_str(),
                Some(".git" | "node_modules" | "__pycache__")
            ) {
                walk(provider, &path, out, paths)
            }
        } else if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case("SKILL.md")
        {
            if paths.is_trial() && sources::under_root(&path, paths.root()).is_err() {
                continue;
            }
            if let Some(cmd) = read(provider, &path) {
                out.push(cmd)
            }
        }
    }
}
