use crate::proto::wire::{Field, GoWire, Schema};
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Model {
    pub display_name: String,
}
impl GoWire for Model {
    const GO_TYPE: &'static str = "Model";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Name {
    pub name: String,
}
impl GoWire for Name {
    const GO_TYPE: &'static str = "Name";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Mode {
    pub mode: String,
}
impl GoWire for Mode {
    const GO_TYPE: &'static str = "Mode";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Repo {
    pub host: String,
    pub owner: String,
    pub name: String,
}
impl GoWire for Repo {
    const GO_TYPE: &'static str = "Repo";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Workspace {
    pub repo: Repo,
}
impl GoWire for Workspace {
    const GO_TYPE: &'static str = "Workspace";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Cost {
    pub total_cost_usd: f64,
    pub total_duration_ms: f64,
    pub total_api_duration_ms: f64,
    pub total_lines_added: i64,
    pub total_lines_removed: i64,
}
impl GoWire for Cost {
    const GO_TYPE: &'static str = "Cost";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Current {
    // Source decodes this field but excludes creation from cache-hit totals.
    #[allow(dead_code)]
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
}
impl GoWire for Current {
    const GO_TYPE: &'static str = "Current";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Context {
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub context_window_size: i64,
    pub used_percentage: f64,
    pub remaining_percentage: f64,
    pub current_usage: Current,
}
impl GoWire for Context {
    const GO_TYPE: &'static str = "Context";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Effort {
    pub level: String,
}
impl GoWire for Effort {
    const GO_TYPE: &'static str = "Effort";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Thinking {
    pub enabled: bool,
}
impl GoWire for Thinking {
    const GO_TYPE: &'static str = "Thinking";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct ClaudeWindow {
    pub used_percentage: f64,
    pub resets_at: i64,
}
impl GoWire for ClaudeWindow {
    const GO_TYPE: &'static str = "ClaudeWindow";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct ClaudeLimits {
    pub five_hour: Option<ClaudeWindow>,
    pub seven_day: Option<ClaudeWindow>,
}
impl GoWire for ClaudeLimits {
    const GO_TYPE: &'static str = "ClaudeLimits";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Claude {
    pub version: String,
    pub model: Model,
    pub output_style: Name,
    pub vim: Mode,
    pub agent: Name,
    pub workspace: Workspace,
    pub cost: Cost,
    pub context_window: Context,
    pub exceeds_200k_tokens: bool,
    pub effort: Effort,
    pub thinking: Thinking,
    // Upper-case aliases are type checked by Go before the exact raw field wins.
    #[allow(dead_code)]
    pub rate_limits: Option<ClaudeLimits>,
}
impl GoWire for Claude {
    const GO_TYPE: &'static str = "Claude";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Stop {
    pub transcript_path: String,
    pub model: String,
}
impl GoWire for Stop {
    const GO_TYPE: &'static str = "Stop";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Numbers {
    pub input: i64,
    pub output: i64,
    pub cached: i64,
    pub total: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_input_tokens: i64,
    pub cached_tokens: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub reasoning_output_tokens: i64,
}
impl GoWire for Numbers {
    const GO_TYPE: &'static str = "Numbers";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Info {
    pub total_token_usage: Numbers,
    pub last_token_usage: Numbers,
    pub model_context_window: i64,
}
impl GoWire for Info {
    const GO_TYPE: &'static str = "Info";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Payload {
    pub r#type: String,
    pub info: Info,
    pub model_context_window: i64,
}
impl GoWire for Payload {
    const GO_TYPE: &'static str = "Payload";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Event {
    #[serde(flatten)]
    pub numbers: Numbers,
    pub timestamp: String,
    pub info: Info,
    pub usage: Numbers,
    pub payload: Payload,
    pub model_context_window: i64,
}
impl GoWire for Event {
    const GO_TYPE: &'static str = "Event";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Window {
    pub used_percent: f64,
    pub window_minutes: i64,
    pub resets_at: i64,
}
impl GoWire for Window {
    const GO_TYPE: &'static str = "Window";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Credits {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: String,
}
impl GoWire for Credits {
    const GO_TYPE: &'static str = "Credits";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub(super) struct Limits {
    pub primary: Option<Window>,
    pub secondary: Option<Window>,
    pub credits: Option<Credits>,
    pub plan_type: String,
}
impl GoWire for Limits {
    const GO_TYPE: &'static str = "Limits";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
const SCHEMAS: &[Schema] = &[
    Schema {
        name: "Model",
        fields: &[Field {
            name: "display_name",
            kind: "string",
        }],
    },
    Schema {
        name: "Name",
        fields: &[Field {
            name: "name",
            kind: "string",
        }],
    },
    Schema {
        name: "Mode",
        fields: &[Field {
            name: "mode",
            kind: "string",
        }],
    },
    Schema {
        name: "Repo",
        fields: &[
            Field {
                name: "host",
                kind: "string",
            },
            Field {
                name: "owner",
                kind: "string",
            },
            Field {
                name: "name",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "Workspace",
        fields: &[Field {
            name: "repo",
            kind: "Repo",
        }],
    },
    Schema {
        name: "Cost",
        fields: &[
            Field {
                name: "total_cost_usd",
                kind: "float64",
            },
            Field {
                name: "total_duration_ms",
                kind: "float64",
            },
            Field {
                name: "total_api_duration_ms",
                kind: "float64",
            },
            Field {
                name: "total_lines_added",
                kind: "int",
            },
            Field {
                name: "total_lines_removed",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "Current",
        fields: &[
            Field {
                name: "cache_creation_input_tokens",
                kind: "int",
            },
            Field {
                name: "cache_read_input_tokens",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "Context",
        fields: &[
            Field {
                name: "total_input_tokens",
                kind: "int",
            },
            Field {
                name: "total_output_tokens",
                kind: "int",
            },
            Field {
                name: "context_window_size",
                kind: "int",
            },
            Field {
                name: "used_percentage",
                kind: "float64",
            },
            Field {
                name: "remaining_percentage",
                kind: "float64",
            },
            Field {
                name: "current_usage",
                kind: "Current",
            },
        ],
    },
    Schema {
        name: "Effort",
        fields: &[Field {
            name: "level",
            kind: "string",
        }],
    },
    Schema {
        name: "Thinking",
        fields: &[Field {
            name: "enabled",
            kind: "bool",
        }],
    },
    Schema {
        name: "ClaudeWindow",
        fields: &[
            Field {
                name: "used_percentage",
                kind: "float64",
            },
            Field {
                name: "resets_at",
                kind: "int64",
            },
        ],
    },
    Schema {
        name: "ClaudeLimits",
        fields: &[
            Field {
                name: "five_hour",
                kind: "*ClaudeWindow",
            },
            Field {
                name: "seven_day",
                kind: "*ClaudeWindow",
            },
        ],
    },
    Schema {
        name: "Claude",
        fields: &[
            Field {
                name: "version",
                kind: "string",
            },
            Field {
                name: "model",
                kind: "Model",
            },
            Field {
                name: "output_style",
                kind: "Name",
            },
            Field {
                name: "vim",
                kind: "Mode",
            },
            Field {
                name: "agent",
                kind: "Name",
            },
            Field {
                name: "workspace",
                kind: "Workspace",
            },
            Field {
                name: "cost",
                kind: "Cost",
            },
            Field {
                name: "context_window",
                kind: "Context",
            },
            Field {
                name: "exceeds_200k_tokens",
                kind: "bool",
            },
            Field {
                name: "effort",
                kind: "Effort",
            },
            Field {
                name: "thinking",
                kind: "Thinking",
            },
            Field {
                name: "rate_limits",
                kind: "*ClaudeLimits",
            },
        ],
    },
    Schema {
        name: "Stop",
        fields: &[
            Field {
                name: "transcript_path",
                kind: "string",
            },
            Field {
                name: "model",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "Numbers",
        fields: &[
            Field {
                name: "input",
                kind: "int",
            },
            Field {
                name: "output",
                kind: "int",
            },
            Field {
                name: "cached",
                kind: "int",
            },
            Field {
                name: "total",
                kind: "int",
            },
            Field {
                name: "input_tokens",
                kind: "int",
            },
            Field {
                name: "output_tokens",
                kind: "int",
            },
            Field {
                name: "cached_input_tokens",
                kind: "int",
            },
            Field {
                name: "cached_tokens",
                kind: "int",
            },
            Field {
                name: "prompt_tokens",
                kind: "int",
            },
            Field {
                name: "completion_tokens",
                kind: "int",
            },
            Field {
                name: "total_tokens",
                kind: "int",
            },
            Field {
                name: "reasoning_output_tokens",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "Info",
        fields: &[
            Field {
                name: "total_token_usage",
                kind: "Numbers",
            },
            Field {
                name: "last_token_usage",
                kind: "Numbers",
            },
            Field {
                name: "model_context_window",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "Payload",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "info",
                kind: "Info",
            },
            Field {
                name: "model_context_window",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "Event",
        fields: &[
            Field {
                name: "timestamp",
                kind: "string",
            },
            Field {
                name: "info",
                kind: "Info",
            },
            Field {
                name: "usage",
                kind: "Numbers",
            },
            Field {
                name: "payload",
                kind: "Payload",
            },
            Field {
                name: "model_context_window",
                kind: "int",
            },
            Field {
                name: "input",
                kind: "int",
            },
            Field {
                name: "output",
                kind: "int",
            },
            Field {
                name: "cached",
                kind: "int",
            },
            Field {
                name: "total",
                kind: "int",
            },
            Field {
                name: "input_tokens",
                kind: "int",
            },
            Field {
                name: "output_tokens",
                kind: "int",
            },
            Field {
                name: "cached_input_tokens",
                kind: "int",
            },
            Field {
                name: "cached_tokens",
                kind: "int",
            },
            Field {
                name: "prompt_tokens",
                kind: "int",
            },
            Field {
                name: "completion_tokens",
                kind: "int",
            },
            Field {
                name: "total_tokens",
                kind: "int",
            },
            Field {
                name: "reasoning_output_tokens",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "Window",
        fields: &[
            Field {
                name: "used_percent",
                kind: "float64",
            },
            Field {
                name: "window_minutes",
                kind: "int",
            },
            Field {
                name: "resets_at",
                kind: "int64",
            },
        ],
    },
    Schema {
        name: "Credits",
        fields: &[
            Field {
                name: "has_credits",
                kind: "bool",
            },
            Field {
                name: "unlimited",
                kind: "bool",
            },
            Field {
                name: "balance",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "Limits",
        fields: &[
            Field {
                name: "primary",
                kind: "*Window",
            },
            Field {
                name: "secondary",
                kind: "*Window",
            },
            Field {
                name: "credits",
                kind: "*Credits",
            },
            Field {
                name: "plan_type",
                kind: "string",
            },
        ],
    },
];
