//! Go1.26.8 `strconv.Quote` for valid UTF-8 strings, using pinned Unicode 15.0.0.
//!
//! This is Go string-literal/error formatting, not JSON encoding or shell quoting.
//! Invalid UTF-8 Go strings are outside the `&str` API's contract.

#[path = "go_quote_data.rs"]
mod data;

/// Return a double-quoted Go string literal, including Go's short control escapes
/// and lowercase `\x`, `\u`, or `\U` hexadecimal escapes for non-printable scalars.
pub fn quote(value: &str) -> String {
    use std::fmt::Write;

    let mut out = String::with_capacity(value.len().saturating_add(2));
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            c if c < ' ' || c == '\u{7f}' => {
                write!(out, "\\x{:02x}", u32::from(c)).expect("writing to a String cannot fail");
            }
            c if is_print(c) => out.push(c),
            c if u32::from(c) < 0x10000 => {
                write!(out, "\\u{:04x}", u32::from(c)).expect("writing to a String cannot fail");
            }
            c => {
                write!(out, "\\U{:08x}", u32::from(c)).expect("writing to a String cannot fail");
            }
        }
    }
    out.push('"');
    out
}

fn is_print(value: char) -> bool {
    if value.is_ascii() {
        return (' '..='~').contains(&value);
    }
    let value = u32::from(value);
    data::PRINTABLE
        .binary_search_by(|&(lo, hi)| {
            if value < lo {
                std::cmp::Ordering::Greater
            } else if value > hi {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use sha2::{Digest, Sha256};

    #[derive(Deserialize)]
    struct Corpus {
        baseline: String,
        go_version: String,
        unicode_version: String,
        valid_scalar_count: usize,
        printable_scalar_count: usize,
        all_scalar_quote_sha256: String,
        cases: Vec<QuoteCase>,
    }

    #[derive(Deserialize)]
    struct QuoteCase {
        name: String,
        input: String,
        quote: String,
    }

    fn corpus() -> Corpus {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/foundation/go-quote/oracle.json"
        ))
        .expect("pinned Go Quote corpus")
    }

    #[test]
    fn targeted_quotes_match_pinned_go_oracle() {
        let corpus = corpus();
        assert_eq!(corpus.baseline, "21d0bc7935a2c4696fb89ccff2e324157a528c2d");
        assert_eq!(corpus.go_version, "go1.26.8");
        assert_eq!(corpus.unicode_version, "15.0.0");
        for case in corpus.cases {
            assert_eq!(quote(&case.input), case.quote, "{}", case.name);
        }
    }

    #[test]
    fn printable_ranges_are_canonical_valid_non_ascii_scalars() {
        let mut previous_end = 0x7f;
        for &(lo, hi) in data::PRINTABLE {
            assert!(lo > previous_end + 1, "ranges overlap or are not merged");
            assert!(lo <= hi);
            assert!(char::from_u32(lo).is_some());
            assert!(char::from_u32(hi).is_some());
            assert!(hi < 0xd800 || lo > 0xdfff, "range includes a surrogate");
            previous_end = hi;
        }
    }

    #[test]
    fn every_valid_scalar_quote_matches_pinned_go_digest() {
        let corpus = corpus();
        let mut digest = Sha256::new();
        let mut scalar_count = 0;
        let mut printable_count = 0;
        for value in 0..=0x10ffff {
            let Some(value) = char::from_u32(value) else {
                continue;
            };
            scalar_count += 1;
            printable_count += usize::from(is_print(value));
            digest.update(quote(value.encode_utf8(&mut [0; 4])).as_bytes());
        }
        assert_eq!(scalar_count, corpus.valid_scalar_count);
        assert_eq!(printable_count, corpus.printable_scalar_count);
        let actual_hex: String = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(actual_hex, corpus.all_scalar_quote_sha256);
    }
}
