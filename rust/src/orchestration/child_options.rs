//! Fixed-Go child_permission.go: the same permission table drives disclosure
//! and launch preparation. Human confirmation preserves autonomous defaults;
//! direct verified UI launches use attended defaults when interactive.
use crate::{
    config::{self, Config, ConfigError, OrchestrationConfig},
    proto::{
        ChildApproval,
        core::{ChildSpawnRequest, ResolvedChildSpawn, VerifiedSpawnOrigin},
    },
};
use std::collections::BTreeMap;

pub const ORCHESTRATION_PROVIDERS: &[&str] = &[
    "claude",
    "codex",
    "copilot",
    "cursor-agent",
    "opencode",
    "grok",
    "command-code",
];

/// AI CLIs registered by the user (providers.d, the Hub's registration API, or
/// config.yaml) that an orchestrate child may use besides the built-in seven.
/// A provider qualifies when the Hub's current registry has it enabled with a
/// `launch` definition; the flag says whether it also has `launch.headless`.
/// The default value holds no registered provider, so only built-ins are valid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegisteredProviders {
    headless: BTreeMap<String, bool>,
    /// Whether the definition maps `--model` (`launch.model_args`) / effort
    /// (`launch.effort_args`) to flags. Without a mapping the value would be
    /// carried into the session without ever reaching the CLI.
    takes_model: BTreeMap<String, bool>,
    takes_effort: BTreeMap<String, bool>,
}

impl RegisteredProviders {
    pub fn from_registry(registry: &crate::profile::registry::Registry) -> Self {
        let mut headless = BTreeMap::new();
        let mut takes_model = BTreeMap::new();
        let mut takes_effort = BTreeMap::new();
        for summary in registry.list() {
            if summary.id == "shell" || ORCHESTRATION_PROVIDERS.contains(&summary.id.as_str()) {
                continue;
            }
            let Some(entry) = registry.lookup(&summary.id) else {
                continue;
            };
            if !entry.definition.enabled.unwrap_or(true) {
                continue;
            }
            let Some(launch) = entry.definition.launch.as_ref() else {
                continue;
            };
            takes_model.insert(summary.id.clone(), !launch.model_args.is_empty());
            takes_effort.insert(summary.id.clone(), !launch.effort_args.is_empty());
            headless.insert(summary.id, launch.headless.is_some());
        }
        Self {
            headless,
            takes_model,
            takes_effort,
        }
    }
    /// A registered provider whose definition has no `model_args`.
    fn ignores_model(&self, provider: &str) -> bool {
        self.takes_model.get(provider) == Some(&false)
    }
    /// A registered provider whose definition has no effort mapping.
    fn ignores_effort(&self, provider: &str) -> bool {
        self.takes_effort.get(provider) == Some(&false)
    }
    /// Built-in seven, or a usable registered provider.
    pub fn is_valid(&self, provider: &str) -> bool {
        ORCHESTRATION_PROVIDERS.contains(&provider) || self.headless.contains_key(provider)
    }
    pub fn is_registered(&self, provider: &str) -> bool {
        self.headless.contains_key(provider)
    }
    /// Registered provider with a `launch.headless` definition.
    pub fn has_headless(&self, provider: &str) -> bool {
        self.headless.get(provider).copied().unwrap_or(false)
    }
    /// Whether `execution_mode: headless` can run for this provider: a built-in
    /// or config.yaml headless definition, or a registered `launch.headless`.
    pub fn headless_capable(&self, provider: &str, cfg: &Config) -> bool {
        self.has_headless(provider) || config::headless_def_for(provider, Some(cfg)).is_some()
    }
    /// Every id the Hub accepts for `--provider`, built-ins first.
    pub fn valid_ids(&self) -> Vec<String> {
        ORCHESTRATION_PROVIDERS
            .iter()
            .map(|id| (*id).to_owned())
            .chain(self.headless.keys().cloned())
            .collect()
    }
    pub fn valid_list(&self) -> String {
        self.valid_ids().join(", ")
    }
}

/// The 400 text for an unknown child provider. `quoted` is the already-quoted
/// requested id.
pub fn invalid_provider_message(quoted: &str, providers: &RegisteredProviders) -> String {
    format!(
        "invalid provider {quoted}; valid providers are: {}",
        providers.valid_list()
    )
}

