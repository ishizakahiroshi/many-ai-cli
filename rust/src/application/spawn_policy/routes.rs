use super::{LocalModelSnapshot, invalid};
use crate::config::{self, Config};
use std::io;

pub(super) fn validate(_provider: &str, route: &str) -> io::Result<()> {
    if !matches!(route, "" | "anthropic" | "openai" | "ollama" | "lm-studio") {
        return Err(invalid("invalid route"));
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
pub(super) fn environment(cfg: &Config, provider: &str, route: &str) -> io::Result<Vec<String>> {
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
