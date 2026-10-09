use super::{UsageRequest, input::*};
use crate::proto::{
    time::{self, Timestamp},
    wire,
};
use std::collections::BTreeMap;
fn raw_object(fields: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut bytes = vec![b'{'];
    for (i, (name, value)) in fields.iter().enumerate() {
        if i > 0 {
            bytes.push(b',');
        }
        bytes.extend_from_slice(serde_json::to_string(name).unwrap().as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(value);
    }
    bytes.push(b'}');
    bytes
}
fn go_float_int(v: f64) -> i64 {
    if (-9223372036854775808.0..9223372036854775808.0).contains(&v) {
        v as i64
    } else {
        i64::MIN
    }
}
pub fn tokens(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}
pub fn claude(
    raw: &[u8],
    observed: Timestamp,
    started: Timestamp,
) -> Result<(String, UsageRequest), serde_json::Error> {
    let mut fields = wire::decode_go_json_object(raw)?.unwrap_or_default();
    let limits_raw = fields.remove("rate_limits");
    let present = limits_raw.is_some();
    let input: Claude = wire::decode(&raw_object(&fields))?;
    let mut present = present;
    let mut limits = None;
    let mut five_field = false;
    let mut seven_field = false;
    if let Some(raw) = limits_raw.filter(|v| v != b"null") {
        match wire::decode::<ClaudeLimits>(&raw) {
            Ok(value) => {
                let fields = wire::decode_go_json_object(&raw)?.unwrap_or_default();
                five_field = fields.contains_key("five_hour");
                seven_field = fields.contains_key("seven_day");
                limits = Some(value);
            }
            Err(_) => present = false,
        }
    }
    let context = input.context_window;
    let status = format!(
        "${:.4}  {}  ↑{} ↓{}\n",
        input.cost.total_cost_usd,
        input.model.display_name,
        tokens(context.total_input_tokens),
        tokens(context.total_output_tokens)
    );
    let mut payload = UsageRequest {
        provider: "claude".into(),
        cost_from_relay: true,
        cost_usd: input.cost.total_cost_usd,
        model: input.model.display_name,
        tokens_in: context.total_input_tokens,
        tokens_out: context.total_output_tokens,
        tokens_cache: context.current_usage.cache_read_input_tokens,
        tokens_total: context
            .total_input_tokens
            .wrapping_add(context.total_output_tokens),
        ctx_window: context.context_window_size,
        ctx_used_pct: context.used_percentage,
        started_at: time::format_rfc3339(started).unwrap_or_default(),
        usage_observed_at: time::format_with_offset(observed, 0, false).unwrap_or_default(),
        claude_rate_limits_present: present,
        claude_5h_field_present: five_field,
        claude_7d_field_present: seven_field,
        lines_added: input.cost.total_lines_added,
        lines_removed: input.cost.total_lines_removed,
        effort_level: input.effort.level,
        thinking: input.thinking.enabled,
        exceeds_200k: input.exceeds_200k_tokens,
        duration_ms: go_float_int(input.cost.total_duration_ms),
        api_duration_ms: go_float_int(input.cost.total_api_duration_ms),
        version: input.version,
        output_style: input.output_style.name,
        vim_mode: input.vim.mode,
        agent_name: input.agent.name,
        repo_host: input.workspace.repo.host,
        repo_owner: input.workspace.repo.owner,
        repo_name: input.workspace.repo.name,
        remaining_pct: context.remaining_percentage,
        ..Default::default()
    };
    if let Some(limits) = limits {
        if let Some(v) = limits.five_hour {
            payload.claude_5h_present = true;
            payload.rl_5h_pct = v.used_percentage;
            payload.rl_5h_reset = v.resets_at;
        }
        if let Some(v) = limits.seven_day {
            payload.claude_7d_present = true;
            payload.rl_7d_pct = v.used_percentage;
            payload.rl_7d_reset = v.resets_at;
        }
    }
    Ok((status, payload))
}
impl Numbers {
    fn resolve(&self) -> [i64; 5] {
        let input = first(&[self.input_tokens, self.input, self.prompt_tokens]);
        let output = first(&[self.output_tokens, self.output, self.completion_tokens]);
        let cache = first(&[self.cached_input_tokens, self.cached, self.cached_tokens]);
        let mut total = first(&[self.total_tokens, self.total]);
        if total == 0 && (input > 0 || output > 0) {
            total = input.wrapping_add(output);
        }
        [input, output, cache, total, self.reasoning_output_tokens]
    }
}
fn first(values: &[i64]) -> i64 {
    values.iter().copied().find(|v| *v != 0).unwrap_or(0)
}
fn positive(v: &[i64; 5]) -> bool {
    v[0] > 0 || v[1] > 0 || v[3] > 0
}
fn fold_field(v: &str) -> String {
    v.chars()
        .map(|c| match c {
            '\u{017f}' => 's',
            '\u{212a}' => 'k',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}
// encoding/json continues decoding after a field type error. Codex's caller
// uses that partially decoded auxiliary object even when its presence is false.
fn codex_limits(raw: &[u8]) -> (Limits, bool) {
    let mut result = Limits::default();
    let Ok(Some(members)) = wire::decode_go_json_members(raw) else {
        return (result, false);
    };
    let mut valid = true;
    for (key, value) in members {
        let key = fold_field(&key);
        if key == "plan_type" {
            if value == b"null" {
                continue;
            }
            let raw = raw_object(&BTreeMap::from([(key, value)]));
            match wire::decode::<Limits>(&raw) {
                Ok(v) => result.plan_type = v.plan_type,
                Err(_) => valid = false,
            }
            continue;
        }
        if !matches!(key.as_str(), "primary" | "secondary" | "credits") {
            continue;
        }
        if value == b"null" {
            match key.as_str() {
                "primary" => result.primary = None,
                "secondary" => result.secondary = None,
                _ => result.credits = None,
            }
            continue;
        }
        match key.as_str() {
            "primary" => {
                result.primary.get_or_insert_with(Window::default);
            }
            "secondary" => {
                result.secondary.get_or_insert_with(Window::default);
            }
            _ => {
                result.credits.get_or_insert_with(Credits::default);
            }
        }
        let Ok(Some(fields)) = wire::decode_go_json_members(&value) else {
            valid = false;
            continue;
        };
        for (field, value) in fields {
            let field = fold_field(&field);
            let known = if key == "credits" {
                matches!(field.as_str(), "has_credits" | "unlimited" | "balance")
            } else {
                matches!(
                    field.as_str(),
                    "used_percent" | "window_minutes" | "resets_at"
                )
            };
            if !known || value == b"null" {
                continue;
            }
            let inner = raw_object(&BTreeMap::from([(field.clone(), value)]));
            let raw = raw_object(&BTreeMap::from([(key.clone(), inner)]));
            let decoded = match wire::decode::<Limits>(&raw) {
                Ok(v) => v,
                Err(_) => {
                    valid = false;
                    continue;
                }
            };
            if key == "credits" {
                let target = result.credits.as_mut().unwrap();
                let source = decoded.credits.unwrap();
                match field.as_str() {
                    "has_credits" => target.has_credits = source.has_credits,
                    "unlimited" => target.unlimited = source.unlimited,
                    _ => target.balance = source.balance,
                }
            } else {
                let (target, source) = if key == "primary" {
                    (result.primary.as_mut().unwrap(), decoded.primary.unwrap())
                } else {
                    (
                        result.secondary.as_mut().unwrap(),
                        decoded.secondary.unwrap(),
                    )
                };
                match field.as_str() {
                    "used_percent" => target.used_percent = source.used_percent,
                    "window_minutes" => target.window_minutes = source.window_minutes,
                    _ => target.resets_at = source.resets_at,
                }
            }
        }
    }
    (result, valid)
}
impl Event {
    fn resolve(&self) -> [i64; 6] {
        let merge = |v: [i64; 5], ctx| [v[0], v[1], v[2], v[3], ctx, v[4]];
        if self.payload.r#type == "token_count" {
            for u in [
                &self.payload.info.total_token_usage,
                &self.payload.info.last_token_usage,
            ] {
                let v = u.resolve();
                if positive(&v) {
                    return merge(
                        v,
                        first(&[
                            self.payload.info.model_context_window,
                            self.payload.model_context_window,
                        ]),
                    );
                }
            }
        }
        for (u, ctx) in [
            (&self.numbers, self.model_context_window),
            (&self.info.total_token_usage, self.info.model_context_window),
            (&self.info.last_token_usage, self.info.model_context_window),
            (&self.usage, 0),
        ] {
            let v = u.resolve();
            if positive(&v) {
                return merge(v, ctx);
            }
        }
        [0; 6]
    }
}
#[derive(Default)]
pub struct Scan {
    pub numbers: [i64; 6],
    pub(super) limits: Limits,
    pub limits_present: bool,
    pub observed: Option<Timestamp>,
}
impl Scan {
    fn line(&mut self, line: &[u8]) {
        if !line
            .windows(b"\"token_count\"".len())
            .any(|v| v == b"\"token_count\"")
        {
            return;
        }
        let Ok(event) = wire::decode::<Event>(line) else {
            return;
        };
        if event.payload.r#type == "token_count" {
            self.limits = Limits::default();
            self.limits_present = false;
            self.observed = time::parse_rfc3339(&event.timestamp)
                .ok()
                .filter(|v| v.unix_seconds() != -62135596800 || v.subsec_nanos() != 0);
            let mut limit_raw = None;
            if let Ok(Some(members)) = wire::decode_go_json_members(line) {
                for (key, raw) in members {
                    if key.eq_ignore_ascii_case("payload")
                        && raw != b"null"
                        && let Ok(Some(inner)) = wire::decode_go_json_members(&raw)
                        && let Some(raw) = wire::last_go_raw_field(&inner, "rate_limits")
                    {
                        limit_raw = Some(raw.to_vec());
                    }
                }
            }
            if let Some(raw) = limit_raw {
                if raw == b"null" {
                    self.limits_present = true;
                } else {
                    (self.limits, self.limits_present) = codex_limits(&raw);
                }
            }
        }
        let v = event.resolve();
        if v[0] > 0 || v[1] > 0 || v[3] > 0 {
            self.numbers[0..4].copy_from_slice(&v[0..4]);
            self.numbers[5] = v[5];
            if v[4] > 0 {
                self.numbers[4] = v[4];
            }
        }
    }
}
pub fn scan(reader: &mut dyn std::io::BufRead) -> std::io::Result<Scan> {
    const CAP: usize = 256 * 1024;
    let mut result = Scan::default();
    let mut line = Vec::new();
    let mut overlong = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            if !overlong && !line.is_empty() {
                result.line(&line);
            }
            break;
        }
        let end = buffer.iter().position(|v| *v == b'\n');
        let count = end.map_or(buffer.len(), |v| v + 1);
        if !overlong {
            if line.len() + count > CAP {
                overlong = true;
                line.clear();
            } else {
                line.extend_from_slice(&buffer[..count]);
            }
        }
        reader.consume(count);
        if end.is_some() {
            if !overlong {
                result.line(&line);
            }
            line.clear();
            overlong = false;
        }
    }
    Ok(result)
}
pub fn codex_payload(stop: Stop, scan: Scan, started: Timestamp) -> UsageRequest {
    let n = scan.numbers;
    let mut p = UsageRequest {
        provider: "codex".into(),
        model: stop.model,
        tokens_in: n[0],
        tokens_out: n[1],
        tokens_cache: n[2],
        tokens_total: n[3],
        ctx_window: n[4],
        reasoning_output_tokens: n[5],
        transcript_path: stop.transcript_path,
        started_at: time::format_rfc3339(started).unwrap_or_default(),
        codex_rate_limits_present: scan.limits_present,
        ..Default::default()
    };
    let limits = scan.limits;
    if let Some(v) = limits.primary {
        p.codex_primary_present = true;
        p.codex_primary_used_pct = v.used_percent;
        p.codex_primary_window_minutes = v.window_minutes;
        p.codex_primary_reset = v.resets_at;
    }
    if let Some(v) = limits.secondary {
        p.codex_secondary_present = true;
        p.codex_secondary_used_pct = v.used_percent;
        p.codex_secondary_window_minutes = v.window_minutes;
        p.codex_secondary_reset = v.resets_at;
    }
    if let Some(v) = limits.credits {
        p.codex_credits_present = true;
        p.codex_has_credits = v.has_credits;
        p.codex_credits_unlimited = v.unlimited;
        p.codex_credits_balance = v.balance;
    }
    p.codex_plan_type = limits.plan_type;
    p.usage_observed_at = scan
        .observed
        .and_then(|v| time::format_with_offset(v, 0, true).ok())
        .unwrap_or_default();
    p
}
