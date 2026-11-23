use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        let n = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("mai-followup-{millis}-{n}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn claim(&self) -> PathBuf {
        self.0.join("claims.json")
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Board {
    items: Mutex<Vec<ItemSnapshot>>,
    /// Item whose `append_note` fails, standing in for a board write error.
    fail_for: Mutex<Option<String>>,
}
impl Board {
    fn new(items: Vec<ItemSnapshot>) -> Self {
        Self {
            items: Mutex::new(items),
            fail_for: Mutex::new(None),
        }
    }
    fn with_intent(work_id: &str, intent: &Intent) -> Self {
        Self::new(vec![ItemSnapshot {
            id: work_id.to_owned(),
            events: vec![
                "session started".to_owned(),
                intent_note(work_id, intent).unwrap(),
            ],
        }])
    }
    fn texts(&self) -> Vec<String> {
        self.items.lock().unwrap()[0].events.clone()
    }
    fn texts_of(&self, item_id: &str) -> Vec<String> {
        self.items
            .lock()
            .unwrap()
            .iter()
            .find(|item| item.id == item_id)
            .map(|item| item.events.clone())
            .unwrap_or_default()
    }
}
impl BoardPort for Board {
    fn list_items(&self) -> Result<Vec<ItemSnapshot>, FollowupError> {
        Ok(self.items.lock().unwrap().clone())
    }
    fn append_note(&self, item_id: &str, text: &str) -> Result<(), FollowupError> {
        if self.fail_for.lock().unwrap().as_deref() == Some(item_id) {
            return Err(FollowupError::Board);
        }
        let mut items = self.items.lock().unwrap();
        let item = items
            .iter_mut()
            .find(|item| item.id == item_id)
            .ok_or(FollowupError::Board)?;
        item.events.push(text.to_owned());
        Ok(())
    }
}

struct Screens(Mutex<Result<Option<Screen>, FollowupError>>);
impl ScreenSource for Screens {
    fn screen(&self, _work_id: &str) -> Result<Option<Screen>, FollowupError> {
        self.0.lock().unwrap().clone()
    }
}

struct Exec {
    calls: Mutex<Vec<Action>>,
    fail: bool,
}
impl FollowupExecutor for Exec {
    fn act(&self, action: &Action) -> Result<(), FollowupError> {
        self.calls.lock().unwrap().push(action.clone());
        if self.fail {
            Err(FollowupError::Action)
        } else {
            Ok(())
        }
    }
}

fn approval_intent() -> Intent {
    Intent {
        observation_id: "obs-1".to_owned(),
        operation: "github.update_pr_body".to_owned(),
        target: "https://github.com/example/repo/pull/49".to_owned(),
        wait: WaitKind::Approval,
        deadline_ms: Some(5_000),
    }
}

fn continue_intent() -> Intent {
    Intent {
        observation_id: "obs-2".to_owned(),
        operation: "github.continue_task".to_owned(),
        target: "https://github.com/example/repo/pull/49".to_owned(),
        wait: WaitKind::Continue,
        deadline_ms: Some(5_000),
    }
}

fn permit(effect: Effect, work_id: &str, operation: &str, target: &str) -> Permit {
    Permit {
        permit_id: "permit-1".to_owned(),
        work_id: work_id.to_owned(),
        operation: operation.to_owned(),
        target: target.to_owned(),
        effect,
        expires_at_ms: 9_000,
        revoked: false,
    }
}

fn approval_screen() -> Screen {
    Screen::Approval {
        card_id: "card-1".to_owned(),
        operation: "github.update_pr_body".to_owned(),
        target: "https://github.com/example/repo/pull/49".to_owned(),
    }
}

fn open_watcher(
    dir: &TempDir,
    board: Board,
    screen: Result<Option<Screen>, FollowupError>,
    permits: Vec<Permit>,
    fail: bool,
) -> Watcher<Board, Screens, Exec> {
    Watcher::new(
        board,
        Screens(Mutex::new(screen)),
        Exec {
            calls: Mutex::new(Vec::new()),
            fail,
        },
        ClaimStore::open(dir.claim()).unwrap(),
        permits,
    )
}

fn calls(watcher: &Watcher<Board, Screens, Exec>) -> Vec<Action> {
    watcher.executor.calls.lock().unwrap().clone()
}

#[test]
fn permits_reject_unknown_fields_and_duplicates() {
    let row = |id: &str| {
        format!(
            r#"{{"permit_id":"{id}","work_id":"task/1","operation":"github.update_pr_body","target":"https://github.com/example/repo/pull/49","effect":"approve_once","expires_at_ms":9,"revoked":false}}"#
        )
    };
    let file =
        |rows: &[String]| format!(r#"{{"schema_version":1,"permits":[{}]}}"#, rows.join(","));
    let ok = file(&[row("permit-1")]);
    assert_eq!(load_permits(&ok).unwrap().len(), 1);
    assert!(load_permits(&ok.replace("revoked", "revoked_at")).is_err());
    // Two well-formed rows load, so the duplicate below fails on the id alone.
    assert_eq!(
        load_permits(&file(&[row("permit-1"), row("permit-2")]))
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        load_permits(&file(&[row("permit-1"), row("permit-1")])),
        Err(FollowupError::Permits)
    );
}

#[test]
fn a_renewed_permit_beside_a_revoked_one_acts() {
    let work = "task/1";
    let intent = approval_intent();
    let mut old = permit(Effect::ApproveOnce, work, &intent.operation, &intent.target);
    old.revoked = true;
    let mut renewed = permit(Effect::ApproveOnce, work, &intent.operation, &intent.target);
    renewed.permit_id = "permit-2".to_owned();
    assert_eq!(
        judge(
            work,
            &intent,
            Some(&approval_screen()),
            &[old.clone(), renewed.clone()],
            1_000,
            None
        ),
        Judgement::Act {
            permit_id: "permit-2".to_owned(),
            effect: Effect::ApproveOnce,
            card_id: "card-1".to_owned(),
        }
    );
    // Two live permits for the same scope are still ambiguous.
    let mut twin = renewed.clone();
    twin.permit_id = "permit-3".to_owned();
    assert_eq!(
        judge(
            work,
            &intent,
            Some(&approval_screen()),
            &[old, renewed, twin],
            1_000,
            None
        ),
        Judgement::Hold {
            reason: REASON_AMBIGUOUS,
            terminal: true
        }
    );
}

#[test]
fn intent_note_refuses_what_the_watcher_would_call_malformed() {
    let mut intent = approval_intent();
    intent.observation_id = "obs 1".to_owned();
    assert_eq!(intent_note("task/1", &intent), Err(FollowupError::Invalid));
    let mut intent = approval_intent();
    intent.target = "line\nbreak".to_owned();
    assert_eq!(intent_note("task/1", &intent), Err(FollowupError::Invalid));
    let mut intent = approval_intent();
    intent.deadline_ms = Some(0);
    assert_eq!(intent_note("task/1", &intent), Err(FollowupError::Invalid));
    assert_eq!(
        intent_note("", &approval_intent()),
        Err(FollowupError::Invalid)
    );
}

#[test]
fn approval_and_continue_are_different_waits() {
    let work = "task/1";
    let approval = approval_intent();
    let next = continue_intent();
    let now = 1_000;
    let approval_permit = permit(
        Effect::ApproveOnce,
        work,
        &approval.operation,
        &approval.target,
    );
    assert!(matches!(
        judge(
            work,
            &approval,
            Some(&approval_screen()),
            std::slice::from_ref(&approval_permit),
            now,
            None
        ),
        Judgement::Act {
            effect: Effect::ApproveOnce,
            ..
        }
    ));
    assert!(matches!(
        judge(
            work,
            &next,
            Some(&Screen::Continue {
                prompt_id: "prompt-1".to_owned(),
                operation: next.operation.clone(),
                target: next.target.clone(),
            }),
            &[permit(
                Effect::ContinueOnce,
                work,
                &next.operation,
                &next.target
            )],
            now,
            None,
        ),
        Judgement::Act {
            effect: Effect::ContinueOnce,
            ..
        }
    ));
    assert!(matches!(
        judge(
            work,
            &next,
            Some(&approval_screen()),
            &[approval_permit],
            now,
            None
        ),
        Judgement::Hold {
            reason: REASON_MISMATCH,
            terminal: true
        }
    ));
}

#[test]
fn missing_screen_is_not_an_approval() {
    let intent = approval_intent();
    let decision = judge(
        "task/1",
        &intent,
        None,
        &[permit(
            Effect::ApproveOnce,
            "task/1",
            &intent.operation,
            &intent.target,
        )],
        1_000,
        None,
    );
    assert_eq!(
        decision,
        Judgement::Hold {
            reason: REASON_SCREEN_UNCONFIRMED,
            terminal: false,
        }
    );
}

#[test]
fn tick_approves_once_and_a_second_tick_does_not() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    let board = Board::with_intent(work, &intent);
    let watcher = open_watcher(
        &dir,
        board,
        Ok(Some(approval_screen())),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    let first = watcher.tick(1_000).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].status, Status::Acted);
    assert_eq!(first[0].reason, "approve_once");
    assert_eq!(calls(&watcher).len(), 1);
    assert_eq!(calls(&watcher)[0].card_id, "card-1");
    let second = watcher.tick(1_100).unwrap();
    assert_eq!(second[0].status, Status::Skipped);
    assert_eq!(calls(&watcher).len(), 1);
    assert!(
        watcher
            .board
            .texts()
            .iter()
            .any(|text| text.contains("\"rec\":\"result\""))
    );
}

