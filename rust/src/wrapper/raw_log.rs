//! Raw session log writer. Output is masked before it reaches disk; the pattern
//! masker only sees a secret that is whole in its input, so bytes are held back
//! until a line ends (or the hold-back cap is reached) instead of masking each
//! 4 KiB read independently.
use std::{fs::File, io::Write};

/// A line longer than this is masked and written anyway, so a TUI that never
/// prints a newline cannot grow the buffer without bound.
const HOLD_CAP: usize = 64 * 1024;

pub(super) struct RawLog {
    file: File,
    limit: i64,
    written: i64,
    pending: Vec<u8>,
}

impl RawLog {
    pub(super) fn new(file: File, limit: i64) -> Self {
        Self {
            file,
            limit,
            written: 0,
            pending: Vec::new(),
        }
    }
    /// A raw-log write failure never blocks provider progress.
    pub(super) fn write(&mut self, bytes: &[u8]) {
        if self.full() {
            return;
        }
        self.pending.extend_from_slice(bytes);
        self.drain(false);
    }
    pub(super) fn finish(&mut self) {
        self.drain(true);
    }
    fn full(&self) -> bool {
        self.limit > 0 && self.written >= self.limit
    }
    fn drain(&mut self, all: bool) {
        let len = self.pending.len();
        let line = match self.pending.iter().rposition(|b| *b == b'\n') {
            Some(i) if !open_private_key(&self.pending[..=i]) => i + 1,
            _ => 0,
        };
        let cut = if all {
            len
        } else if line == 0 && len >= HOLD_CAP {
            word_cut(&self.pending)
        } else {
            line
        };
        if cut == 0 {
            return;
        }
        let rest = self.pending.split_off(cut);
        let head = std::mem::replace(&mut self.pending, rest);
        let masked = crate::storage::mask_secret_bytes(&head);
        if !self.full() {
            let _ = self.file.write_all(&masked);
            self.written = self.written.saturating_add(masked.len() as i64);
        }
    }
}

/// Where to split a line that reached the hold-back cap. A token never contains
/// the masker's delimiters, but a `KEY: value` or `Bearer value` pair does, and
/// the value may still be growing, so the last two words are always held back.
/// A cut is used only if it leaves the masked result unchanged; a single run
/// without a usable cut is written whole, as before.
fn word_cut(bytes: &[u8]) -> usize {
    let delimiter = |b: u8| matches!(b, b'\t' | b'\n' | b'\x0c' | b'\r' | b' ');
    let full = crate::storage::mask_secret_bytes(bytes);
    (1..bytes.len())
        .rev()
        .filter(|&i| delimiter(bytes[i - 1]) && !delimiter(bytes[i]))
        .skip(1)
        .take(16)
        .find(|&i| {
            let (head, tail) = bytes.split_at(i);
            if open_private_key(head) {
                return false;
            }
            let mut split = crate::storage::mask_secret_bytes(head);
            split.extend(crate::storage::mask_secret_bytes(tail));
            split == full
        })
        .unwrap_or(bytes.len())
}

/// A PEM block spans lines, so it is only masked once its END line has arrived.
fn open_private_key(bytes: &[u8]) -> bool {
    let count = |needle: &[u8]| bytes.windows(needle.len()).filter(|w| *w == needle).count();
    count(b"-----BEGIN ") > count(b"-----END ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn run(chunks: &[&[u8]]) -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("raw.log");
        let mut log = RawLog::new(File::create(&path).unwrap(), 0);
        for chunk in chunks {
            log.write(chunk);
        }
        log.finish();
        let mut out = Vec::new();
        File::open(&path).unwrap().read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn a_secret_split_across_reads_is_still_masked() {
        let token = format!("ghp_{}", "A".repeat(30));
        let (a, b) = token.split_at(10);
        let out = run(&[b"token: ", a.as_bytes(), b.as_bytes(), b"\r\nnext\r\n"]);
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("AAAA"), "{text}");
        assert!(text.contains("token: ***"), "{text}");
    }

    #[test]
    fn a_multi_line_private_key_is_masked_as_one_block() {
        let key = format!(
            "-----{} RSA PRIVATE {}-----\nsynthetic-body\n-----{} RSA PRIVATE {}-----\n",
            "BEGIN", "KEY", "END", "KEY"
        );
        let (a, b) = key.split_at(key.find("synthetic").unwrap() + 4);
        let text = String::from_utf8(run(&[a.as_bytes(), b.as_bytes()])).unwrap();
        assert!(!text.contains("synthetic-body"), "{text}");
    }

    /// Feeds `text` in 4 KiB reads, like the PTY reader, with no newline.
    fn run_without_newlines(text: &str) -> String {
        let chunks: Vec<&[u8]> = text.as_bytes().chunks(4096).collect();
        String::from_utf8(run(&chunks)).unwrap()
    }

    #[test]
    fn a_secret_across_the_hold_back_cap_is_still_masked() {
        let token = format!("ghp_{}", "A".repeat(30));
        for offset in [1, 10, 20, 33] {
            let pad = "x ".repeat((HOLD_CAP - offset) / 2);
            let text = run_without_newlines(&format!("{pad}{token} tail"));
            assert!(!text.contains("AAAA"), "offset {offset}");
            assert!(text.ends_with("*** tail"), "offset {offset}");
        }
    }

    #[test]
    fn a_key_and_value_across_the_hold_back_cap_are_masked_together() {
        for (key, value) in [
            ("Bearer ", "synthetic-value"),
            ("PASSWORD: ", "synthetic-value"),
        ] {
            for offset in [1, 4, 8, 12, 20] {
                let pad = "x ".repeat((HOLD_CAP - offset) / 2);
                let text = run_without_newlines(&format!("{pad}{key}{value} tail"));
                assert!(!text.contains("synthetic"), "{key} offset {offset}");
                assert!(
                    text.ends_with(&format!("{key}*** tail")),
                    "{key} offset {offset}"
                );
            }
        }
    }

    #[test]
    fn a_run_without_delimiters_is_still_written_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("raw.log");
        let mut log = RawLog::new(File::create(&path).unwrap(), 0);
        for chunk in "x".repeat(HOLD_CAP + 10).as_bytes().chunks(4096) {
            log.write(chunk);
        }
        assert!(log.pending.len() < 4096);
        assert_eq!(std::fs::read(&path).unwrap().len(), HOLD_CAP);
    }

    #[test]
    fn output_without_a_newline_is_written_when_the_log_closes() {
        assert_eq!(run(&[b"prompt> "]), b"prompt> ");
    }

    #[test]
    fn the_size_limit_stops_further_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("raw.log");
        let mut log = RawLog::new(File::create(&path).unwrap(), 4);
        log.write(b"abcdef\n");
        log.write(b"more\n");
        log.finish();
        assert_eq!(std::fs::read(&path).unwrap(), b"abcdef\n");
    }
}
