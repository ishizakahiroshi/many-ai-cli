//! Provider registry layers and immutable snapshots from the frozen Go oracle.
use super::validation::{sanitize_presentation, validate_definition};
use crate::proto::provider::*;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub fn embedded_definitions() -> Result<Vec<Definition>, serde_json::Error> {
    let mut definitions = vec![];
    for bytes in [
        include_bytes!("../../../internal/provider/manifests/claude.json").as_slice(),
        include_bytes!("../../../internal/provider/manifests/codex.json").as_slice(),
        include_bytes!("../../../internal/provider/manifests/copilot.json").as_slice(),
        include_bytes!("../../../internal/provider/manifests/cursor-agent.json").as_slice(),
        include_bytes!("../../../internal/provider/manifests/opencode.json").as_slice(),
        include_bytes!("../../../internal/provider/manifests/grok.json").as_slice(),
        include_bytes!("../../../internal/provider/manifests/command-code.json").as_slice(),
    ] {
        let mut def = crate::proto::decode_wire::<Definition>(bytes)?;
        def.source.origin = ORIGIN_EMBEDDED.into();
        definitions.push(def);
    }
    Ok(definitions)
}
pub fn default_adapters() -> AdapterCatalog {
    let keys = [
        "approval:claude-v1",
        "approval:codex-v1",
        "approval:copilot-v1",
        "approval:cursor-agent-v1",
        "approval:opencode-v1",
        "approval:grok-v1",
        "approval:command-code-v1",
        "approval:generic-v1",
        "history:claude-v1",
        "history:codex-v1",
        "history:cursor-agent-v1",
        "history:opencode-v1",
        "history:command-code-v1",
        "usage:claude-v1",
        "usage:codex-v1",
        "usage:opencode-v1",
        "usage:grok-v1",
        "subscription:claude-v1",
        "subscription:codex-v1",
        "subscription:opencode-v1",
        "subscription:grok-v1",
        "launch:generic-v1",
        "permissions:generic-v1",
        "subagent:claude-v1",
        "subagent:codex-v1",
        "subagent:grok-v1",
    ]
    .into_iter()
    .map(|k| (k.into(), EmptyObject {}))
    .collect();
    AdapterCatalog { keys: Some(keys) }
}
#[derive(Clone, Default)]
pub struct Registry {
    definitions: BTreeMap<String, EffectiveDefinition>,
    order: Vec<String>,
    summaries: Vec<Summary>,
    diagnostics: Vec<Diagnostic>,
    revision: String,
}
impl Registry {
    pub fn lookup(&self, id: &str) -> Option<EffectiveDefinition> {
        self.definitions.get(id).cloned()
    }
    pub fn list(&self) -> Vec<Summary> {
        self.summaries.clone()
    }
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics.clone()
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn build(layers: Layers, adapters: &AdapterCatalog) -> Self {
        let mut diagnostics = vec![];
        let distribution = layers
            .accepted_distribution
            .filter(|v| !v.is_empty())
            .or(layers.distribution);
        let ordered = [
            ("embedded", layers.embedded),
            ("distribution", distribution),
            ("legacy", layers.legacy),
            ("user", layers.user),
            ("override", layers.overrides),
        ];
        let mut merged: BTreeMap<String, (Value, SourceRef, BTreeMap<String, SourceRef>)> =
            BTreeMap::new();
        for (origin, items) in ordered {
            let mut seen = BTreeSet::new();
            for (index, definition) in items.unwrap_or_default().into_iter().enumerate() {
                if definition.id.is_empty() {
                    diagnostics.push(diag(
                        "missing_id",
                        "error",
                        &format!("{origin}[{index}].id"),
                        "provider id is required",
                    ));
                    continue;
                }
                if !seen.insert(definition.id.clone()) {
                    diagnostics.push(diag(
                        "duplicate_id",
                        "error",
                        &format!("{origin}[{index}].id"),
                        "provider id is duplicated in this layer",
                    ));
                    continue;
                }
                let mut value = serde_json::to_value(&definition).expect("typed provider JSON");
                value["id"] = definition.id.clone().into();
                if let Some((old, source, _)) = merged.get(&definition.id) {
                    if origin == "user" && source.origin.as_str() == "embedded" {
                        diagnostics.push(diag(
                            "builtin_collision",
                            "error",
                            &definition.id,
                            "user definition cannot replace an embedded provider",
                        ));
                        continue;
                    }
                    if origin == "legacy" && source.origin.as_str() == "embedded" {
                        diagnostics.push(diag(
                            "builtin_collision",
                            "warning",
                            &definition.id,
                            "legacy definition is ignored for an embedded provider",
                        ));
                        continue;
                    }
                    if origin == "user" && source.origin.as_str() == "legacy" {
                        diagnostics.push(diag(
                            "legacy_replaced",
                            "warning",
                            &definition.id,
                            "explicit user definition replaces the legacy definition",
                        ));
                    }
                    value = merge(old.clone(), value);
                }
                let mut source = if definition.source.origin.is_empty() {
                    SourceRef::default()
                } else {
                    definition.source.clone()
                };
                source.origin = origin.into();
                let fields = value
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(|k| (k.clone(), source.clone()))
                    .collect();
                merged.insert(definition.id, (value, source, fields));
            }
        }
        let mut registry = Self {
            order: merged.keys().cloned().collect(),
            ..Self::default()
        };
        registry.order.sort_by_key(|id| {
            (
                BUILTIN_PROVIDER_IDS
                    .iter()
                    .position(|v| *v == id)
                    .unwrap_or(BUILTIN_PROVIDER_IDS.len()),
                id.clone(),
            )
        });
        for id in &registry.order {
            let (value, source, fields) = merged.remove(id).unwrap();
            let raw = serde_json::to_vec(&value).expect("JSON value");
            match validate_definition(&raw, adapters) {
                Ok((mut def, mut found)) => {
                    for d in &mut found {
                        d.field = format!("{id}.{}", d.field);
                    }
                    diagnostics.extend(found);
                    if def.enabled.is_none() {
                        def.enabled = Some(true);
                    }
                    let capabilities = derive_capabilities(&def);
                    registry.definitions.insert(
                        id.clone(),
                        EffectiveDefinition {
                            definition: def,
                            effective_source: source,
                            field_origins: fields,
                            capabilities,
                            ..Default::default()
                        },
                    );
                }
                Err(e) => diagnostics.push(diag("invalid_definition", "error", id, &e)),
            }
        }
        registry
            .order
            .retain(|id| registry.definitions.contains_key(id));
        #[derive(Serialize)]
        struct RevisionItem<'a> {
            id: &'a str,
            definition: &'a Definition,
        }
        let items: Vec<_> = registry
            .order
            .iter()
            .map(|id| RevisionItem {
                id,
                definition: &registry.definitions[id].definition,
            })
            .collect();
        let digest = Sha256::digest(to_go_json(&items).expect("typed registry encoding"));
        registry.revision = digest.iter().map(|b| format!("{b:02x}")).collect();
        for id in &registry.order {
            let item = registry.definitions.get_mut(id).unwrap();
            item.revision = registry.revision.clone();
            registry.summaries.push(Summary {
                id: id.clone(),
                display_name: item.definition.display_name.clone(),
                enabled: item.definition.enabled.unwrap_or(true),
                origin: item.effective_source.origin.clone(),
                revision: registry.revision.clone(),
                capabilities: item.capabilities.clone(),
                presentation: sanitize_presentation(item.definition.presentation.as_ref()),
                ..Default::default()
            });
        }
        sort_diagnostics(&mut diagnostics);
        registry.diagnostics = diagnostics;
        registry
    }
}
fn merge(mut base: Value, overlay: Value) -> Value {
    for (key, value) in overlay.as_object().expect("definition object") {
        let slot = &mut base[key];
        if key != "source" && slot.is_object() && value.is_object() {
            *slot = merge(slot.take(), value.clone());
        } else {
            *slot = value.clone();
        }
    }
    base
}
pub(super) fn diag(code: &str, severity: &str, field: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        severity: severity.into(),
        field: field.into(),
        message: message.into(),
    }
}
pub(super) fn sort_diagnostics(d: &mut [Diagnostic]) {
    d.sort_by(|a, b| (&a.field, &a.code, &a.message).cmp(&(&b.field, &b.code, &b.message)));
}
fn available(s: &str) -> bool {
    !matches!(s, "" | "none" | "unsupported")
}
fn derive_capabilities(d: &Definition) -> CapabilitySummary {
    CapabilitySummary {
        launch: d.launch.is_some(),
        models: d.models.is_some(),
        effort: d.launch.as_ref().is_some_and(|l| !l.effort_args.is_empty()),
        headless: d.launch.as_ref().is_some_and(|l| l.headless.is_some()),
        approval: available(&d.adapters.approval),
        transcript: available(&d.adapters.transcript),
        usage: available(&d.adapters.usage),
        subscription: available(&d.adapters.subscription),
        permissions: available(&d.adapters.permissions),
        subagents: available(&d.adapters.subagents),
    }
}