#[derive(Clone, Copy)]
struct Fields {
    permission_mode: &'static str,
    sandbox: &'static str,
    ask_for_approval: &'static str,
    risk_confirmed: bool,
}
const EMPTY: Fields = Fields {
    permission_mode: "",
    sandbox: "",
    ask_for_approval: "",
    risk_confirmed: false,
};
const BYPASS: Fields = Fields {
    permission_mode: "bypassPermissions",
    risk_confirmed: true,
    ..EMPTY
};
struct Row {
    provider: &'static str,
    bounded: Option<Fields>,
    full: Fields,
}
const TABLE: &[Row] = &[
    Row {
        provider: "claude",
        bounded: Some(Fields {
            permission_mode: "dontAsk",
            risk_confirmed: true,
            ..EMPTY
        }),
        full: BYPASS,
    },
    Row {
        provider: "codex",
        bounded: Some(Fields {
            sandbox: "workspace-write",
            ask_for_approval: "never",
            risk_confirmed: true,
            ..EMPTY
        }),
        full: Fields {
            sandbox: "danger-full-access",
            ask_for_approval: "never",
            risk_confirmed: true,
            ..EMPTY
        },
    },
    Row {
        provider: "copilot",
        bounded: Some(Fields {
            permission_mode: config::PERMISSION_MODE_BOUNDED,
            risk_confirmed: true,
            ..EMPTY
        }),
        full: BYPASS,
    },
    // OpenCode's only unattended mode (--auto) runs any command its deny rules
    // do not name, so a bounded request is shown and run as full access, like
    // Grok (U49 follow-up, owner's decision B2).
    Row {
        provider: "opencode",
        bounded: None,
        full: BYPASS,
    },
    Row {
        provider: "grok",
        bounded: None,
        full: BYPASS,
    },
    Row {
        provider: "cursor-agent",
        bounded: None,
        full: BYPASS,
    },
    Row {
        provider: "command-code",
        bounded: None,
        full: BYPASS,
    },
    Row {
        provider: "shell",
        bounded: Some(EMPTY),
        full: EMPTY,
    },
];
const FALLBACK: Row = Row {
    provider: "",
    bounded: None,
    full: BYPASS,
};

/// Neither a built-in child provider nor the shell: a registered AI CLI.
fn is_registered_child_provider(provider: &str) -> bool {
    let provider = provider.trim();
    !provider.is_empty() && provider != "shell" && !ORCHESTRATION_PROVIDERS.contains(&provider)
}

fn launch_origin(origin: &VerifiedSpawnOrigin) -> &'static str {
    match origin {
        VerifiedSpawnOrigin::HumanUi(_) => config::LAUNCH_ORIGIN_UI,
        VerifiedSpawnOrigin::Autonomous | VerifiedSpawnOrigin::HumanConfirmation { .. } => {
            config::LAUNCH_ORIGIN_CONDUCTOR
        }
    }
}

pub fn resolve_permission(
    provider: &str,
    preset: &str,
    execution_mode: &str,
    origin: &VerifiedSpawnOrigin,
    cfg: &OrchestrationConfig,
) -> ChildApproval {
    resolve_permission_for_origin(provider, preset, execution_mode, launch_origin(origin), cfg)
}
fn resolve_permission_for_origin(
    provider: &str,
    preset: &str,
    execution_mode: &str,
    origin: &str,
    cfg: &OrchestrationConfig,
) -> ChildApproval {
    // A registered (non built-in) AI CLI gets no built-in permission flags,
    // exactly like a new session of it: nothing is attached and nothing is asked.
    if is_registered_child_provider(provider) {
        return ChildApproval {
            tier: config::PERMISSION_PRESET_ATTENDED.into(),
            ..Default::default()
        };
    }
    let requested = if !cfg.child_full_bypass_enabled() {
        config::PERMISSION_PRESET_ATTENDED.to_owned()
    } else if !preset.trim().is_empty() {
        preset.trim().to_owned()
    } else if origin == config::LAUNCH_ORIGIN_UI
        && execution_mode.trim() != config::EXECUTION_MODE_HEADLESS
    {
        config::PERMISSION_PRESET_ATTENDED.to_owned()
    } else {
        cfg.child_permission_default_tier()
    };
    permission_preset_approval(provider, &requested, cfg)
}
/// Ordinary UI requests explicitly select a preset. The source's direct table
/// lookup does not apply autonomous defaults or alter human risk confirmation.
pub fn permission_preset_approval(
    provider: &str,
    requested: &str,
    cfg: &OrchestrationConfig,
) -> ChildApproval {
    let row = TABLE
        .iter()
        .find(|row| row.provider == provider.trim())
        .unwrap_or(&FALLBACK);
    let (fields, tier, fallback_from) = match requested.trim() {
        config::PERMISSION_PRESET_ATTENDED => (EMPTY, config::PERMISSION_PRESET_ATTENDED, ""),
        config::PERMISSION_PRESET_BOUNDED => match row.bounded {
            Some(fields) => (fields, config::PERMISSION_PRESET_BOUNDED, ""),
            None => (
                row.full,
                config::PERMISSION_PRESET_FULL,
                config::PERMISSION_PRESET_BOUNDED,
            ),
        },
        _ => (row.full, config::PERMISSION_PRESET_FULL, ""),
    };
    ChildApproval {
        permission_mode: fields.permission_mode.into(),
        sandbox: fields.sandbox.into(),
        ask_for_approval: fields.ask_for_approval.into(),
        risk_confirmed: fields.risk_confirmed,
        allowed_tools: if tier == config::PERMISSION_PRESET_BOUNDED {
            cfg.bounded_allowed_tools_for(provider)
        } else {
            Vec::new()
        },
        tier: tier.into(),
        fallback_from: fallback_from.into(),
    }
}

