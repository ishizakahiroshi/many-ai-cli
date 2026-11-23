//! Bounded Hub input state and wrapper processed watermark, from input_gate.go.
//! These state components live inside the single session owner; they are not a
//! second session manager. Sending/settling is the owner's asynchronous lane.
use crate::proto::core::{
    AckDisposition, INITIAL_PROMPT_GATE_TIMEOUT, INPUT_QUEUE_LIMIT, InputFrame,
    InputHighWatermarks, InputSeq, ProcessedInput, WrapperConnectionId,
};
use crate::proto::time::Timestamp;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Mutex,
};

#[derive(Clone, Debug)]
struct Inflight {
    connection: WrapperConnectionId,
    bytes: Vec<u8>,
    /// Written as `GuardedInternal`: its resend after a reconnect has to pass
    /// the same approval check again (audit N03), never go out as plain input.
    guarded: bool,
}
#[derive(Default)]
pub struct InputState {
    sequence: i64,
    pending: VecDeque<Vec<u8>>,
    inflight: BTreeMap<InputSeq, Inflight>,
    resend: Vec<InputFrame>,
    /// The frames in `resend` that came from guarded inflight entries.
    resend_guarded: BTreeSet<InputSeq>,
    ack_capable: bool,
    initial_gate: Option<Timestamp>,
}
impl InputState {
    /// Cold reattach must start above the wrapper's processed/received range;
    /// warm reattach keeps any higher locally allocated sequence.
    pub fn observe_high_watermark(&mut self, sequence: InputSeq) {
        self.sequence = self.sequence.max(sequence.0);
    }
    pub fn set_initial_gate(&mut self, now: Timestamp) {
        self.initial_gate = Some(now);
    }
    pub fn clear_initial_gate(&mut self) {
        self.initial_gate = None;
    }
    pub fn initial_prompt_phase(&self) -> bool {
        self.initial_gate.is_some()
    }
    pub fn gated(&self, now: Timestamp) -> bool {
        self.initial_gate.is_some_and(|at| {
            now.duration_since(at).unwrap_or_default() < INITIAL_PROMPT_GATE_TIMEOUT
        })
    }
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
    pub fn inflight_len(&self) -> usize {
        self.inflight.len()
    }
    pub fn resend_len(&self) -> usize {
        self.resend.len()
    }
    pub fn ack_capable(&self) -> bool {
        self.ack_capable
    }
    pub fn enqueue(&mut self, bytes: Vec<u8>) -> bool {
        self.pending.push_back(bytes);
        let dropped = self.pending.len() > INPUT_QUEUE_LIMIT;
        if dropped {
            self.pending.pop_front();
        }
        dropped
    }
    pub fn requeue_front(&mut self, frames: impl IntoIterator<Item = Vec<u8>>) {
        let mut all: VecDeque<_> = frames.into_iter().collect();
        all.append(&mut self.pending);
        while all.len() > INPUT_QUEUE_LIMIT {
            all.pop_front();
        }
        self.pending = all;
    }
    pub fn take_pending(&mut self) -> Vec<Vec<u8>> {
        self.pending.drain(..).collect()
    }
    pub fn reserve(&mut self, connection: WrapperConnectionId, bytes: Vec<u8>) -> InputFrame {
        if bytes.is_empty() {
            return InputFrame {
                seq: InputSeq(0),
                bytes,
            };
        }
        loop {
            self.sequence = self.sequence.checked_add(1).filter(|n| *n > 0).unwrap_or(1);
            if !self.inflight.contains_key(&InputSeq(self.sequence)) {
                break;
            }
        }
        let frame = InputFrame {
            seq: InputSeq(self.sequence),
            bytes,
        };
        self.readmit(connection, &frame);
        frame
    }
    /// `reserve` for a `GuardedInternal` frame. Should the wrapper drop before
    /// it ACKs, the frame is resent only through the guarded check.
    pub fn reserve_guarded(
        &mut self,
        connection: WrapperConnectionId,
        bytes: Vec<u8>,
    ) -> InputFrame {
        let frame = self.reserve(connection, bytes);
        if let Some(inflight) = self.inflight.get_mut(&frame.seq) {
            inflight.guarded = true;
        }
        frame
    }
    pub fn readmit(&mut self, connection: WrapperConnectionId, frame: &InputFrame) -> bool {
        self.admit(connection, frame, false)
    }
    /// `readmit` for a guarded frame being resent.
    pub fn readmit_guarded(&mut self, connection: WrapperConnectionId, frame: &InputFrame) -> bool {
        self.admit(connection, frame, true)
    }
    fn admit(
        &mut self,
        connection: WrapperConnectionId,
        frame: &InputFrame,
        guarded: bool,
    ) -> bool {
        if frame.seq.0 <= 0 {
            return false;
        }
        self.inflight.insert(
            frame.seq,
            Inflight {
                connection,
                bytes: frame.bytes.clone(),
                guarded,
            },
        );
        while self.inflight.len() > INPUT_QUEUE_LIMIT {
            self.inflight.pop_first();
        }
        true
    }
    pub fn release(&mut self, connection: WrapperConnectionId, seq: InputSeq) -> bool {
        if self
            .inflight
            .get(&seq)
            .is_some_and(|f| f.connection == connection)
        {
            self.inflight.remove(&seq);
            true
        } else {
            false
        }
    }
    pub fn acknowledge(
        &mut self,
        connection: WrapperConnectionId,
        seq: InputSeq,
    ) -> AckDisposition {
        self.ack_capable = true;
        if self
            .inflight
            .get(&seq)
            .is_some_and(|f| f.connection != connection)
        {
            return AckDisposition::WrongConnection;
        }
        if self.release(connection, seq) {
            AckDisposition::Removed
        } else {
            AckDisposition::CapabilityObserved
        }
    }
    pub fn disconnected(
        &mut self,
        connection: WrapperConnectionId,
        connection_ack_seen: bool,
    ) -> usize {
        let ids: Vec<_> = self
            .inflight
            .iter()
            .filter(|(_, f)| f.connection == connection)
            .map(|(seq, _)| *seq)
            .collect();
        let mut frames = Vec::new();
        for seq in ids {
            let data = self.inflight.remove(&seq).unwrap();
            if self.ack_capable || connection_ack_seen {
                frames.push((
                    InputFrame {
                        seq,
                        bytes: data.bytes,
                    },
                    data.guarded,
                ));
            }
        }
        let n = frames.len();
        self.requeue_resend_marked(frames);
        n
    }
    pub fn requeue_resend(&mut self, frames: impl IntoIterator<Item = InputFrame>) {
        self.requeue_resend_marked(frames.into_iter().map(|frame| (frame, false)));
    }
    /// Requeues frames for resend with whether each was written guarded.
    pub fn requeue_resend_marked(&mut self, frames: impl IntoIterator<Item = (InputFrame, bool)>) {
        for (frame, guarded) in frames {
            if guarded {
                self.resend_guarded.insert(frame.seq);
            } else {
                self.resend_guarded.remove(&frame.seq);
            }
            self.resend.push(frame);
        }
        self.resend.sort_by_key(|f| f.seq);
        if self.resend.len() > INPUT_QUEUE_LIMIT {
            for dropped in self.resend.drain(..self.resend.len() - INPUT_QUEUE_LIMIT) {
                self.resend_guarded.remove(&dropped.seq);
            }
        }
    }
    pub fn take_resend(&mut self) -> Vec<InputFrame> {
        self.take_resend_marked()
            .into_iter()
            .map(|(frame, _)| frame)
            .collect()
    }
    /// Takes the resend queue with whether each frame was written guarded.
    pub fn take_resend_marked(&mut self) -> Vec<(InputFrame, bool)> {
        let guarded = std::mem::take(&mut self.resend_guarded);
        std::mem::take(&mut self.resend)
            .into_iter()
            .map(|frame| {
                let marked = guarded.contains(&frame.seq);
                (frame, marked)
            })
            .collect()
    }
    pub fn clear(&mut self) {
        self.pending.clear();
        self.inflight.clear();
        self.resend.clear();
        self.resend_guarded.clear();
        self.initial_gate = None;
    }
}

#[derive(Default)]
pub struct ProcessedInputState(Mutex<(i64, i64)>);
impl ProcessedInput for ProcessedInputState {
    fn watermarks(&self) -> InputHighWatermarks {
        let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        InputHighWatermarks {
            processed: InputSeq(state.0),
            received: InputSeq(state.1),
        }
    }
    fn received(&self, sequence: InputSeq) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.1 = state.1.max(sequence.0);
    }
    fn mark_processed(&self, sequence: InputSeq) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.0 = state.0.max(sequence.0);
    }
    fn already_processed(&self, sequence: InputSeq) -> bool {
        sequence.0 > 0 && sequence.0 <= self.0.lock().unwrap_or_else(|p| p.into_inner()).0
    }
}
pub fn split_bracketed_paste_submit(bytes: &[u8]) -> (&[u8], &[u8]) {
    if bytes.ends_with(b"\x1b[201~\r") {
        bytes.split_at(bytes.len() - 1)
    } else {
        (bytes, &[])
    }
}