#[test]
fn tick_continues_once_for_a_continue_prompt() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = continue_intent();
    let watcher = open_watcher(
        &dir,
        Board::with_intent(work, &intent),
        Ok(Some(Screen::Continue {
            prompt_id: "prompt-1".to_owned(),
            operation: intent.operation.clone(),
            target: intent.target.clone(),
        })),
        vec![permit(
            Effect::ContinueOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    let outcome = watcher.tick(1_000).unwrap();
    assert_eq!(outcome[0].reason, "continue_once");
    assert_eq!(calls(&watcher).len(), 1);
}

#[test]
fn out_of_scope_expired_revoked_and_ambiguous_do_not_act() {
    let work = "task/1";
    let intent = approval_intent();
    let screen = approval_screen();
    let now = 1_000;
    let mut expired = permit(Effect::ApproveOnce, work, &intent.operation, &intent.target);
    expired.expires_at_ms = 1_000;
    assert_eq!(
        judge(work, &intent, Some(&screen), &[expired], now, None),
        Judgement::Hold {
            reason: REASON_EXPIRED,
            terminal: true
        }
    );
    let mut revoked = permit(Effect::ApproveOnce, work, &intent.operation, &intent.target);
    revoked.revoked = true;
    assert_eq!(
        judge(work, &intent, Some(&screen), &[revoked], now, None),
        Judgement::Hold {
            reason: REASON_REVOKED,
            terminal: true
        }
    );
    assert_eq!(
        judge(work, &intent, Some(&screen), &[], now, None),
        Judgement::Hold {
            reason: REASON_OUT_OF_SCOPE,
            terminal: true
        }
    );
    let mut blank = approval_screen();
    if let Screen::Approval { card_id, .. } = &mut blank {
        card_id.clear();
    }
    assert_eq!(
        judge(
            work,
            &intent,
            Some(&blank),
            &[permit(
                Effect::ApproveOnce,
                work,
                &intent.operation,
                &intent.target
            )],
            now,
            None,
        ),
        Judgement::Hold {
            reason: REASON_AMBIGUOUS,
            terminal: true
        }
    );
}

#[test]
fn a_changed_card_is_not_answered() {
    let work = "task/1";
    let intent = approval_intent();
    let decision = judge(
        work,
        &intent,
        Some(&approval_screen()),
        &[permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        1_000,
        Some("card-0"),
    );
    assert_eq!(
        decision,
        Judgement::Hold {
            reason: REASON_CHANGED,
            terminal: true
        }
    );
}

#[test]
fn unconfirmed_then_visible_acts_once() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    let watcher = open_watcher(
        &dir,
        Board::with_intent(work, &intent),
        Ok(None),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    let waiting = watcher.tick(1_000).unwrap();
    assert_eq!(waiting[0].status, Status::Waiting);
    assert_eq!(waiting[0].reason, REASON_SCREEN_UNCONFIRMED);
    assert!(calls(&watcher).is_empty());
    *watcher.screens.0.lock().unwrap() = Ok(Some(approval_screen()));
    let acted = watcher.tick(1_100).unwrap();
    assert_eq!(acted[0].status, Status::Acted);
    assert_eq!(calls(&watcher).len(), 1);
}

#[test]
fn disconnect_does_not_finish_the_observation() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    let watcher = open_watcher(
        &dir,
        Board::with_intent(work, &intent),
        Ok(Some(Screen::Disconnected)),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    assert_eq!(watcher.tick(1_000).unwrap()[0].reason, REASON_DISCONNECTED);
    *watcher.screens.0.lock().unwrap() = Ok(Some(approval_screen()));
    assert_eq!(watcher.tick(1_100).unwrap()[0].status, Status::Acted);
    assert_eq!(calls(&watcher).len(), 1);
}

#[test]
fn done_screen_closes_the_intent_without_a_later_click() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    let watcher = open_watcher(
        &dir,
        Board::with_intent(work, &intent),
        Ok(Some(Screen::Done)),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    assert_eq!(watcher.tick(1_000).unwrap()[0].reason, REASON_NOT_PRESENTED);
    *watcher.screens.0.lock().unwrap() = Ok(Some(approval_screen()));
    let again = watcher.tick(1_100).unwrap();
    assert_eq!(again[0].status, Status::Skipped);
    assert!(calls(&watcher).is_empty());
}

#[test]
fn restart_after_an_acting_claim_does_not_act_again() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    fs::write(dir.claim(), acting_claim(work, &intent.observation_id)).unwrap();

    let watcher = open_watcher(
        &dir,
        Board::with_intent(work, &intent),
        Ok(Some(approval_screen())),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    let outcome = watcher.tick(1_000).unwrap();
    assert_eq!(outcome[0].reason, REASON_UNKNOWN_AFTER_INTERRUPT);
    assert!(calls(&watcher).is_empty());
    let stored = fs::read_to_string(dir.claim()).unwrap();
    assert!(stored.contains("settled"));
    assert_eq!(watcher.tick(1_100).unwrap()[0].status, Status::Skipped);
}

#[test]
fn a_failed_action_is_not_retried() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    let watcher = open_watcher(
        &dir,
        Board::with_intent(work, &intent),
        Ok(Some(approval_screen())),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        true,
    );
    assert_eq!(watcher.tick(1_000).unwrap()[0].reason, REASON_ACTION_FAILED);
    assert_eq!(calls(&watcher).len(), 1);
    assert_eq!(watcher.tick(1_100).unwrap()[0].status, Status::Skipped);
    assert_eq!(calls(&watcher).len(), 1);
}

#[test]
fn ordinary_notes_are_ignored_and_a_foreign_work_id_does_not_act() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    let foreign = intent_note("task/2", &intent).unwrap();
    let board = Board::new(vec![ItemSnapshot {
        id: work.to_owned(),
        events: vec![
            "作業しました".to_owned(),
            foreign,
            "followup/1 {".to_owned(),
        ],
    }]);
    let watcher = open_watcher(
        &dir,
        board,
        Ok(Some(approval_screen())),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    let outcomes = watcher.tick(1_000).unwrap();
    assert!(calls(&watcher).is_empty());
    assert!(outcomes.iter().any(|item| item.reason == REASON_MISMATCH));
    assert!(outcomes.iter().any(|item| item.reason == REASON_MALFORMED));
}

#[test]
fn observation_failure_does_not_act() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    let watcher = open_watcher(
        &dir,
        Board::with_intent(work, &intent),
        Err(FollowupError::Board),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    assert_eq!(
        watcher.tick(1_000).unwrap()[0].reason,
        REASON_OBSERVATION_FAILED
    );
    assert!(calls(&watcher).is_empty());
}

#[test]
fn an_open_claim_file_from_a_new_watcher_keeps_the_refusal() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    let first = open_watcher(
        &dir,
        Board::with_intent(work, &intent),
        Ok(Some(approval_screen())),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    assert_eq!(first.tick(1_000).unwrap()[0].status, Status::Acted);
    let events = first.board.texts();
    drop(first);
    let second = open_watcher(
        &dir,
        Board::new(vec![ItemSnapshot {
            id: work.to_owned(),
            events,
        }]),
        Ok(Some(approval_screen())),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    assert_eq!(second.tick(1_200).unwrap()[0].status, Status::Skipped);
    assert!(calls(&second).is_empty());
}

fn acting_claim(work_id: &str, observation_id: &str) -> String {
    format!(
        "{{\"schema_version\":1,\"claims\":[{{\"work_id\":\"{work_id}\",\"observation_id\":\"{observation_id}\",\"phase\":\"acting\",\"event_text\":null}}]}}"
    )
}

#[test]
fn the_same_observation_id_on_two_work_items_is_judged_for_each() {
    let dir = TempDir::new();
    let intent = approval_intent();
    let board = Board::new(vec![
        ItemSnapshot {
            id: "task/1".to_owned(),
            events: vec![intent_note("task/1", &intent).unwrap()],
        },
        ItemSnapshot {
            id: "task/2".to_owned(),
            events: vec![intent_note("task/2", &intent).unwrap()],
        },
    ]);
    let mut other = permit(
        Effect::ApproveOnce,
        "task/2",
        &intent.operation,
        &intent.target,
    );
    other.permit_id = "permit-2".to_owned();
    let watcher = open_watcher(
        &dir,
        board,
        Ok(Some(approval_screen())),
        vec![
            permit(
                Effect::ApproveOnce,
                "task/1",
                &intent.operation,
                &intent.target,
            ),
            other,
        ],
        false,
    );
    let outcomes = watcher.tick(1_000).unwrap();
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|item| item.status == Status::Acted));
    let acted: Vec<String> = calls(&watcher)
        .iter()
        .map(|action| action.work_id.clone())
        .collect();
    assert_eq!(acted, vec!["task/1".to_owned(), "task/2".to_owned()]);
    // One item's record never lands on the other item.
    assert!(
        !watcher
            .board
            .texts_of("task/2")
            .iter()
            .any(|text| text.contains("\"work\":\"task/1\""))
    );
    assert!(
        watcher
            .tick(1_100)
            .unwrap()
            .iter()
            .all(|item| item.status == Status::Skipped)
    );
    assert_eq!(calls(&watcher).len(), 2);
}

#[test]
fn a_card_answered_for_one_observation_is_not_answered_for_another() {
    let dir = TempDir::new();
    let work = "task/1";
    let first = approval_intent();
    let mut later = approval_intent();
    later.observation_id = "obs-3".to_owned();
    let board = Board::new(vec![ItemSnapshot {
        id: work.to_owned(),
        events: vec![
            intent_note(work, &first).unwrap(),
            intent_note(work, &later).unwrap(),
        ],
    }]);
    let watcher = open_watcher(
        &dir,
        board,
        Ok(Some(approval_screen())),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &first.operation,
            &first.target,
        )],
        false,
    );
    let outcomes = watcher.tick(1_000).unwrap();
    assert_eq!(outcomes[0].status, Status::Acted);
    assert_eq!(outcomes[1].status, Status::Waiting);
    assert_eq!(outcomes[1].reason, REASON_DUPLICATE);
    assert_eq!(calls(&watcher).len(), 1);
    // The same card still on screen at the next poll stays answered once.
    let again = watcher.tick(1_100).unwrap();
    assert_eq!(again[1].reason, REASON_DUPLICATE);
    assert_eq!(calls(&watcher).len(), 1);
    // A new card is the later observation's own.
    *watcher.screens.0.lock().unwrap() = Ok(Some(Screen::Approval {
        card_id: "card-2".to_owned(),
        operation: first.operation.clone(),
        target: first.target.clone(),
    }));
    let next = watcher.tick(1_200).unwrap();
    assert_eq!(next[1].status, Status::Acted);
    let made = calls(&watcher);
    assert_eq!(made.len(), 2);
    assert_eq!(made[1].observation_id, "obs-3");
    assert_eq!(made[1].card_id, "card-2");
}

