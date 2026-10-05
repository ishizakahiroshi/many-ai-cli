// Portions derived from Go regexp/syntax: Copyright 2009, 2013 The Go Authors.
// Use is governed by the BSD-style license in go_regex/GO-LICENSE.

//! Go 1.26.8 regexp compatibility for authorization on valid UTF-8 strings.
//!
//! Every Unicode property and case-insensitive atom is expanded into explicit
//! rune intervals derived from the pinned Go source. The backend never applies
//! its own Unicode properties or case-fold tables. The only public operation is
//! compilation for boolean matching; capture names and match preference do not
//! affect this contract. Go-derived data and parser semantics are attributed in
//! `go_regex/GO-LICENSE`; reproduction/source hashes are in the fixture directory.
use regex::{Regex, RegexBuilder};
use std::fmt::Write;

#[path = "go_regex/tables.rs"]
mod tables;
#[cfg(test)]
#[path = "go_regex/tests.rs"]
mod tests;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RegexIssue {
    Invalid,
    Unsupported(&'static str),
}

/// Compile the pinned Go dialect without widening its matching language.
///
/// Surrogate runes are legal in Go patterns but cannot occur in a Rust `str`.
/// Sets include them first and clip at emission, except source-proven straight
/// anchored literals that reproduce Go's RuneError-encoded one-pass prefix.
/// Broader singleton-surrogate optimizer candidates and resource limits remain
/// explicit fail-closed compatibility limitations.
pub(super) fn compile(pattern: &str) -> Result<Regex, RegexIssue> {
    let output = translate(pattern)?;
    RegexBuilder::new(&output)
        .nest_limit(1000)
        .build()
        .map_err(|_| RegexIssue::Unsupported("translated regex exceeds backend limits"))
}

const SURROGATE_GAP: &str =
    "singleton surrogate in a Go one-pass candidate requires optimizer equivalence";

const MAX_RUNE: u32 = 0x10ffff;
const MAX_TRANSLATED: usize = 16 * 1024 * 1024;
type Ranges = Vec<(u32, u32)>;

fn normalize(ranges: &mut Ranges) {
    ranges.sort_unstable();
    let mut n = 0;
    for i in 0..ranges.len() {
        let (lo, hi) = ranges[i];
        if n > 0 && lo <= ranges[n - 1].1 + 1 {
            ranges[n - 1].1 = ranges[n - 1].1.max(hi);
        } else {
            ranges[n] = (lo, hi);
            n += 1;
        }
    }
    ranges.truncate(n);
}
fn contains(ranges: &[(u32, u32)], rune: u32) -> bool {
    ranges
        .binary_search_by(|&(lo, hi)| {
            if rune < lo {
                std::cmp::Ordering::Greater
            } else if rune > hi {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}
fn complement(ranges: &[(u32, u32)]) -> Ranges {
    let mut out = Vec::new();
    let mut lo = 0;
    for &(start, end) in ranges {
        if lo < start {
            out.push((lo, start - 1));
        }
        lo = end + 1;
    }
    if lo <= MAX_RUNE {
        out.push((lo, MAX_RUNE));
    }
    out
}
fn simple_fold(rune: u32) -> u32 {
    match tables::SIMPLE_FOLD.binary_search_by_key(&rune, |&(from, _)| from) {
        Ok(index) => tables::SIMPLE_FOLD[index].1,
        Err(_) => rune,
    }
}
fn fold(ranges: &mut Ranges) {
    normalize(ranges);
    let mut extra = Vec::new();
    for &(rune, _) in tables::SIMPLE_FOLD {
        if contains(ranges, rune) {
            let mut next = simple_fold(rune);
            while next != rune {
                extra.push((next, next));
                next = simple_fold(next);
            }
        }
    }
    ranges.extend(extra);
    normalize(ranges);
}
fn emit_ranges(ranges: &[(u32, u32)]) -> String {
    let mut out = String::from("[");
    for &(lo, hi) in ranges {
        // Go runes include surrogates; Rust strings only contain scalar values.
        for (start, end) in [(lo, hi.min(0xd7ff)), (lo.max(0xe000), hi)] {
            if start <= end {
                write!(out, "\\x{{{start:x}}}").unwrap();
                if start != end {
                    write!(out, "-\\x{{{end:x}}}").unwrap();
                }
            }
        }
    }
    if out.len() == 1 {
        // Explicit empty language. `(?:)` would incorrectly match empty input.
        return r"[\x{0}&&\x{1}]".into();
    }
    out.push(']');
    out
}
fn canonical(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}
fn property(name: &str, casefold: bool) -> Result<Ranges, RegexIssue> {
    let key = canonical(name);
    let key = match tables::ALIASES.binary_search_by_key(&key.as_str(), |&(key, _)| key) {
        Ok(i) => tables::ALIASES[i].1,
        Err(_) => &key,
    };
    let index = tables::PROPERTIES
        .binary_search_by_key(&key, |&(key, _, _)| key)
        .map_err(|_| RegexIssue::Invalid)?;
    let (_, normal, folded) = tables::PROPERTIES[index];
    Ok(if casefold { folded } else { normal }.to_vec())
}

#[derive(Clone, Copy, Default)]
struct Flags {
    casefold: bool,
    multiline: bool,
    dot_newline: bool,
}
#[derive(Clone, Copy)]
struct Atom {
    start: usize,
    repeat_weight: usize,
    repeat_depth: usize,
}
struct Frame {
    start: usize,
    saved_flags: Flags,
    preceding_weight: usize,
    saw_atom: bool,
    last: Option<Atom>,
}
impl Frame {
    fn weight(&self) -> usize {
        self.preceding_weight
            .max(self.last.map_or(0, |atom| atom.repeat_weight))
    }
    fn finish_atom(&mut self) {
        self.preceding_weight = self.weight();
        self.last = None;
    }
}
#[derive(Clone, Copy)]
enum FlatAtom {
    Begin,
    End,
    Literal { rune: u32, start: usize, end: usize },
}
struct Parser {
    chars: Vec<char>,
    at: usize,
    output: String,
    frames: Vec<Frame>,
    flags: Flags,
    repeated: bool,
    has_begin: bool,
    surrogate_singleton: bool,
    flat: Option<Vec<FlatAtom>>,
}
fn translate(pattern: &str) -> Result<String, RegexIssue> {
    Parser {
        chars: pattern.chars().collect(),
        at: 0,
        output: String::new(),
        frames: vec![Frame {
            start: 0,
            saved_flags: Flags::default(),
            preceding_weight: 0,
            saw_atom: false,
            last: None,
        }],
        flags: Flags::default(),
        repeated: false,
        has_begin: false,
        surrogate_singleton: false,
        flat: Some(Vec::new()),
    }
    .parse()
}
impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }
    fn take(&mut self) -> Result<char, RegexIssue> {
        let c = self.peek().ok_or(RegexIssue::Invalid)?;
        self.at += 1;
        Ok(c)
    }
    fn atom(&mut self, text: &str, repeat_weight: usize) {
        let start = self.output.len();
        self.output.push_str(text);
        self.record_atom(start, repeat_weight);
    }
    fn record_atom(&mut self, start: usize, repeat_weight: usize) {
        let frame = self.frames.last_mut().unwrap();
        frame.finish_atom();
        frame.saw_atom = true;
        frame.last = Some(Atom {
            start,
            repeat_weight,
            repeat_depth: 0,
        });
        self.repeated = false;
    }
    fn scalar_atom(&mut self, ranges: &[(u32, u32)]) {
        let start = self.output.len();
        self.atom(&emit_ranges(ranges), 1);
        if let [(lo, hi)] = ranges
            && lo == hi
        {
            self.surrogate_singleton |= (0xd800..=0xdfff).contains(lo);
            if let Some(flat) = &mut self.flat {
                flat.push(FlatAtom::Literal {
                    rune: *lo,
                    start,
                    end: self.output.len(),
                });
            }
        } else {
            self.flat = None;
        }
    }
    fn anchor(&mut self, begin: bool, multiline: bool) {
        self.atom(
            match (begin, multiline) {
                (true, true) => "(?m:^)",
                (false, true) => "(?m:$)",
                (true, false) => r"\A",
                (false, false) => r"\z",
            },
            1,
        );
        if multiline {
            self.flat = None;
        } else {
            self.has_begin |= begin;
            if let Some(flat) = &mut self.flat {
                flat.push(if begin {
                    FlatAtom::Begin
                } else {
                    FlatAtom::End
                });
            }
        }
    }
    fn finish_surrogates(&mut self) -> Result<(), RegexIssue> {
        if !self.surrogate_singleton || !self.has_begin {
            return Ok(());
        }
        // Go's onePassPrefix encodes surrogate literal runes as RuneError and
        // skips those instructions. A straight BeginText/literal/EndText chain
        // has a source-provable one-pass program. More general candidates need
        // a port of Go's one-pass optimizer and remain explicitly unsupported.
        let Some(flat) = &self.flat else {
            return Err(RegexIssue::Unsupported(SURROGATE_GAP));
        };
        let Some((FlatAtom::Begin, tail)) = flat.split_first() else {
            return Err(RegexIssue::Unsupported(SURROGATE_GAP));
        };
        let body = if matches!(tail.last(), Some(FlatAtom::End)) {
            &tail[..tail.len() - 1]
        } else {
            tail
        };
        if body
            .iter()
            .any(|atom| !matches!(atom, FlatAtom::Literal { .. }))
        {
            return Err(RegexIssue::Unsupported(SURROGATE_GAP));
        }
        let mut replacements = Vec::new();
        let mut prefix = true;
        for atom in body {
            if let FlatAtom::Literal { rune, start, end } = *atom {
                // The Go prefix loop stops before the actual RuneError rune.
                if rune == 0xfffd {
                    prefix = false;
                }
                if prefix && (0xd800..=0xdfff).contains(&rune) {
                    replacements.push((start, end));
                }
            }
        }
        for (start, end) in replacements.into_iter().rev() {
            self.output.replace_range(start..end, r"[\x{fffd}]");
        }
        Ok(())
    }
    fn literal(&mut self, rune: u32) {
        let mut ranges = vec![(rune, rune)];
        if self.flags.casefold {
            let mut next = simple_fold(rune);
            while next != rune {
                ranges.push((next, next));
                next = simple_fold(next);
            }
            normalize(&mut ranges);
        }
        self.scalar_atom(&ranges);
    }
    fn parse(mut self) -> Result<String, RegexIssue> {
        while let Some(c) = self.peek() {
            if self.output.len() > MAX_TRANSLATED || self.frames.len() > 4096 {
                return Err(RegexIssue::Unsupported(
                    "translated regex exceeds adapter limits",
                ));
            }
            self.at += 1;
            match c {
                '\\' if self.peek() == Some('Q') => {
                    self.at += 1;
                    while let Some(c) = self.peek() {
                        if self.output.len() > MAX_TRANSLATED {
                            return Err(RegexIssue::Unsupported(
                                "translated regex exceeds adapter limits",
                            ));
                        }
                        if c == '\\' && self.chars.get(self.at + 1) == Some(&'E') {
                            self.at += 2;
                            break;
                        }
                        self.at += 1;
                        self.literal(c as u32);
                    }
                    // Go resets lastRepeat even for an empty quoted sequence.
                    self.repeated = false;
                }
                '\\' => {
                    let escaped = self.take()?;
                    match escaped {
                        'b' | 'B' => {
                            self.flat = None;
                            self.atom(&format!("(?-u:\\{escaped})"), 1);
                        }
                        'A' | 'z' => self.anchor(escaped == 'A', false),
                        'd' | 'D' | 's' | 'S' | 'w' | 'W' | 'p' | 'P' => {
                            let ranges = self.escape_set(escaped)?;
                            self.scalar_atom(&ranges);
                        }
                        _ => {
                            let rune = self.escape_rune(escaped)?;
                            self.literal(rune);
                        }
                    }
                }
                '[' => {
                    let ranges = self.class()?;
                    self.scalar_atom(&ranges);
                }
                '(' => self.group()?,
                ')' => {
                    if self.frames.len() == 1 {
                        return Err(RegexIssue::Invalid);
                    }
                    let frame = self.frames.pop().unwrap();
                    if !frame.saw_atom {
                        self.flat = None;
                    } // Go compiles an empty group as a Nop.
                    self.output.push(')');
                    self.flags = frame.saved_flags;
                    self.record_atom(frame.start, if frame.saw_atom { frame.weight() } else { 1 });
                }
                '|' => {
                    self.flat = None;
                    self.frames.last_mut().unwrap().finish_atom();
                    self.output.push('|');
                    self.repeated = false;
                }
                '*' | '+' | '?' => self.repeat(&c.to_string(), 1)?,
                '{' => {
                    if let Some((text, weight)) = self.counted_repeat()? {
                        self.repeat(&text, weight)?;
                    } else {
                        self.literal(c as u32);
                    }
                }
                '^' => self.anchor(true, self.flags.multiline),
                '$' => self.anchor(false, self.flags.multiline),
                '.' => {
                    self.flat = None;
                    self.atom(
                        if self.flags.dot_newline {
                            "(?s:.)"
                        } else {
                            "."
                        },
                        1,
                    );
                }
                _ => self.literal(c as u32),
            }
        }
        if self.frames.len() != 1 {
            return Err(RegexIssue::Invalid);
        }
        if self.output.len() > MAX_TRANSLATED {
            return Err(RegexIssue::Unsupported(
                "translated regex exceeds adapter limits",
            ));
        }
        self.finish_surrogates()?;
        Ok(self.output)
    }
    fn repeat(&mut self, text: &str, weight: usize) -> Result<(), RegexIssue> {
        self.flat = None;
        if self.repeated {
            return Err(RegexIssue::Invalid);
        }
        let atom = self
            .frames
            .last_mut()
            .unwrap()
            .last
            .as_mut()
            .ok_or(RegexIssue::Invalid)?;
        atom.repeat_weight = atom
            .repeat_weight
            .max(1)
            .checked_mul(weight)
            .filter(|&w| w <= 1000)
            .ok_or(RegexIssue::Invalid)?;
        atom.repeat_depth += 1;
        if atom.repeat_depth >= 1000 {
            return Err(RegexIssue::Unsupported(
                "translated regex exceeds backend nesting limit",
            ));
        }
        // Intervening flag directives and empty \Q\E reset Go's lastRepeat.
        // Grouping the already-emitted atom also allows those valid repeats in
        // the Rust dialect, which otherwise diagnoses adjacent quantifiers.
        self.output.insert_str(atom.start, "(?:");
        self.output.push(')');
        self.output.push_str(text);
        if self.peek() == Some('?') {
            self.at += 1;
        }
        // Greedy/non-greedy order (including U) cannot change boolean is_match.
        self.repeated = true;
        Ok(())
    }
    fn counted_repeat(&mut self) -> Result<Option<(String, usize)>, RegexIssue> {
        let start = self.at;
        let mut end = start;
        while self.chars.get(end).is_some_and(char::is_ascii_digit) {
            end += 1;
        }
        if end == start || (end - start > 1 && self.chars[start] == '0') {
            return Ok(None);
        }
        let min: usize = self.chars[start..end]
            .iter()
            .collect::<String>()
            .parse()
            .unwrap_or(usize::MAX);
        let mut max = min;
        let mut unbounded = false;
        if self.chars.get(end) == Some(&',') {
            end += 1;
            if self.chars.get(end) == Some(&'}') {
                unbounded = true;
            } else {
                let number_start = end;
                while self.chars.get(end).is_some_and(char::is_ascii_digit) {
                    end += 1;
                }
                if end == number_start
                    || (end - number_start > 1 && self.chars[number_start] == '0')
                {
                    return Ok(None);
                }
                max = self.chars[number_start..end]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(usize::MAX);
            }
        }
        if self.chars.get(end) != Some(&'}') {
            return Ok(None);
        }
        if min > max || max > 1000 {
            return Err(RegexIssue::Invalid);
        }
        self.at = end + 1;
        Ok(Some((
            format!("{{{}}}", self.chars[start..end].iter().collect::<String>()),
            if unbounded { min.max(1) } else { max },
        )))
    }
    fn open_group(&mut self, saved_flags: Flags) {
        let start = self.output.len();
        self.output.push_str("(?:");
        self.frames.push(Frame {
            start,
            saved_flags,
            preceding_weight: 0,
            saw_atom: false,
            last: None,
        });
        self.repeated = false;
    }
    fn group(&mut self) -> Result<(), RegexIssue> {
        let saved_flags = self.flags;
        if self.peek() != Some('?') {
            self.flat = None;
            self.open_group(saved_flags);
            return Ok(());
        }
        self.at += 1;
        if self.peek() == Some('P') && self.chars.get(self.at + 1) == Some(&'<') {
            self.at += 1;
        }
        if self.peek() == Some('<') {
            self.flat = None;
            self.at += 1;
            let start = self.at;
            while self
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                self.at += 1;
            }
            if self.at == start || self.take()? != '>' {
                return Err(RegexIssue::Invalid);
            }
            self.open_group(saved_flags);
            return Ok(());
        }
        let mut negative = false;
        let mut needs_flag = false;
        loop {
            let c = self.take()?;
            match c {
                '-' if !negative => {
                    negative = true;
                    needs_flag = true;
                }
                'i' | 'm' | 's' | 'U' => {
                    match c {
                        'i' => self.flags.casefold = !negative,
                        'm' => self.flags.multiline = !negative,
                        's' => self.flags.dot_newline = !negative,
                        _ => (),
                    }
                    needs_flag = false;
                }
                ':' | ')' if !needs_flag => {
                    if c == ':' {
                        self.open_group(saved_flags);
                    }
                    self.repeated = false;
                    return Ok(());
                }
                _ => return Err(RegexIssue::Invalid),
            }
        }
    }
    fn escape_rune(&mut self, c: char) -> Result<u32, RegexIssue> {
        Ok(match c {
            'a' => 7,
            'f' => 12,
            'n' => 10,
            'r' => 13,
            't' => 9,
            'v' => 11,
            '0'..='7' => {
                if c != '0' && !self.peek().is_some_and(|c| matches!(c, '0'..='7')) {
                    return Err(RegexIssue::Invalid);
                }
                let mut value = c as u32 - '0' as u32;
                for _ in 0..2 {
                    if let Some(c @ '0'..='7') = self.peek() {
                        self.at += 1;
                        value = value * 8 + c as u32 - '0' as u32;
                    } else {
                        break;
                    }
                }
                value
            }
            'x' => {
                let braced = self.peek() == Some('{');
                if braced {
                    self.at += 1;
                }
                let mut value = 0;
                let mut count = 0;
                while let Some(c) = self.peek() {
                    if braced && c == '}' {
                        break;
                    }
                    let digit = c
                        .to_digit(16)
                        .filter(|_| c.is_ascii())
                        .ok_or(RegexIssue::Invalid)?;
                    self.at += 1;
                    value = value * 16 + digit;
                    if value > MAX_RUNE {
                        return Err(RegexIssue::Invalid);
                    }
                    count += 1;
                    if !braced && count == 2 {
                        break;
                    }
                }
                if braced {
                    if count == 0 || self.take()? != '}' {
                        return Err(RegexIssue::Invalid);
                    }
                } else if count != 2 {
                    return Err(RegexIssue::Invalid);
                }
                value
            }
            c if c.is_ascii() && !c.is_ascii_alphanumeric() => c as u32,
            _ => return Err(RegexIssue::Invalid),
        })
    }
    fn class_rune(&mut self) -> Result<u32, RegexIssue> {
        let c = self.take()?;
        if c == '\\' {
            let c = self.take()?;
            self.escape_rune(c)
        } else {
            Ok(c as u32)
        }
    }
    fn escape_set(&mut self, c: char) -> Result<Ranges, RegexIssue> {
        let mut negate = c.is_ascii_uppercase();
        let mut ranges = match c {
            'p' | 'P' => {
                let name = if self.peek() == Some('{') {
                    self.at += 1;
                    let start = self.at;
                    while self.peek().is_some_and(|c| c != '}') {
                        self.at += 1;
                    }
                    let value: String = self.chars[start..self.at].iter().collect();
                    if self.take()? != '}' {
                        return Err(RegexIssue::Invalid);
                    }
                    value
                } else {
                    self.take()?.to_string()
                };
                let name = if let Some(name) = name.strip_prefix('^') {
                    negate = !negate;
                    name
                } else {
                    &name
                };
                property(name, self.flags.casefold)?
            }
            'd' | 'D' => vec![(0x30, 0x39)],
            's' | 'S' => vec![(9, 10), (12, 13), (32, 32)],
            'w' | 'W' => vec![(0x30, 0x39), (0x41, 0x5a), (0x5f, 0x5f), (0x61, 0x7a)],
            _ => return Err(RegexIssue::Invalid),
        };
        if self.flags.casefold && !matches!(c, 'p' | 'P') {
            fold(&mut ranges);
        }
        if negate {
            ranges = complement(&ranges);
        }
        Ok(ranges)
    }
    fn named_class(&mut self) -> Result<Option<Ranges>, RegexIssue> {
        // Go treats an unterminated POSIX [:name as ordinary class literals.
        let Some(relative_end) = self.chars[self.at + 2..]
            .windows(2)
            .position(|pair| pair == [':', ']'])
        else {
            return Ok(None);
        };
        let end = self.at + 2 + relative_end;
        let name: String = self.chars[self.at + 2..end].iter().collect();
        self.at = end + 2;
        let (name, negate) = name
            .strip_prefix('^')
            .map_or((name.as_str(), false), |name| (name, true));
        let mut ranges = match name {
            "alnum" => vec![(48, 57), (65, 90), (97, 122)],
            "alpha" => vec![(65, 90), (97, 122)],
            "ascii" => vec![(0, 127)],
            "blank" => vec![(9, 9), (32, 32)],
            "cntrl" => vec![(0, 31), (127, 127)],
            "digit" => vec![(48, 57)],
            "graph" => vec![(33, 126)],
            "lower" => vec![(97, 122)],
            "print" => vec![(32, 126)],
            "punct" => vec![(33, 47), (58, 64), (91, 96), (123, 126)],
            "space" => vec![(9, 13), (32, 32)],
            "upper" => vec![(65, 90)],
            "word" => vec![(48, 57), (65, 90), (95, 95), (97, 122)],
            "xdigit" => vec![(48, 57), (65, 70), (97, 102)],
            _ => return Err(RegexIssue::Invalid),
        };
        if self.flags.casefold {
            fold(&mut ranges);
        }
        if negate {
            ranges = complement(&ranges);
        }
        Ok(Some(ranges))
    }
    fn class(&mut self) -> Result<Ranges, RegexIssue> {
        let negate = self.peek() == Some('^');
        if negate {
            self.at += 1;
        }
        let mut ranges = Vec::new();
        let mut first = true;
        while self.peek() != Some(']') || first {
            first = false;
            if self.peek() == Some('[')
                && self.chars.get(self.at + 1) == Some(&':')
                && let Some(named) = self.named_class()?
            {
                ranges.extend(named);
                continue;
            }
            if self.peek() == Some('\\')
                && self
                    .chars
                    .get(self.at + 1)
                    .is_some_and(|c| "dDsSwWpP".contains(*c))
            {
                self.at += 1;
                let c = self.take()?;
                ranges.extend(self.escape_set(c)?);
                continue;
            }
            let lo = self.class_rune()?;
            let hi = if self.peek() == Some('-')
                && self.chars.get(self.at + 1).is_some_and(|c| *c != ']')
            {
                self.at += 1;
                self.class_rune()?
            } else {
                lo
            };
            if hi < lo {
                return Err(RegexIssue::Invalid);
            }
            let mut range = vec![(lo, hi)];
            if self.flags.casefold {
                fold(&mut range);
            }
            ranges.extend(range);
        }
        self.at += 1;
        normalize(&mut ranges);
        if negate {
            ranges = complement(&ranges);
        }
        Ok(ranges)
    }
}
