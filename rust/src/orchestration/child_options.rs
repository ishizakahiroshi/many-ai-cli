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
    Row {
        provider: "opencode",
        bounded: Some(Fields {
            permission_mode: config::PERMISSION_MODE_BOUNDED,
            risk_confirmed: true,
            ..EMPTY
        }),
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
    let row = TABLE
        .iter()
        .find(|row| row.provider == provider.trim())
        .unwrap_or(&FALLBACK);
    let requested = if !cfg.child_full_bypass_enabled() {
        config::PERMISSION_PRESET_ATTENDED.to_owned()
    } else if !preset.trim().is_empty() {
        preset.trim().to_owned()
    } else if launch_origin(origin) == config::LAUNCH_ORIGIN_UI
        && execution_mode.trim() != config::EXECUTION_MODE_HEADLESS
    {
        config::PERMISSION_PRESET_ATTENDED.to_owned()
    } else {
        cfg.child_permission_default_tier()
    };
    let (fields, tier, fallback_from) = match requested.as_str() {
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
) -> Result<String, ConfigError> {
    let mut requested = config::normalize_execution_mode(&body.execution_mode);
    if requested.is_empty() {
        requested = cfg.orchestration.child_execution_mode_default();
    }
    config::resolve_execution_mode(
        &requested,
        config::headless_def_for(&body.provider, Some(cfg)).is_some(),
        launch_origin(origin),
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
pub fn prepare(
    mut body: ResolvedChildSpawn,
    cfg: &Config,
) -> Result<ResolvedChildSpawn, ConfigError> {
    let mode = execution_mode(body.request(), body.origin(), cfg)?;
    body.request_mut().execution_mode = mode;
    let permission = resolve_permission(
        &body.request().provider,
        &body.request().permission_preset,
        &body.request().execution_mode,
        body.origin(),
        &cfg.orchestration,
    );
    fill_request(body.request_mut(), &permission);
    body.fill_allowed_tools_from_config(permission.allowed_tools);
    Ok(body)
}

/// The dialog enumerates every provider/tier using the same table and ordering
/// as preparation. Unsupported explicit headless requests remain visible here;
/// actual preparation returns the source error instead of silently downgrading.
pub fn preview_tiers(
    body: &ResolvedChildSpawn,
    cfg: &Config,
) -> BTreeMap<String, BTreeMap<String, ChildApproval>> {
    let mut providers: Vec<&str> = ORCHESTRATION_PROVIDERS.to_vec();
    let requested_provider = body.request().provider.trim();
    if !requested_provider.is_empty() && !providers.contains(&requested_provider) {
        providers.push(requested_provider);
    }
    let mut tiers = vec![String::new()];
    tiers.extend(config::known_permission_presets());
    let mut out: BTreeMap<String, BTreeMap<String, ChildApproval>> = BTreeMap::new();
    for tier in tiers {
        for provider in &providers {
            let mut request = body.request().clone();
            request.provider = (*provider).into();
            if !tier.is_empty() {
                request.permission_preset.clone_from(&tier);
            }
            request.execution_mode =
                execution_mode(&request, body.origin(), cfg).unwrap_or_else(|_| {
                    config::normalize_execution_mode(&body.request().execution_mode)
                });
            let mut permission = resolve_permission(
                provider,
                &request.permission_preset,
                &request.execution_mode,
                body.origin(),
                &cfg.orchestration,
            );
            fill_request(&mut request, &permission);
            permission.permission_mode = request.permission_mode;
            permission.sandbox = request.sandbox;
            permission.ask_for_approval = request.ask_for_approval;
            permission.risk_confirmed = request.risk_confirmed;
            if !body.grants().allowed_tools().is_empty() {
                permission.allowed_tools = body.grants().allowed_tools().to_vec();
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
        AuthEpoch, InternalSpawnGrants, SpawnConfirmationId, SpawnConfirmationResponse, UiBinding,
        UiConnectionId, VerifiedUiOrigin,
    };

    fn proof() -> VerifiedUiOrigin {
        VerifiedUiOrigin::after_server_verification(UiBinding {
            connection: UiConnectionId(1),
            auth_epoch: AuthEpoch(7),
        })
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
                    proof(),
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
            let preview = preview_tiers(&body, &cfg);
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
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                case["approval"],
                "prepared {name}"
            );
            assert_eq!(
                serde_json::to_value(&preview[&body.provider][""]).unwrap(),
                case["approval"],
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
                    proof(),
                )
                .unwrap();
            let prepared = prepare(confirmed, &Config::default()).unwrap();
            assert!(prepared.grants().grant_folder_trust());
            if tools.is_empty() {
                assert!(
                    prepared
                        .grants()
                        .allowed_tools()
                        .contains(&"Bash(git status)".into())
                );
            } else {
                assert_eq!(prepared.grants().allowed_tools(), tools);
            }
            let wire = serde_json::to_value(prepared.request()).unwrap();
            assert!(wire.get("allowed_tools").is_none());
            assert!(wire.get("grant_folder_trust").is_none());
        }
    }
}
