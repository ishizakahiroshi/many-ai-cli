use crate::proto::{WfAgent, WfPhase, WorkflowProgress, unicode::simple_lower};
use regex::Regex;
use std::sync::LazyLock;
fn re(pattern: &str) -> Regex {
    Regex::new(
        &pattern
            .replace(r"\s", r"[\t\n\x0c\r ]")
            .replace(r"\b", r"(?-u:\b)"),
    )
    .unwrap()
}
macro_rules! pattern {
    ($name:ident,$expr:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| re($expr));
    };
}
pattern!(CSI, r"\x1b\[[0-9;?]*[ -/]*[@-~]");
pattern!(OSC, r"\x1b\][^\x07]*(?:\x07|\x1b\\)");
pattern!(TREE, r"^[\s\x{00A0}│├└─╰╭╮╯┃┣┗┏┓┛┆┊▕▏▸▹‣•·⎿]+");
pattern!(HEADER, r"(?i)\bworkflows?\b");
pattern!(SUMMARY, r"(?i)([0-9]{1,4})\s*/\s*([0-9]{1,4})\s+agents?\b");
pattern!(
    WAIT,
    r"(?i)\bwaiting for\s+([0-9]{1,3})\s+dynamic\s+workflows?\s+to\s+finish\b"
);
pattern!(PERCENT, r"([0-9]{1,3})\s*%");
pattern!(TIME, r"(?i)([0-9]+)\s*([hms])");
pattern!(METRICS, r"\s{2,}");
pattern!(TIP, r"(?i)^tip[:：]");
pattern!(BOUNDARY, r"^[\s\x{00A0}─━═]*[─━═]{3,}[\s\x{00A0}─━═]*$|❯");
fn ansi(line: &str) -> String {
    CSI.replace_all(&OSC.replace_all(line, ""), "").into_owned()
}
fn tree(line: &str) -> String {
    TREE.replace_all(&ansi(line), "").into_owned()
}
fn compact(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn header(line: &str) -> Option<String> {
    if TIP.is_match(line.trim()) || (!line.contains('⚙') && !HEADER.is_match(line)) {
        return None;
    }
    let name = HEADER.replace_all(&line.replace('⚙', ""), "").into_owned();
    let name = compact(name.trim().trim_start_matches([':', '：']));
    Some(
        if ["running", "done", "complete", "completed", "in progress"]
            .contains(&simple_lower(&name).as_str())
        {
            String::new()
        } else {
            name.chars().take(60).collect()
        },
    )
}
fn summary(line: &str) -> Option<(i64, i64, i64, String)> {
    let captures = SUMMARY.captures(line)?;
    let done = captures[1].parse().ok()?;
    let total = captures[2].parse().ok()?;
    if total <= 0 || done < 0 || done > total {
        return None;
    }
    let mut elapsed = 0;
    let mut tokens = String::new();
    let mut found = false;
    for segment in line[captures.get(0)?.end()..].split(['·', '•']) {
        let segment = segment.trim();
        if segment.starts_with('↓') {
            tokens = segment.into();
            continue;
        }
        let times = TIME.captures_iter(segment).collect::<Vec<_>>();
        if times.is_empty() || found {
            continue;
        }
        for time in times {
            let value = time[1].parse::<i64>().unwrap_or(0);
            elapsed += value
                * match simple_lower(&time[2]).as_str() {
                    "h" => 3600,
                    "m" => 60,
                    _ => 1,
                };
        }
        found = true;
    }
    Some((done, total, elapsed, tokens))
}
fn agent(line: &str) -> Option<WfAgent> {
    let first = line.chars().next()?;
    let state = if "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏⣾⣽⣻⢿⡿⣟⣯⣷◐◓◑◒◜◝◞◟●".contains(first)
    {
        "running"
    } else if "✓✔".contains(first) {
        "done"
    } else if "✗✘".contains(first) {
        "failed"
    } else if "○◌◯".contains(first) {
        "pending"
    } else {
        return None;
    };
    let rest = &line[first.len_utf8()..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim();
    let (label, metrics) = METRICS
        .find(rest)
        .map(|m| (&rest[..m.start()], &rest[m.end()..]))
        .unwrap_or((rest, ""));
    let label = compact(label.trim());
    if label.is_empty() || label.len() > 200 {
        return None;
    }
    Some(WfAgent {
        label,
        state: state.into(),
        metrics: metrics.trim().into(),
        ..Default::default()
    })
}
pub fn parse(lines: &[String]) -> Option<WorkflowProgress> {
    if lines.is_empty() {
        return None;
    }
    let waiting = lines
        .iter()
        .rev()
        .filter_map(|line| {
            WAIT.captures(&ansi(line))
                .and_then(|c| c[1].parse::<i64>().ok())
        })
        .find(|n| *n > 0)
        .unwrap_or(0);
    let mut start = None;
    let mut name = String::new();
    for (i, raw) in lines.iter().enumerate().rev() {
        let line = tree(raw);
        if summary(&line).is_some() {
            start = Some(i);
            break;
        }
        if WAIT.is_match(&line) {
            continue;
        }
        if let Some(n) = header(&line) {
            start = Some(i);
            name = n;
            break;
        }
    }
    let Some(start) = start else {
        return (waiting > 0).then(|| WorkflowProgress {
            detected: true,
            source: "vt-summary".into(),
            waiting_dynamic: waiting,
            ..Default::default()
        });
    };
    let mut progress = WorkflowProgress {
        name,
        waiting_dynamic: waiting,
        ..Default::default()
    };
    let mut pending = String::new();
    let mut percent = None;
    let mut has_summary = false;
    for (i, raw) in lines.iter().enumerate().skip(start) {
        if i > start && BOUNDARY.is_match(&ansi(raw)) {
            break;
        }
        let line = tree(raw);
        if line.is_empty() {
            continue;
        }
        if let Some((done, total, elapsed, tokens)) = summary(&line) {
            progress.done = done;
            progress.total = total;
            progress.running = total - done;
            progress.elapsed_sec = elapsed;
            progress.tokens_raw = tokens;
            has_summary = true;
            continue;
        }
        if i == start {
            continue;
        }
        if let Some(agent) = agent(&line) {
            if progress.phases.is_empty() || !pending.is_empty() {
                progress.phases.push(WfPhase {
                    title: std::mem::take(&mut pending),
                    agents: Some(Vec::new()),
                })
            }
            progress
                .phases
                .last_mut()
                .unwrap()
                .agents
                .as_mut()
                .unwrap()
                .push(agent);
            continue;
        }
        if let Some(value) = PERCENT
            .captures(&line)
            .and_then(|c| c[1].parse::<i64>().ok())
            .filter(|v| *v <= 100)
        {
            percent = Some(value);
            continue;
        }
        pending = compact(&line).chars().take(80).collect();
    }
    progress.source = if has_summary { "vt-summary" } else { "vt-tree" }.into();
    if !has_summary {
        for phase in &progress.phases {
            for agent in phase.agents.as_deref().unwrap_or(&[]) {
                progress.total += 1;
                match agent.state.as_str() {
                    "running" => progress.running += 1,
                    "done" => progress.done += 1,
                    "failed" => progress.failed += 1,
                    _ => progress.pending += 1,
                }
            }
        }
    }
    if progress.total == 0 {
        if waiting > 0 {
            progress.detected = true;
            progress.source = "vt-summary".into();
            return Some(progress);
        }
        return None;
    }
    progress.detected = true;
    progress.percent = if has_summary {
        progress.done * 100 / progress.total
    } else {
        percent.unwrap_or((progress.done + progress.failed) * 100 / progress.total)
    };
    Some(progress)
}