fn execution_mode(
    body: &ChildSpawnRequest,
    origin: &VerifiedSpawnOrigin,
    cfg: &Config,
    providers: &RegisteredProviders,
) -> Result<String, ConfigError> {
    execution_mode_for_origin(body, launch_origin(origin), cfg, providers)
}
fn execution_mode_for_origin(
    body: &ChildSpawnRequest,
    origin: &str,
    cfg: &Config,
    providers: &RegisteredProviders,
) -> Result<String, ConfigError> {
    let mut requested = config::normalize_execution_mode(&body.execution_mode);
    if requested.is_empty() {
        requested = cfg.orchestration.child_execution_mode_default();
    }
    config::resolve_execution_mode(
        &requested,
        providers.headless_capable(&body.provider, cfg),
        origin,
        false,
    )
}
fn fill_request(body: &mut ChildSpawnRequest, permission: &ChildApproval) {
    if body.permission_mode.is_empty() {
        body.permission_mode.clone_from(&permission.permission_mode);
    }
    if body.sandbox.is_empty() {
        body.sandbox.clone_from(&permission.sandbox);
    }
    if body.ask_for_approval.is_empty() {
        body.ask_for_approval
            .clone_from(&permission.ask_for_approval);
    }
    body.risk_confirmed |= permission.risk_confirmed;
}

/// Only server-resolved provenance can reach the actual launch preparation.
/// Configuration adds bounded tools through the internal grant field; request
/// JSON cannot manufacture that list or a folder-trust grant.
pub fn prepare(body: ResolvedChildSpawn, cfg: &Config) -> Result<ResolvedChildSpawn, ConfigError> {
    prepare_with(body, cfg, &RegisteredProviders::default())
}
/// Like [`prepare`], with the registered AI CLIs the Hub currently knows, so a
/// registered provider's own `launch.headless` counts for `execution_mode`.
pub fn prepare_with(
    mut body: ResolvedChildSpawn,
    cfg: &Config,
    providers: &RegisteredProviders,
) -> Result<ResolvedChildSpawn, ConfigError> {
    let mode = execution_mode(body.request(), body.origin(), cfg, providers)?;
    body.request_mut().execution_mode = mode;
    // A registered CLI can only be told a model or an effort it has a flag for.
    // Otherwise the request would show a model in the session that never reached
    // the program.
    let provider = body.request().provider.trim().to_owned();
    if is_registered_child_provider(&provider) {
        if providers.ignores_model(&provider) {
            body.request_mut().model.clear();
        }
        if providers.ignores_effort(&provider) {
            body.request_mut().effort.clear();
        }
    }
    let permission = resolve_permission(
        &body.request().provider,
        &body.request().permission_preset,
        &body.request().execution_mode,
        body.origin(),
        &cfg.orchestration,
    );
    fill_request(body.request_mut(), &permission);
    // A registered CLI has no tier table, so `resolve_permission` never marks
    // its launch as risk-confirmed. Its approval mode is the one a person chose
    // or saw (the confirmation dialog shows the requested mode and can clear it
    // with `default`). That choice is the confirmation; without it a human who
    // picked bypassPermissions always hit "risk confirmation required" (a 500).
    // An autonomous request is never confirmed this way, and
    // `child_full_bypass: false` keeps refusing high-risk modes for children.
    if is_registered_child_provider(&provider)
        && body.origin().is_human()
        && cfg.orchestration.child_full_bypass_enabled()
        && !matches!(body.request().permission_mode.trim(), "" | "default")
    {
        body.request_mut().risk_confirmed = true;
    }
    body.fill_allowed_tools_from_config(permission.allowed_tools);
    Ok(body)
}

