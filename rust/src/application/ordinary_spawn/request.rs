use super::*;
use crate::proto::wire::{Field, GoWire, Schema};
use serde::Deserialize;

#[derive(Default, Deserialize)]
#[serde(default)]
pub struct OrdinarySpawnRequest {
    pub provider: String,
    pub cwd: String,
    pub model: String,
    pub model_selection_mode: String,
    pub risk_confirmed: bool,
    pub label: String,
    pub permission_mode: String,
    pub sandbox: String,
    pub ask_for_approval: String,
    pub route: String,
    pub utf8_session: bool,
    pub isolate_worktree: Option<bool>,
    pub worktree_cleanup: String,
    pub delegation: Option<bool>,
    pub subscription_profile_id: String,
    pub effort: String,
    pub execution_mode: String,
    pub permission_preset: String,
    pub orchestration: bool,
    pub orchestration_roles: Option<std::collections::BTreeMap<String, Option<RoleAssignment>>>,
    pub initial_prompt: String,
    pub handoff_from: i64,
}
impl GoWire for OrdinarySpawnRequest {
    const GO_TYPE: &'static str = "OrdinarySpawnRequest";
    const SCHEMAS: &'static [Schema] = &[
        Schema {
            name: "OrdinarySpawnRequest",
            fields: &[
                Field {
                    name: "provider",
                    kind: "string",
                },
                Field {
                    name: "cwd",
                    kind: "string",
                },
                Field {
                    name: "model",
                    kind: "string",
                },
                Field {
                    name: "model_selection_mode",
                    kind: "string",
                },
                Field {
                    name: "risk_confirmed",
                    kind: "bool",
                },
                Field {
                    name: "label",
                    kind: "string",
                },
                Field {
                    name: "permission_mode",
                    kind: "string",
                },
                Field {
                    name: "sandbox",
                    kind: "string",
                },
                Field {
                    name: "ask_for_approval",
                    kind: "string",
                },
                Field {
                    name: "route",
                    kind: "string",
                },
                Field {
                    name: "utf8_session",
                    kind: "bool",
                },
                Field {
                    name: "isolate_worktree",
                    kind: "*bool",
                },
                Field {
                    name: "worktree_cleanup",
                    kind: "string",
                },
                Field {
                    name: "delegation",
                    kind: "*bool",
                },
                Field {
                    name: "subscription_profile_id",
                    kind: "string",
                },
                Field {
                    name: "effort",
                    kind: "string",
                },
                Field {
                    name: "execution_mode",
                    kind: "string",
                },
                Field {
                    name: "permission_preset",
                    kind: "string",
                },
                Field {
                    name: "orchestration",
                    kind: "bool",
                },
                Field {
                    name: "orchestration_roles",
                    kind: "map[string]*OrdinaryRoleAssignment",
                },
                Field {
                    name: "initial_prompt",
                    kind: "string",
                },
                Field {
                    name: "handoff_from",
                    kind: "int",
                },
            ],
        },
        Schema {
            name: "OrdinaryRoleAssignment",
            fields: &[
                Field {
                    name: "provider",
                    kind: "string",
                },
                Field {
                    name: "model",
                    kind: "string",
                },
                Field {
                    name: "subscription",
                    kind: "string",
                },
                Field {
                    name: "effort",
                    kind: "string",
                },
                Field {
                    name: "execution_mode",
                    kind: "string",
                },
                Field {
                    name: "permission_preset",
                    kind: "string",
                },
            ],
        },
    ];
}
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct RoleAssignment {
    pub provider: String,
    pub model: String,
    pub subscription: String,
    pub effort: String,
    pub execution_mode: String,
    pub permission_preset: String,
}
impl OrdinarySpawnRequest {
    pub(super) fn validate(&self) -> Result<(), SpawnError> {
        if ![
            "",
            "default",
            "plan",
            "acceptEdits",
            "auto",
            "bypassPermissions",
            "dontAsk",
        ]
        .contains(&self.permission_mode.as_str())
            || !["", "read-only", "workspace-write", "danger-full-access"]
                .contains(&self.sandbox.as_str())
            || !["", "untrusted", "on-request", "never"].contains(&self.ask_for_approval.as_str())
            || !["", "auto", "explicit", "required"].contains(&self.model_selection_mode.as_str())
        {
            return Err(SpawnError::bad("bad request"));
        }
        if ![
            "",
            "anthropic",
            "openai",
            "ollama",
            "lm-studio",
            "nvidia-nim",
        ]
        .contains(&self.route.as_str())
        {
            return Err(SpawnError::bad("invalid route"));
        }
        if self.route == "nvidia-nim" && self.provider != "opencode" {
            return Err(SpawnError::bad("route is not available for this provider"));
        }
        if !valid_worktree_cleanup(&self.worktree_cleanup) {
            return Err(SpawnError::bad("invalid worktree cleanup policy"));
        }
        if self.handoff_from < 0 {
            return Err(SpawnError::bad("invalid handoff_from"));
        }
        if self.model.starts_with('-') {
            return Err(SpawnError::bad("invalid model value"));
        }
        if self.label.starts_with('-') {
            return Err(SpawnError::bad("invalid label value"));
        }
        if !crate::profile::validation::valid_spawn_model_label(&self.model)
            || !crate::profile::validation::valid_spawn_model_label(&self.label)
        {
            return Err(SpawnError::bad("invalid model or label value"));
        }
        Ok(())
    }
    pub(super) fn spec(
        &self,
        cwd: PathBuf,
        attempt: SpawnAttemptId,
        grants: InternalSpawnGrants,
    ) -> WrappedSpawnSpec {
        WrappedSpawnSpec {
            registration_metadata: SpawnRegistrationMetadata::default(),
            spawn_attempt: Some(attempt),
            registration_proof: None,
            provider: self.provider.clone(),
            cwd,
            model: self.model.clone(),
            model_selection: self.model_selection_mode.clone(),
            risk_confirmed: self.risk_confirmed,
            label: self.label.clone(),
            permission_mode: self.permission_mode.clone(),
            sandbox: self.sandbox.clone(),
            ask_for_approval: self.ask_for_approval.clone(),
            route: self.route.clone(),
            utf8_session: self.utf8_session,
            effort: self.effort.clone(),
            execution_mode: self.execution_mode.clone(),
            permission_preset: self.permission_preset.clone(),
            initial_prompt: String::new(),
            subscription_profile_id: self.subscription_profile_id.clone(),
            subscription_login: false,
            usage_probe: false,
            grants,
            cancellation: TaskCancellation::default(),
        }
    }
}
