//! Synthetic C1 contract tests. No provider, network, production files or SQLite.
use many_ai_cli::{
    process,
    proto::{self, core::*},
};
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

#[test]
fn storage_public_method_inventory_is_covered() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("../inventory/core.json")).unwrap();
    let source = include_str!("../src/proto/core.rs");
    let storage = source
        .split("pub trait SessionStorage: Send + Sync {")
        .nth(1)
        .unwrap()
        .split("\n}\n")
        .next()
        .unwrap();
    let methods = oracle["storage_public_method_inventory"]
        .as_array()
        .unwrap();
    assert_eq!(methods.len(), 34);
    for method in methods {
        let name = method["proposed_rust_method"].as_str().unwrap();
        assert!(
            storage.contains(&format!("fn {name}(")),
            "missing baseline Store method {name}"
        );
    }
    // Compile-time object safety: C3 can use a shared trait object, not a second DB.
    fn accepts_repository(_: &dyn SessionStorage) {}
    let _ = accepts_repository;
}

#[test]
fn session_snapshot_preserves_baseline_fields_and_hides_internal_identity() {
    let session = SessionSnapshot {
        id: LiveSessionId(7),
        provider: "codex".into(),
        display: "Codex".into(),
        cwd: "/synthetic/repo".into(),
        label: "edited".into(),
        launch_label: "routine:one".into(),
        handoff_from: LiveSessionId(4),
        parent_session_id: LiveSessionId(2),
        execution_mode: "headless".into(),
        permission_mode: "bounded".into(),
        ..Default::default()
    };
    let value = serde_json::to_value(session).unwrap();
    assert_eq!(value["id"], 7);
    assert_eq!(value["launch_label"], "routine:one");
    assert_eq!(value["label"], "edited");
    assert_eq!(value["handoff_from"], 4);
    assert_eq!(value["parent_session_id"], 2);
    assert_eq!(
        value["NormalWorktree"],
        json!({"Path":"","ParentDir":"","Branch":"","Created":false})
    );
    assert_eq!(value["WorktreeCleanup"], "");
    assert_eq!(
        value["activity"],
        json!({"output_idle":false,"workflow_active":false,"awaiting_user":false,"awaiting_approval":false})
    );
    for hidden in [
        "home_dir",
        "codex_home",
        "token",
        "StoreID",
        "pendingApproval",
        "inputMu",
        "approval_action_reserved",
        "subagentReaderState",
    ] {
        assert!(value.get(hidden).is_none(), "leaked {hidden}");
    }
    assert!(value.get("relays").is_none());
}

#[test]
fn storage_rows_preserve_go_spelling_omission_and_nil_slices() {
    let chat = ChatMessage {
        id: 1,
        session_id: DbSessionId(44),
        live_session_id: LiveSessionId(7),
        raw_text: "hello".into(),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(chat).unwrap(),
        json!({"id":1,"session_db_id":44,"session_id":7,"ts":"","role":"","kind":"","rawText":"hello"})
    );
    let null: UsageSummary = serde_json::from_value(json!({"providers":null})).unwrap();
    let empty: UsageSummary = serde_json::from_value(json!({"providers":[]})).unwrap();
    assert_eq!(
        serde_json::to_value(null).unwrap()["providers"],
        json!(null)
    );
    assert_eq!(serde_json::to_value(empty).unwrap()["providers"], json!([]));
    let row: ApprovalRow =
        serde_json::from_value(json!({"source_epoch":null,"options":null})).unwrap();
    assert_eq!(row.source_epoch, ApprovalSourceEpoch(0));
    assert!(row.options.is_empty());
    assert_eq!(
        serde_json::to_value(row).unwrap(),
        json!({"id":0,"session_db_id":0,"session_id":0,"sig":"","state":""})
    );
    assert_eq!(
        serde_json::to_value(ResetResult::default()).unwrap(),
        json!({"sessions":0,"events":0,"messages":0,"approvals":0,"attachments":0,"preserved_sessions":0})
    );
}

