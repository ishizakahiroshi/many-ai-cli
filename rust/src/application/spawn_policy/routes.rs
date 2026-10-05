use super::{LocalModelSnapshot, invalid};
use crate::{
    config::{self, Config, RuntimePaths},
    files::safe_fs::Dir,
};
use serde_json::{Map, Value};
use std::io;

pub(super) fn validate(provider: &str, route: &str) -> io::Result<()> {
    if !matches!(
        route,
        "" | "anthropic" | "openai" | "ollama" | "lm-studio" | "nvidia-nim"
    ) {
        return Err(invalid("invalid route"));
    }
    if route == "nvidia-nim" && provider != "opencode" {
        return Err(invalid("NVIDIA NIM route is only available for OpenCode"));
    }
    Ok(())
}
pub(super) fn infer<'a>(provider: &str, model: &str, models: &LocalModelSnapshot) -> &'a str {
    let model = model.trim();
    if model.is_empty() {
        ""
    } else if models.lm_studio.contains(model) {
        "lm-studio"
    } else if models.ollama.contains(model) || model.contains(":cloud") {
        "ollama"
    } else if provider == "opencode" && model.starts_with("nvidia/") {
        "nvidia-nim"
    } else if provider == "claude" {
        "anthropic"
    } else if provider == "codex" {
        "openai"
    } else {
        ""
    }
}
pub(super) fn env_value<'a>(environment: &'a [String], key: &str) -> Option<&'a str> {
    environment.iter().rev().find_map(|entry| {
        entry
            .split_once('=')
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v)
    })
}
pub(super) fn environment(
    paths: &RuntimePaths,
    cfg: &Config,
    provider: &str,
    route: &str,
    model: &str,
    base: &[String],
) -> io::Result<Vec<String>> {
    if route == "nvidia-nim" {
        if !cfg.nvidia_nim.enabled {
            return Err(invalid("NVIDIA NIM route is disabled"));
        }
        let id =
            nim_model(model).ok_or_else(|| invalid("NVIDIA NIM model selection is invalid"))?;
        let key = nim_key(paths, base).map_err(|_| invalid("NVIDIA API key is unavailable"))?;
        let runtime = nim_config(env_value(base, "OPENCODE_CONFIG_CONTENT").unwrap_or(""), id)?;
        return Ok(vec![
            format!("NVIDIA_API_KEY={key}"),
            format!("OPENCODE_CONFIG_CONTENT={runtime}"),
        ]);
    }
    let (endpoint, token) = match route {
        "ollama" => (
            config::effective_ollama_base_url(&cfg.ollama.base_url),
            "ollama",
        ),
        "lm-studio" => (
            config::effective_lm_studio_base_url(&cfg.lm_studio.base_url),
            "lmstudio",
        ),
        _ => return Ok(vec![]),
    };
    Ok(match provider {
        "claude" => vec![
            format!("ANTHROPIC_AUTH_TOKEN={token}"),
            "ANTHROPIC_API_KEY=".into(),
            format!("ANTHROPIC_BASE_URL={endpoint}"),
        ],
        "codex" => vec![
            format!("OPENAI_API_KEY={token}"),
            format!("OPENAI_BASE_URL={endpoint}/v1"),
        ],
        _ => vec![],
    })
}
fn nim_key(paths: &RuntimePaths, environment: &[String]) -> io::Result<String> {
    let value = env_value(environment, "NVIDIA_API_KEY")
        .unwrap_or("")
        .trim();
    let value = if !value.is_empty() {
        value.to_owned()
    } else {
        let dir = Dir::open(paths.root())?.child_dir("secrets", false)?;
        let bytes = dir.read("nvidia_api_key", 1024 * 1024 + 1)?;
        if bytes.len() > 1024 * 1024 {
            return Err(invalid("NVIDIA API key is unavailable"));
        }
        String::from_utf8(bytes)
            .map_err(|_| invalid("NVIDIA API key is unavailable"))?
            .trim()
            .to_owned()
    };
    if value.is_empty() || value.contains(['\0', '\r', '\n']) {
        return Err(invalid("NVIDIA API key is unavailable"));
    }
    Ok(value)
}
fn nim_model(model: &str) -> Option<&str> {
    let id = model.trim().strip_prefix("nvidia/")?;
    id.split('/')
        .all(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
        .then_some(id)
}
fn object<'a>(
    root: &'a mut Map<String, Value>,
    key: &str,
) -> io::Result<&'a mut Map<String, Value>> {
    root.entry(key.to_owned())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| invalid("invalid OpenCode runtime config"))
}
fn nim_config(existing: &str, model: &str) -> io::Result<String> {
    let mut root: Map<String, Value> = if existing.trim().is_empty() {
        Map::new()
    } else {
        serde_json::from_str(existing).map_err(|_| invalid("invalid OpenCode runtime config"))?
    };
    let nvidia = object(object(&mut root, "provider")?, "nvidia")?;
    // Validate every object before replacing scalar fields. Failure returns a
    // fixed diagnostic rather than source JSON or credentials.
    object(nvidia, "options")?;
    object(nvidia, "models")?;
    nvidia.insert("name".into(), "NVIDIA NIM".into());
    nvidia.insert("npm".into(), "@ai-sdk/openai-compatible".into());
    let options = object(nvidia, "options")?;
    options.insert(
        "baseURL".into(),
        "https://integrate.api.nvidia.com/v1".into(),
    );
    options.insert("apiKey".into(), "{env:NVIDIA_API_KEY}".into());
    object(nvidia, "models")?.insert(model.into(), serde_json::json!({"name": model}));
    serde_json::to_string(&root).map_err(|_| invalid("invalid OpenCode runtime config"))
}