#[test]
fn a_board_write_failure_holds_one_item_and_the_next_still_runs() {
    let dir = TempDir::new();
    let intent = approval_intent();
    let board = Board::new(vec![
        ItemSnapshot {
            id: "task/1".to_owned(),
            events: vec![intent_note("task/1", &intent).unwrap()],
        },
        ItemSnapshot {
            id: "task/2".to_owned(),
            events: vec![intent_note("task/2", &intent).unwrap()],
        },
    ]);
    *board.fail_for.lock().unwrap() = Some("task/1".to_owned());
    let mut other = permit(
        Effect::ApproveOnce,
        "task/2",
        &intent.operation,
        &intent.target,
    );
    other.permit_id = "permit-2".to_owned();
    let watcher = open_watcher(
        &dir,
        board,
        Ok(Some(approval_screen())),
        vec![
            permit(
                Effect::ApproveOnce,
                "task/1",
                &intent.operation,
                &intent.target,
            ),
            other,
        ],
        false,
    );
    let outcomes = watcher.tick(1_000).unwrap();
    assert_eq!(outcomes[0].work_id, "task/1");
    assert_eq!(outcomes[0].status, Status::Waiting);
    assert_eq!(outcomes[0].reason, REASON_BOARD_UNRECORDED);
    assert_eq!(outcomes[1].work_id, "task/2");
    assert_eq!(outcomes[1].status, Status::Acted);
    // The card is not answered while its sighting cannot be recorded.
    let made = calls(&watcher);
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].work_id, "task/2");
    // Once the board takes notes again, the first item proceeds.
    *watcher.board.fail_for.lock().unwrap() = None;
    let later = watcher.tick(1_100).unwrap();
    assert_eq!(later[0].status, Status::Acted);
    assert_eq!(calls(&watcher).len(), 2);
}

