//! Issue #44: the number on screen is the day's running count, and the
//! handoff records are named by that day and number.
use super::observer_tests::{Io, now};
use super::*;
use crate::{
    config::RuntimePaths,
    orchestration::handoff::record_day,
    terminal::{events::CoreEventBus, journal::JournalOptions},
};

#[derive(Default)]
struct Index {
    highest: BTreeMap<String, i64>,
    legacy: bool,
}
impl SessionRecordIndex for Index {
    fn highest_number_on(&self, day: &str) -> std::io::Result<i64> {
        Ok(self.highest.get(day).copied().unwrap_or(0))
    }
    fn save_number(&self, _: &str, _: i64) -> std::io::Result<()> {
        Ok(())
    }
    fn continues_legacy_launch(&self, _: i64, _: &str, _: &str, _: &str) -> bool {
        self.legacy
    }
}
fn engine(index: Option<Index>) -> (tempfile::TempDir, SessionEngine) {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49689, &root.path().join("installed")).unwrap();
    let core = engine_at(
        paths,
        index.map(|index| Arc::new(index) as Arc<dyn SessionRecordIndex>),
    );
    (root, core)
}
fn engine_at(paths: RuntimePaths, index: Option<Arc<dyn SessionRecordIndex>>) -> SessionEngine {
    SessionEngine::new(
        EngineOptions {
            record_index: index,
            ..Default::default()
        },
        Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
        Arc::new(Io),
        Arc::new(Io),
        Arc::new(Io),
        CoreEventBus::new(64).unwrap(),
    )
}
fn later(days: u64) -> Timestamp {
    now() + Duration::from_secs(days * 24 * 60 * 60)
}
fn day_of(at: Timestamp) -> String {
    record_day(&proto::time::format_rfc3339(at).unwrap()).unwrap()
}
fn message(pid: i64) -> proto::Message {
    proto::Message {
        provider: "claude".into(),
        pid,
        cwd: "synthetic-numbering-cwd".into(),
        cols: 120,
        rows: 30,
        ..Default::default()
    }
}
async fn register_at(core: &SessionEngine, pid: i64, at: Timestamp) -> SessionBinding {
    try_register_at(core, pid, at).await.unwrap().binding
}
async fn try_register_at(
    core: &SessionEngine,
    pid: i64,
    at: Timestamp,
) -> Result<Registration, SessionError> {
    core.register(
        RegisterRequest {
            message: message(pid),
            spawn_proof: None,
        },
        WrapperConnectionId(pid as u64),
        at,
    )
    .await
}
/// The returned reattachment holds the persistence order until it is
/// dropped; take its binding before the next registration.
async fn reattach_at(
    core: &SessionEngine,
    id: i64,
    pid: i64,
    started_at: &str,
    at: Timestamp,
) -> Result<Reattachment, SessionError> {
    core.reattach(
        ReattachRequest {
            restored_metadata: None,
            message: proto::Message {
                session_id: id,
                started_at: started_at.into(),
                ..message(pid)
            },
        },
        WrapperConnectionId(pid as u64 + 10_000),
        at,
    )
    .await
}
fn key(core: &SessionEngine, id: LiveSessionId) -> String {
    core.snapshot(id).unwrap().record_key
}

#[tokio::test]
async fn restarts_continue_numbering_even_without_handoff_records() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49689, &root.path().join("installed")).unwrap();
    let restart = || {
        engine_at(
            paths.clone(),
            Some(Arc::new(crate::orchestration::handoff::HandoffStore::new(
                paths.clone(),
            ))),
        )
    };
    let first = restart();
    assert_eq!(register_at(&first, 1, now()).await.session.0, 1);
    drop(first);
    let second = restart();
    // A failed reservation must not hand out the number or consume it in memory.
    let marker = paths.root().join("session-number.json");
    let backup = root.path().join("saved-number");
    // Load today's floor before forcing an atomic replacement error.
    assert_eq!(register_at(&second, 2, now()).await.session.0, 2);
    std::fs::rename(&marker, &backup).unwrap();
    std::fs::create_dir(&marker).unwrap();
    assert!(matches!(
        try_register_at(&second, 3, now()).await,
        Err(SessionError::Storage(_))
    ));
    std::fs::remove_dir(&marker).unwrap();
    std::fs::rename(&backup, &marker).unwrap();
    // A wrapper returning under a lower number must not move the saved count back.
    let started = proto::time::format_rfc3339(now()).unwrap();
    reattach_at(&second, 1, 1, &started, now()).await.unwrap();
    drop(second);
    let third = restart();
    assert_eq!(register_at(&third, 3, now()).await.session.0, 3);
    assert_eq!(register_at(&third, 4, later(1)).await.session.0, 1);
    assert!(!paths.resource(crate::config::Resource::Handoff).exists());
    assert_eq!(
        std::fs::read(&marker).unwrap(),
        serde_json::to_vec(&(day_of(later(1)), 1)).unwrap()
    );
    drop(third);
    std::fs::write(&marker, b"incomplete").unwrap();
    assert!(matches!(
        try_register_at(&restart(), 5, later(1)).await,
        Err(SessionError::Storage(_))
    ));
    assert_eq!(std::fs::read(marker).unwrap(), b"incomplete");
}

#[tokio::test]
async fn a_hub_started_again_the_same_day_numbers_above_that_days_records() {
    let today = day_of(now());
    let (_root, core) = engine(Some(Index {
        highest: [(today.clone(), 5), ("19990101".into(), 40)].into(),
        ..Default::default()
    }));
    let first = register_at(&core, 1, now()).await;
    let second = register_at(&core, 2, now()).await;
    assert_eq!((first.session.0, second.session.0), (6, 7));
    assert_eq!(key(&core, first.session), format!("{today}-6"));
    assert_eq!(key(&core, second.session), format!("{today}-7"));
}