#[test]
fn wire_claims_do_not_grant_human_origin_or_internal_permissions() {
    let request: ChildSpawnRequest = serde_json::from_value(json!({"origin":"ui","allowed_tools":["Bash(*)"],"grant_folder_trust":true,"same_tree":false,"remember_permission":false})).unwrap();
    assert_eq!(request.same_tree, Some(false));
    assert_eq!(request.remember_permission, Some(false));
    assert_eq!(
        request.verify_origin(None),
        Err(SessionError::AuthenticationExpired)
    );
    let encoded = serde_json::to_value(request).unwrap();
    assert!(encoded.get("allowed_tools").is_none());
    assert!(encoded.get("grant_folder_trust").is_none());
    let omitted: ChildSpawnRequest = serde_json::from_value(json!({})).unwrap();
    let null: ChildSpawnRequest = serde_json::from_value(json!({"same_tree":null})).unwrap();
    assert_eq!(omitted.same_tree, None);
    assert_eq!(null.same_tree, None);
    assert_eq!(
        omitted.verify_origin(None).unwrap(),
        VerifiedSpawnOrigin::Autonomous
    );
    let invalid: ChildSpawnRequest = serde_json::from_value(json!({"origin":"future-ui"})).unwrap();
    assert!(matches!(
        invalid.verify_origin(None),
        Err(SessionError::InvalidRequest(_))
    ));
}

#[test]
fn confirmation_absent_empty_false_are_distinct() {
    let absent: SpawnConfirmationResponse = serde_json::from_value(json!({})).unwrap();
    let explicit: SpawnConfirmationResponse = serde_json::from_value(json!({"effort":"","execution_mode":"","permission_preset":"","remember_permission":false,"grant_folder_trust":false})).unwrap();
    assert_eq!(absent.effort, None);
    assert_eq!(explicit.effort, Some(String::new()));
    assert_eq!(explicit.execution_mode, Some(String::new()));
    assert_eq!(explicit.permission_preset, Some(String::new()));
    assert_eq!(explicit.remember_permission, Some(false));
    assert_eq!(explicit.grant_folder_trust, Some(false));
}

#[test]
fn approval_identity_ignores_shape_and_preserves_record_immutability() {
    let identity = CandidateIdentity {
        key: "candidate".into(),
        shape: "old shape".into(),
        source_epoch: ApprovalSourceEpoch(2),
    };
    assert!(identity.same_candidate(&CandidateIdentity {
        shape: "new shape".into(),
        ..identity.clone()
    }));
    assert!(!identity.same_candidate(&CandidateIdentity {
        source_epoch: ApprovalSourceEpoch(3),
        ..identity.clone()
    }));
    let data = ApprovalRecordData {
        candidate: identity,
        sig: "ledger-ref".into(),
        origin: "native".into(),
        source: "go_vt".into(),
        kind: "native".into(),
        block: String::new(),
        question: "Proceed?".into(),
        context: String::new(),
        options: vec![],
        summary: proto::ApprovalSummary::default(),
        detected_at: UNIX_EPOCH,
    };
    let record = ImmutableApprovalRecord::new(data.clone());
    let snapshot = record.clone();
    let mut replacement = data;
    replacement.question = "New candidate?".into();
    let replacement = ImmutableApprovalRecord::new(replacement);
    assert_eq!(snapshot.data().question, "Proceed?");
    assert_eq!(record.data().question, "Proceed?");
    assert_eq!(replacement.data().question, "New candidate?");
    assert_eq!(
        serde_json::to_value(ApprovalCloseReason::HistoryReset).unwrap(),
        "history_reset"
    );
}

#[test]
fn activity_idle_and_display_are_separate() {
    let mut activity = proto::SessionActivity {
        output_idle: true,
        awaiting_approval: true,
        ..Default::default()
    };
    activity.normalize();
    assert!(activity.awaiting_user);
    assert!(activity.is_idle());
    assert_eq!(activity.display_state(), "waiting");
    activity.workflow_active = true;
    assert!(!activity.is_idle());
    assert_eq!(activity.display_state(), "waiting");
    activity.awaiting_user = false;
    activity.awaiting_approval = false;
    assert_eq!(activity.display_state(), "running");
}

#[test]
fn cancellation_domains_do_not_cross_cancel() {
    let waiter = HttpWaitCancellation::default();
    let task = TaskCancellation::default();
    let hub = HubShutdownCancellation::default();
    let waiter_clone = waiter.clone();
    waiter.cancel();
    assert!(waiter_clone.token().is_cancelled());
    assert!(!task.token().is_cancelled());
    assert!(!hub.token().is_cancelled());
    task.cancel();
    assert!(!hub.token().is_cancelled());
}

