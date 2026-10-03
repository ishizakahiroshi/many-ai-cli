//! Pure opt-in shell function output; never modifies a user's shell profile.
use crate::config::{Config, effective_custom_providers};
use crate::proto::provider::BUILTIN_PROVIDER_IDS;

pub fn init_script() -> String {
    init_script_for_providers(BUILTIN_PROVIDER_IDS.iter().copied())
}
pub fn init_script_for_config(cfg: &Config) -> String {
    let custom = effective_custom_providers(&cfg.custom_providers);
    init_script_for_providers(
        BUILTIN_PROVIDER_IDS
            .iter()
            .copied()
            .chain(custom.iter().map(|p| p.id.as_str())),
    )
}
pub fn init_script_for_providers<'a>(ids: impl IntoIterator<Item = &'a str>) -> String {
    let mut script = "\nif [ \"${MANY_AI_CLI_AUTO:-0}\" = \"1\" ]; then\n".to_owned();
    for id in ids {
        if !shell_function_id(id) {
            continue;
        }
        script.push_str(&format!("  {id}(){{ many-ai-cli {id} \"$@\"; }}\n"));
    }
    script.push_str("fi\n");
    script
}
fn shell_function_id(id: &str) -> bool {
    id.as_bytes()
        .first()
        .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_init_preserves_output_order_and_omits_unsafe_ids() {
        assert_eq!(
            init_script_for_providers([
                "claude",
                "cursor-agent",
                "1bad",
                "bad;id",
                "_ok",
                "日本語"
            ]),
            "\nif [ \"${MANY_AI_CLI_AUTO:-0}\" = \"1\" ]; then\n  claude(){ many-ai-cli claude \"$@\"; }\n  cursor-agent(){ many-ai-cli cursor-agent \"$@\"; }\n  _ok(){ many-ai-cli _ok \"$@\"; }\nfi\n"
        );
    }
}
