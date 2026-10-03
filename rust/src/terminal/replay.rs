//! Raw replay and bounded UI priming. Replay offsets are byte counts, not UTF-8
//! positions: cutting a raw retention window must not alter cumulative offsets.
use crate::proto::core::ReplayEpoch;
use std::collections::VecDeque;
pub const REPLAY_LIMIT: usize = 2 * 1024 * 1024;
pub const INACTIVE_REPLAY_TAIL: usize = 64 * 1024;
pub const ALT_SCREEN_ENTER: &[u8] = b"\x1b[?1049h";

#[derive(Clone, Debug)]
pub struct ReplayBuffer {
    bytes: VecDeque<u8>,
    total: i64,
    limit: usize,
}
impl Default for ReplayBuffer {
    fn default() -> Self {
        Self::new(REPLAY_LIMIT)
    }
}
impl ReplayBuffer {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::new(),
            total: 0,
            limit,
        }
    }
    pub fn append(&mut self, chunk: &[u8]) {
        self.total = self
            .total
            .saturating_add(i64::try_from(chunk.len()).unwrap_or(i64::MAX));
        if chunk.len() >= self.limit {
            self.bytes.clear();
            self.bytes.extend(&chunk[chunk.len() - self.limit..]);
        } else {
            let excess = (self.bytes.len() + chunk.len()).saturating_sub(self.limit);
            self.bytes.drain(..excess);
            self.bytes.extend(chunk);
        }
    }
    /// Caller holds the session/replay lock over both values.
    pub fn snapshot(&self) -> (Vec<u8>, i64) {
        (self.bytes.iter().copied().collect(), self.total)
    }
    pub fn reset(&mut self) {
        self.bytes.clear();
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
    pub fn total(&self) -> i64 {
        self.total
    }
    /// Retains a last user-turn marker beyond the inactive pane window. Prefix
    /// alternate mode only AFTER choosing the window (ui_broadcast.go).
    pub fn ui_snapshot(&self, active: bool, alt_screen: bool, turn_marker: &[u8]) -> Vec<u8> {
        let (all, _) = self.snapshot();
        let mut start = 0;
        if !active && all.len() > INACTIVE_REPLAY_TAIL {
            start = all.len() - INACTIVE_REPLAY_TAIL;
            if !turn_marker.is_empty()
                && !all[start..]
                    .windows(turn_marker.len())
                    .any(|w| w == turn_marker)
                && let Some(last) = all
                    .windows(turn_marker.len())
                    .rposition(|w| w == turn_marker)
            {
                start = last;
            }
        }
        let mut result = Vec::with_capacity(
            all.len() - start
                + if alt_screen {
                    ALT_SCREEN_ENTER.len()
                } else {
                    0
                },
        );
        if alt_screen {
            result.extend_from_slice(ALT_SCREEN_ENTER);
        }
        result.extend_from_slice(&all[start..]);
        result
    }
}
pub fn next_replay_epoch(epoch: ReplayEpoch) -> ReplayEpoch {
    ReplayEpoch(epoch.0.wrapping_add(1).max(1))
}

/// Priming buffers complete frames; callers disconnect on overflow instead of
/// delivering a partial initial state. Authentication invalidation drops all.
pub struct PrimingQueue<T> {
    frames: VecDeque<T>,
    capacity: usize,
    priming: bool,
    invalidated: bool,
}
impl<T> PrimingQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            capacity,
            priming: true,
            invalidated: false,
        }
    }
    pub fn enqueue(&mut self, frame: T) -> Result<(), T> {
        if self.invalidated || !self.priming || self.frames.len() >= self.capacity {
            return Err(frame);
        }
        self.frames.push_back(frame);
        Ok(())
    }
    pub fn finish(&mut self) -> Vec<T> {
        self.priming = false;
        self.frames.drain(..).collect()
    }
    pub fn invalidate(&mut self) {
        self.invalidated = true;
        self.priming = false;
        self.frames.clear();
    }
    pub fn is_priming(&self) -> bool {
        self.priming
    }
}