fn expand(args: &[String], key: &str, value: &str) -> Result<Vec<String>, String> {
    args.iter()
        .map(|arg| {
            let v = arg.replace(&format!("{{{key}}}"), value);
            if v.contains(['{', '}']) {
                Err("argv contains an unresolved placeholder".into())
            } else {
                Ok(v)
            }
        })
        .collect()
}
pub fn effort_args(def: &EffectiveDefinition, effort: &str) -> Result<Vec<String>, String> {
    let Some(launch) = &def.definition.launch else {
        return Ok(vec![]);
    };
    if effort.is_empty() {
        return Ok(vec![]);
    }
    if !launch.effort_levels.is_empty() && !launch.effort_levels.iter().any(|s| s == effort) {
        return Err(format!(
            "invalid effort {effort:?} for provider {:?}",
            def.definition.id
        ));
    }
    expand(&launch.effort_args, "effort", effort)
}
pub fn model_args(def: &EffectiveDefinition, model: &str) -> Result<Vec<String>, String> {
    if model.is_empty() {
        return Ok(vec![]);
    }
    def.definition
        .launch
        .as_ref()
        .map(|l| expand(&l.model_args, "model", model))
        .unwrap_or(Ok(vec![]))
}
pub fn resolve_launch(
    def: &EffectiveDefinition,
    request: &LaunchRequest,
) -> Result<ResolvedLaunch, String> {
    let launch = def
        .definition
        .launch
        .as_ref()
        .ok_or_else(|| format!("provider {:?} has no launch definition", def.definition.id))?;
    let candidates: Vec<_> = std::iter::once(&launch.executable)
        .filter(|s| !s.is_empty())
        .chain(launch.executable_candidates.iter())
        .cloned()
        .collect();
    if candidates.is_empty() {
        return Err(format!(
            "provider {:?} has no executable",
            def.definition.id
        ));
    }
    let mut args = launch.args.clone();
    if request.headless
        && let Some(headless) = &launch.headless
    {
        args.extend(headless.args.clone());
        if !request.prompt.is_empty() && headless.prompt_via == "arg" {
            args.push(request.prompt.clone());
        }
    }
    if !request.model.is_empty() {
        args.extend(expand(&launch.model_args, "model", &request.model)?);
    }
    if !request.effort.is_empty() {
        args.extend(expand(&launch.effort_args, "effort", &request.effort)?);
    }
    Ok(ResolvedLaunch {
        executable_candidates: Some(candidates),
        args: if args.is_empty() { None } else { Some(args) },
        allowed_env: if launch.allowed_env.is_empty() {
            None
        } else {
            Some(launch.allowed_env.clone())
        },
        revision: def.revision.clone(),
    })
}