#[tokio::test]
async fn a_new_day_counts_from_one_and_skips_numbers_still_on_screen() {
    let (_root, core) = engine(None);
    let day1 = day_of(now());
    let day2 = day_of(later(1));
    assert_ne!(day1, day2);
    let one = register_at(&core, 1, now()).await;
    let two = register_at(&core, 2, now()).await;
    let three = register_at(&core, 3, now()).await;
    assert_eq!([one.session.0, two.session.0, three.session.0], [1, 2, 3]);
    // #2 leaves the screen; #1 and #3 keep running past midnight.
    core.dismiss(two.session, now()).unwrap();
    let next = register_at(&core, 4, later(1)).await;
    assert_eq!(next.session.0, 2);
    assert_eq!(key(&core, next.session), format!("{day2}-2"));
    let after = register_at(&core, 5, later(1)).await;
    assert_eq!(after.session.0, 4, "#3 is still in use");
    // The sessions from the day before keep their own records.
    assert_eq!(key(&core, one.session), format!("{day1}-1"));
    assert_eq!(key(&core, three.session), format!("{day1}-3"));
}

#[tokio::test]
async fn a_session_from_an_earlier_day_keeps_its_records_and_its_number_is_skipped() {
    let today = day_of(later(1));
    let yesterday = proto::time::format_rfc3339(now()).unwrap();
    let (_root, core) = engine(Some(Index {
        highest: [(today.clone(), 7)].into(),
        ..Default::default()
    }));
    // After a restart the wrapper started yesterday comes back as #9.
    let back = reattach_at(&core, 9, 9, &yesterday, later(1))
        .await
        .unwrap()
        .binding;
    assert_eq!(back.session.0, 9);
    assert_eq!(key(&core, back.session), format!("{}-9", day_of(now())));
    // It does not move today's count; its number is skipped while in use.
    let eight = register_at(&core, 1, later(1)).await;
    let ten = register_at(&core, 2, later(1)).await;
    assert_eq!((eight.session.0, ten.session.0), (8, 10));
    assert_eq!(key(&core, ten.session), format!("{today}-10"));
}

#[tokio::test]
async fn a_session_of_the_same_day_coming_back_keeps_todays_count_above_it() {
    let (_root, core) = engine(None);
    let started = proto::time::format_rfc3339(now()).unwrap();
    let back = reattach_at(&core, 4, 4, &started, now())
        .await
        .unwrap()
        .binding;
    assert_eq!(back.session.0, 4);
    assert_eq!(register_at(&core, 1, now()).await.session.0, 5);
}

#[tokio::test]
async fn a_renumbered_wrapper_gets_new_records_of_today() {
    let (_root, core) = engine(None);
    let today = day_of(now());
    let first = register_at(&core, 1, now()).await;
    let yesterday =
        proto::time::format_rfc3339(Timestamp::from_unix(1791158400 - 86_400, 0).unwrap()).unwrap();
    // Another wrapper claims #1 while #1 is connected: it is renumbered.
    let other = reattach_at(&core, first.session.0, 2, &yesterday, now())
        .await
        .unwrap()
        .binding;
    assert_eq!(other.session.0, 2);
    assert_eq!(key(&core, other.session), format!("{today}-2"));
    assert_eq!(key(&core, first.session), format!("{today}-1"));
}

#[tokio::test]
async fn a_launch_an_earlier_version_recorded_keeps_writing_under_its_bare_number() {
    let (_root, core) = engine(Some(Index {
        legacy: true,
        ..Default::default()
    }));
    let started = proto::time::format_rfc3339(now()).unwrap();
    let back = reattach_at(&core, 6, 6, &started, now())
        .await
        .unwrap()
        .binding;
    assert_eq!(key(&core, back.session), "6");
    // A warm reconnect keeps the name it already had.
    let again = reattach_at(&core, 6, 6, &started, now())
        .await
        .unwrap()
        .binding;
    assert_eq!(key(&core, again.session), "6");
}

#[tokio::test]
async fn a_dismissal_stops_only_the_launch_it_was_made_for() {
    let (_root, core) = engine(None);
    let day1_started = proto::time::format_rfc3339(now()).unwrap();
    let old = register_at(&core, 1, now()).await;
    core.disconnected(old, now()).unwrap();
    core.dismiss(old.session, now()).unwrap();
    assert!(matches!(
        reattach_at(&core, 1, 1, &day1_started, now()).await,
        Err(SessionError::InvalidRequest(reason)) if reason == "session dismissed"
    ));
    // The next day #1 is given out again and its own wrapper may come back.
    let fresh = register_at(&core, 2, later(1)).await;
    assert_eq!(fresh.session.0, 1);
    let fresh_started = core.snapshot(fresh.session).unwrap().started_at;
    core.disconnected(fresh, later(1)).unwrap();
    let back = reattach_at(&core, 1, 2, &fresh_started, later(1))
        .await
        .unwrap()
        .binding;
    assert_eq!(back.session.0, 1);
    // The launch dismissed the day before still may not.
    core.disconnected(back, later(1)).unwrap();
    assert!(matches!(
        reattach_at(&core, 1, 1, &day1_started, later(1)).await,
        Err(SessionError::InvalidRequest(reason)) if reason == "session dismissed"
    ));
}
