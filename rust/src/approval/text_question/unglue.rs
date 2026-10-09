//! Byte positions and scan advancement follow Go's handwritten JS lookahead
//! equivalents; prose numbering is split only at the same strong/weak marks.
use super::*;
re!(
    ANCHOR,
    r"(?i)\s*\bN\.[ \t]*(User\s*specifies|その他指定)\b\s*"
);
re!(NUMBER_START, r"^\s*\d{1,2}\.\s+\S");
re!(STRONG, r"(\d{1,2})\.\s*\[([^\]\[\n]{1,16})\]");
re!(BULLET, r"^[>❯›❱*\-•・]+$");

fn inline_at(text: &str) -> Option<usize> {
    for (i, c) in text.char_indices().skip(1) {
        if !matches!(c, 'Q' | 'Ｑ') {
            continue;
        }
        let mut j = i + c.len_utf8();
        let begin = j;
        while text.as_bytes().get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        if (1..=2).contains(&(j - begin))
            && text[j..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_ascii_alphanumeric())
        {
            return Some(i);
        }
    }
    None
}
fn inline(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for raw in lines {
        let mut rest = raw.as_str();
        if NUMBER_START.is_match(rest) {
            if !rest.trim().is_empty() {
                out.push(rest.into());
            }
            continue;
        }
        while rest.chars().count() > 1 {
            let Some(at) = inline_at(rest) else {
                break;
            };
            if rest[..at]
                .chars()
                .next_back()
                .is_some_and(|c| "[「『【（(".contains(c))
            {
                break;
            }
            let head = rest[..at].trim();
            if !head.is_empty() {
                out.push(head.into());
            }
            rest = &rest[at..];
        }
        if !rest.trim().is_empty() {
            out.push(rest.into());
        }
    }
    out
}
fn anchors(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for raw in lines {
        let mut rest = raw.as_str();
        loop {
            let Some(m) = ANCHOR.captures(rest) else {
                out.push(rest.into());
                break;
            };
            let span = m.get(0).unwrap();
            let before = rest[..span.start()].trim();
            if !before.is_empty() {
                out.push(before.into());
            }
            out.push(
                if m[1].contains("その他指定") {
                    "N. その他指定"
                } else {
                    "N. User specifies"
                }
                .into(),
            );
            rest = rest[span.end()..].trim();
            if rest.is_empty() {
                break;
            }
        }
    }
    out
}
#[derive(Clone, Copy)]
struct Mark {
    at: usize,
    num: i64,
}
fn digits(text: &str, pos: usize) -> Option<(i64, usize)> {
    let mut j = pos;
    while text.as_bytes().get(j).is_some_and(u8::is_ascii_digit) {
        j += 1;
    }
    if !(1..=2).contains(&(j - pos)) || text.as_bytes().get(j) != Some(&b'.') {
        return None;
    }
    let num = text[pos..j].parse().ok()?;
    let mut k = j + 1;
    let c = text[k..].chars().next()?;
    if c.is_whitespace() {
        while let Some(c) = text[k..].chars().next() {
            if !c.is_whitespace() {
                break;
            }
            k += c.len_utf8();
        }
        Some((num, k))
    } else if c.is_ascii_digit() {
        None
    } else {
        Some((num, k))
    }
}
fn weak(text: &str) -> Vec<Mark> {
    let mut marks = Vec::new();
    let mut pos = 0;
    while pos < text.len() {
        if pos == 0
            && let Some((num, end)) = digits(text, 0)
        {
            marks.push(Mark { at: 0, num });
            pos = end;
            continue;
        }
        let c = text[pos..].chars().next().unwrap();
        let size = c.len_utf8();
        if (c.is_whitespace() || ")）」』】。．？?！!…".contains(c))
            && let Some((num, end)) = digits(text, pos + size)
        {
            marks.push(Mark {
                at: pos + size,
                num,
            });
            pos = end;
            continue;
        }
        pos += size;
    }
    marks
}
fn at_marks(text: &str, marks: &[Mark]) -> Vec<String> {
    let mut out = Vec::new();
    let head = text[..marks[0].at].trim();
    if !head.is_empty() {
        out.push(head.into());
    }
    for (i, m) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map_or(text.len(), |m| m.at);
        let seg = text[m.at..end].trim();
        if !seg.is_empty() {
            out.push(seg.into());
        }
    }
    out
}
fn numbered(text: &str) -> Vec<String> {
    let mut strong: Vec<Mark> = Vec::new();
    let mut good = true;
    for m in STRONG.captures_iter(text) {
        let num = m[1].parse().unwrap_or(0);
        if strong.last().is_some_and(|prev| num <= prev.num) {
            good = false;
            break;
        }
        strong.push(Mark {
            at: m.get(0).unwrap().start(),
            num,
        });
    }
    if good && strong.len() >= 2 {
        return at_marks(text, &strong);
    }
    if strong.len() == 1 {
        let head = text[..strong[0].at].trim();
        let seg = text[strong[0].at..].trim();
        if !head.is_empty() && !seg.is_empty() && !head.as_bytes()[0].is_ascii_digit() {
            return vec![head.into(), seg.into()];
        }
    }
    let marks = weak(text);
    if marks.len() < 2 {
        if marks.len() == 1 && marks[0].num == 1 {
            let head = text[..marks[0].at].trim();
            if !head.is_empty() && !BULLET.is_match(head) {
                let seg = text[marks[0].at..].trim();
                return if seg.is_empty() {
                    vec![head.into()]
                } else {
                    vec![head.into(), seg.into()]
                };
            }
        }
        return vec![text.into()];
    }
    if marks.windows(2).any(|v| v[1].num != v[0].num + 1) {
        return vec![text.into()];
    }
    at_marks(text, &marks)
}
pub fn unglued_lines(lines: &[String]) -> Vec<String> {
    anchors(&inline(lines))
        .iter()
        .flat_map(|s| numbered(s))
        .collect()
}
