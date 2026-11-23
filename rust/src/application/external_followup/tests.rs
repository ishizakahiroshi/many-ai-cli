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
}
impl Board {
    fn with_intent(work_id: &str, intent: &Intent) -> Self {
        Self {
            items: Mutex::new(vec![ItemSnapshot {
                id: work_id.to_owned(),
                events: vec![
                    "session started".to_owned(),
                    intent_note(work_id, intent).unwrap(),
                ],
            }]),
        }
    }
    fn texts(&self) -> Vec<String> {
        self.items.lock().unwrap()[0].events.clone()
    }
}
impl BoardPort for Board {
    fn list_items(&self) -> Result<Vec<ItemSnapshot>, FollowupError> {
        Ok(self.items.lock().unwrap().clone())
    }
    fn append_note(&self, item_id: &str, text: &str) -> Result<(), FollowupError> {
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
    let ok = r#"{"schema_version":1,"permits":[{"permit_id":"permit-1","work_id":"task/1","operation":"github.update_pr_body","target":"https://github.com/example/repo/pull/49","effect":"approve_once","expires_at_ms":9,"revoked":false}]}"#;
    assert_eq!(load_permits(ok).unwrap().len(), 1);
    assert!(load_permits(&ok.replace("revoked", "revoked_at")).is_err());
    let dup = ok.replacen("permit-1", "permit-1", 1);
    let two = format!(
        "{{\"schema_version\":1,\"permits\":[{},{}]}}",
        &ok[ok.find("{\"permit_id\"").unwrap()..ok.rfind('}').unwrap()],
        &ok[ok.find("{\"permit_id\"").unwrap()..ok.rfind('}').unwrap()]
    );
    let _ = dup;
    assert!(load_permits(&two).is_err());
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
            &[approval_permit.clone()],
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
    let claim = format!(
        "{{\"schema_version\":1,\"claims\":[{{\"observation_id\":\"{}\",\"phase\":\"acting\",\"event_text\":null}}]}}",
        intent.observation_id
    );
    fs::write(dir.claim(), claim).unwrap();
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
    let board = Board {
        items: Mutex::new(vec![ItemSnapshot {
            id: work.to_owned(),
            events: vec![
                "作業しました".to_owned(),
                foreign,
                "followup/1 {".to_owned(),
            ],
        }]),
    };
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
        Board {
            items: Mutex::new(vec![ItemSnapshot {
                id: work.to_owned(),
                events,
            }]),
        },
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
            target: "git status".to_owned(),
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
            target: "next-step".to_owned(),
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
