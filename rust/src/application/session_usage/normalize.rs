use super::{UsageRequest, pricing};
use crate::proto::{
    Message,
    time::{self, Timestamp},
};
fn pct(v: f64) -> f64 {
    if v.is_nan() || !(0.0..=100.0).contains(&v) {
        0.0
    } else {
        v
    }
}
fn used(v: f64) -> f64 {
    if v.is_nan() || v < 0.0 {
        0.0
    } else {
        v.min(100.0)
    }
}
fn bounded(v: i64, max: i64) -> i64 {
    if (0..=max).contains(&v) { v } else { 0 }
}
fn text(s: &str, max: usize) -> String {
    s.trim()
        .chars()
        .filter(|c| *c >= ' ' && *c != '\u{7f}')
        .take(max)
        .collect()
}
pub(super) fn normalize(
    mut req: UsageRequest,
    fallback_model: &str,
    received: Timestamp,
) -> Result<(Message, Timestamp), &'static str> {
    const TOKENS: i64 = 1_000_000_000;
    if [
        req.tokens_in,
        req.tokens_out,
        req.tokens_cache,
        req.tokens_total,
    ]
    .iter()
    .any(|v| !(0..=TOKENS).contains(v))
    {
        return Err("token counts out of range");
    }
    if !(0..=TOKENS).contains(&req.ctx_window) {
        return Err("ctx_window out of range");
    }
    if req.cost_usd.is_nan() || !(0.0..=1_000_000.0).contains(&req.cost_usd) {
        return Err("cost_usd out of range");
    }
    req.ctx_used_pct = pct(req.ctx_used_pct);
    req.rl_5h_pct = used(req.rl_5h_pct);
    req.rl_7d_pct = used(req.rl_7d_pct);
    req.rl_5h_reset = req.rl_5h_reset.max(0);
    req.rl_7d_reset = req.rl_7d_reset.max(0);
    req.codex_primary_used_pct = pct(req.codex_primary_used_pct);
    req.codex_secondary_used_pct = pct(req.codex_secondary_used_pct);
    req.codex_primary_window_minutes =
        bounded(req.codex_primary_window_minutes, 10 * 365 * 24 * 60);
    req.codex_secondary_window_minutes =
        bounded(req.codex_secondary_window_minutes, 10 * 365 * 24 * 60);
    req.codex_primary_reset = req.codex_primary_reset.max(0);
    req.codex_secondary_reset = req.codex_secondary_reset.max(0);
    req.lines_added = bounded(req.lines_added, TOKENS);
    req.lines_removed = bounded(req.lines_removed, TOKENS);
    req.reasoning_output_tokens = bounded(req.reasoning_output_tokens, TOKENS);
    req.duration_ms = bounded(req.duration_ms, 30 * 24 * 3600 * 1000);
    req.api_duration_ms = bounded(req.api_duration_ms, 30 * 24 * 3600 * 1000);
    req.effort_level = text(&req.effort_level, 16);
    req.version = text(&req.version, 64);
    req.output_style = text(&req.output_style, 64);
    req.vim_mode = text(&req.vim_mode, 32);
    req.agent_name = text(&req.agent_name, 64);
    req.repo_host = text(&req.repo_host, 64);
    req.repo_owner = text(&req.repo_owner, 128);
    req.repo_name = text(&req.repo_name, 128);
    req.remaining_pct = pct(req.remaining_pct);
    // Credits and plan are stripped/truncated WITHOUT trim in fixed Go.
    req.codex_credits_balance = req
        .codex_credits_balance
        .chars()
        .filter(|c| *c >= ' ' && *c != '\u{7f}')
        .take(64)
        .collect();
    req.codex_plan_type = req
        .codex_plan_type
        .chars()
        .filter(|c| *c >= ' ' && *c != '\u{7f}')
        .take(32)
        .collect();
    req.model = text(&req.model, 128);
    if req.model.is_empty() {
        req.model = fallback_model.trim().into();
    }
    let (cost, cost_known) = if req.cost_from_relay {
        (req.cost_usd, true)
    } else {
        pricing::cost(&req.model, req.tokens_in, req.tokens_out, req.tokens_cache)
    };
    let observed = if req.usage_observed_at.trim().is_empty() {
        None
    } else {
        time::parse_rfc3339(&req.usage_observed_at)
            .ok()
            .filter(|v| v.unix_seconds() != -62135596800 || v.subsec_nanos() != 0)
    };
    let record_at = observed.unwrap_or(received);
    req.usage_observed_at = observed
        .and_then(|v| time::format_with_offset(v, 0, true).ok())
        .unwrap_or_default();
    let message = Message {
        r#type: "usage_stat".into(),
        cost_known,
        provider: req.provider,
        session_id: req.session_id,
        cost_usd: req.cost_usd,
        usage_model: req.model,
        tokens_in: req.tokens_in,
        tokens_out: req.tokens_out,
        tokens_cache: req.tokens_cache,
        tokens_total: req.tokens_total,
        ctx_window: req.ctx_window,
        ctx_used_pct: req.ctx_used_pct,
        usage_started_at: req.started_at,
        rl_5h_pct: req.rl_5h_pct,
        rl_5h_reset: req.rl_5h_reset,
        rl_7d_pct: req.rl_7d_pct,
        rl_7d_reset: req.rl_7d_reset,
        claude_rate_limits_present: req.claude_rate_limits_present,
        claude_5h_field_present: req.claude_5h_field_present,
        claude_5h_present: req.claude_5h_present,
        claude_7d_field_present: req.claude_7d_field_present,
        claude_7d_present: req.claude_7d_present,
        codex_rate_limits_present: req.codex_rate_limits_present,
        codex_primary_present: req.codex_primary_present,
        codex_primary_used_pct: req.codex_primary_used_pct,
        codex_primary_window_minutes: req.codex_primary_window_minutes,
        codex_primary_reset: req.codex_primary_reset,
        codex_secondary_used_pct: req.codex_secondary_used_pct,
        codex_secondary_present: req.codex_secondary_present,
        codex_secondary_window_minutes: req.codex_secondary_window_minutes,
        codex_secondary_reset: req.codex_secondary_reset,
        codex_credits_present: req.codex_credits_present,
        codex_has_credits: req.codex_has_credits,
        codex_credits_unlimited: req.codex_credits_unlimited,
        codex_credits_balance: req.codex_credits_balance,
        codex_plan_type: req.codex_plan_type,
        usage_observed_at: req.usage_observed_at,
        lines_added: req.lines_added,
        lines_removed: req.lines_removed,
        effort_level: req.effort_level,
        thinking: req.thinking,
        exceeds_200k: req.exceeds_200k,
        duration_ms: req.duration_ms,
        api_duration_ms: req.api_duration_ms,
        version: req.version,
        output_style: req.output_style,
        vim_mode: req.vim_mode,
        agent_name: req.agent_name,
        repo_host: req.repo_host,
        repo_owner: req.repo_owner,
        repo_name: req.repo_name,
        remaining_pct: req.remaining_pct,
        reasoning_output_tokens: req.reasoning_output_tokens,
        ..Default::default()
    };
    let mut message = message;
    message.cost_usd = cost;
    Ok((message, record_at))
}
