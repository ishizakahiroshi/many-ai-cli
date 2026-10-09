//! Reverse scans of the named provider usage logs, never authentication files.
use super::appserver::{CodexUsage, Credits, Window};
use crate::{
    config::RuntimePaths,
    proto::{
        time::{self, Timestamp},
        wire::{self, Field, GoWire, Schema},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};
#[derive(Clone, Default, Serialize, PartialEq)]
pub struct GrokUsage {
    pub used_percent: f64,
    pub remaining_percent: f64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub period_start: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub period_end: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub period_type: String,
}
pub struct Observation {
    pub codex: Option<CodexUsage>,
    pub grok: Option<GrokUsage>,
    pub observed: Option<Timestamp>,
    pub observed_display: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Event {
    timestamp: String,
    payload: Payload,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Payload {
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct RateLimits {
    primary: Option<RawWindow>,
    secondary: Option<RawWindow>,
    credits: Option<Credits>,
    plan_type: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawWindow {
    used_percent: f64,
    window_minutes: i64,
    resets_at: i64,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct GrokRecord {
    ts: String,
    msg: String,
    ctx: GrokContext,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct GrokContext {
    config: GrokConfig,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct GrokConfig {
    #[serde(rename = "creditUsagePercent")]
    used: Option<f64>,
    #[serde(rename = "currentPeriod")]
    period: GrokPeriod,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct GrokPeriod {
    start: String,
    end: String,
    #[serde(rename = "type")]
    kind: String,
}
const SCHEMAS: &[Schema] = &[
    Schema {
        name: "Event",
        fields: &[
            Field {
                name: "timestamp",
                kind: "string",
            },
            Field {
                name: "payload",
                kind: "Payload",
            },
        ],
    },
    Schema {
        name: "Payload",
        fields: &[Field {
            name: "type",
            kind: "string",
        }],
    },
    Schema {
        name: "RateLimits",
        fields: &[
            Field {
                name: "primary",
                kind: "*RawWindow",
            },
            Field {
                name: "secondary",
                kind: "*RawWindow",
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
    Schema {
        name: "RawWindow",
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
        name: "GrokRecord",
        fields: &[
            Field {
                name: "ts",
                kind: "string",
            },
            Field {
                name: "msg",
                kind: "string",
            },
            Field {
                name: "ctx",
                kind: "GrokContext",
            },
        ],
    },
    Schema {
        name: "GrokContext",
        fields: &[Field {
            name: "config",
            kind: "GrokConfig",
        }],
    },
    Schema {
        name: "GrokConfig",
        fields: &[
            Field {
                name: "creditUsagePercent",
                kind: "*float64",
            },
            Field {
                name: "currentPeriod",
                kind: "GrokPeriod",
            },
        ],
    },
    Schema {
        name: "GrokPeriod",
        fields: &[
            Field {
                name: "start",
                kind: "string",
            },
            Field {
                name: "end",
                kind: "string",
            },
            Field {
                name: "type",
                kind: "string",
            },
        ],
    },
];
impl GoWire for Event {
    const GO_TYPE: &'static str = "Event";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for RateLimits {
    const GO_TYPE: &'static str = "RateLimits";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for GrokRecord {
    const GO_TYPE: &'static str = "GrokRecord";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
pub fn read_profile(paths: &RuntimePaths, provider: &str, profile: &Path) -> Option<Observation> {
    if !matches!(provider, "codex" | "grok") {
        return None;
    }
    let path = if provider == "codex" {
        newest_rollout(&profile.join("sessions")).ok()??
    } else {
        profile.join("logs/unified.jsonl")
    };
    crate::profile::subscriptions::check_path(paths, &path).ok()?;
    let mut found = None;
    let mut malformed = false;
    scan_reverse(&path, |line| {
        if provider == "codex" {
            if !line
                .windows(b"\"token_count\"".len())
                .any(|part| part == b"\"token_count\"")
            {
                return false;
            }
            let event: Event = match wire::decode(line) {
                Ok(event) => event,
                Err(_) => {
                    malformed = true;
                    return true;
                }
            };
            if event.payload.kind != "token_count" {
                return false;
            }
            let members = match wire::decode_go_json_members(line) {
                Ok(Some(members)) => members,
                _ => {
                    malformed = true;
                    return true;
                }
            };
            let mut raw = None;
            for (name, value) in members {
                if crate::proto::unicode::simple_fold_key(&name)
                    == crate::proto::unicode::simple_fold_key("payload")
                    && let Ok(Some(payload)) = wire::decode_go_json_members(&value)
                {
                    for (name, value) in payload {
                        if crate::proto::unicode::simple_fold_key(&name)
                            == crate::proto::unicode::simple_fold_key("rate_limits")
                        {
                            raw = Some(value);
                        }
                    }
                }
            }
            let Some(raw) = raw else {
                return true;
            };
            let limits: RateLimits = match wire::decode(&raw) {
                Ok(limits) => limits,
                Err(_) => {
                    malformed = true;
                    return true;
                }
            };
            let credits_balance = limits
                .credits
                .as_ref()
                .filter(|v| v.has_credits && !v.unlimited)
                .map(|v| v.balance.clone())
                .unwrap_or_default();
            let (observed, observed_display) = observed_time(&event.timestamp, false);
            found = Some(Observation {
                codex: Some(CodexUsage {
                    primary: limits.primary.and_then(convert_window),
                    secondary: limits.secondary.and_then(convert_window),
                    plan_type: limits.plan_type,
                    credits: limits.credits,
                    credits_balance,
                }),
                grok: None,
                observed,
                observed_display,
            });
            true
        } else {
            let record: GrokRecord = match wire::decode(line) {
                Ok(record) => record,
                Err(_) => return false,
            };
            if record.msg != "billing: fetched credits config" {
                return false;
            }
            let Some(used) = record.ctx.config.used else {
                return false;
            };
            let used = used.clamp(0.0, 100.0);
            let (observed, observed_display) = observed_time(&record.ts, true);
            let period = record.ctx.config.period;
            found = Some(Observation {
                codex: None,
                grok: Some(GrokUsage {
                    used_percent: used,
                    remaining_percent: 100.0 - used,
                    period_start: period.start,
                    period_end: period.end,
                    period_type: period.kind,
                }),
                observed,
                observed_display,
            });
            true
        }
    })
    .ok()?;
    if malformed { None } else { found }
}
fn convert_window(window: RawWindow) -> Option<Window> {
    if window.used_percent.is_nan() {
        return None;
    }
    let used = window.used_percent.clamp(0.0, 100.0);
    Some(Window {
        used_percent: used,
        remaining_percent: 100.0 - used,
        window_minutes: window.window_minutes,
        resets_at: window.resets_at,
    })
}
fn observed_time(raw: &str, trim: bool) -> (Option<Timestamp>, String) {
    let raw = if trim { raw.trim() } else { raw };
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(raw) else {
        return (None, String::new());
    };
    let Ok(time) = Timestamp::from_unix(parsed.timestamp(), parsed.timestamp_subsec_nanos()) else {
        return (None, String::new());
    };
    let display = time::format_with_offset(time, parsed.offset().local_minus_utc(), false)
        .unwrap_or_default();
    (Some(time), display)
}
fn newest_rollout(root: &Path) -> io::Result<Option<PathBuf>> {
    if !std::fs::symlink_metadata(root)?.is_dir() {
        return Ok(None);
    }
    let mut pending = vec![root.to_owned()];
    let mut selected: Option<(SystemTime, PathBuf)> = None;
    while let Some(dir) = pending.pop() {
        let mut entries = std::fs::read_dir(&dir)?.collect::<io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let ty = entry.file_type()?;
            let path = entry.path();
            if ty.is_dir() {
                pending.push(path);
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            if selected.as_ref().is_none_or(|(time, current)| {
                modified > *time
                    || (modified == *time && path.to_string_lossy() > current.to_string_lossy())
            }) {
                selected = Some((modified, path));
            }
        }
    }
    Ok(selected.map(|(_, path)| path))
}
fn scan_reverse(path: &Path, mut visit: impl FnMut(&[u8]) -> bool) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut end = file.metadata()?.len();
    let mut pending = vec![];
    while end > 0 {
        let start = end.saturating_sub(64 * 1024);
        let mut chunk = vec![0; (end - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&pending);
        let mut cursor = chunk.len();
        pending.clear();
        while cursor > 0 {
            let Some(index) = chunk[..cursor].iter().rposition(|byte| *byte == b'\n') else {
                pending.extend_from_slice(&chunk[..cursor]);
                break;
            };
            let line = &chunk[index + 1..cursor];
            if !line.is_empty() && visit(line) {
                return Ok(());
            }
            cursor = index;
            pending.clear();
        }
        end = start;
    }
    if !pending.is_empty() {
        visit(&pending);
    }
    Ok(())
}
#[cfg(test)]
mod tests;
