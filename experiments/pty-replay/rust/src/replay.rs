use std::sync::Mutex;

pub const LIMIT: usize = 2 * 1024 * 1024;

#[derive(Default)]
struct State {
    bytes: Vec<u8>,
    start: usize,
    total: i64,
}

// A safe contiguous allocation plus a consumed prefix. This is not a ring.
// Vec's growth policy and compaction are not identical to Go bytes.Buffer;
// the comparison is of these two implementations, not language alone.
#[derive(Default)]
pub struct Replay {
    state: Mutex<State>,
}

impl Replay {
    pub fn append(&self, mut chunk: &[u8]) {
        let mut state = self.state.lock().expect("replay mutex poisoned");
        // The harness rejects total overflow before measured work starts.
        state.total += chunk.len() as i64;
        if chunk.len() >= LIMIT {
            state.bytes.clear();
            state.start = 0;
            chunk = &chunk[chunk.len() - LIMIT..];
        }
        if state.bytes.capacity() - state.bytes.len() < chunk.len() && state.start > 0 {
            let start = state.start;
            let retained = state.bytes.len() - start;
            state.bytes.copy_within(start.., 0);
            state.bytes.truncate(retained);
            state.start = 0;
        }
        state.bytes.extend_from_slice(chunk);
        let retained = state.bytes.len() - state.start;
        if retained > LIMIT {
            state.start += retained - LIMIT;
        }
    }

    pub fn snapshot(&self) -> (Vec<u8>, i64) {
        let state = self.state.lock().expect("replay mutex poisoned");
        (state.bytes[state.start..].to_vec(), state.total)
    }
}
