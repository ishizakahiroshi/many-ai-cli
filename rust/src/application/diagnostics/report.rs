//! Fixed Go report privacy and allowlisted templates. No configuration serialization.
use crate::config::Config;
use regex::{Captures, Regex};
use std::{collections::BTreeMap, net::IpAddr, sync::OnceLock};
const SECRET: &str = "<REDACTED_SECRET>";
fn patterns() -> &'static Vec<(Regex, &'static str)> {
    static CACHE: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    CACHE.get_or_init(||vec![
 (r#"(?i)[a-z]:[\\/]+dev[\\/]+(?:kb|\.ssh|github[\\/]+private)(?:[\\/]+[^\s"'<>|]*)?"#,"<REDACTED_PRIVATE_PATH>"),
 (r#"(?i)/(?:srv/)?dev/(?:kb|\.ssh|github/private)(?:/[^\s"'<>|]*)?"#,"<REDACTED_PRIVATE_PATH>"),
 (r"(?i)[a-z]:[\\/]+users[\\/]+[^\\/\s]+[\\/]+","~/"),(r"/(?:home|Users)/[^/\s]+/","~/"),
 (r#"(?i)([?&](?:access_)?token=)[^&#\s"']+"#,"${1}<REDACTED_SECRET>"),
 (r#"(?i)((?:authorization\s*:\s*)?bearer\s+)[^\s,"']+"#,"${1}<REDACTED_SECRET>"),
 (r#"(?i)(\b[a-z0-9_-]*(?:secret|token|password|passwd|passphrase|credentials?|(?:api|auth|access|client|private|secret|signing|encryption|master|session|refresh|vapid|pin)[_-]?(?:key|hash))\b(?:\\?["'])?\s*[:=]\s*)(?:\\?["'])?[^\s,"';&?#}]+["']?"#,"${1}<REDACTED_SECRET>"),
 (r"([a-zA-Z][a-zA-Z0-9+.\-]*://[^\s:/@]+:)([^\s/@]+)(@)","${1}<REDACTED_SECRET>${3}"),
 (r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b",SECRET),
 (r"\bsk-(?:ant-api[0-9]+-)?[A-Za-z0-9_-]{10,}\b",SECRET),(r"\b(?:ghp_|gho_|ghu_|ghs_|ghr_|github_pat_)[A-Za-z0-9_]{10,}\b",SECRET),
 (r"\bglpat-[A-Za-z0-9_-]{10,}\b",SECRET),(r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b",SECRET),(r"\bAIza[A-Za-z0-9_-]{10,}\b",SECRET),
 (r"\bhf_[A-Za-z0-9]{10,}\b",SECRET),(r"\bnpm_[A-Za-z0-9]{10,}\b",SECRET),(r"\bpypi-[A-Za-z0-9_-]{10,}\b",SECRET),(r"\bxai-[A-Za-z0-9_-]{10,}\b",SECRET),(r"\bgsk_[A-Za-z0-9]{10,}\b",SECRET),(r"\b(?:AKIA|ASIA|AROA)[A-Z0-9]{16}\b",SECRET),
 (r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----",SECRET),
 ].into_iter().map(|(p,r)|(Regex::new(p).expect("fixed report expression"),r)).collect())
}
pub fn redact(text: &str) -> String {
    let mut text = text.to_owned();
    for (pattern, replacement) in patterns() {
        text = pattern.replace_all(&text, *replacement).into_owned();
    }
    for pattern in [
        r"\b(?:[0-9]{1,3}\.){3}[0-9]{1,3}\b",
        r"(?i)[0-9a-f]*:[0-9a-f:]+",
    ] {
        let re = Regex::new(pattern).expect("fixed IP expression");
        text = re
            .replace_all(&text, |c: &Captures| match c[0].parse::<IpAddr>() {
                Ok(ip) if !ip.is_loopback() => "<REDACTED_IP>".to_owned(),
                _ => c[0].to_owned(),
            })
            .into_owned();
    }
    let public = Regex::new(r"(?i)ishizakahiroshi\.dev@gmail\.com").unwrap();
    text = public
        .replace_all(&text, "MANYAICLI_PUBLIC_EMAIL_PLACEHOLDER")
        .into_owned();
    let email=Regex::new(r"[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)+").unwrap();
    text = email
        .replace_all(&text, "<REDACTED_EMAIL>")
        .into_owned()
        .replace(
            "MANYAICLI_PUBLIC_EMAIL_PLACEHOLDER",
            "ishizakahiroshi.dev@gmail.com",
        );
    Regex::new(r"(?i)\bishiz\.[a-z0-9.-]+\b")
        .unwrap()
        .replace_all(&text, "<REDACTED_HOST>")
        .into_owned()
}
const PROVIDERS: &[&str] = &[
    "claude",
    "codex",
    "copilot",
    "cursor-agent",
    "opencode",
    "grok",
    "command-code",
];
pub struct Environment {
    pub version: String,
    pub platform: String,
    pub arch: String,
    pub runtime_version: String,
    pub provider: String,
    pub model: String,
    pub user_agent: String,
    pub allowed: BTreeMap<String, String>,
}
pub struct EnvironmentInput<'a> {
    pub version: &'a str,
    pub platform: &'a str,
    pub arch: &'a str,
    pub runtime_version: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub user_agent: &'a str,
}
pub fn collect(cfg: &Config, input: EnvironmentInput<'_>) -> Environment {
    let EnvironmentInput {
        version,
        platform,
        arch,
        runtime_version,
        provider,
        model,
        user_agent,
    } = input;
    let mut allowed = BTreeMap::from([("hub_port".into(), cfg.hub.port.to_string())]);
    let mut providers = vec![];
    for name in PROVIDERS {
        if let Some(model) = cfg.user_prefs.spawn.last_model.get(*name)
            && !model.trim().is_empty()
        {
            providers.push(*name);
            allowed.insert(format!("model.{name}"), redact(model.trim()));
        }
    }
    providers.sort();
    if !providers.is_empty() {
        allowed.insert("providers".into(), providers.join(","));
    }
    Environment {
        version: redact(version).trim().into(),
        platform: platform.into(),
        arch: arch.into(),
        runtime_version: runtime_version.into(),
        provider: if PROVIDERS.contains(&provider.trim()) {
            provider.trim().into()
        } else {
            String::new()
        },
        model: redact(model).trim().into(),
        user_agent: redact(user_agent).trim().into(),
        allowed,
    }
}
pub fn render_environment(env: &Environment, locale: &str) -> String {
    let ja = locale.trim().to_lowercase().starts_with("ja");
    let labels = if ja {
        [
            "many-ai-cli バージョン",
            "OS",
            "アーキテクチャ",
            "Rust バージョン",
            "Provider",
            "モデル",
            "ブラウザ",
        ]
    } else {
        [
            "many-ai-cli version",
            "OS",
            "Architecture",
            "Rust version",
            "Provider",
            "Model",
            "Browser",
        ]
    };
    let mut fields = labels
        .into_iter()
        .zip([
            &env.version,
            &env.platform,
            &env.arch,
            &env.runtime_version,
            &env.provider,
            &env.model,
            &env.user_agent,
        ])
        .map(|(k, v)| (k.to_owned(), v.clone()))
        .collect::<Vec<_>>();
    fields.extend(env.allowed.iter().map(|(k, v)| (k.clone(), v.clone())));
    redact(
        &fields
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| format!("- {k}: `{}`", redact(&v).trim().replace('`', "'")))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}
pub fn render_markdown(
    locale: &str,
    symptom: &str,
    reproduction: &str,
    environment: &str,
) -> String {
    let ja = locale.trim().to_lowercase().starts_with("ja");
    let (a, b, c, empty) = if ja {
        ("症状", "再現手順（任意）", "環境情報", "未記入")
    } else {
        (
            "Symptom",
            "Steps to reproduce (optional)",
            "Environment",
            "Not provided",
        )
    };
    redact(&format!(
        "## {a}\n\n{}\n\n## {b}\n\n{}\n\n## {c}\n\n{}\n",
        symptom.trim(),
        if reproduction.trim().is_empty() {
            empty
        } else {
            reproduction.trim()
        },
        environment.trim()
    ))
}
pub fn default_title(symptom: &str) -> String {
    let first = symptom.replace("\r\n", "\n");
    let first = first
        .split('\n')
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let first = if first.chars().count() > 80 {
        format!("{}…", first.chars().take(80).collect::<String>())
    } else {
        first
    };
    if first.is_empty() {
        "Bug report".into()
    } else {
        format!("Bug: {}", redact(&first))
    }
}
pub fn issue_url(title: &str, body: &str) -> Option<String> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("body", &redact(body))
        .append_pair("title", &redact(title.trim()))
        .finish();
    let result = format!("https://github.com/ishizakahiroshi/many-ai-cli/issues/new?{query}");
    (result.len() <= 8192).then_some(result)
}