#[test]
fn a_transient_note_that_cannot_be_written_does_not_stop_the_tick() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    let board = Board::with_intent(work, &intent);
    *board.fail_for.lock().unwrap() = Some(work.to_owned());
    let watcher = open_watcher(
        &dir,
        board,
        Ok(None),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    let outcome = watcher.tick(1_000).unwrap();
    assert_eq!(outcome[0].status, Status::Waiting);
    assert_eq!(outcome[0].reason, REASON_BOARD_UNRECORDED);
    assert!(calls(&watcher).is_empty());
}

#[test]
fn an_interrupted_claim_says_when_the_board_did_not_take_the_hold() {
    let dir = TempDir::new();
    let work = "task/1";
    let intent = approval_intent();
    fs::write(dir.claim(), acting_claim(work, &intent.observation_id)).unwrap();
    let board = Board::with_intent(work, &intent);
    *board.fail_for.lock().unwrap() = Some(work.to_owned());
    let watcher = open_watcher(
        &dir,
        board,
        Ok(Some(approval_screen())),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    let outcome = watcher.tick(1_000).unwrap();
    assert_eq!(outcome[0].status, Status::Held);
    assert_eq!(outcome[0].reason, REASON_BOARD_UNRECORDED);
    assert!(calls(&watcher).is_empty());
    // The settled hold reaches the board on the next poll.
    *watcher.board.fail_for.lock().unwrap() = None;
    assert_eq!(watcher.tick(1_100).unwrap()[0].status, Status::Skipped);
    assert!(
        watcher
            .board
            .texts()
            .iter()
            .any(|text| text.contains(REASON_UNKNOWN_AFTER_INTERRUPT))
    );
    assert!(calls(&watcher).is_empty());
}

#[test]
fn msp_list_pending_separates_approval_continue_and_idle() {
    let approval = serde_json::json!({
        "approvals": [{
            "approvalId": "card-1",
            "sessionId": "sess-1",
            "availableChoices": [{"choiceId": "allow_once"}, {"choiceId": "reject"}],
            "subject": {"kind": "shell", "command": "git status"}
        }],
        "userInputs": []
    });
    assert_eq!(
        screen_from_msp_pending("sess-1", &approval),
        Screen::Approval {
            card_id: "card-1".to_owned(),
            operation: "msp.shell".to_owned(),
            target: r#"{"command":"git status"}"#.to_owned(),
        }
    );
    let prompt = serde_json::json!({
        "approvals": [],
        "userInputs": [{
            "sessionId": "sess-1",
            "userInputId": "prompt-1",
            "questions": [{
                "id": "next-step",
                "header": "next",
                "question": "次段の作業を始めて",
                "options": [],
                "selection": {"kind": "single"}
            }]
        }]
    });
    assert_eq!(
        screen_from_msp_pending("sess-1", &prompt),
        Screen::Continue {
            prompt_id: "prompt-1".to_owned(),
            operation: "msp.user_input".to_owned(),
            target: r#"{"header":"next","id":"next-step","options":[],"question":"次段の作業を始めて","selection":{"kind":"single"}}"#.to_owned(),
        }
    );
    let idle = serde_json::json!({"approvals": [], "userInputs": []});
    assert_eq!(screen_from_msp_pending("sess-1", &idle), Screen::Running);
}

#[test]
fn msp_list_pending_does_not_choose_an_unclear_card() {
    let unknown_kind = serde_json::json!({
        "approvals": [{
            "approvalId": "card-1",
            "sessionId": "sess-1",
            "availableChoices": [{"choiceId": "allow_once"}],
            "subject": {"kind": "computerUseTakeover"}
        }],
        "userInputs": []
    });
    assert_eq!(
        screen_from_msp_pending("sess-1", &unknown_kind),
        Screen::Unknown
    );
    let other_session = serde_json::json!({
        "approvals": [{
            "approvalId": "card-1",
            "sessionId": "sess-2",
            "availableChoices": [{"choiceId": "allow_once"}],
            "subject": {"kind": "shell", "command": "git status"}
        }],
        "userInputs": []
    });
    assert_eq!(
        screen_from_msp_pending("sess-1", &other_session),
        Screen::Unknown
    );
    let both = serde_json::json!({
        "approvals": [{
            "approvalId": "card-1",
            "sessionId": "sess-1",
            "availableChoices": [{"choiceId": "allow_once"}],
            "subject": {"kind": "tool", "toolName": "example.tool"}
        }],
        "userInputs": [{
            "sessionId": "sess-1",
            "userInputId": "prompt-1",
            "questions": [{"id": "next-step"}]
        }]
    });
    assert_eq!(screen_from_msp_pending("sess-1", &both), Screen::Unknown);
    let no_once = serde_json::json!({
        "approvals": [{
            "approvalId": "card-1",
            "sessionId": "sess-1",
            "availableChoices": [{"choiceId": "reject"}],
            "subject": {"kind": "shell", "command": "git status"}
        }],
        "userInputs": []
    });
    assert_eq!(screen_from_msp_pending("sess-1", &no_once), Screen::Unknown);
}

fn msp_approval(subject: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "approvals": [{
            "approvalId": "card-1",
            "sessionId": "sess-1",
            "availableChoices": [{"choiceId": "allow_once"}],
            "subject": subject
        }],
        "userInputs": []
    })
}

