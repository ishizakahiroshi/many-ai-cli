use regex::Regex;
use std::sync::LazyLock;
static HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[\t\n\x0c\r ]*[•●⏺]?[\t\n\x0c\r ]*(message from|received message from)[\t\n\x0c\r ]+(.+?)[\t\n\x0c\r ]*$").unwrap()
});
#[derive(Clone, PartialEq, Eq)]
pub struct Candidate {
    pub sender: String,
    pub line: String,
    pub signature: String,
}
pub fn detect(lines: &[String]) -> Option<Candidate> {
    let mut matches = Vec::new();
    let mut candidate = None;
    for raw in lines {
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }
        let Some(parts) = HEADER.captures(line) else {
            continue;
        };
        let sender = parts[2].trim();
        if sender.is_empty() {
            continue;
        }
        matches.push(line);
        candidate = Some(Candidate {
            sender: sender.into(),
            line: line.into(),
            signature: String::new(),
        });
    }
    candidate.map(|mut candidate| {
        candidate.signature = matches.join("\n");
        candidate
    })
}
