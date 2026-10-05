use crate::proto::{WorkflowProgress, time::Timestamp};
use sha2::{Digest, Sha256};
use std::time::Duration;
pub fn signature(progress: &WorkflowProgress) -> String {
    let mut value = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        progress.detected,
        progress.source,
        progress.name,
        progress.done,
        progress.total,
        progress.running,
        progress.failed,
        progress.pending,
        progress.waiting_dynamic,
        progress.percent,
        progress.settled,
        progress.settled_by
    );
    for phase in &progress.phases {
        value.push_str("|P:");
        value.push_str(&phase.title);
        for agent in phase.agents.as_deref().unwrap_or(&[]) {
            value.push_str(&format!("|A:{}:{}", agent.state, agent.label));
        }
    }
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[derive(Default)]
pub struct VtState {
    pub progress: Option<WorkflowProgress>,
    pub has_signal: bool,
    pub signature: String,
    missing: usize,
    frozen: usize,
    pub elapsed_base: i64,
    pub elapsed_at: Option<Timestamp>,
    pub last_scan: Option<Timestamp>,
    pub due: Option<Timestamp>,
    last_broadcast: Option<Timestamp>,
    broadcast_signature: String,
    notified: bool,
    completion_signature: String,
}
pub struct Publication {
    pub progress: WorkflowProgress,
    pub completion: bool,
}
impl VtState {
    pub fn queue(&mut self, requested: Timestamp, resize_until: Option<Timestamp>) {
        let mut due = requested;
        if let Some(last) = self.last_scan {
            due = due.max(last.checked_add(Duration::from_millis(500)).unwrap_or(due));
        }
        if let Some(until) = resize_until {
            due = due.max(until)
        }
        if self.due.is_none_or(|old| due < old) {
            self.due = Some(due)
        }
    }
    pub fn scan(&mut self, mut parsed: Option<WorkflowProgress>, idle: bool, now: Timestamp) {
        self.due = None;
        self.last_scan = Some(now);
        self.has_signal = parsed
            .as_ref()
            .is_some_and(|p| p.detected || p.waiting_dynamic > 0);
        if self.has_signal {
            self.missing = 0;
            let progress = parsed.as_mut().unwrap();
            if progress.source == "vt-summary"
                && progress.total > 0
                && progress.done == progress.total
                && progress.waiting_dynamic == 0
            {
                progress.settled = true;
                progress.settled_by = "vt".into()
            }
            let sig = signature(progress);
            let same = sig == self.signature;
            let live = progress.elapsed_sec > self.elapsed_base;
            if !same
                || progress.settled
                || live
                || self.progress.as_ref().is_none_or(|old| !old.settled)
            {
                if !same || live {
                    self.elapsed_base = progress.elapsed_sec;
                    self.elapsed_at = Some(now)
                }
                self.signature = sig;
                self.progress = parsed;
            }
            if !same || live || !idle {
                self.frozen = 0
            } else if self.progress.as_ref().is_some_and(|p| !p.settled) {
                self.frozen += 1;
                if self.frozen >= 3 {
                    self.settle()
                }
            }
        } else if self.progress.as_ref().is_some_and(|p| !p.settled) {
            if idle {
                self.missing += 1
            } else {
                self.missing = 0
            }
            if self.missing >= 3 {
                self.settle()
            }
        }
    }
    fn settle(&mut self) {
        if let Some(progress) = self.progress.as_mut() {
            progress.running = 0;
            progress.pending = 0;
            progress.waiting_dynamic = 0;
            progress.settled = true;
            progress.settled_by = "vt".into()
        }
    }
    pub fn publish(&mut self, mut out: WorkflowProgress, now: Timestamp) -> Option<Publication> {
        if !out.settled
            && self.elapsed_base > 0
            && let Some(observed) = self.elapsed_at
        {
            out.elapsed_sec = self.elapsed_base
                + now
                    .duration_since(observed)
                    .map(|age| age.as_secs() as i64)
                    .unwrap_or(0)
        }
        let sig = signature(&out);
        let changed = sig != self.broadcast_signature;
        let heartbeat = !out.settled
            && self.last_broadcast.is_some_and(|at| {
                now.duration_since(at)
                    .is_ok_and(|age| age >= Duration::from_secs(7))
            });
        let publish = changed || self.last_broadcast.is_none() || heartbeat;
        if publish {
            self.broadcast_signature = sig.clone();
            self.last_broadcast = Some(now)
        }
        let completion = if out.settled {
            let result = !self.notified || self.completion_signature != sig;
            self.notified = true;
            self.completion_signature = sig;
            result
        } else {
            if self.notified {
                self.notified = false;
                self.completion_signature.clear()
            }
            false
        };
        if !out.settled {
            self.queue(now.checked_add(Duration::from_secs(7)).unwrap_or(now), None)
        }
        publish.then_some(Publication {
            progress: out,
            completion,
        })
    }
    pub fn finalize(&mut self, now: Timestamp) -> Option<Publication> {
        self.due = None;
        if self.progress.as_ref().is_some_and(|p| !p.settled) {
            self.settle();
            self.progress.as_mut().unwrap().settled_by = "timeout".into();
        }
        let mut publication = self.publish(self.progress.clone()?, now)?;
        publication.completion = false;
        Some(publication)
    }
}
pub fn compose(
    vt: Option<&WorkflowProgress>,
    journal: Option<&WorkflowProgress>,
    started: i64,
) -> Option<WorkflowProgress> {
    let mut output = vt.cloned().or_else(|| {
        journal.map(|_| WorkflowProgress {
            detected: true,
            ..Default::default()
        })
    })?;
    let Some(journal) = journal else {
        return Some(output);
    };
    output.detected = true;
    output.source = "journal".into();
    output.done = journal.done;
    if output.total > 0 && output.done > output.total {
        output.done = output.total
    }
    if output.total == 0 {
        output.total = journal.total.max(started)
    }
    if output.total > 0 {
        output.percent = (output.done * 100 / output.total).min(100)
    }
    output.settled = journal.settled;
    output.settled_by = journal.settled_by.clone();
    Some(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn at(seconds: u64) -> Timestamp {
        Timestamp::UNIX_EPOCH
            .checked_add(Duration::from_secs(seconds))
            .unwrap()
    }
    fn running() -> WorkflowProgress {
        WorkflowProgress {
            detected: true,
            source: "vt-summary".into(),
            name: "review".into(),
            done: 1,
            total: 2,
            running: 1,
            elapsed_sec: 5,
            ..Default::default()
        }
    }
    #[test]
    fn resize_and_last_scan_bound_the_final_debounced_scan() {
        let mut state = VtState {
            last_scan: Some(at(1)),
            ..Default::default()
        };
        state.queue(at(1), Some(at(3)));
        state.queue(at(2), None);
        assert_eq!(state.due, Some(at(2)));
        state.queue(at(1), None);
        assert_eq!(
            state.due,
            Some(at(1).checked_add(Duration::from_millis(500)).unwrap())
        );
    }
    #[test]
    fn frozen_idle_settle_is_sticky_until_live_elapsed_evidence() {
        let mut state = VtState::default();
        state.scan(Some(running()), false, at(1));
        for seconds in [8, 15, 22] {
            state.scan(Some(running()), true, at(seconds));
        }
        let settled = state.progress.clone().unwrap();
        assert!(settled.settled);
        assert_eq!(
            (settled.running, settled.pending, settled.waiting_dynamic),
            (0, 0, 0)
        );
        state.scan(Some(running()), true, at(29));
        assert!(state.progress.as_ref().unwrap().settled);
        let mut fresh = running();
        fresh.elapsed_sec = 6;
        state.scan(Some(fresh), true, at(36));
        assert!(!state.progress.as_ref().unwrap().settled);
    }
    #[test]
    fn heartbeat_does_not_repeat_completion_and_terminal_finalize_suppresses_it() {
        let mut state = VtState::default();
        state.scan(Some(running()), false, at(1));
        assert!(
            state
                .publish(state.progress.clone().unwrap(), at(1))
                .is_some()
        );
        assert!(
            state
                .publish(state.progress.clone().unwrap(), at(2))
                .is_none()
        );
        assert!(
            state
                .publish(state.progress.clone().unwrap(), at(8))
                .is_some()
        );
        let final_frame = state.finalize(at(9)).unwrap();
        assert!(final_frame.progress.settled);
        assert_eq!(final_frame.progress.settled_by, "timeout");
        assert!(!final_frame.completion);
        assert!(state.due.is_none());
        assert!(state.finalize(at(10)).is_none());
    }
    #[test]
    fn journal_controls_done_and_settle_without_discarding_vt_agent_phase() {
        let vt = running();
        let journal = WorkflowProgress {
            detected: true,
            total: 3,
            done: 3,
            settled: true,
            settled_by: "journal".into(),
            ..Default::default()
        };
        let composed = compose(Some(&vt), Some(&journal), 3).unwrap();
        assert_eq!(
            (composed.done, composed.total, composed.percent),
            (2, 2, 100)
        );
        assert_eq!(composed.running, 1);
        assert_eq!(composed.name, "review");
        assert_eq!(composed.source, "journal");
        assert_eq!(composed.settled_by, "journal");
    }
}
