//! Provider CRUD, validation, revision, backup and recovery routes. The router
//! must apply its full token/Host/Origin guard before dispatching here.
//! Distribution acceptance and icon upload/serving have separate owners.
use super::http::{Request, Response, decode_json};
use crate::{
    config::{Resource, RuntimePaths},
    files::safe_fs::Dir,
    profile::{
        registry::{Registry, default_adapters},
        store::{ProviderRegistryStore, ProviderSnapshot, StoreError, is_builtin},
        validation::validate_definition,
    },
    proto::{provider::*, wire::GoWire},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Default, Deserialize)]
#[serde(default)]
struct ProviderPatchRequest {
    expected_revision: Option<String>,
    definition: Definition,
}
impl GoWire for ProviderPatchRequest {
    const GO_TYPE: &'static str = "ProviderPatchRequest";
    const SCHEMAS: &'static [crate::proto::wire::Schema] =
        crate::profile::provider_schema::PROVIDER_SCHEMAS;
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct HistoryMutationRequest {
    expected_revision: Option<String>,
    revision: String,
}
impl GoWire for HistoryMutationRequest {
    const GO_TYPE: &'static str = "HistoryMutationRequest";
    const SCHEMAS: &'static [crate::proto::wire::Schema] = &[crate::proto::wire::Schema {
        name: "HistoryMutationRequest",
        fields: &[
            crate::proto::wire::Field {
                name: "expected_revision",
                kind: "*string",
            },
            crate::proto::wire::Field {
                name: "revision",
                kind: "string",
            },
        ],
    }];
}
#[derive(Serialize)]
struct ListResponse {
    revision: String,
    providers: Vec<Summary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<Diagnostic>,
}
#[derive(Serialize)]
struct DetailResponse {
    revision: String,
    provider: EffectiveDefinition,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<Diagnostic>,
}
fn method() -> Response {
    Response::error(405, "method_not_allowed", "method not allowed")
}
fn unavailable() -> Response {
    Response::error(
        503,
        "provider_registry_unavailable",
        "provider registry is unavailable",
    )
}
struct StoreWarning {
    operation: &'static str,
    provider_id: String,
    error: StoreError,
}
fn failure(
    status: u16,
    code: &'static str,
    error: StoreError,
    id: &str,
    warnings: &mut Vec<StoreWarning>,
) -> Response {
    let detail = if let StoreError::Invalid(message) = &error {
        message.clone()
    } else {
        warnings.push(StoreWarning {
            operation: code,
            provider_id: id.into(),
            error,
        });
        match code {
            "provider_save_failed" => "could not save the provider",
            "provider_override_failed" => "could not save the provider override",
            "provider_backup_failed" => "could not back up the provider before changing it",
            "provider_reload_failed" => "could not reload the providers after the change",
            "provider_delete_failed" => "could not remove the provider",
            "provider_reset_failed" => "could not reset the provider",
            "provider_restore_failed" => "could not restore the provider revision",
            "history_not_found" => "could not read the provider history",
            "backups_not_found" => "could not read the provider backups",
            "backup_verify_failed" => "could not verify the provider backup",
            "backup_restore_failed" => "could not restore the provider backup",
            "invalid_provider_id" => "could not check whether the provider needs recovery",
            "provider_recovery_failed" => "could not recover the provider",
            _ => "the provider operation failed",
        }
        .into()
    };
    Response::error(status, code, &detail)
}
fn history_failure(error: StoreError, id: &str, warnings: &mut Vec<StoreWarning>) -> Response {
    match error {
        e @ StoreError::RevisionConflict { .. } => {
            Response::error(409, "revision_conflict", &e.to_string())
        }
        StoreError::OverrideClearsValue(fields) => Response::json(
            422,
            &json!({"ok":false,"error":"provider_override_clears_value","detail":format!("provider override cannot clear a distributed value: {}", fields.join(", ")),"fields":fields}),
        ),
        other => failure(422, "provider_override_failed", other, id, warnings),
    }
}
fn nullable_diagnostics(diagnostics: Vec<Diagnostic>) -> Option<Vec<Diagnostic>> {
    if diagnostics.is_empty() {
        None
    } else {
        Some(diagnostics)
    }
}
fn changed(
    store: &ProviderRegistryStore,
    status: u16,
    state: Option<(&str, bool)>,
    id: &str,
    warnings: &mut Vec<StoreWarning>,
) -> Response {
    match store.reload() {
        Err(e) => failure(500, "provider_reload_failed", e, id, warnings),
        Ok(snapshot) => {
            let mut value = json!({"ok":true,"revision":snapshot.registry.revision(),"diagnostics":nullable_diagnostics(snapshot.diagnostics)});
            if let Some((key, value_)) = state {
                value[key] = value_.into();
            }
            Response::json(status, &value)
        }
    }
}
fn diagnostics(registry: &Registry, command_found: &dyn Fn(&str) -> bool) -> Vec<Diagnostic> {
    let mut diagnostics = registry.diagnostics();
    for summary in registry.list() {
        let Some(def) = registry.lookup(&summary.id) else {
            continue;
        };
        let Some(launch) = def.definition.launch else {
            continue;
        };
        let candidates: Vec<_> = std::iter::once(&launch.executable)
            .filter(|s| !s.is_empty())
            .chain(launch.executable_candidates.iter())
            .collect();
        if !candidates.is_empty()
            && !candidates
                .iter()
                .any(|candidate| !candidate.is_empty() && command_found(candidate))
        {
            diagnostics.push(Diagnostic {
                code: "command_missing".into(),
                severity: SEVERITY_WARNING.into(),
                field: summary.id.clone(),
                message: format!("executable for {:?} was not found on PATH", summary.id),
            });
        }
    }
    diagnostics
}
fn icon_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
}
fn icon_version(paths: &RuntimePaths, id: &str) -> String {
    crate::application::provider_assets::icon_version(paths, id)
}
fn remove_icon(paths: &RuntimePaths, id: &str, warnings: &mut Vec<StoreWarning>) {
    if !icon_id(id) {
        return;
    }
    let Ok(_icon_gate) = crate::application::provider_assets::ICON_GATE.lock() else {
        return;
    };
    if let Err(error) = Dir::open(&paths.resource(Resource::ProviderIcons))
        .and_then(|dir| dir.remove_file(&format!("{id}.bin")))
        && error.kind() != std::io::ErrorKind::NotFound
    {
        warnings.push(StoreWarning {
            operation: "provider_icon_remove_failed",
            provider_id: id.into(),
            error: StoreError::Io(error),
        });
    }
}
fn query_has(request: &Request, name: &str) -> bool {
    request
        .query
        .split('&')
        .filter(|p| !p.contains(';'))
        .any(|p| {
            let bytes = p.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' {
                    if i + 2 >= bytes.len()
                        || !bytes[i + 1].is_ascii_hexdigit()
                        || !bytes[i + 2].is_ascii_hexdigit()
                    {
                        return false;
                    }
                    i += 3;
                } else {
                    i += 1;
                }
            }
            url::form_urlencoded::parse(p.as_bytes()).any(|(key, _)| key == name)
        })
}

