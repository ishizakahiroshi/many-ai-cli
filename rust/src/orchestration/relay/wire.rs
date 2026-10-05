//! Fixed-Go relay persistence/API schemas. All ingress goes through GoWire so
//! duplicate keys, Unicode/case-folded names, typed nulls, and map replacement
//! behave like encoding/json instead of direct Serde DTO decoding.
use super::{RelayFile, Role, text::Verdict};
use crate::{
    application::relay_program::{RelayControlRequest, RelayStartRequest},
    proto::wire::{Field, GoWire, Schema},
};
use serde::{Deserialize, Deserializer};

/// Maps/slices may be nil in historical Go records or requests. Their owners
/// use empty collections internally, while nullable role entries stay None.
pub(crate) fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

const RELAY_SCHEMAS: &[Schema] = &[
    Schema {
        name: "RelayFile",
        fields: &[
            Field {
                name: "version",
                kind: "int",
            },
            Field {
                name: "orchestration_id",
                kind: "string",
            },
            Field {
                name: "board_path",
                kind: "string",
            },
            Field {
                name: "parent_session_id",
                kind: "int",
            },
            Field {
                name: "parent_started_at",
                kind: "string",
            },
            Field {
                name: "parent_provider",
                kind: "string",
            },
            Field {
                name: "parent_cwd",
                kind: "string",
            },
            Field {
                name: "plan_path",
                kind: "string",
            },
            Field {
                name: "mode",
                kind: "string",
            },
            Field {
                name: "max_rounds",
                kind: "int",
            },
            Field {
                name: "escalate_after",
                kind: "int",
            },
            Field {
                name: "completed_cs",
                kind: "int",
            },
            Field {
                name: "round",
                kind: "int",
            },
            Field {
                name: "final_seen",
                kind: "bool",
            },
            Field {
                name: "state",
                kind: "string",
            },
            Field {
                name: "reason",
                kind: "string",
            },
            Field {
                name: "active_implementer",
                kind: "string",
            },
            Field {
                name: "roles",
                kind: "map[string]RelayRole",
            },
            Field {
                name: "extra",
                kind: "map[string]string",
            },
            Field {
                name: "revoked_child_labels",
                kind: "[]string",
            },
            Field {
                name: "child_cwd",
                kind: "string",
            },
            Field {
                name: "implementation_label",
                kind: "string",
            },
            Field {
                name: "implementation_session_id",
                kind: "int",
            },
            Field {
                name: "implementation_progress_id",
                kind: "int",
            },
            Field {
                name: "impl_done_baseline",
                kind: "int",
            },
            Field {
                name: "strong_label",
                kind: "string",
            },
            Field {
                name: "strong_session_id",
                kind: "int",
            },
            Field {
                name: "strong_progress_id",
                kind: "int",
            },
            Field {
                name: "strong_done_baseline",
                kind: "int",
            },
            Field {
                name: "review_label",
                kind: "string",
            },
            Field {
                name: "review_session_id",
                kind: "int",
            },
            Field {
                name: "review_progress_id",
                kind: "int",
            },
            Field {
                name: "review_done_baseline",
                kind: "int",
            },
            Field {
                name: "review_path",
                kind: "string",
            },
            Field {
                name: "last_verdict",
                kind: "*RelayVerdict",
            },
            Field {
                name: "worktree_path",
                kind: "string",
            },
            Field {
                name: "branch",
                kind: "string",
            },
            Field {
                name: "base_commit",
                kind: "string",
            },
            Field {
                name: "last_reviewed_commit",
                kind: "string",
            },
            Field {
                name: "events",
                kind: "[]RelayEvent",
            },
            Field {
                name: "updated_at",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "RelayRole",
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
    Schema {
        name: "RelayVerdict",
        fields: &[
            Field {
                name: "kind",
                kind: "string",
            },
            Field {
                name: "must",
                kind: "int",
            },
            Field {
                name: "should",
                kind: "int",
            },
            Field {
                name: "file",
                kind: "string",
            },
            Field {
                name: "reason",
                kind: "string",
            },
        ],
    },
    Schema {
        name: "RelayStartRequest",
        fields: &[
            Field {
                name: "plan_path",
                kind: "string",
            },
            Field {
                name: "max_rounds",
                kind: "int",
            },
            Field {
                name: "mode",
                kind: "string",
            },
            Field {
                name: "roles",
                kind: "map[string]*RelayRole",
            },
            Field {
                name: "escalate_after",
                kind: "int",
            },
            Field {
                name: "extra",
                kind: "map[string]string",
            },
            Field {
                name: "acknowledge_child_full_bypass",
                kind: "bool",
            },
        ],
    },
    Schema {
        name: "RelayControlRequest",
        fields: &[Field {
            name: "orchestration_id",
            kind: "string",
        }],
    },
];

impl GoWire for RelayFile {
    const GO_TYPE: &'static str = "RelayFile";
    const SCHEMAS: &'static [Schema] = RELAY_SCHEMAS;
}

impl GoWire for Role {
    const GO_TYPE: &'static str = "RelayRole";
    const SCHEMAS: &'static [Schema] = RELAY_SCHEMAS;
}

impl GoWire for Verdict {
    const GO_TYPE: &'static str = "RelayVerdict";
    const SCHEMAS: &'static [Schema] = RELAY_SCHEMAS;
}

impl GoWire for RelayStartRequest {
    const GO_TYPE: &'static str = "RelayStartRequest";
    const SCHEMAS: &'static [Schema] = RELAY_SCHEMAS;
}

impl GoWire for RelayControlRequest {
    const GO_TYPE: &'static str = "RelayControlRequest";
    const SCHEMAS: &'static [Schema] = RELAY_SCHEMAS;
}

/// Go append(nil, events...) serializes an empty event history as null.
pub fn serialize_events<S: serde::Serializer>(
    events: &[crate::proto::RelayEvent],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if events.is_empty() {
        serializer.serialize_none()
    } else {
        serde::Serialize::serialize(events, serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{decode_wire, wire::decode_http_json};
    use serde_json::json;

    #[test]
    fn historical_record_fields_fold_and_future_version_is_not_rejected() {
        let file: RelayFile = decode_wire(br#"{
            "VERSION":77,"ORCHESTRATION_ID":"r17-123456","BOARD_PATH":"board/board.md",
            "PARENT_SESSION_ID":17,"PARENT_STARTED_AT":"2026-01-01T01:02:03Z",
            "PARENT_PROVIDER":"codex","PARENT_CWD":"repo","PLAN_PATH":"plan.md",
            "MODE":"worktree","MAX_ROUNDS":9,"ESCALATE_AFTER":5,
            "COMPLETED_CS":2,"ROUND":3,"FINAL_SEEN":true,"STATE":"stopped",
            "REASON":"hub_restart","ACTIVE_IMPLEMENTER":"implementation-strong",
            "ROLES":{"implementation":{"PROVIDER":"codex","MODEL":"m1","SUBSCRIPTION":"work","EFFORT":"high","EXECUTION_MODE":"headless","PERMISSION_PRESET":"attended"},"review":{"provider":"claude","model":"m2"}},
            "EXTRA":{"implementation":"specific instruction"},"CHILD_CWD":"relay-tree",
            "IMPLEMENTATION_LABEL":"impl-label","IMPLEMENTATION_SESSION_ID":20,
            "IMPLEMENTATION_PROGRESS_ID":10,"IMPL_DONE_BASELINE":4,
            "STRONG_LABEL":"strong-label","STRONG_SESSION_ID":21,
            "STRONG_PROGRESS_ID":11,"STRONG_DONE_BASELINE":5,
            "REVIEW_LABEL":"review-label","REVIEW_SESSION_ID":22,
            "REVIEW_PROGRESS_ID":12,"REVIEW_DONE_BASELINE":6,
            "REVIEW_PATH":"board/review.md",
            "LAST_VERDICT":{"KIND":"findings","MUST":1,"SHOULD":2,"FILE":"board/review.md","REASON":"details"},
            "WORKTREE_PATH":"relay-tree","BRANCH":"relay/topic","BASE_COMMIT":"base123",
            "LAST_REVIEWED_COMMIT":"reviewed123",
            "EVENTS":[{"AT":"2026-01-01T01:02:03Z","KIND":"review_started","C":3,"ROUND":2,"TEXT":"reviewing","REVIEW_PATH":"board/review.md","COMMIT":"abc123","FILES_CHANGED":7}],
            "UPDATED_AT":"2026-01-01T01:02:04Z","future_field":{"unknown":1e1000}
        }"#).unwrap();
        assert_eq!(file.version, 77);
        assert_eq!(file.orchestration_id, "r17-123456");
        assert_eq!(file.board_path, "board/board.md");
        assert_eq!(
            (file.parent_session_id, file.max_rounds, file.escalate_after),
            (17, 9, 5)
        );
        assert_eq!(
            (
                file.parent_started_at.as_str(),
                file.parent_provider.as_str(),
                file.parent_cwd.as_str()
            ),
            ("2026-01-01T01:02:03Z", "codex", "repo")
        );
        assert_eq!(
            (
                file.plan_path.as_str(),
                file.mode.as_str(),
                file.active_implementer.as_str()
            ),
            ("plan.md", "worktree", "implementation-strong")
        );
        assert_eq!(
            (file.completed_cs, file.round, file.final_seen),
            (2, 3, true)
        );
        assert_eq!(
            (file.state.as_str(), file.reason.as_str()),
            ("stopped", "hub_restart")
        );
        assert_eq!(
            file.roles["implementation"],
            Role {
                provider: "codex".into(),
                model: "m1".into(),
                subscription: "work".into(),
                effort: "high".into(),
                execution_mode: "headless".into(),
                permission_preset: "attended".into()
            }
        );
        assert_eq!(file.roles["review"].model, "m2");
        assert_eq!(file.extra["implementation"], "specific instruction");
        assert_eq!(file.child_cwd, "relay-tree");
        assert_eq!(
            (
                file.implementation_label.as_str(),
                file.implementation_session_id,
                file.implementation_progress_id,
                file.impl_done_baseline
            ),
            ("impl-label", 20, 10, 4)
        );
        assert_eq!(
            (
                file.strong_label.as_str(),
                file.strong_session_id,
                file.strong_progress_id,
                file.strong_done_baseline
            ),
            ("strong-label", 21, 11, 5)
        );
        assert_eq!(
            (
                file.review_label.as_str(),
                file.review_session_id,
                file.review_progress_id,
                file.review_done_baseline
            ),
            ("review-label", 22, 12, 6)
        );
        assert_eq!(file.review_path, "board/review.md");
        assert_eq!(
            file.last_verdict,
            Some(Verdict {
                kind: "findings".into(),
                must: 1,
                should: 2,
                file: "board/review.md".into(),
                reason: "details".into()
            })
        );
        assert_eq!(
            (
                file.worktree_path.as_str(),
                file.branch.as_str(),
                file.base_commit.as_str(),
                file.last_reviewed_commit.as_str()
            ),
            ("relay-tree", "relay/topic", "base123", "reviewed123")
        );
        assert_eq!(
            serde_json::to_value(&file.events).unwrap(),
            json!([{"at":"2026-01-01T01:02:03Z","kind":"review_started","c":3,"round":2,"text":"reviewing","review_path":"board/review.md","commit":"abc123","files_changed":7}])
        );
        assert_eq!(file.updated_at, "2026-01-01T01:02:04Z");
    }

    #[test]
    fn request_fields_nullable_roles_and_map_entry_replacement_match_go() {
        let request:RelayStartRequest=decode_http_json(br#"{
            "PLAN_PATH":"plan.md","plan_path":null,"MAX_ROUNDS":4,"MODE":"same-tree",
            "ESCALATE_AFTER":3,"ACKNOWLEDGE_CHILD_FULL_BYPASS":true,
            "ROLES":{"implementation":{"provider":"codex","model":"old","effort":"high"},"review":null},
            "roles":{"implementation":{"MODEL":"new","SUBSCRIPTION":"work","EXECUTION_MODE":"interactive","PERMISSION_PRESET":"attended"},"Review":{"provider":"claude"}},
            "EXTRA":{"implementation":"old","review":"retain"},"extra":{"implementation":null}
        } ignored trailing HTTP bytes"#).unwrap();
        assert_eq!(request.plan_path, "plan.md");
        assert_eq!(
            (
                request.max_rounds,
                request.escalate_after,
                request.acknowledge_child_full_bypass
            ),
            (4, 3, true)
        );
        assert_eq!(request.mode, "same-tree");
        assert_eq!(
            request.roles["implementation"],
            Some(Role {
                model: "new".into(),
                subscription: "work".into(),
                execution_mode: "interactive".into(),
                permission_preset: "attended".into(),
                ..Role::default()
            })
        );
        assert_eq!(request.roles["review"], None);
        assert_eq!(request.roles["Review"].as_ref().unwrap().provider, "claude");
        assert_eq!(request.extra["implementation"], "");
        assert_eq!(request.extra["review"], "retain");
    }

    #[test]
    fn null_maps_slices_structs_and_pointer_members_follow_destination_types() {
        let request:RelayStartRequest=decode_http_json(br#"{"roles":{"implementation":{"provider":"codex"}},"roles":null,"extra":null,"plan_path":null,"max_rounds":null,"acknowledge_child_full_bypass":null}"#).unwrap();
        assert!(request.roles.is_empty());
        assert!(request.extra.is_empty());
        assert!(request.plan_path.is_empty());
        assert_eq!(request.max_rounds, 0);
        assert!(!request.acknowledge_child_full_bypass);
        let file: RelayFile = decode_wire(
            br#"{"roles":null,"extra":null,"events":null,"last_verdict":null,"version":null}"#,
        )
        .unwrap();
        assert!(file.roles.is_empty());
        assert!(file.extra.is_empty());
        assert!(file.events.is_empty());
        assert!(file.last_verdict.is_none());
        assert_eq!(file.version, 0);
        let file:RelayFile=decode_wire(br#"{"roles":{"implementation":null},"last_verdict":{"kind":"findings","must":2},"LAST_VERDICT":{"should":3},"events":[{"kind":"started","c":2}],"EVENTS":[{"round":3}]}"#).unwrap();
        assert_eq!(file.roles["implementation"], Role::default());
        assert_eq!(
            file.last_verdict,
            Some(Verdict {
                kind: "findings".into(),
                must: 2,
                should: 3,
                ..Verdict::default()
            })
        );
        assert_eq!(
            (
                file.events[0].kind.as_str(),
                file.events[0].c,
                file.events[0].round
            ),
            ("started", 2, 3)
        );
        let file:RelayFile=decode_wire(br#"{"last_verdict":{"kind":"findings","must":2},"LAST_VERDICT":null,"last_verdict":{"should":3}}"#).unwrap();
        assert_eq!(
            file.last_verdict,
            Some(Verdict {
                should: 3,
                ..Verdict::default()
            })
        );
        assert!(decode_wire::<RelayFile>(b"null").unwrap().roles.is_empty());
        assert!(
            decode_http_json::<RelayStartRequest>(b"null")
                .unwrap()
                .roles
                .is_empty()
        );
    }

    #[test]
    fn typed_errors_survive_valid_later_duplicates_and_persistence_requires_end_of_input() {
        for bytes in [
            &br#"{"max_rounds":"bad","MAX_ROUNDS":4}"#[..],
            &br#"{"roles":{"implementation":{"model":7}},"roles":{"implementation":{"model":"valid"}}}"#[..],
            &br#"{"roles":{"review":true}}"#[..],
            &br#"{"roles":[]}"#[..],
            &br#"{"extra":{"implementation":4}}"#[..],
            &br#"{"acknowledge_child_full_bypass":"true"}"#[..],
        ] { assert!(decode_http_json::<RelayStartRequest>(bytes).is_err(),"{}",String::from_utf8_lossy(bytes)); }
        for bytes in [
            &br#"{"last_verdict":{"must":"1"}}"#[..],
            &br#"{"events":[{"round":false}]}"#[..],
            &br#"{"strong_progress_id":9223372036854775808}"#[..],
            &br#"{"version":1.0}"#[..],
            &br#"{"roles":{"review":false}}"#[..],
            &br#"{} {}"#[..],
        ] {
            assert!(
                decode_wire::<RelayFile>(bytes).is_err(),
                "{}",
                String::from_utf8_lossy(bytes)
            );
        }
        assert!(decode_wire::<RelayFile>(br#"{"version":-23}"#).is_ok());
    }

    #[test]
    fn control_and_role_use_folded_null_aware_wire_boundary() {
        let control: RelayControlRequest =
            decode_http_json(br#"{"ORCHESTRATION_ID":"first","orchestration_id":null} trailing"#)
                .unwrap();
        assert_eq!(control.orchestration_id, "first");
        assert!(
            decode_http_json::<RelayControlRequest>(b"null")
                .unwrap()
                .orchestration_id
                .is_empty()
        );
        assert!(decode_http_json::<RelayControlRequest>(br#"{"orchestration_id":7}"#).is_err());
        let role:Role=decode_wire(br#"{"PROVIDER":"codex","provider":null,"MODEL":"m","SUBSCRIPTION":null,"EFFORT":"high","EXECUTION_MODE":"interactive","PERMISSION_PRESET":"attended"}"#).unwrap();
        assert_eq!(
            role,
            Role {
                provider: "codex".into(),
                model: "m".into(),
                effort: "high".into(),
                execution_mode: "interactive".into(),
                permission_preset: "attended".into(),
                ..Role::default()
            }
        );
        let role: Role = decode_wire(
            "{\"EXECUTION_MODE\":\"headless\",\"PERMISSION_PREſET\":\"attended\"}".as_bytes(),
        )
        .unwrap();
        assert_eq!(role.permission_preset, "attended");
        assert_eq!(role.execution_mode, "headless");
    }
}
