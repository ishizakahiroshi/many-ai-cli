//! stdio framing: newline-delimited JSON messages.

use serde_json::Value;

/// A single message larger than this is treated as a broken stream.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// Serialises one message followed by `\n`. `serde_json` escapes control
/// characters, so the output never contains an embedded newline.
pub fn encode_message(message: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(message).unwrap_or_else(|_| b"null".to_vec());
    bytes.push(b'\n');
    bytes
}

/// Reassembles lines from arbitrary byte chunks (a message can arrive split
/// over several reads, and several messages can arrive in one read).
#[derive(Default)]
pub struct LineDecoder {
    buffer: Vec<u8>,
}

impl LineDecoder {
    /// Returns the complete lines (without the terminator, empty lines
    /// dropped). `Err` means the pending line is over the size limit.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        self.buffer.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=end).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.len() > MAX_LINE_BYTES {
                return Err("message line over the size limit".into());
            }
            if !line.is_empty() {
                lines.push(line);
            }
        }
        if self.buffer.len() > MAX_LINE_BYTES {
            return Err("message line over the size limit".into());
        }
        Ok(lines)
    }
}
