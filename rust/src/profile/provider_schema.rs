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
            Field {
                name: "transcript",
                kind: "*TranscriptDefinition",
            },
            Field {
                name: "approval",
                kind: "*ApprovalDefinition",
            },
            Field {
                name: "instructions",
                kind: "*InstructionsDefinition",
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
            Field {
                name: "choices",
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
            Field {
                name: "full_frame_cursor",
                kind: "bool",
            },
            Field {
                name: "no_line_clear",
                kind: "bool",
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
        name: "InstructionsDefinition",
        fields: &[Field {
            name: "rules_file",
            kind: "string",
        }],
    },
    Schema {
        name: "ScreenDefinition",
        fields: &[
            Field {
                name: "ready_markers",
                kind: "[]string",
            },
            Field {
                name: "startup_blockers",
                kind: "[]string",
            },
            Field {
                name: "composer_prefix",
                kind: "string",
            },
            Field {
                name: "composer_end_markers",
                kind: "[]string",
            },
        ],
    },
    Schema {
        name: "TranscriptDefinition",
        fields: &[
            Field {
                name: "root",
                kind: "string",
            },
            Field {
                name: "file",
                kind: "string",
            },
            Field {
                name: "extension",
                kind: "string",
            },
            Field {
                name: "find",
                kind: "[]string",
            },
            Field {
                name: "id_format",
                kind: "string",
            },
            Field {
                name: "dir_fallback",
                kind: "bool",
            },
            Field {
                name: "label",
                kind: "string",
            },
            Field {
                name: "dir_label",
                kind: "string",
            },
            Field {
                name: "unavailable_reason",
                kind: "string",
            },
            Field {
                name: "not_found_reason",
                kind: "string",
            },
            Field {
                name: "meta",
                kind: "*TranscriptMeta",
            },
            Field {
                name: "index",
                kind: "*TranscriptIndex",
            },
            Field {
                name: "reader",
                kind: "*TranscriptReader",
            },
        ],
    },
    Schema {
        name: "TranscriptMeta",
        fields: &[
            Field {
                name: "kind_key",
                kind: "string",
            },
            Field {
                name: "kind_value",
                kind: "string",
            },
            Field {
                name: "cwd_key",
                kind: "string",
            },
            Field {
                name: "time_key",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "TranscriptIndex",
        fields: &[
            Field {
                name: "file",
                kind: "string",
            },
            Field {
                name: "table",
                kind: "string",
            },
            Field {
                name: "id_column",
                kind: "string",
            },
            Field {
                name: "cwd_column",
                kind: "string",
            },
            Field {
                name: "time_column",
                kind: "string",
            },
            Field {
                name: "time_unit",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "TranscriptReader",
        fields: &[
            Field {
                name: "format",
                kind: "string",
            },
            Field {
                name: "messages",
                kind: "string",
            },
            Field {
                name: "role_key",
                kind: "string",
            },
            Field {
                name: "text_key",
                kind: "string",
            },
            Field {
                name: "time_key",
                kind: "string",
            },
            Field {
                name: "roles",
                kind: "map[string]string",
            },
        ],
    },
    Schema {
        name: "ApprovalDefinition",
        fields: &[
            Field {
                name: "style",
                kind: "string",
            },
            Field {
                name: "line_extras",
                kind: "[]string",
            },
            Field {
                name: "keys",
                kind: "[]ApprovalKey",
            },
            Field {
                name: "bracket_keys",
                kind: "[]ApprovalKey",
            },
            Field {
                name: "buttons",
                kind: "[]ApprovalButtons",
            },
            Field {
                name: "arrow_send",
                kind: "bool",
            },
            Field {
                name: "feedback_survey",
                kind: "bool",
            },
            Field {
                name: "shortcut_kind",
                kind: "bool",
            },
            Field {
                name: "hint_phrases",
                kind: "[]string",
            },
            Field {
                name: "not_approval_phrases",
                kind: "[]string",
            },
            Field {
                name: "suppress_screens",
                kind: "[]ApprovalSuppress",
            },
        ],
    },
    Schema {
        name: "ApprovalKey",
        fields: &[
            Field {
                name: "keys",
                kind: "[]string",
            },
            Field {
                name: "num",
                kind: "int",
            },
            Field {
                name: "send",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "ApprovalButtons",
        fields: &[
            Field {
                name: "line_all",
                kind: "[]string",
            },
            Field {
                name: "start_line",
                kind: "string",
            },
            Field {
                name: "start_required",
                kind: "bool",
            },
            Field {
                name: "labels",
                kind: "[]string",
            },
        ],
    },
    Schema {
        name: "ApprovalSuppress",
        fields: &[
            Field {
                name: "all",
                kind: "[]string",
            },
            Field {
                name: "any",
                kind: "[]string",
            },
        ],
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
            Field {
                name: "refresh_args",
                kind: "[]string",
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
                name: "permission_args",
                kind: "map[string][]string",
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
            Field {
                name: "change_pattern",
                kind: "string",
            },
            Field {
                name: "change_effort_suffix",
                kind: "bool",
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
            Field {
                name: "handoff_order",
                kind: "int",
            },
            Field {
                name: "label",
                kind: "string",
            },
            Field {
                name: "long_label",
                kind: "string",
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
impl GoWire for InstructionsDefinition {
    const GO_TYPE: &'static str = "InstructionsDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for ScreenDefinition {
    const GO_TYPE: &'static str = "ScreenDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for TranscriptDefinition {
    const GO_TYPE: &'static str = "TranscriptDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for TranscriptMeta {
    const GO_TYPE: &'static str = "TranscriptMeta";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for TranscriptIndex {
    const GO_TYPE: &'static str = "TranscriptIndex";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for TranscriptReader {
    const GO_TYPE: &'static str = "TranscriptReader";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for ApprovalDefinition {
    const GO_TYPE: &'static str = "ApprovalDefinition";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for ApprovalKey {
    const GO_TYPE: &'static str = "ApprovalKey";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for ApprovalButtons {
    const GO_TYPE: &'static str = "ApprovalButtons";
    const SCHEMAS: &'static [Schema] = PROVIDER_SCHEMAS;
}
impl GoWire for ApprovalSuppress {
    const GO_TYPE: &'static str = "ApprovalSuppress";
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