/// Required warning sink is called only after dispatch returns and all provider
/// mutation/store guards have been released. Production supplies its private
/// logger; no silent logging default is installed by this module.
pub fn handle(
    request: &Request,
    store: &ProviderRegistryStore,
    paths: &RuntimePaths,
    command_found: &dyn Fn(&str) -> bool,
    warning: &dyn Fn(&'static str, &str, &StoreError),
) -> Option<Response> {
    let mut warnings = vec![];
    let response = dispatch(request, store, paths, command_found, &mut warnings);
    for event in warnings {
        warning(event.operation, &event.provider_id, &event.error);
    }
    response
}
fn dispatch(
    request: &Request,
    store: &ProviderRegistryStore,
    paths: &RuntimePaths,
    command_found: &dyn Fn(&str) -> bool,
    warnings: &mut Vec<StoreWarning>,
) -> Option<Response> {
    if request.path == "/api/providers" {
        return Some(match request.method.as_str() {
            "GET" => match store.snapshot() {
                Ok(snapshot) => {
                    let mut providers = snapshot.registry.list();
                    for p in &mut providers {
                        p.icon_image_version = icon_version(paths, &p.id);
                    }
                    Response::json(
                        200,
                        &ListResponse {
                            revision: snapshot.registry.revision().into(),
                            providers,
                            diagnostics: diagnostics(&snapshot.registry, command_found),
                        },
                    )
                    .no_store()
                }
                Err(_) => unavailable(),
            },
            "POST" => create(request, store, warnings),
            _ => method(),
        });
    }
    let path = request.path.strip_prefix("/api/providers/")?;
    if path == "validate" {
        return Some(validate(request));
    }
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() == 4 && parts[1] == "history" && parts[3] == "diff" {
        return Some(if request.method != "GET" {
            method()
        } else {
            let records = store
                .history
                .get_revision(parts[0], parts[2])
                .and_then(|selected| {
                    store
                        .history
                        .current(parts[0])
                        .map(|current| (selected, current))
                });
            match records {
                Ok((selected, current)) => Response::json(
                    200,
                    &json!({
                        "provider_id":parts[0], "revision":parts[2], "current_revision":current.revision,
                        "diff":crate::profile::store::diff_distribution(&[current.payload], &[selected.payload], &[]),
                    }),
                ),
                Err(error) => failure(404, "history_not_found", error, parts[0], warnings),
            }
        });
    }
    if parts.len() == 2 && parts[1] == "backups" {
        return Some(backups(request, store, parts[0], None, warnings));
    }
    if parts.len() == 4 && parts[1] == "backups" && matches!(parts[3], "verify" | "restore") {
        return Some(backups(
            request,
            store,
            parts[0],
            Some((parts[2], parts[3])),
            warnings,
        ));
    }
    if parts.len() == 2 && parts[1] == "recovery" {
        return Some(recovery(request, store, parts[0], warnings));
    }
    for operation in ["reset", "restore"] {
        if let Some(id) = path
            .strip_suffix(&format!("/{operation}"))
            .filter(|id| !id.contains('/') && !id.is_empty())
        {
            return Some(history_mutation(
                request, store, paths, id, operation, warnings,
            ));
        }
    }
    if let Some(id) = path.strip_suffix("/history").filter(|id| !id.contains('/')) {
        return Some(if request.method != "GET" {
            method()
        } else {
            match store.history.list(id) {
                Ok(records) => Response::json(200, &json!({"provider_id":id,"revisions":records})),
                Err(e) => failure(404, "history_not_found", e, id, warnings),
            }
        });
    }
    if path.is_empty() || path.contains('/') {
        return None;
    }
    Some(match request.method.as_str() {
        "GET" => match store.snapshot() {
            Ok(snapshot) => match snapshot.registry.lookup(path) {
                None => Response::error(404, "provider_not_found", "provider was not found"),
                Some(provider) => Response::json(
                    200,
                    &DetailResponse {
                        revision: snapshot.registry.revision().into(),
                        provider,
                        diagnostics: diagnostics(&snapshot.registry, command_found)
                            .into_iter()
                            .filter(|d| d.field == path || d.field.starts_with(&format!("{path}.")))
                            .collect(),
                    },
                )
                .no_store(),
            },
            Err(_) => unavailable(),
        },
        "PATCH" => patch(request, store, path, warnings),
        "DELETE" => delete(request, store, paths, path, warnings),
        _ => method(),
    })
}
fn create(
    request: &Request,
    store: &ProviderRegistryStore,
    warnings: &mut Vec<StoreWarning>,
) -> Response {
    let definition: Definition = match decode_json(request) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if is_builtin(&definition.id) || definition.id == "shell" {
        return Response::error(409, "provider_id_reserved", "provider id is reserved");
    }
    let Ok(_guard) = store.mutation_gate.lock() else {
        return unavailable();
    };
    match store.definitions.create_new(&definition) {
        Err(StoreError::AlreadyExists) => {
            return Response::error(409, "provider_id_exists", "provider id already exists");
        }
        Err(e) => return failure(422, "provider_save_failed", e, &definition.id, warnings),
        Ok(()) => {}
    }
    if let Err(e) = store
        .history
        .backup_snapshot(&definition.id, &definition, "create")
    {
        let _ = store.definitions.delete(&definition.id);
        return failure(500, "provider_backup_failed", e, &definition.id, warnings);
    }
    changed(store, 201, None, &definition.id, warnings)
}
fn history_mutation(
    request: &Request,
    store: &ProviderRegistryStore,
    paths: &RuntimePaths,
    id: &str,
    operation: &str,
    warnings: &mut Vec<StoreWarning>,
) -> Response {
    if request.method != "POST" {
        return method();
    }
    if operation == "reset" && !is_builtin(id) {
        return Response::error(
            400,
            "provider_reset_not_builtin",
            "only built-in providers can be reset",
        );
    }
    let body: HistoryMutationRequest = match decode_json(request) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let expected = match body.expected_revision {
        Some(s) => s,
        None if operation == "reset" => {
            return Response::error(
                400,
                "expected_revision_required",
                "expected_revision is required",
            );
        }
        None => String::new(),
    };
    if operation == "restore" && (body.revision.trim().is_empty() || expected.trim().is_empty()) {
        return Response::error(
            400,
            "revision_required",
            "revision and expected_revision are required",
        );
    }
    let Ok(_guard) = store.mutation_gate.lock() else {
        return unavailable();
    };
    let result = if operation == "reset" {
        store.history.reset(id, &expected)
    } else {
        store.history.restore(id, &body.revision, &expected)
    };
    if let Err(error) = result {
        return match error {
            e @ StoreError::RevisionConflict { .. } => {
                Response::error(409, "revision_conflict", &e.to_string())
            }
            StoreError::OverrideClearsValue(fields) => Response::json(
                422,
                &json!({"ok":false,"error":"provider_override_clears_value","detail":format!("provider override cannot clear a distributed value: {}",fields.join(", ")),"fields":fields}),
            ),
            e => failure(
                422,
                if operation == "reset" {
                    "provider_reset_failed"
                } else {
                    "provider_restore_failed"
                },
                e,
                id,
                warnings,
            ),
        };
    }
    if operation == "reset" {
        remove_icon(paths, id, warnings);
    }
    changed(store, 200, None, id, warnings)
}
fn backups(
    request: &Request,
    store: &ProviderRegistryStore,
    id: &str,
    action: Option<(&str, &str)>,
    warnings: &mut Vec<StoreWarning>,
) -> Response {
    let method_ = if action.is_some_and(|(_, action)| action == "restore") {
        "POST"
    } else {
        "GET"
    };
    if request.method != method_ {
        return method();
    }
    match action {
        None => match store.history.list_backups_nullable(id) {
            Ok(backups) => Response::json(200, &json!({"provider_id":id,"backups":backups})),
            Err(error) => failure(404, "backups_not_found", error, id, warnings),
        },
        Some((backup, "verify")) => match store.history.verify_backup(id, backup) {
            Ok(backup) => Response::json(200, &json!({"ok":true,"backup":backup})),
            Err(error) => failure(422, "backup_verify_failed", error, id, warnings),
        },
        Some((backup, _)) => {
            let body: HistoryMutationRequest = match decode_json(request) {
                Ok(body) => body,
                Err(error) => return error,
            };
            let Some(expected) = body.expected_revision else {
                return Response::error(
                    400,
                    "expected_revision_required",
                    "expected_revision is required",
                );
            };
            let Ok(_guard) = store.mutation_gate.lock() else {
                return unavailable();
            };
            if let Err(error) = store.history.restore_backup(id, backup, &expected) {
                return match error {
                    e @ StoreError::RevisionConflict { .. } => {
                        Response::error(409, "revision_conflict", &e.to_string())
                    }
                    StoreError::OverrideClearsValue(fields) => Response::json(
                        422,
                        &json!({"ok":false,"error":"provider_override_clears_value","detail":format!("provider override cannot clear a distributed value: {}",fields.join(", ")),"fields":fields}),
                    ),
                    error => failure(422, "backup_restore_failed", error, id, warnings),
                };
            }
            changed(store, 200, None, id, warnings)
        }
    }
}
fn recovery(
    request: &Request,
    store: &ProviderRegistryStore,
    id: &str,
    warnings: &mut Vec<StoreWarning>,
) -> Response {
    if !matches!(request.method.as_str(), "GET" | "POST") {
        return method();
    }
    if request.method == "GET" {
        let needed = match store.history.needs_recovery(id) {
            Ok(needed) => needed,
            Err(error) => return failure(400, "invalid_provider_id", error, id, warnings),
        };
        let mut candidates = vec![];
        if needed {
            if let Ok(Some(revision)) = store.history.last_verified_revision(id) {
                candidates.push(json!({"kind":"user_revision","revision":revision.revision,"created_at":revision.created_at}));
            }
            candidates.push(json!({"kind":"base"}));
        }
        return Response::json(200,&json!({"provider_id":id,"state":if needed{"recovery_required"}else{"ok"},"candidates":candidates})).no_store();
    }
    let body: HistoryMutationRequest = match decode_json(request) {
        Ok(body) => body,
        Err(error) => return error,
    };
    let Ok(_guard) = store.mutation_gate.lock() else {
        return unavailable();
    };
    let recovered = match store.history.recover_head(id, &body.revision) {
        Ok(recovered) => recovered,
        Err(error) => return failure(422, "provider_recovery_failed", error, id, warnings),
    };
    if let Err(error) = store.reload() {
        return failure(500, "provider_reload_failed", error, id, warnings);
    }
    Response::json(200, &json!({"ok":true,"revision":recovered.revision}))
}
fn validate(request: &Request) -> Response {
    if request.method != "POST" {
        return method();
    }
    // RawMessage preserves original members for unknown-field diagnostics. Take
    // exactly the first value just as decodeJSON does, retaining original bytes.
    let mut stream = serde_json::Deserializer::from_slice(
        &request.body[..request.body.len().min(super::http::JSON_BODY_LIMIT)],
    )
    .into_iter::<Box<serde_json::value::RawValue>>();
    let raw = match stream.next() {
        Some(Ok(raw)) => raw,
        None | Some(Err(_)) => return Response::error(400, "bad_request", "invalid json"),
    };
    match validate_definition(raw.get().as_bytes(), &default_adapters()) {
        Ok((_, diagnostics)) => Response::json(
            200,
            &json!({"valid":!diagnostics.iter().any(Diagnostic::is_error),"diagnostics":nullable_diagnostics(diagnostics)}),
        ),
        Err(e) => Response::error(400, "invalid_provider_definition", &e),
    }
}
fn patch(
    request: &Request,
    store: &ProviderRegistryStore,
    id: &str,
    warnings: &mut Vec<StoreWarning>,
) -> Response {
    let mut body: ProviderPatchRequest = match decode_json(request) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(expected) = body.expected_revision else {
        return Response::error(
            400,
            "expected_revision_required",
            "expected_revision is required",
        );
    };
    if body.definition.id.is_empty() {
        body.definition.id = id.into();
    }
    if body.definition.id != id {
        return Response::error(400, "provider_id_mismatch", "provider id cannot change");
    }
    let Ok(_guard) = store.mutation_gate.lock() else {
        return unavailable();
    };
    if is_builtin(id) {
        let baseline = match store.baseline(id) {
            Ok(b) => b,
            Err(e) => return failure(422, "provider_override_failed", e, id, warnings),
        };
        if let Err(e) =
            store
                .history
                .save_effective_override(id, body.definition, &baseline, &expected, "edit")
        {
            return history_failure(e, id, warnings);
        }
    } else {
        let snapshot = match expected_snapshot(store, &expected) {
            Ok(s) => s,
            Err(e) => return e,
        };
        if let Some(current) = snapshot.registry.lookup(id)
            && let Err(e) = store
                .history
                .backup_snapshot(id, &current.definition, "before-edit")
        {
            return failure(500, "provider_backup_failed", e, id, warnings);
        }
        if let Err(e) = store.definitions.save(&body.definition) {
            return failure(422, "provider_save_failed", e, id, warnings);
        }
    }
    changed(store, 200, None, id, warnings)
}
fn expected_snapshot(
    store: &ProviderRegistryStore,
    expected: &str,
) -> std::result::Result<ProviderSnapshot, Response> {
    match store.snapshot() {
        Ok(snapshot) if snapshot.registry.revision() == expected => Ok(snapshot),
        _ => Err(Response::error(
            409,
            "revision_conflict",
            "provider registry changed; reload and try again",
        )),
    }
}
fn delete(
    request: &Request,
    store: &ProviderRegistryStore,
    paths: &RuntimePaths,
    id: &str,
    warnings: &mut Vec<StoreWarning>,
) -> Response {
    if !query_has(request, "expected_revision") {
        return Response::error(
            400,
            "expected_revision_required",
            "expected_revision is required",
        );
    }
    let expected = request.query("expected_revision");
    let Ok(_guard) = store.mutation_gate.lock() else {
        return unavailable();
    };
    if is_builtin(id) {
        let snapshot = match store.snapshot() {
            Ok(s) => s,
            Err(_) => return unavailable(),
        };
        if snapshot.registry.lookup(id).is_none() {
            return Response::error(404, "provider_not_found", "provider was not found");
        }
        let baseline = match store.baseline(id) {
            Ok(b) => b,
            Err(e) => return failure(422, "provider_override_failed", e, id, warnings),
        };
        if let Err(e) = store.history.save_override(
            id,
            Definition {
                id: id.into(),
                enabled: Some(false),
                ..Default::default()
            },
            &baseline,
            &expected,
            "delete",
        ) {
            return history_failure(e, id, warnings);
        }
        changed(store, 200, Some(("disabled", true)), id, warnings)
    } else {
        let snapshot = match expected_snapshot(store, &expected) {
            Ok(s) => s,
            Err(e) => return e,
        };
        if let Some(current) = snapshot.registry.lookup(id)
            && let Err(e) = store
                .history
                .backup_snapshot(id, &current.definition, "before-delete")
        {
            return failure(500, "provider_backup_failed", e, id, warnings);
        }
        if let Err(e) = store.definitions.delete(id) {
            return failure(404, "provider_delete_failed", e, id, warnings);
        }
        remove_icon(paths, id, warnings);
        changed(store, 200, Some(("removed", true)), id, warnings)
    }
}
#[cfg(test)]
#[path = "provider_routes/tests.rs"]
mod tests;
