//! Decisions for an external AI that may be waiting on an approval card or a
//! continuation prompt (issue #48).
//!
//! Progress and outcomes stay on the Iris board as `note` events. A screen
//! observation is a separate input: a stalled board record is not treated as
//! an approval. Permits are an operator file, not board text, so a writer who
//! can post events cannot widen what this watcher will do.
//!
//! [`screen_from_msp_pending`] reads a Muse Code `approval/listPending` result.
//! This module does not spawn `muse` and does not send `approval/decide`. Hub
//! startup does not construct a [`Watcher`]. One process should call [`Watcher::tick`]
//! at a time; a claim left in `acting` is never executed again. Claims are kept
//! per work item and observation, and one card is answered under at most one
//! observation of a work item.
//!
//! Silence and a disconnected observation are not completion. This function
//! does not turn a quiet poll into a retry.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub const SCHEMA_VERSION: u32 = 1;
/// Prefix on a board `note`. Other notes are ignored.
pub const EVENT_PREFIX: &str = "followup/1 ";
const EVENT_MAX: usize = 2000;
const TOKEN_MAX: usize = 128;
const LABEL_MAX: usize = 200;

pub const REASON_SCREEN_UNCONFIRMED: &str = "screen_unconfirmed";
pub const REASON_DISCONNECTED: &str = "disconnected";
pub const REASON_UNKNOWN: &str = "unknown";
pub const REASON_RUNNING: &str = "running";
pub const REASON_OUT_OF_SCOPE: &str = "out_of_scope";
pub const REASON_EXPIRED: &str = "expired";
pub const REASON_REVOKED: &str = "revoked";
pub const REASON_AMBIGUOUS: &str = "ambiguous";
pub const REASON_MISMATCH: &str = "mismatch";
pub const REASON_CHANGED: &str = "changed";
pub const REASON_UNKNOWN_AFTER_INTERRUPT: &str = "unknown_after_interrupt";
pub const REASON_NOT_PRESENTED: &str = "not_presented";
pub const REASON_ACTION_FAILED: &str = "action_failed";
pub const REASON_MALFORMED: &str = "malformed";
pub const REASON_OBSERVATION_FAILED: &str = "observation_failed";
pub const REASON_BOARD_UNRECORDED: &str = "board_unrecorded";
pub const REASON_DUPLICATE: &str = "duplicate";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FollowupError {
    Board,
    Claim,
    Permits,
    Action,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitKind {
    Approval,
    Continue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    ApproveOnce,
    ContinueOnce,
}

impl Effect {
    fn as_str(self) -> &'static str {
        match self {
            Self::ApproveOnce => "approve_once",
            Self::ContinueOnce => "continue_once",
        }
    }

    fn matches(self, wait: WaitKind) -> bool {
        matches!(
            (self, wait),
            (Self::ApproveOnce, WaitKind::Approval) | (Self::ContinueOnce, WaitKind::Continue)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Approval {
        card_id: String,
        operation: String,
        target: String,
    },
    Continue {
        prompt_id: String,
        operation: String,
        target: String,
    },
    Running,
    Done,
    Disconnected,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub observation_id: String,
    pub operation: String,
    pub target: String,
    pub wait: WaitKind,
    pub deadline_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permit {
    pub permit_id: String,
    pub work_id: String,
    pub operation: String,
    pub target: String,
    pub effect: Effect,
    pub expires_at_ms: u64,
    pub revoked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub work_id: String,
    pub observation_id: String,
    pub effect: Effect,
    pub operation: String,
    pub target: String,
    pub permit_id: String,
    pub card_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Acted,
    Held,
    Waiting,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub work_id: String,
    pub observation_id: String,
    pub status: Status,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Judgement {
    Act {
        permit_id: String,
        effect: Effect,
        card_id: String,
    },
    Hold {
        reason: &'static str,
        terminal: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemSnapshot {
    pub id: String,
    pub events: Vec<String>,
}

pub trait BoardPort {
    fn list_items(&self) -> Result<Vec<ItemSnapshot>, FollowupError>;
    fn append_note(&self, item_id: &str, text: &str) -> Result<(), FollowupError>;
}

pub trait ScreenSource {
    fn screen(&self, work_id: &str) -> Result<Option<Screen>, FollowupError>;
}

pub trait FollowupExecutor {
    fn act(&self, action: &Action) -> Result<(), FollowupError>;
}

/// Pure comparison. `last_card` is the card or prompt id already written on
/// the board for this observation. A different id is another prompt, and this
/// function will not answer it under the old record.
pub fn judge(
    work_id: &str,
    intent: &Intent,
    screen: Option<&Screen>,
    permits: &[Permit],
    now_ms: u64,
    last_card: Option<&str>,
) -> Judgement {
    let Some(screen) = screen else {
        return hold(REASON_SCREEN_UNCONFIRMED, false);
    };
    let (card_id, operation, target, seen_wait) = match screen {
        Screen::Disconnected => return hold(REASON_DISCONNECTED, false),
        Screen::Unknown => return hold(REASON_UNKNOWN, false),
        Screen::Running => return hold(REASON_RUNNING, false),
        Screen::Done => return hold(REASON_NOT_PRESENTED, true),
        Screen::Approval {
            card_id,
            operation,
            target,
        } => (card_id, operation, target, WaitKind::Approval),
        Screen::Continue {
            prompt_id,
            operation,
            target,
        } => (prompt_id, operation, target, WaitKind::Continue),
    };
    if !valid_token(card_id) || !valid_label(operation) || !valid_label(target) {
        return hold(REASON_AMBIGUOUS, true);
    }
    if seen_wait != intent.wait || operation != &intent.operation || target != &intent.target {
        return hold(REASON_MISMATCH, true);
    }
    if last_card.is_some_and(|seen| seen != card_id) {
        return hold(REASON_CHANGED, true);
    }
    if intent
        .deadline_ms
        .is_some_and(|deadline| deadline <= now_ms)
    {
        return hold(REASON_EXPIRED, true);
    }
    let scoped: Vec<&Permit> = permits
        .iter()
        .filter(|permit| {
            permit.work_id == work_id
                && permit.operation == intent.operation
                && permit.target == intent.target
                && permit.effect.matches(intent.wait)
        })
        .collect();
    if scoped.is_empty() {
        return hold(REASON_OUT_OF_SCOPE, true);
    }
    // A revoked or lapsed row beside its replacement is history, not a second
    // choice. Only two rows that could both answer are ambiguous.
    let live: Vec<&&Permit> = scoped
        .iter()
        .filter(|permit| !permit.revoked && permit.expires_at_ms > now_ms)
        .collect();
    let permit = match live.as_slice() {
        [permit] => permit,
        [] if scoped.iter().all(|permit| permit.revoked) => {
            return hold(REASON_REVOKED, true);
        }
        [] => return hold(REASON_EXPIRED, true),
        _ => return hold(REASON_AMBIGUOUS, true),
    };
    Judgement::Act {
        permit_id: permit.permit_id.clone(),
        effect: permit.effect,
        card_id: card_id.clone(),
    }
}

fn hold(reason: &'static str, terminal: bool) -> Judgement {
    Judgement::Hold { reason, terminal }
}

pub fn load_permits(text: &str) -> Result<Vec<Permit>, FollowupError> {
    let file: PermitFile = serde_json::from_str(text).map_err(|_| FollowupError::Permits)?;
    if file.schema_version != SCHEMA_VERSION {
        return Err(FollowupError::Permits);
    }
    let mut seen = Vec::new();
    let mut permits = Vec::with_capacity(file.permits.len());
    for row in file.permits {
        let permit = permit_from_row(row)?;
        if seen.iter().any(|id: &String| id == &permit.permit_id) {
            return Err(FollowupError::Permits);
        }
        seen.push(permit.permit_id.clone());
        permits.push(permit);
    }
    Ok(permits)
}

fn permit_from_row(row: PermitRow) -> Result<Permit, FollowupError> {
    let effect = match row.effect.as_str() {
        "approve_once" => Effect::ApproveOnce,
        "continue_once" => Effect::ContinueOnce,
        _ => return Err(FollowupError::Permits),
    };
    if !valid_token(&row.permit_id)
        || !valid_token(&row.work_id)
        || !valid_label(&row.operation)
        || !valid_label(&row.target)
        || row.expires_at_ms == 0
    {
        return Err(FollowupError::Permits);
    }
    Ok(Permit {
        permit_id: row.permit_id,
        work_id: row.work_id,
        operation: row.operation,
        target: row.target,
        effect,
        expires_at_ms: row.expires_at_ms,
        revoked: row.revoked,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PermitFile {
    schema_version: u32,
    permits: Vec<PermitRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PermitRow {
    permit_id: String,
    work_id: String,
    operation: String,
    target: String,
    effect: String,
    expires_at_ms: u64,
    revoked: bool,
}

pub struct ClaimStore {
    path: PathBuf,
    inner: Mutex<ClaimFile>,
}

impl ClaimStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, FollowupError> {
        let path = path.into();
        let inner = if path.exists() {
            let text = fs::read_to_string(&path).map_err(|_| FollowupError::Claim)?;
            let file: ClaimFile = serde_json::from_str(&text).map_err(|_| FollowupError::Claim)?;
            if file.schema_version != SCHEMA_VERSION {
                return Err(FollowupError::Claim);
            }
            file
        } else {
            ClaimFile {
                schema_version: SCHEMA_VERSION,
                claims: Vec::new(),
            }
        };
        Ok(Self {
            path,
            inner: Mutex::new(inner),
        })
    }

    /// Observation ids are written by each work item's external AI, so two
    /// items may reuse one. A claim is the pair, never the observation alone.
    fn begin(&self, work_id: &str, observation_id: &str) -> Result<Begin, FollowupError> {
        let mut file = self.inner.lock().map_err(|_| FollowupError::Claim)?;
        if let Some(row) = file
            .claims
            .iter()
            .find(|row| row.is(work_id, observation_id))
        {
            return Ok(Begin::Existing(row.phase.clone()));
        }
        file.claims.push(ClaimRow {
            work_id: work_id.to_owned(),
            observation_id: observation_id.to_owned(),
            phase: "acting".to_owned(),
            event_text: None,
        });
        if let Err(error) = self.persist(&file) {
            // Nothing was executed; memory must not claim more than the disk.
            file.claims.pop();
            return Err(error);
        }
        Ok(Begin::Started)
    }

    fn settle(
        &self,
        work_id: &str,
        observation_id: &str,
        event_text: &str,
    ) -> Result<(), FollowupError> {
        let mut file = self.inner.lock().map_err(|_| FollowupError::Claim)?;
        let Some(row) = file
            .claims
            .iter_mut()
            .find(|row| row.is(work_id, observation_id))
        else {
            return Err(FollowupError::Claim);
        };
        row.phase = "settled".to_owned();
        row.event_text = Some(event_text.to_owned());
        self.persist(&file)
    }

    fn lookup(
        &self,
        work_id: &str,
        observation_id: &str,
    ) -> Result<Option<(String, Option<String>)>, FollowupError> {
        let file = self.inner.lock().map_err(|_| FollowupError::Claim)?;
        Ok(file
            .claims
            .iter()
            .find(|row| row.is(work_id, observation_id))
            .map(|row| (row.phase.clone(), row.event_text.clone())))
    }

    /// Replaces the file whole, so an interrupted write leaves the previous
    /// claims rather than a truncated file that refuses to open.
    fn persist(&self, file: &ClaimFile) -> Result<(), FollowupError> {
        let text = serde_json::to_string(file).map_err(|_| FollowupError::Claim)?;
        let path = if self
            .path
            .parent()
            .is_some_and(|parent| parent.as_os_str().is_empty())
        {
            Path::new(".").join(&self.path)
        } else {
            self.path.clone()
        };
        crate::config::private_io::write_atomic(&path, text.as_bytes())
            .map_err(|_| FollowupError::Claim)
    }
}

enum Begin {
    Started,
    Existing(String),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimFile {
    schema_version: u32,
    claims: Vec<ClaimRow>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimRow {
    work_id: String,
    observation_id: String,
    phase: String,
    event_text: Option<String>,
}

impl ClaimRow {
    fn is(&self, work_id: &str, observation_id: &str) -> bool {
        self.work_id == work_id && self.observation_id == observation_id
    }
}

pub struct Watcher<B, S, E> {
    board: B,
    screens: S,
    executor: E,
    claims: ClaimStore,
    permits: Vec<Permit>,
    tick_lock: Mutex<()>,
}

impl<B, S, E> Watcher<B, S, E>
where
    B: BoardPort,
    S: ScreenSource,
    E: FollowupExecutor,
{
    pub fn new(
        board: B,
        screens: S,
        executor: E,
        claims: ClaimStore,
        permits: Vec<Permit>,
    ) -> Self {
        Self {
            board,
            screens,
            executor,
            claims,
            permits,
            tick_lock: Mutex::new(()),
        }
    }

    pub fn tick(&self, now_ms: u64) -> Result<Vec<Outcome>, FollowupError> {
        let _guard = self.tick_lock.lock().map_err(|_| FollowupError::Claim)?;
        let items = self.board.list_items()?;
        let mut outcomes = Vec::new();
        for item in items {
            self.tick_item(&item, now_ms, &mut outcomes)?;
        }
        Ok(outcomes)
    }

    fn tick_item(
        &self,
        item: &ItemSnapshot,
        now_ms: u64,
        outcomes: &mut Vec<Outcome>,
    ) -> Result<(), FollowupError> {
        let mut opens = Vec::new();
        // Every card already recorded against an observation of this item, in
        // board order. A card belongs to the first observation that saw it.
        let mut sightings: Vec<(String, String)> = Vec::new();
        for text in &item.events {
            let Some(rest) = text.strip_prefix(EVENT_PREFIX) else {
                continue;
            };
            match parse_event(rest) {
                Ok(record) if record.work_id != item.id => outcomes.push(outcome(
                    &item.id,
                    &record.observation_id,
                    Status::Held,
                    REASON_MISMATCH,
                )),
                Ok(record) => {
                    if let Event::Seen { card_id, .. } = &record.event {
                        sightings.push((record.observation_id.clone(), card_id.clone()));
                    }
                    apply_record(&mut opens, record.event);
                }
                Err(()) => outcomes.push(Outcome {
                    work_id: item.id.clone(),
                    observation_id: String::new(),
                    status: Status::Held,
                    reason: REASON_MALFORMED.to_owned(),
                }),
            }
        }
        for open in opens {
            if let Some(outcome) =
                self.tick_open(&item.id, &open, &item.events, &mut sightings, now_ms)?
            {
                outcomes.push(outcome);
            }
        }
        Ok(())
    }

    fn tick_open(
        &self,
        work_id: &str,
        open: &OpenIntent,
        events: &[String],
        sightings: &mut Vec<(String, String)>,
        now_ms: u64,
    ) -> Result<Option<Outcome>, FollowupError> {
        if open.conflict {
            return self.finish_hold(work_id, open, REASON_MISMATCH);
        }
        if open.settled {
            return Ok(Some(skipped(work_id, &open.observation_id, "settled")));
        }
        match self.claims.lookup(work_id, &open.observation_id)? {
            Some((phase, text)) if phase == "settled" => {
                if let Some(text) = text
                    && !events.iter().any(|event| event == &text)
                    && self.board.append_note(work_id, &text).is_err()
                {
                    return Ok(Some(outcome(
                        work_id,
                        &open.observation_id,
                        Status::Skipped,
                        REASON_BOARD_UNRECORDED,
                    )));
                }
                return Ok(Some(skipped(work_id, &open.observation_id, "settled")));
            }
            Some((phase, _)) if phase == "acting" => {
                return self.finish_hold(work_id, open, REASON_UNKNOWN_AFTER_INTERRUPT);
            }
            _ => {}
        }
        let screen = match self.screens.screen(work_id) {
            Ok(screen) => screen,
            Err(_) => {
                return Ok(Some(outcome(
                    work_id,
                    &open.observation_id,
                    Status::Waiting,
                    REASON_OBSERVATION_FAILED,
                )));
            }
        };
        let judgement = judge(
            work_id,
            &open.intent,
            screen.as_ref(),
            &self.permits,
            now_ms,
            open.last_card.as_deref(),
        );
        match judgement {
            Judgement::Hold { reason, terminal } if !terminal => {
                self.note_transient(work_id, open, events, reason)
            }
            Judgement::Hold { reason, .. } => self.finish_hold(work_id, open, reason),
            Judgement::Act { card_id, .. }
                if sightings
                    .iter()
                    .any(|(obs, card)| card == &card_id && obs != &open.observation_id) =>
            {
                // Answered, or tried and failed, under another observation.
                // Answering it here would be a second click on one card.
                self.note_transient(work_id, open, events, REASON_DUPLICATE)
            }
            Judgement::Act {
                permit_id,
                effect,
                card_id,
            } => {
                sightings.push((open.observation_id.clone(), card_id.clone()));
                self.act(work_id, open, events, permit_id, effect, card_id)
            }
        }
    }

    fn note_transient(
        &self,
        work_id: &str,
        open: &OpenIntent,
        events: &[String],
        reason: &str,
    ) -> Result<Option<Outcome>, FollowupError> {
        if open.last_transient.as_deref() != Some(reason) {
            let text = encode(
                work_id,
                &Event::Hold {
                    observation_id: open.observation_id.clone(),
                    reason: reason.to_owned(),
                    terminal: false,
                },
            )?;
            if !events.iter().any(|event| event == &text)
                && self.board.append_note(work_id, &text).is_err()
            {
                // Still waiting; the note is written again on the next poll.
                return Ok(Some(outcome(
                    work_id,
                    &open.observation_id,
                    Status::Waiting,
                    REASON_BOARD_UNRECORDED,
                )));
            }
        }
        Ok(Some(outcome(
            work_id,
            &open.observation_id,
            Status::Waiting,
            reason,
        )))
    }

    fn finish_hold(
        &self,
        work_id: &str,
        open: &OpenIntent,
        reason: &str,
    ) -> Result<Option<Outcome>, FollowupError> {
        let text = encode(
            work_id,
            &Event::Hold {
                observation_id: open.observation_id.clone(),
                reason: reason.to_owned(),
                terminal: true,
            },
        )?;
        match self.claims.begin(work_id, &open.observation_id)? {
            Begin::Started => {}
            Begin::Existing(phase) if phase == "acting" => {}
            Begin::Existing(phase) if phase == "settled" => {
                return Ok(Some(outcome(
                    work_id,
                    &open.observation_id,
                    Status::Held,
                    reason,
                )));
            }
            Begin::Existing(_) => return Err(FollowupError::Claim),
        }
        self.finish_after_claim(work_id, open, reason, Status::Held, &text)
    }

    fn act(
        &self,
        work_id: &str,
        open: &OpenIntent,
        events: &[String],
        permit_id: String,
        effect: Effect,
        card_id: String,
    ) -> Result<Option<Outcome>, FollowupError> {
        if open.last_card.as_deref() != Some(card_id.as_str()) {
            let seen = encode(
                work_id,
                &Event::Seen {
                    observation_id: open.observation_id.clone(),
                    card_id: card_id.clone(),
                },
            )?;
            // The sighting is what tells a later poll, or a restarted watcher,
            // which card this observation owns. Do not answer without it.
            if !events.iter().any(|event| event == &seen)
                && self.board.append_note(work_id, &seen).is_err()
            {
                return Ok(Some(outcome(
                    work_id,
                    &open.observation_id,
                    Status::Waiting,
                    REASON_BOARD_UNRECORDED,
                )));
            }
        }
        match self.claims.begin(work_id, &open.observation_id)? {
            Begin::Started => {}
            Begin::Existing(_) => {
                return Ok(Some(skipped(work_id, &open.observation_id, "claimed")));
            }
        }
        let action = Action {
            work_id: work_id.to_owned(),
            observation_id: open.observation_id.clone(),
            effect,
            operation: open.intent.operation.clone(),
            target: open.intent.target.clone(),
            permit_id: permit_id.clone(),
            card_id,
        };
        if self.executor.act(&action).is_err() {
            let text = encode(
                work_id,
                &Event::Hold {
                    observation_id: open.observation_id.clone(),
                    reason: REASON_ACTION_FAILED.to_owned(),
                    terminal: true,
                },
            )?;
            return self.finish_after_claim(
                work_id,
                open,
                REASON_ACTION_FAILED,
                Status::Held,
                &text,
            );
        }
        let text = encode(
            work_id,
            &Event::Result {
                observation_id: open.observation_id.clone(),
                effect: effect.as_str().to_owned(),
                permit_id,
            },
        )?;
        self.finish_after_claim(work_id, open, effect.as_str(), Status::Acted, &text)
    }

    /// Settles a claim already in `acting`, then writes the record. A board
    /// that refuses the write is reported; the settled claim re-sends the same
    /// text on a later poll instead of acting again.
    fn finish_after_claim(
        &self,
        work_id: &str,
        open: &OpenIntent,
        reason: &str,
        status: Status,
        text: &str,
    ) -> Result<Option<Outcome>, FollowupError> {
        self.claims.settle(work_id, &open.observation_id, text)?;
        if self.board.append_note(work_id, text).is_err() {
            return Ok(Some(outcome(
                work_id,
                &open.observation_id,
                status,
                REASON_BOARD_UNRECORDED,
            )));
        }
        Ok(Some(outcome(work_id, &open.observation_id, status, reason)))
    }
}

struct OpenIntent {
    intent: Intent,
    last_card: Option<String>,
    last_transient: Option<String>,
    settled: bool,
    conflict: bool,
    observation_id: String,
}

fn apply_record(opens: &mut Vec<OpenIntent>, record: Event) {
    match record {
        Event::Intent {
            observation_id,
            operation,
            target,
            wait,
            deadline_ms,
        } => {
            if let Some(open) = opens
                .iter_mut()
                .find(|open| open.observation_id == observation_id)
            {
                if open.intent.operation != operation
                    || open.intent.target != target
                    || open.intent.wait != wait
                    || open.intent.deadline_ms != deadline_ms
                {
                    open.conflict = true;
                }
                return;
            }
            opens.push(OpenIntent {
                observation_id: observation_id.clone(),
                intent: Intent {
                    observation_id,
                    operation,
                    target,
                    wait,
                    deadline_ms,
                },
                last_card: None,
                last_transient: None,
                settled: false,
                conflict: false,
            });
        }
        Event::Seen {
            observation_id,
            card_id,
        } => {
            if let Some(open) = opens
                .iter_mut()
                .find(|open| open.observation_id == observation_id)
            {
                open.last_card = Some(card_id);
            }
        }
        Event::Hold {
            observation_id,
            reason,
            terminal,
        } => {
            if let Some(open) = opens
                .iter_mut()
                .find(|open| open.observation_id == observation_id)
            {
                if terminal {
                    open.settled = true;
                } else {
                    open.last_transient = Some(reason);
                }
            }
        }
        Event::Result { observation_id, .. } => {
            if let Some(open) = opens
                .iter_mut()
                .find(|open| open.observation_id == observation_id)
            {
                open.settled = true;
            }
        }
    }
}

enum Event {
    Intent {
        observation_id: String,
        operation: String,
        target: String,
        wait: WaitKind,
        deadline_ms: Option<u64>,
    },
    Seen {
        observation_id: String,
        card_id: String,
    },
    Hold {
        observation_id: String,
        reason: String,
        terminal: bool,
    },
    Result {
        observation_id: String,
        effect: String,
        permit_id: String,
    },
}

struct Parsed {
    work_id: String,
    observation_id: String,
    event: Event,
}

fn encode(work_id: &str, event: &Event) -> Result<String, FollowupError> {
    let body = match event {
        Event::Intent {
            observation_id,
            operation,
            target,
            wait,
            deadline_ms,
        } => serde_json::json!({
            "v": SCHEMA_VERSION,
            "rec": "intent",
            "work": work_id,
            "obs": observation_id,
            "op": operation,
            "target": target,
            "wait": match wait { WaitKind::Approval => "approval", WaitKind::Continue => "continue" },
            "deadline_ms": deadline_ms,
        }),
        Event::Seen {
            observation_id,
            card_id,
        } => serde_json::json!({
            "v": SCHEMA_VERSION,
            "rec": "seen",
            "work": work_id,
            "obs": observation_id,
            "card": card_id,
        }),
        Event::Hold {
            observation_id,
            reason,
            terminal,
        } => serde_json::json!({
            "v": SCHEMA_VERSION,
            "rec": "hold",
            "work": work_id,
            "obs": observation_id,
            "reason": reason,
            "terminal": terminal,
        }),
        Event::Result {
            observation_id,
            effect,
            permit_id,
        } => serde_json::json!({
            "v": SCHEMA_VERSION,
            "rec": "result",
            "work": work_id,
            "obs": observation_id,
            "effect": effect,
            "permit": permit_id,
        }),
    };
    let text = format!("{EVENT_PREFIX}{body}");
    if text.chars().count() > EVENT_MAX {
        return Err(FollowupError::Board);
    }
    Ok(text)
}

fn parse_event(rest: &str) -> Result<Parsed, ()> {
    let value: serde_json::Value = serde_json::from_str(rest).map_err(|_| ())?;
    let obj = value.as_object().ok_or(())?;
    if obj.get("v").and_then(serde_json::Value::as_u64) != Some(u64::from(SCHEMA_VERSION)) {
        return Err(());
    }
    let rec = obj
        .get("rec")
        .and_then(serde_json::Value::as_str)
        .ok_or(())?;
    let allowed: &[&str] = match rec {
        "intent" => &[
            "v",
            "rec",
            "work",
            "obs",
            "op",
            "target",
            "wait",
            "deadline_ms",
        ],
        "seen" => &["v", "rec", "work", "obs", "card"],
        "hold" => &["v", "rec", "work", "obs", "reason", "terminal"],
        "result" => &["v", "rec", "work", "obs", "effect", "permit"],
        _ => return Err(()),
    };
    if obj.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(());
    }
    let work_id = required_token(obj, "work")?;
    let obs = required_token(obj, "obs")?;
    let event = match rec {
        "intent" => {
            let operation = required_label(obj, "op")?;
            let target = required_label(obj, "target")?;
            let wait = match obj.get("wait").and_then(serde_json::Value::as_str) {
                Some("approval") => WaitKind::Approval,
                Some("continue") => WaitKind::Continue,
                _ => return Err(()),
            };
            let deadline_ms = match obj.get("deadline_ms") {
                None | Some(serde_json::Value::Null) => None,
                Some(value) => Some(value.as_u64().filter(|n| *n > 0).ok_or(())?),
            };
            Ok(Event::Intent {
                observation_id: obs.clone(),
                operation,
                target,
                wait,
                deadline_ms,
            })
        }
        "seen" => Ok(Event::Seen {
            observation_id: obs.clone(),
            card_id: required_token(obj, "card")?,
        }),
        "hold" => {
            let reason = obj
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .filter(|reason| valid_token(reason))
                .ok_or(())?
                .to_owned();
            let terminal = obj
                .get("terminal")
                .and_then(serde_json::Value::as_bool)
                .ok_or(())?;
            Ok(Event::Hold {
                observation_id: obs.clone(),
                reason,
                terminal,
            })
        }
        "result" => Ok(Event::Result {
            observation_id: obs.clone(),
            effect: required_token(obj, "effect")?,
            permit_id: required_token(obj, "permit")?,
        }),
        _ => Err(()),
    }?;
    Ok(Parsed {
        work_id,
        observation_id: obs,
        event,
    })
}

fn required_token(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, ()> {
    let text = obj.get(key).and_then(serde_json::Value::as_str).ok_or(())?;
    if valid_token(text) {
        Ok(text.to_owned())
    } else {
        Err(())
    }
}

fn required_label(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, ()> {
    let text = obj.get(key).and_then(serde_json::Value::as_str).ok_or(())?;
    if valid_label(text) {
        Ok(text.to_owned())
    } else {
        Err(())
    }
}

fn valid_token(text: &str) -> bool {
    let count = text.chars().count();
    (1..=TOKEN_MAX).contains(&count)
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'))
}

fn valid_label(text: &str) -> bool {
    let count = text.chars().count();
    (1..=LABEL_MAX).contains(&count) && text.chars().all(|c| !c.is_control())
}

/// Maps one official `approval/listPending` result into a screen.
///
/// An empty result is [`Screen::Running`]: the read succeeded and nothing is
/// waiting. It is not completion. More than one pending record, an unknown
/// approval kind, a missing `allow_once` choice, or a session id that is not
/// `session_id` becomes [`Screen::Unknown`], so the watcher does not act.
pub fn screen_from_msp_pending(session_id: &str, body: &serde_json::Value) -> Screen {
    if !valid_token(session_id) {
        return Screen::Unknown;
    }
    let Some(obj) = body.as_object() else {
        return Screen::Unknown;
    };
    let Some(approvals) = obj.get("approvals").and_then(serde_json::Value::as_array) else {
        return Screen::Unknown;
    };
    let Some(inputs) = obj.get("userInputs").and_then(serde_json::Value::as_array) else {
        return Screen::Unknown;
    };
    match (approvals.len(), inputs.len()) {
        (0, 0) => Screen::Running,
        (1, 0) => approval_screen(session_id, &approvals[0]),
        (0, 1) => continue_screen(session_id, &inputs[0]),
        _ => Screen::Unknown,
    }
}

fn approval_screen(session_id: &str, value: &serde_json::Value) -> Screen {
    let Some(obj) = value.as_object() else {
        return Screen::Unknown;
    };
    if obj.get("sessionId").and_then(serde_json::Value::as_str) != Some(session_id) {
        return Screen::Unknown;
    }
    let Some(card_id) = obj.get("approvalId").and_then(serde_json::Value::as_str) else {
        return Screen::Unknown;
    };
    if !valid_token(card_id) || !choice_offered(obj, "allow_once") {
        return Screen::Unknown;
    }
    let Some(subject) = obj.get("subject").and_then(serde_json::Value::as_object) else {
        return Screen::Unknown;
    };
    let Some(kind) = subject.get("kind").and_then(serde_json::Value::as_str) else {
        return Screen::Unknown;
    };
    let Some((operation, target)) = approval_subject(kind, subject) else {
        return Screen::Unknown;
    };
    if !valid_label(&operation) || !valid_label(&target) {
        return Screen::Unknown;
    }
    Screen::Approval {
        card_id: card_id.to_owned(),
        operation,
        target,
    }
}

fn approval_subject(
    kind: &str,
    subject: &serde_json::Map<String, serde_json::Value>,
) -> Option<(String, String)> {
    let text = |key: &str| {
        subject
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let pair = match kind {
        "shell" => ("msp.shell", text("command")?),
        "fileAccess" => ("msp.file_access", text("path")?),
        "network" => ("msp.network", network_target(subject, &text("host")?)?),
        "unixSocket" => ("msp.unix_socket", text("path")?),
        "process" => ("msp.process", text("target")?),
        "tool" => ("msp.tool", text("toolName")?),
        _ => return None,
    };
    Some((pair.0.to_owned(), pair.1))
}

fn network_target(
    subject: &serde_json::Map<String, serde_json::Value>,
    host: &str,
) -> Option<String> {
    match subject.get("port").and_then(serde_json::Value::as_u64) {
        Some(port) if port <= u64::from(u16::MAX) => Some(format!("{host}:{port}")),
        Some(_) => None,
        None => Some(host.to_owned()),
    }
}

fn continue_screen(session_id: &str, value: &serde_json::Value) -> Screen {
    let Some(obj) = value.as_object() else {
        return Screen::Unknown;
    };
    if obj.get("sessionId").and_then(serde_json::Value::as_str) != Some(session_id) {
        return Screen::Unknown;
    }
    let Some(prompt_id) = obj.get("userInputId").and_then(serde_json::Value::as_str) else {
        return Screen::Unknown;
    };
    let Some(questions) = obj.get("questions").and_then(serde_json::Value::as_array) else {
        return Screen::Unknown;
    };
    let [question] = questions.as_slice() else {
        return Screen::Unknown;
    };
    let Some(question_id) = question.get("id").and_then(serde_json::Value::as_str) else {
        return Screen::Unknown;
    };
    if !valid_token(prompt_id) || !valid_label(question_id) {
        return Screen::Unknown;
    }
    Screen::Continue {
        prompt_id: prompt_id.to_owned(),
        operation: "msp.user_input".to_owned(),
        target: question_id.to_owned(),
    }
}

fn choice_offered(obj: &serde_json::Map<String, serde_json::Value>, choice_id: &str) -> bool {
    obj.get("availableChoices")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|choices| {
            choices.iter().any(|choice| {
                choice.get("choiceId").and_then(serde_json::Value::as_str) == Some(choice_id)
            })
        })
}

fn outcome(work_id: &str, observation_id: &str, status: Status, reason: &str) -> Outcome {
    Outcome {
        work_id: work_id.to_owned(),
        observation_id: observation_id.to_owned(),
        status,
        reason: reason.to_owned(),
    }
}

fn skipped(work_id: &str, observation_id: &str, reason: &str) -> Outcome {
    outcome(work_id, observation_id, Status::Skipped, reason)
}

/// Writes the note an external AI posts before an operation that may wait.
/// Refuses what [`Watcher::tick`] would read back as malformed, so a writer
/// learns at once instead of leaving a record nobody acts on.
pub fn intent_note(work_id: &str, intent: &Intent) -> Result<String, FollowupError> {
    if !valid_token(work_id)
        || !valid_token(&intent.observation_id)
        || !valid_label(&intent.operation)
        || !valid_label(&intent.target)
        || intent.deadline_ms == Some(0)
    {
        return Err(FollowupError::Invalid);
    }
    encode(
        work_id,
        &Event::Intent {
            observation_id: intent.observation_id.clone(),
            operation: intent.operation.clone(),
            target: intent.target.clone(),
            wait: intent.wait,
            deadline_ms: intent.deadline_ms,
        },
    )
}

#[cfg(test)]
mod tests;