fn msp_target(body: &serde_json::Value) -> Option<String> {
    match screen_from_msp_pending("sess-1", body) {
        Screen::Approval { target, .. } | Screen::Continue { target, .. } => Some(target),
        _ => None,
    }
}

#[test]
fn msp_subject_target_carries_every_authorizing_field() {
    // Read and write on one path are two targets, so a permit for one does
    // not answer the other.
    let read = msp_target(&msp_approval(
        serde_json::json!({"kind": "fileAccess", "path": "/repo/a", "access": "read"}),
    ))
    .unwrap();
    let write = msp_target(&msp_approval(
        serde_json::json!({"kind": "fileAccess", "path": "/repo/a", "access": "write"}),
    ))
    .unwrap();
    assert_eq!(read, r#"{"access":"read","path":"/repo/a"}"#);
    assert_ne!(read, write);
    let process = msp_target(&msp_approval(serde_json::json!({
        "kind": "process",
        "target": "git",
        "cwd": "/repo",
        "args": ["push", "--force"]
    })))
    .unwrap();
    assert_eq!(
        process,
        r#"{"args":["push","--force"],"cwd":"/repo","target":"git"}"#
    );
    let network = msp_target(&msp_approval(serde_json::json!({
        "kind": "network", "host": "example.com", "port": 443, "protocol": "https"
    })))
    .unwrap();
    assert_eq!(
        network,
        r#"{"host":"example.com","port":443,"protocol":"https"}"#
    );
    let tool = msp_target(&msp_approval(serde_json::json!({
        "kind": "tool", "toolName": "fs.delete", "arguments": {"z": 1, "a": ["x"]}
    })))
    .unwrap();
    assert_eq!(
        tool,
        r#"{"arguments":{"a":["x"],"z":1},"toolName":"fs.delete"}"#
    );

    // A permit written for the path alone does not cover the write.
    let work = "task/1";
    let intent = Intent {
        observation_id: "obs-1".to_owned(),
        operation: "msp.file_access".to_owned(),
        target: read.clone(),
        wait: WaitKind::Approval,
        deadline_ms: None,
    };
    let path_only = permit(Effect::ApproveOnce, work, "msp.file_access", "/repo/a");
    let write_screen = screen_from_msp_pending(
        "sess-1",
        &msp_approval(
            serde_json::json!({"kind": "fileAccess", "path": "/repo/a", "access": "write"}),
        ),
    );
    assert_eq!(
        judge(
            work,
            &intent,
            Some(&write_screen),
            std::slice::from_ref(&path_only),
            1_000,
            None
        ),
        Judgement::Hold {
            reason: REASON_MISMATCH,
            terminal: true
        }
    );
    let read_screen = screen_from_msp_pending(
        "sess-1",
        &msp_approval(
            serde_json::json!({"kind": "fileAccess", "path": "/repo/a", "access": "read"}),
        ),
    );
    assert_eq!(
        judge(work, &intent, Some(&read_screen), &[path_only], 1_000, None),
        Judgement::Hold {
            reason: REASON_OUT_OF_SCOPE,
            terminal: true
        }
    );
    let whole = permit(Effect::ApproveOnce, work, "msp.file_access", &read);
    assert!(matches!(
        judge(work, &intent, Some(&read_screen), &[whole], 1_000, None),
        Judgement::Act { .. }
    ));
}

#[test]
fn msp_subject_with_an_unknown_or_malformed_field_is_unknown() {
    for subject in [
        serde_json::json!({"kind": "fileAccess", "path": "/repo/a", "recursive": true}),
        serde_json::json!({"kind": "network", "host": "example.com", "method": "POST"}),
        serde_json::json!({"kind": "process", "target": "git", "env": {"A": "1"}}),
        serde_json::json!({"kind": "tool", "toolName": "fs.delete", "input": {}}),
        serde_json::json!({"kind": "shell", "command": "ls", "sandbox": false}),
        serde_json::json!({"kind": "unixSocket", "path": "/run/a.sock", "mode": "rw"}),
        // Known keys of the wrong shape.
        serde_json::json!({"kind": "process", "target": "git", "args": "push --force"}),
        serde_json::json!({"kind": "network", "host": "example.com", "port": 70000}),
        serde_json::json!({"kind": "fileAccess", "path": ""}),
        serde_json::json!({"kind": "fileAccess", "access": "read"}),
    ] {
        assert_eq!(
            screen_from_msp_pending("sess-1", &msp_approval(subject.clone())),
            Screen::Unknown,
            "{subject}"
        );
    }
}

fn msp_question(question: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "approvals": [],
        "userInputs": [{
            "sessionId": "sess-1",
            "userInputId": "prompt-1",
            "questions": [question]
        }]
    })
}

