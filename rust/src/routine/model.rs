//! Routine records keep the frozen Go JSON shape and historical read semantics.
use super::schedule::Schedule;
use crate::proto::wire::{Field, GoWire, Schema};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
fn null_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
fn zero<T: Default + PartialEq>(v: &T) -> bool {
    v == &T::default()
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Definition {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub provider: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    pub prompt: String,
    pub enabled: bool,
    pub schedule: Schedule,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub next_run_at: String,
    pub created_at: String,
    pub updated_at: String,
    pub completion_mode: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    pub id: String,
    pub routine_id: String,
    pub routine_name: String,
    pub cwd: String,
    pub provider: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    pub prompt: String,
    pub trigger: String,
    pub status: String,
    #[serde(skip_serializing_if = "zero")]
    pub session_id: i64,
    #[serde(skip_serializing_if = "zero")]
    pub session_db_id: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hub_instance_id: String,
    pub started_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub finished_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub result: String,
    #[serde(skip_serializing_if = "zero")]
    pub result_truncated: bool,
    pub result_available: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub request_id: String,
    pub session_label: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct RoutineFile {
    pub version: i64,
    #[serde(deserialize_with = "null_default")]
    pub routines: Vec<Definition>,
    #[serde(deserialize_with = "null_default")]
    pub runs: Vec<Run>,
    #[serde(
        deserialize_with = "null_default",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub requests: BTreeMap<String, String>,
}
impl Default for RoutineFile {
    fn default() -> Self {
        Self {
            version: 1,
            routines: vec![],
            runs: vec![],
            requests: BTreeMap::new(),
        }
    }
}
const SCHEMAS: &[Schema] = &[
    Schema {
        name: "RoutineDefinition",
        fields: &[
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "name",
                kind: "string",
            },
            Field {
                name: "cwd",
                kind: "string",
            },
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "model",
                kind: "string",
            },
            Field {
                name: "prompt",
                kind: "string",
            },
            Field {
                name: "enabled",
                kind: "bool",
            },
            Field {
                name: "schedule",
                kind: "RoutineSchedule",
            },
            Field {
                name: "next_run_at",
                kind: "string",
            },
            Field {
                name: "created_at",
                kind: "string",
            },
            Field {
                name: "updated_at",
                kind: "string",
            },
            Field {
                name: "completion_mode",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "RoutineRun",
        fields: &[
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "routine_id",
                kind: "string",
            },
            Field {
                name: "routine_name",
                kind: "string",
            },
            Field {
                name: "cwd",
                kind: "string",
            },
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "model",
                kind: "string",
            },
            Field {
                name: "prompt",
                kind: "string",
            },
            Field {
                name: "trigger",
                kind: "string",
            },
            Field {
                name: "status",
                kind: "string",
            },
            Field {
                name: "session_id",
                kind: "int",
            },
            Field {
                name: "session_db_id",
                kind: "int64",
            },
            Field {
                name: "hub_instance_id",
                kind: "string",
            },
            Field {
                name: "started_at",
                kind: "string",
            },
            Field {
                name: "updated_at",
                kind: "string",
            },
            Field {
                name: "finished_at",
                kind: "string",
            },
            Field {
                name: "summary",
                kind: "string",
            },
            Field {
                name: "result",
                kind: "string",
            },
            Field {
                name: "result_truncated",
                kind: "bool",
            },
            Field {
                name: "result_available",
                kind: "bool",
            },
            Field {
                name: "error",
                kind: "string",
            },
            Field {
                name: "request_id",
                kind: "string",
            },
            Field {
                name: "session_label",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "RoutineSchedule",
        fields: &[
            Field {
                name: "kind",
                kind: "string",
            },
            Field {
                name: "time",
                kind: "string",
            },
            Field {
                name: "timezone",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "RoutineFile",
        fields: &[
            Field {
                name: "version",
                kind: "int",
            },
            Field {
                name: "routines",
                kind: "[]RoutineDefinition",
            },
            Field {
                name: "runs",
                kind: "[]RoutineRun",
            },
            Field {
                name: "requests",
                kind: "map[string]string",
            },
        ],
    },
];
impl GoWire for Definition {
    const GO_TYPE: &'static str = "RoutineDefinition";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for Run {
    const GO_TYPE: &'static str = "RoutineRun";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for RoutineFile {
    const GO_TYPE: &'static str = "RoutineFile";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
pub fn active(status: &str) -> bool {
    matches!(status, "starting" | "running" | "waiting")
}
pub(crate) fn time(at: crate::proto::time::Timestamp) -> Result<String, super::store::Error> {
    crate::proto::time::format_with_offset(at, 0, true).map_err(|_| super::store::Error::Operation)
}

impl RoutineFile {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let mut value: Self = crate::proto::decode_wire(bytes)?;
        // Go unmarshals into Version=1, not a zero-valued record. Its scalar
        // null keeps that initial value (and any earlier non-null duplicate).
        // Use the shared raw-member reader solely to recognize this initial
        // default; the shared typed decoder still validates every occurrence.
        let members = crate::proto::wire::decode_go_json_members(bytes)?.unwrap_or_default();
        let non_null: Vec<_> = members
            .into_iter()
            .filter(|(_, raw)| raw.trim_ascii() != b"null")
            .collect();
        if crate::proto::wire::last_go_raw_field(&non_null, "version").is_none() {
            value.version = 1;
        }
        Ok(value)
    }
}