fn state_with_parent() -> AdmissionState {
    let mut state = AdmissionState::default();
    state
        .observe_session(
            LiveSessionId(1),
            AdmissionSession {
                parent: LiveSessionId(0),
                provider: "codex".into(),
            },
        )
        .unwrap();
    state
}
fn automatic_request(slots: i64) -> AdmissionRequest {
    AdmissionRequest {
        parent: LiveSessionId(1),
        slots,
        origin: VerifiedSpawnOrigin::Autonomous,
        replace: None,
    }
}
#[test]
fn concurrent_admission_cannot_overbook_and_releases_once() {
    let state = Arc::new(Mutex::new(state_with_parent()));
    let threads: Vec<_> = (0..20)
        .map(|_| {
            let state = state.clone();
            std::thread::spawn(move || {
                state.lock().unwrap().reserve_children(
                    automatic_request(5),
                    AdmissionLimits {
                        max_children_per_parent: 10,
                        max_total_sessions: 11,
                    },
                )
            })
        })
        .collect();
    let leases: Vec<_> = threads
        .into_iter()
        .filter_map(|t| t.join().unwrap().ok())
        .collect();
    assert_eq!(leases.len(), 2);
    let mut state = state.lock().unwrap();
    assert!(state.release_children(&leases[0].id));
    assert!(!state.release_children(&leases[0].id));
    assert!(state.consume_children(&leases[1].id, 2));
    assert!(state.admission_matches(&leases[1].id, LiveSessionId(1), 3));
    assert!(!state.admission_matches(&leases[1].id, LiveSessionId(1), 4));
    assert!(state.consume_children(&leases[1].id, 9));
    assert!(!state.consume_children(&leases[1].id, 1));
}
#[test]
fn admission_replace_is_atomic_and_failure_preserves_existing_reservation() {
    let mut state = state_with_parent();
    let limits = AdmissionLimits {
        max_children_per_parent: 2,
        max_total_sessions: 3,
    };
    let old = state
        .reserve_children(automatic_request(2), limits)
        .unwrap();
    let mut request = automatic_request(3);
    request.replace = Some(old.id.clone());
    assert!(state.reserve_children(request.clone(), limits).is_err());
    assert!(state.admission_matches(&old.id, LiveSessionId(1), 2));
    request.slots = 2;
    let new = state.reserve_children(request, limits).unwrap();
    assert!(!state.admission_matches(&old.id, LiveSessionId(1), 1));
    assert!(state.admission_matches(&new.id, LiveSessionId(1), 2));
    assert_eq!(state.release_parent(LiveSessionId(1)), vec![new.id]);
}
#[test]
fn autonomous_admission_counts_other_parents_and_preserves_default_limit() {
    let mut state = state_with_parent();
    state
        .observe_session(
            LiveSessionId(2),
            AdmissionSession {
                parent: LiveSessionId(0),
                provider: "claude".into(),
            },
        )
        .unwrap();
    let limits = AdmissionLimits {
        max_children_per_parent: 2,
        max_total_sessions: 1,
    };
    state
        .reserve_children(automatic_request(1), limits)
        .unwrap();
    assert!(matches!(
        state.reserve_children(automatic_request(1), limits),
        Err(AdmissionError::TotalSessions { maximum: 3, .. })
    ));
    let mut fresh = state_with_parent();
    assert!(
        fresh
            .reserve_children(
                automatic_request(10),
                AdmissionLimits {
                    max_children_per_parent: 0,
                    max_total_sessions: 0
                }
            )
            .is_ok()
    );
}
#[test]
fn provider_spawn_update_exclusion_covers_registration_gap_and_stale_release() {
    let mut state = AdmissionState::default();
    let spawn = state.begin_provider_spawn("codex").unwrap();
    assert_eq!(
        state.begin_provider_update("codex"),
        Err(ProviderAdmissionError::PendingSpawns { count: 1 })
    );
    let independent = state.begin_provider_update("claude").unwrap();
    assert_eq!(
        state.begin_provider_spawn("claude"),
        Err(ProviderAdmissionError::AlreadyUpdating)
    );
    state
        .register_spawn(&spawn, LiveSessionId(1), LiveSessionId(0))
        .unwrap();
    assert!(!state.end_provider_spawn(&spawn));
    assert_eq!(
        state.begin_provider_update("codex"),
        Err(ProviderAdmissionError::RunningSessions { count: 1 })
    );
    // A disconnected-but-undismissed card remains in state; only dismiss removes.
    assert_eq!(state.provider_session_count("codex"), 1);
    state.dismiss_session(LiveSessionId(1));
    let first = state.begin_provider_update("codex").unwrap();
    assert!(state.end_provider_update(&first));
    let second = state.begin_provider_update("codex").unwrap();
    assert!(!state.end_provider_update(&first));
    assert_eq!(
        state.begin_provider_spawn("codex"),
        Err(ProviderAdmissionError::AlreadyUpdating)
    );
    assert!(state.end_provider_update(&second));
    assert!(state.end_provider_update(&independent));
}
#[test]
fn storage_queue_and_shutdown_keep_compatibility_distinctions() {
    assert_eq!(
        EnqueueOutcome::Queued {
            generation: HistoryGeneration(9)
        }
        .legacy_drop_count(),
        0
    );
    assert_eq!(
        EnqueueOutcome::Dropped {
            cumulative_count: 4
        }
        .legacy_drop_count(),
        4
    );
    assert_eq!(EnqueueOutcome::Closed.legacy_drop_count(), 0);
    assert_ne!(
        ShutdownPolicy::CompatibilityStop,
        ShutdownPolicy::Drain {
            timeout: Duration::from_secs(6)
        }
    );
    let event = QueuedHistoryEvent {
        live_session_id: LiveSessionId(7),
        generation: HistoryGeneration(9),
        event: HistoryEvent(serde_json::Map::new()),
    };
    assert_ne!(event.generation, HistoryGeneration(10));
    let options = StorageOptions::baseline("/synthetic/logs".into());
    assert_eq!(options.queue_capacity, 4096);
    assert_eq!(options.query_timeout, Duration::from_secs(3));
    assert_eq!(options.init_timeout, Duration::from_secs(30));
}
#[test]
fn provider_plan_reuses_resolved_process_and_cancellation() {
    let plan = ProviderCommandPlan {
        provider: "synthetic".into(),
        purpose: ProviderCommandPurpose::Update,
        process: process::ProcessPlan {
            executable: "/synthetic/update-B".into(),
            args: vec!["upgrade".into()],
            cwd: "/synthetic".into(),
            env: Default::default(),
            stdin: vec![],
            timeout: Duration::from_secs(1),
            output_cap: 1024,
            pipe_drain_timeout: Duration::from_millis(50),
        },
        cancellation: process::Cancellation::default(),
    };
    assert_eq!(
        plan.process.executable,
        std::path::PathBuf::from("/synthetic/update-B")
    );
    plan.cancellation.cancel();
    assert!(plan.cancellation.is_cancelled());
}
#[test]
fn all_interfaces_are_object_safe() {
    let _: Option<&dyn SessionCore> = None;
    let _: Option<&dyn InputQueue> = None;
    let _: Option<&dyn ProcessedInput> = None;
    let _: Option<&dyn ApprovalActions> = None;
    let _: Option<&dyn SpawnAdmission> = None;
    let _: Option<&dyn ProviderUpdateAdmission> = None;
    let _: Option<&dyn WrappedSessionSpawner> = None;
    let _: Option<&dyn SpawnConfirmations> = None;
    let _: Option<&dyn CoreEventSubscription> = None;
}

#[test]
fn conductor_cannot_change_remembered_permission() {
    let request: ChildSpawnRequest =
        serde_json::from_value(json!({"remember_permission":true,"permission_preset":"bounded"}))
            .unwrap();
    let resolved =
        ResolvedChildSpawn::from_request(request, None, InternalSpawnGrants::default()).unwrap();
    assert_eq!(resolved.request().remember_permission, None);
    assert_eq!(resolved.request().permission_preset, "bounded");
    assert_eq!(resolved.origin(), &VerifiedSpawnOrigin::Autonomous);
    assert!(!resolved.grants().grant_folder_trust());
    assert!(resolved.grants().allowed_tools().is_empty());
}
