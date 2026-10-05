//! Source grid launches retain each native Start independently of HTTP lifetime.
use super::{ordinary_spawn::SpawnError, spawn_policy::RegistrySnapshot};
use crate::{
    hub::task_owner::HubTaskHandle,
    orchestration::child_launch::cwd_too_broad,
    proto::{
        core::*,
        wire::{Field, GoWire, Schema},
    },
    terminal::session::SessionEngine,
};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct GridRequest {
    pub preset: String,
    pub layout: String,
    pub count: i64,
    pub cwd: String,
    pub label_prefix: String,
    pub provider: String,
}
impl GoWire for GridRequest {
    const GO_TYPE: &'static str = "GridRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "GridRequest",
        fields: &[
            Field {
                name: "preset",
                kind: "string",
            },
            Field {
                name: "layout",
                kind: "string",
            },
            Field {
                name: "count",
                kind: "int",
            },
            Field {
                name: "cwd",
                kind: "string",
            },
            Field {
                name: "label_prefix",
                kind: "string",
            },
            Field {
                name: "provider",
                kind: "string",
            },
        ],
    }];
}
pub struct Dependencies {
    pub core: Arc<SessionEngine>,
    pub registry: RegistrySnapshot,
    pub hub_cwd: PathBuf,
    pub home: PathBuf,
    pub environment: Vec<String>,
    pub tasks: HubTaskHandle,
}
pub struct GridSpawn {
    deps: Dependencies,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridResult {
    pub layout: String,
    pub count: usize,
}
struct Plan {
    layout: String,
    cwd: PathBuf,
    specs: Vec<(String, String)>,
}
fn error(status: u16, code: &'static str, detail: &'static str) -> SpawnError {
    SpawnError {
        status,
        code,
        detail,
    }
}
fn bad(detail: &'static str) -> SpawnError {
    error(400, "bad_request", detail)
}
fn layout(count: i64) -> &'static str {
    match count {
        0..=1 => "1x1",
        2 => "1x2",
        3..=4 => "2x2",
        5..=6 => "2x3",
        7..=9 => "3x3",
        10..=12 => "4x3",
        _ => "6x3",
    }
}
impl GridRequest {
    fn plan(
        &self,
        hub_cwd: &std::path::Path,
        home: &std::path::Path,
        provider_valid: impl FnOnce(&str) -> Result<bool, SpawnError>,
    ) -> Result<Plan, SpawnError> {
        if !matches!(self.preset.as_str(), "shell" | "ai+shell") {
            return Err(bad("invalid preset"));
        }
        if !matches!(
            self.layout.as_str(),
            "" | "1x1" | "1x2" | "2x2" | "2x3" | "3x3" | "4x3" | "6x3"
        ) {
            return Err(bad("invalid layout"));
        }
        if !(1..=18).contains(&self.count) {
            return Err(bad("count must be 1-18"));
        }
        let cwd = if self.cwd.is_empty() {
            hub_cwd.to_owned()
        } else {
            let raw = PathBuf::from(&self.cwd);
            // os.Stat resolves relative request paths against the Hub actor cwd.
            let path = if raw.is_absolute() {
                raw
            } else {
                hub_cwd.join(raw)
            };
            if !path.is_dir() {
                return Err(bad("cwd does not exist or is not a directory"));
            }
            path
        };
        let layout = if self.layout.is_empty() {
            layout(self.count).to_owned()
        } else {
            self.layout.clone()
        };
        if self.label_prefix.starts_with('-') {
            return Err(bad("invalid label_prefix"));
        }
        if self.provider.starts_with('-') {
            return Err(bad("invalid provider"));
        }
        if !crate::profile::validation::valid_spawn_model_label(&self.label_prefix) {
            return Err(bad("invalid label_prefix"));
        }
        if cwd_too_broad(&cwd, Some(home)) {
            return Err(bad("cwd is too broad (system root or home root)"));
        }
        let provider = if self.provider.is_empty() {
            "claude"
        } else {
            &self.provider
        };
        if self.preset == "ai+shell" && (provider == "shell" || !provider_valid(provider)?) {
            return Err(bad("invalid ai provider for ai+shell preset"));
        }
        let prefix = if self.label_prefix.is_empty() {
            "grid"
        } else {
            &self.label_prefix
        };
        let specs = (0..self.count)
            .map(|i| {
                if self.preset == "shell" {
                    ("shell".into(), format!("{prefix}-{}", i + 1))
                } else if i == 0 {
                    (provider.into(), format!("{prefix}-{provider}-1"))
                } else {
                    ("shell".into(), format!("{prefix}-shell-{i}"))
                }
            })
            .collect();
        Ok(Plan { layout, cwd, specs })
    }
}
impl GridSpawn {
    pub fn new(deps: Dependencies) -> Self {
        Self { deps }
    }
    pub async fn spawn(
        self: &Arc<Self>,
        body: GridRequest,
        verified: &VerifiedConfirmationRequest,
    ) -> Result<GridResult, SpawnError> {
        if verified.auth_epoch() != self.deps.core.auth_epoch() {
            return Err(error(401, "unauthorized", "authentication expired"));
        }
        let plan = body.plan(&self.deps.hub_cwd, &self.deps.home, |provider| {
            let registry = (self.deps.registry)()
                .map_err(|_| error(503, "unavailable", "provider registry unavailable"))?;
            Ok(registry
                .lookup(provider)
                .is_some_and(|d| d.definition.enabled.unwrap_or(true)))
        })?;
        if body.preset == "ai+shell" && self.deps.core.provider_updating(&plan.specs[0].0) {
            return Err(error(
                409,
                "provider_updating",
                "provider is currently updating",
            ));
        }
        let service = self.clone();
        let permit = self
            .deps
            .tasks
            .request_permit()
            .map_err(|_| error(503, "unavailable", "Hub is stopping"))?;
        permit
            .start(async move { service.start(plan).await })
            .wait()
            .await
            .unwrap_or_else(|_| Err(error(500, "spawn_error", "spawn workflow failed")))
    }
    async fn start(&self, plan: Plan) -> Result<GridResult, SpawnError> {
        let count = plan.specs.len();
        // Request disconnection releases only its observer. Native Start remains
        // retained by the common process owner; earlier starts are never rolled back.
        let waiter = HttpWaitCancellation::default();
        for (provider, label) in plan.specs {
            let request = WrappedStartRequest {
                kind: WrappedStartKind::Grid,
                policy: ResolvedSpawnPolicy {
                    base_environment: self.deps.environment.clone(),
                    route_environment: vec![],
                    subscription_environment: vec![],
                    effective_route: String::new(),
                    current_model: String::new(),
                    resolved_model: String::new(),
                    effort_args: vec![],
                    ordinary_model_args: vec![],
                },
                spec: WrappedSpawnSpec {
                    provider,
                    label,
                    cwd: plan.cwd.clone(),
                    registration_metadata: SpawnRegistrationMetadata::default(),
                    spawn_attempt: None,
                    registration_proof: None,
                    model: String::new(),
                    model_selection: String::new(),
                    risk_confirmed: false,
                    permission_mode: String::new(),
                    sandbox: String::new(),
                    ask_for_approval: String::new(),
                    route: String::new(),
                    utf8_session: false,
                    effort: String::new(),
                    execution_mode: String::new(),
                    permission_preset: String::new(),
                    initial_prompt: String::new(),
                    subscription_profile_id: String::new(),
                    subscription_login: false,
                    usage_probe: false,
                    grants: InternalSpawnGrants::default(),
                    cancellation: TaskCancellation::default(),
                },
            };
            match self.deps.core.start_wrapped(request, &waiter).await {
                SpawnStartOutcome::Started { .. } => {}
                _ => return Err(error(500, "spawn_error", "spawn error")),
            }
        }
        Ok(GridResult {
            layout: plan.layout,
            count,
        })
    }
}
#[cfg(test)]
mod tests;
