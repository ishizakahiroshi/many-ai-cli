#[derive(Default, Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct UsageRequest {
    pub provider: String,
    pub session_id: i64,
    pub cost_usd: f64,
    pub cost_from_relay: bool,
    pub model: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tokens_cache: i64,
    pub tokens_total: i64,
    pub ctx_window: i64,
    pub ctx_used_pct: f64,
    pub started_at: String,
    pub rl_5h_pct: f64,
    pub rl_5h_reset: i64,
    pub rl_7d_pct: f64,
    pub rl_7d_reset: i64,
    pub claude_rate_limits_present: bool,
    pub claude_5h_field_present: bool,
    pub claude_5h_present: bool,
    pub claude_7d_field_present: bool,
    pub claude_7d_present: bool,
    pub codex_rate_limits_present: bool,
    pub codex_primary_present: bool,
    pub codex_primary_used_pct: f64,
    pub codex_primary_window_minutes: i64,
    pub codex_primary_reset: i64,
    pub codex_secondary_used_pct: f64,
    pub codex_secondary_present: bool,
    pub codex_secondary_window_minutes: i64,
    pub codex_secondary_reset: i64,
    pub codex_credits_present: bool,
    pub codex_has_credits: bool,
    pub codex_credits_unlimited: bool,
    pub codex_credits_balance: String,
    pub codex_plan_type: String,
    pub usage_observed_at: String,
    pub lines_added: i64,
    pub lines_removed: i64,
    pub effort_level: String,
    pub thinking: bool,
    pub exceeds_200k: bool,
    pub duration_ms: i64,
    pub api_duration_ms: i64,
    pub version: String,
    pub output_style: String,
    pub vim_mode: String,
    pub agent_name: String,
    pub repo_host: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub remaining_pct: f64,
    pub reasoning_output_tokens: i64,
    pub transcript_path: String,
}
impl crate::proto::wire::GoWire for UsageRequest {
    const GO_TYPE: &'static str = "sessionUsageRequest";
    const SCHEMAS: &'static [crate::proto::wire::Schema] = &[crate::proto::wire::Schema {
        name: "sessionUsageRequest",
        fields: &[
            crate::proto::wire::Field {
                name: "provider",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "session_id",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "cost_usd",
                kind: "float64",
            },
            crate::proto::wire::Field {
                name: "cost_from_relay",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "model",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "tokens_in",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "tokens_out",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "tokens_cache",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "tokens_total",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "ctx_window",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "ctx_used_pct",
                kind: "float64",
            },
            crate::proto::wire::Field {
                name: "started_at",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "rl_5h_pct",
                kind: "float64",
            },
            crate::proto::wire::Field {
                name: "rl_5h_reset",
                kind: "int64",
            },
            crate::proto::wire::Field {
                name: "rl_7d_pct",
                kind: "float64",
            },
            crate::proto::wire::Field {
                name: "rl_7d_reset",
                kind: "int64",
            },
            crate::proto::wire::Field {
                name: "claude_rate_limits_present",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "claude_5h_field_present",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "claude_5h_present",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "claude_7d_field_present",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "claude_7d_present",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "codex_rate_limits_present",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "codex_primary_present",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "codex_primary_used_pct",
                kind: "float64",
            },
            crate::proto::wire::Field {
                name: "codex_primary_window_minutes",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "codex_primary_reset",
                kind: "int64",
            },
            crate::proto::wire::Field {
                name: "codex_secondary_used_pct",
                kind: "float64",
            },
            crate::proto::wire::Field {
                name: "codex_secondary_present",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "codex_secondary_window_minutes",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "codex_secondary_reset",
                kind: "int64",
            },
            crate::proto::wire::Field {
                name: "codex_credits_present",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "codex_has_credits",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "codex_credits_unlimited",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "codex_credits_balance",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "codex_plan_type",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "usage_observed_at",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "lines_added",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "lines_removed",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "effort_level",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "thinking",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "exceeds_200k",
                kind: "bool",
            },
            crate::proto::wire::Field {
                name: "duration_ms",
                kind: "int64",
            },
            crate::proto::wire::Field {
                name: "api_duration_ms",
                kind: "int64",
            },
            crate::proto::wire::Field {
                name: "version",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "output_style",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "vim_mode",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "agent_name",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "repo_host",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "repo_owner",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "repo_name",
                kind: "string",
            },
            crate::proto::wire::Field {
                name: "remaining_pct",
                kind: "float64",
            },
            crate::proto::wire::Field {
                name: "reasoning_output_tokens",
                kind: "int",
            },
            crate::proto::wire::Field {
                name: "transcript_path",
                kind: "string",
            },
        ],
    }];
}
