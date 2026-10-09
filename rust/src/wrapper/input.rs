//! Input sequencing survives socket replacement. Only a completed PTY write
//! advances processed; received reserves the high watermark during reconnect.
use crate::process::pty::PtySession;
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputWatermarks {
    pub received: i64,
    pub processed: i64,
}
impl InputWatermarks {
    pub fn received(&mut self, seq: i64) {
        self.received = self.received.max(seq);
    }
    pub fn processed(&mut self, seq: i64) {
        self.processed = self.processed.max(seq);
    }
    pub fn duplicate(&self, seq: i64) -> bool {
        seq > 0 && seq <= self.processed
    }
    pub fn high_watermark(&self) -> i64 {
        self.received.max(self.processed)
    }
}
pub type SharedWatermarks = Arc<Mutex<InputWatermarks>>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputStep {
    Bytes(Vec<u8>),
    Pause(Duration),
}
fn chunks(out: &mut Vec<InputStep>, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let mut end = bytes.len().min(1024);
        if end < bytes.len() {
            while end > 0 && bytes[end] & 0xc0 == 0x80 {
                end -= 1;
            }
            if end == 0 {
                end = 1024;
            }
        }
        out.push(InputStep::Bytes(bytes[..end].to_vec()));
        bytes = &bytes[end..];
        if !bytes.is_empty() {
            out.push(InputStep::Pause(Duration::from_millis(3)));
        }
    }
}
fn trailing(out: &mut Vec<InputStep>, bytes: &[u8], delay: u64) {
    if bytes.last() == Some(&b'\r') {
        chunks(out, &bytes[..bytes.len() - 1]);
        out.push(InputStep::Pause(Duration::from_millis(delay)));
        out.push(InputStep::Bytes(vec![b'\r']));
    } else {
        chunks(out, bytes);
    }
}
pub fn looks_like_inject_path(bytes: &[u8]) -> bool {
    bytes.first() == Some(&b'/')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}
pub fn input_steps(provider: &str, mut bytes: &[u8]) -> Vec<InputStep> {
    let mut out = Vec::new();
    if provider == "cursor-agent" && bytes.len() > 1 && bytes[0] == 0x15 {
        out.push(InputStep::Bytes(vec![0x15]));
        out.push(InputStep::Pause(Duration::from_millis(20)));
        bytes = &bytes[1..];
    }
    if provider == "claude"
        && bytes.len() > 1
        && bytes[0] == b'@'
        && looks_like_inject_path(&bytes[1..])
    {
        if let Some(index) = bytes
            .iter()
            .position(|b| *b == b'\r')
            .filter(|i| *i < bytes.len() - 1)
        {
            trailing(&mut out, &bytes[..=index], 150);
            trailing(&mut out, &bytes[index + 1..], 20);
        } else {
            trailing(&mut out, bytes, 20);
        }
    } else if bytes.last() == Some(&b'\r') && (bytes.len() > 1 || provider == "opencode") {
        trailing(
            &mut out,
            bytes,
            if matches!(provider, "codex" | "opencode") {
                180
            } else {
                20
            },
        );
    } else {
        chunks(&mut out, bytes);
    }
    out
}
pub async fn write_all(pty: &dyn PtySession, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let n = pty.write(bytes).await?;
        if n == 0 || n > bytes.len() {
            return Err(io::Error::new(io::ErrorKind::WriteZero, "short PTY write"));
        }
        bytes = &bytes[n..];
    }
    Ok(())
}
pub async fn write_input(pty: &dyn PtySession, provider: &str, bytes: &[u8]) -> io::Result<()> {
    for step in input_steps(provider, bytes) {
        match step {
            InputStep::Bytes(bytes) => {
                write_all(pty, &bytes).await?;
                crate::logging::input_probe::wrapper_write(provider, &bytes);
            }
            InputStep::Pause(delay) => tokio::time::sleep(delay).await,
        }
    }
    Ok(())
}
