use super::cli::{NativeSubscriptionCli, SubscriptionCli};
use crate::{
    process::{InputSender, ManagedProcess, OutputStream, ProcessEvent, SpawnOptions},
    proto::core::{CoreFuture, TaskCancellation},
};
use serde::{Deserialize, Serialize};
use std::{io, path::Path, time::Duration};
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Window {
    pub used_percent: f64,
    pub remaining_percent: f64,
    #[serde(skip_serializing_if = "is_zero")]
    pub window_minutes: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub resets_at: i64,
}
fn is_zero(value: &i64) -> bool {
    *value == 0
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Credits {
    pub has_credits: bool,
    pub unlimited: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub balance: String,
}
#[derive(Clone, Default, Serialize, PartialEq)]
pub struct CodexUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<Window>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<Window>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub plan_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<Credits>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub credits_balance: String,
}
pub trait SubscriptionUsageCli: SubscriptionCli {
    fn codex_usage<'a>(
        &'a self,
        profile: &'a Path,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, io::Result<CodexUsage>>;
}
impl SubscriptionUsageCli for NativeSubscriptionCli {
    fn codex_usage<'a>(
        &'a self,
        profile: &'a Path,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, io::Result<CodexUsage>> {
        Box::pin(async move {
            #[cfg(all(test, windows))]
            phase_diagnostics::event(phase_diagnostics::Stage::CallEntered, -1, 0);
            if cancel.token().is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "codex subscription usage unavailable",
                ));
            }
            if profile.as_os_str().is_empty() || !profile.is_dir() {
                return Err(unavailable());
            }
            let mut plan = self
                .command("codex", profile, Duration::from_secs(15))
                .map_err(|_| unavailable())?;
            let args = vec!["app-server".into(), "--stdio".into()];
            let (executable, args) = super::cli::vendor_command(
                &self.environment,
                self.lookup("codex").map_err(|_| unavailable())?,
                args,
            );
            plan.executable = executable.into();
            plan.args = args;
            plan.timeout = Duration::ZERO;
            let (mut child, input, mut events) =
                ManagedProcess::spawn_interactive_owned_with_options(
                    plan,
                    1024,
                    SpawnOptions {
                        no_window: true,
                        stderr_null: true,
                        env_clear: true,
                        ..Default::default()
                    },
                );
            let operation = async {
                let mut buffer = vec![];
                rpc(&input,&mut events,&mut buffer,0,"initialize",Some(serde_json::json!({"clientInfo":{"name":"many_ai_cli","title":"many-ai-cli","version":"1"}}))).await?;
                #[cfg(all(test, windows))]
                phase_diagnostics::event(phase_diagnostics::Stage::NotificationRequested, -1, 0);
                input
                    .write(b"{\"method\":\"initialized\",\"params\":{}}\n".to_vec())
                    .await
                    .map_err(|_| unavailable())?;
                #[cfg(all(test, windows))]
                phase_diagnostics::event(phase_diagnostics::Stage::NotificationAcknowledged, -1, 0);
                let account = rpc(
                    &input,
                    &mut events,
                    &mut buffer,
                    1,
                    "account/read",
                    Some(serde_json::json!({"refreshToken":false})),
                )
                .await?;
                validate_account(&account)?;
                let limits = rpc(
                    &input,
                    &mut events,
                    &mut buffer,
                    2,
                    "account/rateLimits/read",
                    None,
                )
                .await?;
                parse_usage(&account, &limits)
            };
            let result = tokio::select! {result=tokio::time::timeout(Duration::from_secs(15),operation)=>result.unwrap_or_else(|_|Err(io::Error::new(io::ErrorKind::TimedOut,"codex subscription usage unavailable"))),_=cancel.token().cancelled()=>Err(io::Error::new(io::ErrorKind::Interrupted,"codex subscription usage unavailable"))};
            #[cfg(all(test, windows))]
            phase_diagnostics::event(phase_diagnostics::Stage::CleanupStarted, -1, 0);
            child.close();
            let cleanup_result = child.wait().await;
            #[cfg(all(test, windows))]
            phase_diagnostics::cleanup(&cleanup_result);
            drop(cleanup_result);
            result
        })
    }
}
fn unavailable() -> io::Error {
    io::Error::other("codex subscription usage unavailable")
}
async fn rpc(
    input: &InputSender,
    events: &mut tokio::sync::broadcast::Receiver<ProcessEvent>,
    buffer: &mut Vec<u8>,
    id: i64,
    method: &str,
    params: Option<serde_json::Value>,
) -> io::Result<Vec<u8>> {
    let mut message = serde_json::json!({"method":method,"id":id});
    if let Some(params) = params {
        message["params"] = params;
    }
    let mut encoded = serde_json::to_vec(&message).map_err(|_| unavailable())?;
    encoded.push(b'\n');
    #[cfg(all(test, windows))]
    phase_diagnostics::event(phase_diagnostics::Stage::WriteRequested, id, encoded.len());
    input.write(encoded).await.map_err(|_| unavailable())?;
    #[cfg(all(test, windows))]
    phase_diagnostics::event(phase_diagnostics::Stage::WriteAcknowledged, id, 0);
    loop {
        while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
            if end >= 4 * 1024 * 1024 {
                return Err(unavailable());
            }
            let line: Vec<u8> = buffer.drain(..=end).collect();
            #[cfg(all(test, windows))]
            phase_diagnostics::event(phase_diagnostics::Stage::ResponseLine, id, line.len());
            let members = match crate::proto::wire::decode_go_json_members(&line) {
                Ok(Some(members)) => members,
                _ => {
                    #[cfg(all(test, windows))]
                    phase_diagnostics::event(phase_diagnostics::Stage::InvalidJson, id, 0);
                    continue;
                }
            };
            let mut reply_id = None;
            let mut invalid_id = false;
            for (key, raw) in &members {
                if crate::proto::unicode::simple_fold_key(key)
                    == crate::proto::unicode::simple_fold_key("id")
                {
                    match serde_json::from_slice::<Option<i64>>(raw) {
                        Ok(id) => reply_id = id,
                        Err(_) => {
                            invalid_id = true;
                            break;
                        }
                    }
                }
            }
            if invalid_id || reply_id != Some(id) {
                #[cfg(all(test, windows))]
                phase_diagnostics::event(phase_diagnostics::Stage::UnmatchedId, id, 0);
                continue;
            }
            #[cfg(all(test, windows))]
            phase_diagnostics::event(phase_diagnostics::Stage::MatchingResponseId, id, 0);
            if crate::proto::wire::last_go_raw_field(&members, "error").is_some() {
                return Err(unavailable());
            }
            return crate::proto::wire::last_go_raw_field(&members, "result")
                .map(Vec::from)
                .ok_or_else(unavailable);
        }
        if buffer.len() >= 4 * 1024 * 1024 {
            return Err(unavailable());
        }
        match events.recv().await {
            Ok(ProcessEvent::Output {
                stream: OutputStream::Stdout,
                bytes,
            }) => {
                #[cfg(all(test, windows))]
                phase_diagnostics::event(phase_diagnostics::Stage::StdoutBytes, id, bytes.len());
                buffer.extend(bytes);
            }
            #[cfg(all(test, windows))]
            Ok(ProcessEvent::Started { .. }) => {
                phase_diagnostics::event(phase_diagnostics::Stage::ProcessStarted, id, 0);
            }
            Ok(_) => {}
            Err(_) => return Err(unavailable()),
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawAccount {
    account: Option<Account>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Account {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "planType")]
    plan_type: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawLimits {
    #[serde(rename = "rateLimits")]
    limits: Option<Limits>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Limits {
    primary: Option<RawWindow>,
    secondary: Option<RawWindow>,
    credits: Option<RawCredits>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawWindow {
    #[serde(rename = "usedPercent")]
    used: Option<f64>,
    #[serde(rename = "windowDurationMins")]
    minutes: i64,
    #[serde(rename = "resetsAt")]
    resets: i64,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct RawCredits {
    #[serde(rename = "hasCredits")]
    has: bool,
    unlimited: bool,
    balance: String,
}
use crate::proto::wire::{Field, GoWire, Schema};
const SCHEMAS: &[Schema] = &[
    Schema {
        name: "RawAccount",
        fields: &[Field {
            name: "account",
            kind: "*Account",
        }],
    },
    Schema {
        name: "Account",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "planType",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "RawLimits",
        fields: &[Field {
            name: "rateLimits",
            kind: "*Limits",
        }],
    },
    Schema {
        name: "Limits",
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
                kind: "*RawCredits",
            },
        ],
    },
    Schema {
        name: "RawWindow",
        fields: &[
            Field {
                name: "usedPercent",
                kind: "*float64",
            },
            Field {
                name: "windowDurationMins",
                kind: "int",
            },
            Field {
                name: "resetsAt",
                kind: "int64",
            },
        ],
    },
    Schema {
        name: "RawCredits",
        fields: &[
            Field {
                name: "hasCredits",
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
];
impl GoWire for RawAccount {
    const GO_TYPE: &'static str = "RawAccount";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for RawLimits {
    const GO_TYPE: &'static str = "RawLimits";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
fn validate_account(raw: &[u8]) -> io::Result<Account> {
    let account: RawAccount = crate::proto::wire::decode(raw).map_err(|_| unavailable())?;
    account
        .account
        .filter(|account| account.kind == "chatgpt")
        .ok_or_else(unavailable)
}
pub fn parse_usage(account: &[u8], limits: &[u8]) -> io::Result<CodexUsage> {
    let account = validate_account(account)?;
    let limits: RawLimits = crate::proto::wire::decode(limits).map_err(|_| unavailable())?;
    let limits = limits.limits.ok_or_else(unavailable)?;
    let window = |value: Option<RawWindow>| -> Option<Window> {
        let value = value?;
        let used = value.used?;
        if !(0.0..=100.0).contains(&used) || value.minutes <= 0 {
            return None;
        }
        Some(Window {
            used_percent: used,
            remaining_percent: 100.0 - used,
            window_minutes: value.minutes,
            resets_at: value.resets,
        })
    };
    let primary = window(limits.primary);
    let secondary = window(limits.secondary);
    if primary.is_none() && secondary.is_none() {
        return Err(unavailable());
    }
    let credits = limits.credits.map(|value| Credits {
        has_credits: value.has,
        unlimited: value.unlimited,
        balance: value.balance,
    });
    let credits_balance = credits
        .as_ref()
        .filter(|credits| credits.has_credits && !credits.unlimited)
        .map(|credits| credits.balance.clone())
        .unwrap_or_default();
    Ok(CodexUsage {
        primary,
        secondary,
        plan_type: account.plan_type,
        credits,
        credits_balance,
    })
}

#[cfg(all(test, windows))]
mod phase_diagnostics;
#[cfg(test)]
mod tests;