#[test]
fn msp_continue_target_names_the_question_not_only_its_id() {
    let base = serde_json::json!({
        "id": "next-step",
        "question": "次段の作業を始めて",
        "options": [{"label": "はい"}, {"label": "いいえ"}],
        "selection": {"kind": "single"}
    });
    let first = msp_target(&msp_question(base.clone())).unwrap();
    let mut reworded = base.clone();
    reworded["question"] = serde_json::json!("本番へ反映して");
    let mut more_choices = base.clone();
    more_choices["options"] = serde_json::json!([{"label": "はい"}, {"label": "全部はい"}]);
    let mut multi = base.clone();
    multi["selection"] = serde_json::json!({"kind": "multiple"});
    for other in [reworded, more_choices, multi] {
        assert_ne!(msp_target(&msp_question(other)).unwrap(), first);
    }

    // A permit for the first question does not continue a reworded one that
    // reuses the id.
    let work = "task/1";
    let intent = Intent {
        observation_id: "obs-2".to_owned(),
        operation: "msp.user_input".to_owned(),
        target: first.clone(),
        wait: WaitKind::Continue,
        deadline_ms: None,
    };
    let mut changed = base.clone();
    changed["question"] = serde_json::json!("本番へ反映して");
    let changed_screen = screen_from_msp_pending("sess-1", &msp_question(changed));
    let allow = permit(Effect::ContinueOnce, work, "msp.user_input", &first);
    assert_eq!(
        judge(
            work,
            &intent,
            Some(&changed_screen),
            std::slice::from_ref(&allow),
            1_000,
            None
        ),
        Judgement::Hold {
            reason: REASON_MISMATCH,
            terminal: true
        }
    );
    let same_screen = screen_from_msp_pending("sess-1", &msp_question(base));
    assert!(matches!(
        judge(work, &intent, Some(&same_screen), &[allow], 1_000, None),
        Judgement::Act {
            effect: Effect::ContinueOnce,
            ..
        }
    ));
}

