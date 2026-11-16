//! Additional explicit schemas for provider request/persistence decoding.
use crate::proto::{
    provider::*,
    wire::{Field, GoWire, Schema},
};
pub const PROVIDER_SCHEMAS: &[Schema] = &[
    Schema {
        name: "Definition",
        fields: &[
            Field {
                name: "schema_version",
                kind: "int",
            },
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "display_name",
                kind: "string",
            },
            Field {
                name: "description",
                kind: "string",
            },
            Field {
                name: "enabled",
                kind: "*bool",
            },
            Field {
                name: "launch",
                kind: "*LaunchDefinition",
            },
            Field {
                name: "models",
                kind: "*ModelsDefinition",
            },
            Field {
                name: "capabilities",
                kind: "map[string]bool",
            },
            Field {
                name: "adapters",
                kind: "AdapterRefs",
            },
            Field {
                name: "presentation",
                kind: "*PresentationDefinition",
            },
            Field {
                name: "approval_pattern_source",
                kind: "string",
            },
            Field {
                name: "source",
                kind: "SourceRef",
            },
            Field {
                name: "update",
                kind: "*UpdateDefinition",
            },
            Field {
                name: "connection",
                kind: "string",
            },
            Field {
                name: "slash_commands",
                kind: "*SlashCommandsDefinition",
            },
            Field {
                name: "permissions",
                kind: "*PermissionsDefinition",
            },
            Field {
                name: "login",
                kind: "*LoginDefinition",
            },
            Field {
                name: "links",
                kind: "*LinksDefinition",
            },
            Field {
                name: "input",
                kind: "*InputDefinition",
            },
            Field {
                name: "routines",
                kind: "*RoutinesDefinition",
            },
            Field {
                name: "screen",
                kind: "*ScreenDefinition",
            },
        ],
    },
    Schema {
        name: "SlashCommandsDefinition",
        fields: &[
            Field {
                name: "source",
                kind: "string",
            },
            Field {
                name: "items",
                kind: "[]SlashCommandItem",
            },
            Field {
                name: "skill_prefix",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "SlashCommandItem",
        fields: &[
            Field {
                name: "cmd",
                kind: "string",
            },
            Field {
                name: "desc",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "PermissionsDefinition",
        fields: &[
            Field {
                name: "modes",
                kind: "map[string][]string",
            },
            Field {
                name: "allowed_tools_args",
                kind: "[]string",
            },
        ],
    },
    Schema {
        name: "LoginDefinition",
        fields: &[Field {
            name: "args",
            kind: "[]string",
        }],
    },
    Schema {
        name: "LinksDefinition",
        fields: &[Field {
            name: "usage_url",
            kind: "string",
        }],
    },
    Schema {
        name: "InputDefinition",
        fields: &[
            Field {
                name: "slow_paste",
                kind: "bool",
            },
            Field {
                name: "interrupt_key",
                kind: "string",
            },
            Field {
                name: "attach_format",
                kind: "string",
            },
            Field {
                name: "slash_submit",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "RoutinesDefinition",
        fields: &[
            Field {
                name: "enabled",
                kind: "*bool",
            },
            Field {
                name: "completion_mode",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "ScreenDefinition",
        fields: &[Field {
            name: "ready_markers",
            kind: "[]string",
        }],
    },
    Schema {
        name: "ModelListDefinition",
        fields: &[
            Field {
                name: "args",
                kind: "[]string",
            },
            Field {
                name: "timeout_seconds",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "LaunchDefinition",
        fields: &[
            Field {
                name: "executable",
                kind: "string",
            },
            Field {
                name: "executable_candidates",
                kind: "[]string",
            },
            Field {
                name: "args",
                kind: "[]string",
            },
            Field {
                name: "model_args",
                kind: "[]string",
            },
            Field {
                name: "effort_args",
                kind: "[]string",
            },
            Field {
                name: "effort_levels",
                kind: "[]string",
            },
            Field {
                name: "allowed_env",
                kind: "[]string",
            },
            Field {
                name: "headless",
                kind: "*HeadlessDefinition",
            },
            Field {
                name: "config_dir_env",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "ModelsDefinition",
        fields: &[
            Field {
                name: "allow_custom",
                kind: "bool",
            },
            Field {
                name: "items",
                kind: "[]ModelDefinition",
            },
            Field {
                name: "source",
                kind: "string",
            },
            Field {
                name: "list",
                kind: "*ModelListDefinition",
            },
            Field {
                name: "context_limit",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "ModelDefinition",
        fields: &[
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "label",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "HeadlessDefinition",
        fields: &[
            Field {
                name: "args",
                kind: "[]string",
            },
            Field {
                name: "format",
                kind: "string",
            },
            Field {
                name: "prompt_via",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "PresentationDefinition",
        fields: &[
            Field {
                name: "icon_text",
                kind: "string",
            },
            Field {
                name: "color",
                kind: "string",
            },
            Field {
                name: "letter",
                kind: "string",
            },
            Field {
                name: "order",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "UpdateDefinition",
        fields: &[
            Field {
                name: "version_args",
                kind: "[]string",
            },
            Field {
                name: "args",
                kind: "[]string",
            },
            Field {
                name: "executable",
                kind: "string",
            },
            Field {
                name: "enabled",
                kind: "*bool",
            },
            Field {
                name: "login_may_be_required",
                kind: "bool",
            },
            Field {
                name: "timeout_seconds",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "AdapterRefs",
        fields: &[
            Field {
                name: "launch",
                kind: "string",
            },
            Field {
                name: "approval",
                kind: "string",
            },
            Field {
                name: "transcript",
                kind: "string",
            },
            Field {
                name: "usage",
                kind: "string",
            },
            Field {
                name: "subscription",
                kind: "string",
            },
            Field {
                name: "permissions",
                kind: "string",
            },
            Field {
                name: "subagents",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "SourceRef",
        fields: &[
            Field {
                name: "origin",
                kind: "string",
            },
            Field {
                name: "version",
                kind: "string",
            },
            Field {
                name: "digest",
                kind: "string",
            },
            Field {
                name: "revision",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "RevisionRecord",
        fields: &[
            Field {
                name: "schema_version",
                kind: "int",
            },
            Field {
                name: "provider_id",
                kind: "string",
            },
            Field {
                name: "revision",
                kind: "string",
            },
            Field {
                name: "created_at",
                kind: "string",
            },
            Field {
                name: "reason",
                kind: "string",
            },
            Field {
                name: "parent_revision",
                kind: "string",
            },
            Field {
                name: "content_digest",
                kind: "string",
            },
            Field {
                name: "payload",
                kind: "Definition",
            },
        ],
    },
    Schema {
        name: "DistributionPayload",
        fields: &[
            Field {
                name: "schema_version",
                kind: "int",
            },
            Field {
                name: "catalog_version",
                kind: "string",
            },
            Field {
                name: "created_at",
                kind: "string",
            },
            Field {
                name: "minimum_app_version",
                kind: "string",
            },
            Field {
                name: "definitions",
                kind: "[]Definition",
            },
            Field {
                name: "digests",
                kind: "map[string]string",
            },
        ],
    },
    Schema {
        name: "DistributionBundle",
        fields: &[
            Field {
                name: "payload",
                kind: "DistributionPayload",
            },
            Field {
                name: "key_id",
                kind: "string",
            },
            Field {
                name: "signature",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "DistributionStatus",
        fields: &[
            Field {
                name: "catalog_version",
                kind: "string",
            },
            Field {
                name: "digest",
                kind: "string",
            },
            Field {
                name: "state",
                kind: "string",
            },
            Field {
                name: "path",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "ProviderPatchRequest",
        fields: &[
            Field {
                name: "expected_revision",
                kind: "*string",
            },
            Field {
                name: "definition",
                kind: "Definition",
            },
        ],
    },
];
impl GoWire for Definition {
    const GO_TYPE: &'static str = "Definition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for LaunchDefinition {
    const GO_TYPE: &'static str = "LaunchDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for ModelsDefinition {
    const GO_TYPE: &'static str = "ModelsDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for ModelDefinition {
    const GO_TYPE: &'static str = "ModelDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for HeadlessDefinition {
    const GO_TYPE: &'static str = "HeadlessDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for PresentationDefinition {
    const GO_TYPE: &'static str = "PresentationDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for UpdateDefinition {
    const GO_TYPE: &'static str = "UpdateDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for SlashCommandsDefinition {
    const GO_TYPE: &'static str = "SlashCommandsDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for SlashCommandItem {
    const GO_TYPE: &'static str = "SlashCommandItem";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for PermissionsDefinition {
    const GO_TYPE: &'static str = "PermissionsDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for LoginDefinition {
    const GO_TYPE: &'static str = "LoginDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for LinksDefinition {
    const GO_TYPE: &'static str = "LinksDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for InputDefinition {
    const GO_TYPE: &'static str = "InputDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for RoutinesDefinition {
    const GO_TYPE: &'static str = "RoutinesDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for ScreenDefinition {
    const GO_TYPE: &'static str = "ScreenDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for ModelListDefinition {
    const GO_TYPE: &'static str = "ModelListDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for AdapterRefs {
    const GO_TYPE: &'static str = "AdapterRefs";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for SourceRef {
    const GO_TYPE: &'static str = "SourceRef";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}

impl GoWire for RevisionRecord {
    const GO_TYPE: &'static str = "RevisionRecord";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}

impl GoWire for DistributionPayload {
    const GO_TYPE: &'static str = "DistributionPayload";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}

impl GoWire for DistributionBundle {
    const GO_TYPE: &'static str = "DistributionBundle";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}

impl GoWire for DistributionStatus {
    const GO_TYPE: &'static str = "DistributionStatus";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
