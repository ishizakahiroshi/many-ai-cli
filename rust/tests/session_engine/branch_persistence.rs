//! Go attachStore persists the resolved branch independently of optional history.
use super::*;

fn branch_fixture(session_enabled: bool) -> (Fixture, Arc<Mutex<String>>) {
    let branch = Arc::new(Mutex::new("initial-branch".to_owned()));
    let current = branch.clone();
    let f = fixture_with_branch_lookup(
        Arc::new(NoSpawn),
        Arc::new(|_, _| {}),
        session_enabled,
        Arc::new(move |_| {
            let branch = current.lock().unwrap().clone();
            Box::pin(async move { branch })
        }),
    );
    (f, branch)
}

fn stored(f: &Fixture, session: LiveSessionId) -> SessionOverview {
    f.store.session_overview_by_live_session(session).unwrap()
}

async fn registration_case(session_enabled: bool) {
    let (f, _) = branch_fixture(session_enabled);
    let registered = register_unapplied(&f).await;
    let session = registered.binding.session;
    assert_eq!(registered.snapshot.branch, "initial-branch");
    // Metadata is already durable before any optional session_start effect.
    assert_eq!(stored(&f, session).branch, "initial-branch");
    f.sink.apply(registered.after_registered).await.unwrap();
    assert_eq!(stored(&f, session).branch, "initial-branch");
}

async fn reattachment_case(session_enabled: bool, warm: bool) {
    let (f, branch) = branch_fixture(session_enabled);
    let (session, previous_row) = if warm {
        let registered = register_unapplied(&f).await;
        let session = registered.binding.session;
        // Simulate a lost optional history batch. Persisted registration data
        // and the following upsert must not depend on that batch being applied.
        drop(registered.after_registered);
        // A saved Go session can already contain a branch. Use the real store's
        // synchronous event API to seed that existing row before replacement.
        f.store
            .store_event(
                session,
                HistoryEvent(
                    serde_json::json!({"type":"session_start", "branch":"previously-saved"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .unwrap();
        let row = stored(&f, session);
        assert_eq!(row.branch, "previously-saved");
        (session, Some(row.id))
    } else {
        (LiveSessionId(42), None)
    };
    *branch.lock().unwrap() = "reattached-branch".to_owned();
    let reattached = f
        .engine
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: session.0,
                    provider: "copilot".into(),
                    cwd: "/fixture/project".into(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    started_at: proto::time::format_rfc3339(now()).unwrap(),
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            now(),
        )
        .await
        .unwrap();
    assert_eq!(reattached.binding.session, session);
    assert_eq!(reattached.snapshot.branch, "reattached-branch");
    let row = stored(&f, session);
    if let Some(previous) = previous_row {
        assert_eq!(row.id, previous, "warm reattach must update the same row");
    }
    // Inspect before session_reattach can repair missing start metadata.
    assert_eq!(row.branch, "reattached-branch");
    f.sink.apply(reattached.after_reattached).await.unwrap();
    assert_eq!(stored(&f, session).branch, "reattached-branch");
}

#[tokio::test]
async fn registration_persists_branch_without_session_logging() {
    registration_case(false).await;
}
#[tokio::test]
async fn registration_persists_branch_before_enabled_history_effects() {
    registration_case(true).await;
}
#[tokio::test]
async fn warm_reattach_persists_branch_without_session_logging() {
    reattachment_case(false, true).await;
}
#[tokio::test]
async fn warm_reattach_persists_branch_before_enabled_history_effects() {
    reattachment_case(true, true).await;
}
#[tokio::test]
async fn cold_reattach_persists_branch_without_session_logging() {
    reattachment_case(false, false).await;
}
#[tokio::test]
async fn cold_reattach_persists_branch_before_enabled_history_effects() {
    reattachment_case(true, false).await;
}