#[test]
fn msp_question_with_an_unknown_or_malformed_field_is_unknown() {
    for question in [
        serde_json::json!({"id": "next-step", "question": "続けて", "default": "yes"}),
        serde_json::json!({"id": "next-step", "question": "続けて", "options": [{"label": "はい", "autoSelect": true}]}),
        serde_json::json!({"id": "next-step", "question": "続けて", "selection": {"kind": "single", "max": 2}}),
        serde_json::json!({"id": "next-step", "question": "続けて", "selection": "single"}),
        serde_json::json!({"id": "next-step"}),
        serde_json::json!("next-step"),
        serde_json::json!({"id": "next-step", "question": "あ".repeat(200)}),
    ] {
        assert_eq!(
            screen_from_msp_pending("sess-1", &msp_question(question.clone())),
            Screen::Unknown,
            "{question}"
        );
    }
}

#[test]
fn a_settled_claim_of_another_work_item_is_not_replayed_or_skipped() {
    let dir = TempDir::new();
    let intent = approval_intent();
    // task/2 already acted under the same observation id and its result never
    // reached the board.
    let foreign_result = format!(
        "{EVENT_PREFIX}{{\"v\":1,\"rec\":\"result\",\"work\":\"task/2\",\"obs\":\"{}\",\"effect\":\"approve_once\",\"permit\":\"permit-2\"}}",
        intent.observation_id
    );
    let claims = serde_json::json!({
        "schema_version": 1,
        "claims": [{
            "work_id": "task/2",
            "observation_id": intent.observation_id,
            "phase": "settled",
            "event_text": foreign_result,
        }]
    });
    fs::write(dir.claim(), claims.to_string()).unwrap();
    let work = "task/1";
    let watcher = open_watcher(
        &dir,
        Board::with_intent(work, &intent),
        Ok(Some(approval_screen())),
        vec![permit(
            Effect::ApproveOnce,
            work,
            &intent.operation,
            &intent.target,
        )],
        false,
    );
    let outcome = watcher.tick(1_000).unwrap();
    assert_eq!(outcome.len(), 1);
    assert_eq!(outcome[0].status, Status::Acted);
    assert_eq!(calls(&watcher).len(), 1);
    assert_eq!(calls(&watcher)[0].work_id, work);
    // task/1's board never receives task/2's record.
    assert!(
        !watcher
            .board
            .texts()
            .iter()
            .any(|text| text.contains("\"work\":\"task/2\""))
    );
}
