//! Go pumpChunk and Windows-1252 mojibake repair. Incomplete UTF-8 bytes survive
//! reads and are flushed unchanged at EOF; wire offsets count emitted bytes.
#[derive(Default)]
pub struct OutputNormalizer {
    carry: Vec<u8>,
    windows: bool,
}
impl OutputNormalizer {
    pub fn new(windows: bool) -> Self {
        Self {
            carry: vec![],
            windows,
        }
    }
    pub fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.carry.extend_from_slice(bytes);
        let mut split = self.carry.len();
        for index in (self.carry.len().saturating_sub(3)..self.carry.len()).rev() {
            let byte = self.carry[index];
            if byte < 0x80 {
                break;
            }
            if byte >= 0xc0 {
                if std::str::from_utf8(&self.carry[index..]).is_err_and(|e| e.valid_up_to() == 0) {
                    split = index;
                }
                break;
            }
        }
        let tail = self.carry.split_off(split);
        let out = std::mem::replace(&mut self.carry, tail);
        if self.windows {
            repair_windows_mojibake(out)
        } else {
            out
        }
    }
    pub fn finish(&mut self) -> Vec<u8> {
        let bytes = std::mem::take(&mut self.carry);
        if self.windows {
            repair_windows_mojibake(bytes)
        } else {
            bytes
        }
    }
}
fn score(text: &str) -> usize {
    text.chars()
        .map(|c| match c {
            'Ã' | 'ã' | 'â' | 'Â' => 2,
            'å' | 'æ' | 'ç' => 1,
            _ => 0,
        })
        .sum()
}
fn signals(text: &str) -> bool {
    text.chars().any(|c|matches!(c as u32,0x3040..=0x30ff|0x3400..=0x9fff|0x2500..=0x257f|0x2190..=0x21ff|0x2600..=0x27bf))
}
fn encode(char: char) -> Option<u8> {
    if char as u32 <= 255 {
        return Some(char as u8);
    }
    const MAPPING: &[(char, u8)] = &[
        ('€', 0x80),
        ('‚', 0x82),
        ('ƒ', 0x83),
        ('„', 0x84),
        ('…', 0x85),
        ('†', 0x86),
        ('‡', 0x87),
        ('ˆ', 0x88),
        ('‰', 0x89),
        ('Š', 0x8a),
        ('‹', 0x8b),
        ('Œ', 0x8c),
        ('Ž', 0x8e),
        ('‘', 0x91),
        ('’', 0x92),
        ('“', 0x93),
        ('”', 0x94),
        ('•', 0x95),
        ('–', 0x96),
        ('—', 0x97),
        ('˜', 0x98),
        ('™', 0x99),
        ('š', 0x9a),
        ('›', 0x9b),
        ('œ', 0x9c),
        ('ž', 0x9e),
        ('Ÿ', 0x9f),
    ];
    MAPPING.iter().find_map(|(c, b)| (*c == char).then_some(*b))
}
pub fn repair_windows_mojibake(bytes: Vec<u8>) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return bytes;
    };
    let before = score(text);
    if before == 0 {
        return bytes;
    }
    let Some(raw) = text.chars().map(encode).collect::<Option<Vec<_>>>() else {
        return bytes;
    };
    let Ok(repaired) = std::str::from_utf8(&raw) else {
        return bytes;
    };
    if score(repaired) >= before || !signals(repaired) {
        bytes
    } else {
        raw
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_unicode_and_incomplete_eof_are_lossless() {
        let bytes = "日本語\u{1f642}".as_bytes();
        for chunk in 1..=bytes.len() {
            let mut decoder = OutputNormalizer::new(false);
            let mut all = vec![];
            for part in bytes.chunks(chunk) {
                all.extend(decoder.push(part));
            }
            all.extend(decoder.finish());
            assert_eq!(all, bytes);
        }
        let mut decoder = OutputNormalizer::new(false);
        assert_eq!(decoder.push(&[b'a', 0xe3, 0x81]), b"a");
        assert_eq!(decoder.finish(), [0xe3, 0x81]);
    }
    #[test]
    fn windows_repairs_only_confident_mojibake() {
        assert_eq!(
            repair_windows_mojibake("ãƒ\u{AD}ãƒ¼ã‚«ãƒ«".as_bytes().to_vec()),
            "ローカル".as_bytes()
        );
        assert_eq!(
            repair_windows_mojibake("â”€".as_bytes().to_vec()),
            "─".as_bytes()
        );
        for text in ["日本語", "café", "Ã"] {
            assert_eq!(
                repair_windows_mojibake(text.as_bytes().to_vec()),
                text.as_bytes()
            );
        }
    }
}

/// Vendor success phrases only; never extract or persist the account identity.
pub fn login_finished(bytes: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(bytes).to_lowercase();
    [
        "signed in as",
        "successfully logged in",
        "login successful",
        "logged in using",
        "logged in as",
        "you are logged in",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}
#[derive(Default)]
pub struct LoginCompletion {
    bytes: Vec<u8>,
    fired: bool,
}
impl LoginCompletion {
    pub fn push(&mut self, bytes: &[u8]) -> bool {
        if self.fired {
            return false;
        }
        self.bytes.extend_from_slice(bytes);
        if self.bytes.len() > 8192 {
            self.bytes.drain(..self.bytes.len() - 8192);
        }
        if login_finished(&self.bytes) {
            self.fired = true;
            self.bytes.clear();
            true
        } else {
            false
        }
    }
}
#[cfg(test)]
mod login_tests {
    use super::*;
    #[test]
    fn login_completion_handles_split_chunks_once_without_false_negative_phrase() {
        for phrase in [
            "You are not logged in.",
            "not authenticated",
            "not signed in",
        ] {
            assert!(!login_finished(phrase.as_bytes()));
        }
        let mut scan = LoginCompletion::default();
        assert!(!scan.push(b"Successfully log"));
        assert!(scan.push(b"ged in."));
        assert!(!scan.push(b"Login successful"));
    }
}