/// The dialog enumerates every provider/tier using the same table and ordering
/// as preparation. Unsupported explicit headless requests remain visible here;
/// actual preparation returns the source error instead of silently downgrading.
/// `providers` must be the same registry reading the launch uses, so a
/// registered provider's own `launch.headless` counts here exactly as it does in
/// [`prepare_with`]; otherwise the dialog would approve a different mode.
pub fn preview_tiers(
    body: &ResolvedChildSpawn,
    cfg: &Config,
    providers: &RegisteredProviders,
) -> BTreeMap<String, BTreeMap<String, ChildApproval>> {
    preview_for(
        body.request(),
        launch_origin(body.origin()),
        body.grants().allowed_tools(),
        cfg,
        providers,
    )
}
/// Pure /api/info disclosure. This returns data, never a UI-origin capability
/// or a resolved launch request, and requires no WebSocket/auth proof minting.
pub fn preview_ui_tiers(cfg: &Config) -> BTreeMap<String, BTreeMap<String, ChildApproval>> {
    preview_for(
        &ChildSpawnRequest::default(),
        config::LAUNCH_ORIGIN_UI,
        &[],
        cfg,
        &RegisteredProviders::default(),
    )
}
fn preview_for(
    body: &ChildSpawnRequest,
    origin: &str,
    allowed_tools: &[String],
    cfg: &Config,
    registered: &RegisteredProviders,
) -> BTreeMap<String, BTreeMap<String, ChildApproval>> {
    let mut providers: Vec<&str> = ORCHESTRATION_PROVIDERS.to_vec();
    let requested_provider = body.provider.trim();
    if !requested_provider.is_empty() && !providers.contains(&requested_provider) {
        providers.push(requested_provider);
    }
    let mut tiers = vec![String::new()];
    tiers.extend(config::known_permission_presets());
    let mut out: BTreeMap<String, BTreeMap<String, ChildApproval>> = BTreeMap::new();
    for tier in tiers {
        for provider in &providers {
            let mut request = body.clone();
            request.provider = (*provider).into();
            if !tier.is_empty() {
                request.permission_preset.clone_from(&tier);
            }
            request.execution_mode = execution_mode_for_origin(&request, origin, cfg, registered)
                .unwrap_or_else(|_| config::normalize_execution_mode(&body.execution_mode));
            let mut permission = resolve_permission_for_origin(
                provider,
                &request.permission_preset,
                &request.execution_mode,
                origin,
                &cfg.orchestration,
            );
            fill_request(&mut request, &permission);
            permission.permission_mode = request.permission_mode;
            permission.sandbox = request.sandbox;
            permission.ask_for_approval = request.ask_for_approval;
            permission.risk_confirmed = request.risk_confirmed;
            if !allowed_tools.is_empty() {
                permission.allowed_tools = allowed_tools.to_vec();
            }
            out.entry((*provider).into())
                .or_default()
                .insert(tier.clone(), permission);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::core::{
        AuthEpoch, InternalSpawnGrants, SpawnConfirmationId, SpawnConfirmationResponse,
        VerifiedConfirmationRequest, VerifiedUiOrigin,
    };

    fn proof() -> VerifiedUiOrigin {
        VerifiedUiOrigin::after_server_verification(AuthEpoch(7))
    }
    fn text<'a>(input: &'a serde_json::Value, field: &str) -> &'a str {
        input[field].as_str().unwrap_or_default()
    }
    fn request(input: &serde_json::Value) -> ResolvedChildSpawn {
        let body = ChildSpawnRequest {
            provider: text(input, "provider").into(),
            permission_preset: text(input, "preset").into(),
            execution_mode: text(input, "mode").into(),
            claimed_origin: text(input, "origin").into(),
            permission_mode: text(input, "permission_mode").into(),
            sandbox: text(input, "sandbox").into(),
            ask_for_approval: text(input, "ask_for_approval").into(),
            ..Default::default()
        };
        let tools: Vec<String> = input["existing_tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .map(|tool| tool.as_str().unwrap().into())
                    .collect()
            })
            .unwrap_or_default();
        let mut resolved = ResolvedChildSpawn::from_request(
            body,
            (text(input, "origin") == "ui").then(proof),
            InternalSpawnGrants::from_config(tools),
        )
        .unwrap();
        if input["confirmed"].as_bool().unwrap_or(false) {
            resolved = resolved
                .approve(
                    &SpawnConfirmationResponse {
                        approved: true,
                        confirmation_id: SpawnConfirmationId("synthetic-confirmation".into()),
                        ..Default::default()
                    },
                    VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(7)),
                )
                .unwrap();
        }
        resolved
    }
    #[test]
    fn child_permission_preparation_and_preview_match_fixed_go() {
        let corpus: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../tests/fixtures/core/orchestration/child-options-go-21d0bc7.json"
        ))
        .unwrap();
        assert_eq!(corpus.len(), 40);
        for case in corpus {
            let input = &case["input"];
            let name = text(input, "name");
            let cfg = Config {
                orchestration: OrchestrationConfig {
                    child_full_bypass: input["full_bypass"].as_bool(),
                    child_permission_default: text(input, "default_tier").into(),
                    child_execution_mode: text(input, "default_mode").into(),
                    bounded_allowed_tools: if input["tools"].is_null() {
                        BTreeMap::new()
                    } else {
                        serde_json::from_value(input["tools"].clone()).unwrap()
                    },
                    ..Default::default()
                },
                ..Default::default()
            };
            let body = request(input);
            let preview = preview_tiers(&body, &cfg, &RegisteredProviders::default());
            let prepared = prepare(body, &cfg);
            if let Some(expected) = case["error"].as_str() {
                match prepared {
                    Err(error) => assert_eq!(error.to_string(), expected, "{name}"),
                    Ok(_) => panic!("{name}: unsupported launch accepted"),
                }
                continue;
            }
            let prepared = prepared.unwrap();
            let body = prepared.request();
            assert_eq!(
                body.execution_mode,
                case["mode"].as_str().unwrap(),
                "{name}"
            );
            let mut actual = resolve_permission(
                &body.provider,
                &body.permission_preset,
                &body.execution_mode,
                prepared.origin(),
                &cfg.orchestration,
            );
            actual.permission_mode.clone_from(&body.permission_mode);
            actual.sandbox.clone_from(&body.sandbox);
            actual.ask_for_approval.clone_from(&body.ask_for_approval);
            actual.risk_confirmed = body.risk_confirmed;
            actual.allowed_tools = prepared.grants().allowed_tools().to_vec();
            // U49 changed the built-in bounded lists on purpose for Copilot and
            // where Claude Code has no sandbox (config's tests pin them). Where
            // it has one, Claude's list is still the Go version's.
            let mut expected = case["approval"].clone();
            if input["tools"].is_null()
                && input["existing_tools"].as_array().is_none_or(Vec::is_empty)
                && expected["allowed_tools"].is_array()
            {
                let built_in = serde_json::to_value(
                    cfg.orchestration.bounded_allowed_tools_for(&body.provider),
                )
                .unwrap();
                if body.provider == "claude" && config::CLAUDE_SANDBOX {
                    assert_eq!(expected["allowed_tools"], built_in, "unchanged {name}");
                }
                expected["allowed_tools"] = built_in;
            }
            // The owner's decision B2 (U49 follow-up): a bounded OpenCode child is
            // disclosed and run as full access, the way Go already treated Grok.
            if body.provider == "opencode" && expected["tier"] == config::PERMISSION_PRESET_BOUNDED
            {
                let full = serde_json::to_value(permission_preset_approval(
                    "opencode",
                    config::PERMISSION_PRESET_FULL,
                    &cfg.orchestration,
                ))
                .unwrap();
                assert_eq!(full["permission_mode"], "bypassPermissions", "{name}");
                expected = full;
                expected["fallback_from"] = config::PERMISSION_PRESET_BOUNDED.into();
            }
            // A registered AI CLI is not a built-in: no built-in permission flags
            // are attached or disclosed, whatever tier was asked for (the oracle
            // case records the old custom-provider fallback to bypass).
            if body.provider == "synthetic-custom" {
                expected = serde_json::json!({"tier": "attended"});
            }
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                expected,
                "prepared {name}"
            );
            assert_eq!(
                serde_json::to_value(&preview[&body.provider][""]).unwrap(),
                expected,
                "disclosed {name}"
            );
        }
    }

    #[test]
    fn confirmation_does_not_reclassify_an_autonomous_launch_as_direct_ui() {
        let cfg = Config {
            orchestration: OrchestrationConfig {
                child_execution_mode: "auto".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let direct = prepare(
            request(&serde_json::json!({"provider":"claude","origin":"ui"})),
            &cfg,
        )
        .unwrap();
        assert_eq!(direct.request().execution_mode, "interactive");
        assert!(direct.request().permission_mode.is_empty());
        let confirmed = prepare(
            request(&serde_json::json!({"provider":"claude","confirmed":true})),
            &cfg,
        )
        .unwrap();
        assert_eq!(confirmed.request().execution_mode, "headless");
        assert_eq!(confirmed.request().permission_mode, "bypassPermissions");
        assert!(matches!(
            confirmed.origin(),
            VerifiedSpawnOrigin::HumanConfirmation { .. }
        ));
    }

    #[test]
    fn bounded_tool_fill_retains_human_folder_trust_and_prior_internal_tools() {
        for tools in [Vec::new(), vec!["Read".into()]] {
            let original = ResolvedChildSpawn::from_request(
                ChildSpawnRequest {
                    provider: "claude".into(),
                    permission_preset: "bounded".into(),
                    ..Default::default()
                },
                None,
                InternalSpawnGrants::from_config(tools.clone()),
            )
            .unwrap();
            let confirmed = original
                .approve(
                    &SpawnConfirmationResponse {
                        approved: true,
                        grant_folder_trust: Some(true),
                        confirmation_id: SpawnConfirmationId("synthetic-confirmation".into()),
                        ..Default::default()
                    },
                    VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(7)),
                )
                .unwrap();
            let prepared = prepare(confirmed, &Config::default()).unwrap();
            assert!(prepared.grants().grant_folder_trust());
            if tools.is_empty() {
                // In the built-in list with or without Claude Code's sandbox.
                assert!(
                    prepared
                        .grants()
                        .allowed_tools()
                        .contains(&"Bash(git commit *)".into())
                );
            } else {
                assert_eq!(prepared.grants().allowed_tools(), tools);
            }
            let wire = serde_json::to_value(prepared.request()).unwrap();
            assert!(wire.get("allowed_tools").is_none());
            assert!(wire.get("grant_folder_trust").is_none());
        }
    }

    /// OpenCode's --auto runs any command a deny rule does not name, so a
    /// bounded request falls back to full access and says so, like Grok.
    #[test]
    fn audit_u49_opencode_bounded_is_disclosed_as_full_access() {
        let cfg = OrchestrationConfig::default();
        for provider in ["opencode", "grok"] {
            let approval =
                permission_preset_approval(provider, config::PERMISSION_PRESET_BOUNDED, &cfg);
            assert_eq!(approval.tier, config::PERMISSION_PRESET_FULL, "{provider}");
            assert_eq!(
                approval.fallback_from,
                config::PERMISSION_PRESET_BOUNDED,
                "{provider}"
            );
            assert_eq!(approval.permission_mode, "bypassPermissions", "{provider}");
            assert!(approval.allowed_tools.is_empty(), "{provider}");
        }
    }

    fn registry_with(
        definitions: Vec<crate::proto::provider::Definition>,
    ) -> crate::profile::registry::Registry {
        use crate::profile::registry::{Registry, default_adapters, embedded_definitions};
        use crate::proto::provider::Layers;
        Registry::build(
            Layers {
                embedded: Some(embedded_definitions().unwrap()),
                user: Some(definitions),
                ..Default::default()
            },
            &default_adapters(),
        )
    }
    fn user_definition(
        id: &str,
        headless: bool,
        enabled: bool,
    ) -> crate::proto::provider::Definition {
        use crate::proto::provider::{Definition, HeadlessDefinition, LaunchDefinition};
        Definition {
            schema_version: 1,
            id: id.into(),
            display_name: "Fixture Agent".into(),
            enabled: (!enabled).then_some(false),
            launch: Some(LaunchDefinition {
                executable: "fixture-agent-bin".into(),
                args: vec!["--base".into()],
                headless: headless.then(|| HeadlessDefinition {
                    args: vec!["-p".into()],
                    format: "text".into(),
                    prompt_via: "arg".into(),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn registered() -> RegisteredProviders {
        RegisteredProviders::from_registry(&registry_with(vec![
            user_definition("fixture-headless", true, true),
            user_definition("fixture-plain", false, true),
            user_definition("fixture-off", true, false),
        ]))
    }

    #[test]
    fn registered_providers_are_enabled_launchable_non_builtin_entries() {
        let providers = registered();
        assert!(providers.is_valid("claude"));
        assert!(providers.is_valid("fixture-headless"));
        assert!(providers.is_valid("fixture-plain"));
        assert!(!providers.is_valid("fixture-off"), "disabled");
        assert!(!providers.is_valid("fixture-unknown"));
        assert!(!providers.is_valid("shell"));
        assert!(!providers.is_valid(""));
        assert!(providers.has_headless("fixture-headless"));
        assert!(!providers.has_headless("fixture-plain"));
        // Built-ins are never listed twice.
        assert_eq!(
            providers.valid_list(),
            "claude, codex, copilot, cursor-agent, opencode, grok, command-code, fixture-headless, fixture-plain"
        );
        // No registry reading leaves exactly the built-ins.
        assert_eq!(
            RegisteredProviders::default().valid_ids(),
            ORCHESTRATION_PROVIDERS
        );
    }

    #[test]
    fn a_registered_child_keeps_only_the_model_and_effort_its_definition_maps_to_flags() {
        let mut flagged = user_definition("fixture-flags", true, true);
        let launch = flagged.launch.as_mut().unwrap();
        launch.model_args = vec!["--model".into(), "{model}".into()];
        launch.effort_args = vec!["--effort".into(), "{effort}".into()];
        let mut model_only = user_definition("fixture-model", true, true);
        model_only.launch.as_mut().unwrap().model_args = vec!["-m".into(), "{model}".into()];
        let providers = RegisteredProviders::from_registry(&registry_with(vec![
            flagged,
            model_only,
            user_definition("fixture-plain", false, true),
        ]));
        let cfg = Config::default();
        let prepared = |provider: &str| {
            let body = ChildSpawnRequest {
                provider: provider.into(),
                claimed_origin: "ui".into(),
                model: "synthetic-model".into(),
                effort: "high".into(),
                ..Default::default()
            };
            let resolved = ResolvedChildSpawn::from_request(
                body,
                Some(proof()),
                InternalSpawnGrants::from_config(vec![]),
            )
            .unwrap();
            let prepared = prepare_with(resolved, &cfg, &providers).unwrap();
            (
                prepared.request().model.clone(),
                prepared.request().effort.clone(),
            )
        };
        assert_eq!(
            prepared("fixture-flags"),
            ("synthetic-model".into(), "high".into())
        );
        assert_eq!(
            prepared("fixture-model"),
            ("synthetic-model".into(), String::new())
        );
        assert_eq!(prepared("fixture-plain"), (String::new(), String::new()));
        // Built-ins carry their own flags and are untouched.
        assert_eq!(
            prepared("claude"),
            ("synthetic-model".into(), "high".into())
        );
    }

    #[test]
    fn unknown_provider_message_lists_built_ins_and_registered_ids() {
        let message = invalid_provider_message("\"nope\"", &registered());
        assert_eq!(
            message,
            "invalid provider \"nope\"; valid providers are: claude, codex, copilot, cursor-agent, opencode, grok, command-code, fixture-headless, fixture-plain"
        );
    }

    #[test]
    fn headless_child_of_a_registered_provider_needs_its_own_headless_definition() {
        let providers = registered();
        let cfg = Config::default();
        let headless = |provider: &str, mode: &str| {
            prepare_with(
                request(&serde_json::json!({"provider":provider,"mode":mode,"confirmed":true})),
                &cfg,
                &providers,
            )
        };
        let mode = |provider: &str, mode: &str| {
            headless(provider, mode)
                .unwrap()
                .request()
                .execution_mode
                .clone()
        };
        assert_eq!(mode("fixture-headless", "headless"), "headless");
        // auto becomes headless for an unattended child that can run headless,
        // and interactive for one that cannot.
        assert_eq!(mode("fixture-headless", "auto"), "headless");
        assert_eq!(mode("fixture-plain", "auto"), "interactive");
        let error = headless("fixture-plain", "headless")
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("headless is not supported for this provider"),
            "{error}"
        );
        // The plain prepare() knows no registry, so it keeps rejecting.
        assert!(
            prepare(
                request(&serde_json::json!({"provider":"fixture-headless","mode":"headless"})),
                &cfg,
            )
            .is_err()
        );
        // Built-ins are unchanged.
        assert_eq!(mode("claude", "headless"), "headless");
    }

    #[test]
    fn registered_child_gets_no_built_in_permission_fields() {
        let providers = registered();
        let cfg = Config::default();
        for preset in ["", "attended", "bounded", "full"] {
            let prepared = prepare_with(
                request(&serde_json::json!({"provider":"fixture-headless","mode":"headless","preset":preset,"confirmed":true})),
                &cfg,
                &providers,
            )
            .unwrap();
            let body = prepared.request();
            assert!(body.permission_mode.is_empty(), "{preset}");
            assert!(body.sandbox.is_empty(), "{preset}");
            assert!(body.ask_for_approval.is_empty(), "{preset}");
            assert!(!body.risk_confirmed, "{preset}");
            assert!(prepared.grants().allowed_tools().is_empty(), "{preset}");
        }
        // The confirmation dialog's table shows no flags for it either.
        let preview = preview_for(
            &ChildSpawnRequest {
                provider: "fixture-headless".into(),
                ..Default::default()
            },
            config::LAUNCH_ORIGIN_UI,
            &[],
            &cfg,
            &providers,
        );
        for approval in preview["fixture-headless"].values() {
            assert!(approval.permission_mode.is_empty());
            assert!(approval.sandbox.is_empty());
            assert!(!approval.risk_confirmed);
        }
    }

    /// pending devin-child (起動 500): a person confirmed a registered child's
    /// bypassPermissions, but nothing marked it risk-confirmed, so the launch
    /// always failed with "risk confirmation required".
    #[test]
    fn a_human_confirmed_registered_mode_is_the_risk_confirmation() {
        let providers = registered();
        let prepared = |input: serde_json::Value, cfg: &Config| {
            prepare_with(request(&input), cfg, &providers)
                .unwrap()
                .request()
                .risk_confirmed
        };
        let cfg = Config::default();
        // The AI asked for the mode; the person saw it in the dialog and approved.
        assert!(prepared(
            serde_json::json!({"provider":"fixture-plain","permission_mode":"bypassPermissions","confirmed":true}),
            &cfg,
        ));
        assert!(prepared(
            serde_json::json!({"provider":"fixture-plain","permission_mode":"acceptEdits","confirmed":true}),
            &cfg,
        ));
        // No mode, or the dialog's explicit "no flag", needs no confirmation.
        for mode in ["", "default"] {
            assert!(
                !prepared(
                    serde_json::json!({"provider":"fixture-plain","permission_mode":mode,"confirmed":true}),
                    &cfg,
                ),
                "{mode:?}"
            );
        }
        // Nobody saw an autonomous request.
        assert!(!prepared(
            serde_json::json!({"provider":"fixture-plain","permission_mode":"bypassPermissions"}),
            &cfg,
        ));
        // child_full_bypass: false keeps refusing a high-risk child mode.
        let strict = Config {
            orchestration: OrchestrationConfig {
                child_full_bypass: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!prepared(
            serde_json::json!({"provider":"fixture-plain","permission_mode":"bypassPermissions","confirmed":true}),
            &strict,
        ));
    }

    #[test]
    fn registered_headless_definition_builds_the_expected_argv() {
        let registry = registry_with(vec![user_definition("fixture-headless", true, true)]);
        let definition = registry.lookup("fixture-headless").unwrap();
        let headless = definition
            .definition
            .launch
            .as_ref()
            .and_then(|launch| launch.headless.as_ref())
            .map(|h| config::HeadlessDef {
                args: h.args.clone(),
                format: h.format.clone(),
                prompt_via: h.prompt_via.clone(),
            })
            .unwrap();
        assert_eq!(
            crate::orchestration::headless::build_argv(&headless, &[], "1+1"),
            vec!["-p".to_owned(), "1+1".to_owned()]
        );
    }
}
