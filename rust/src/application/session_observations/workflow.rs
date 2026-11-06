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
// 見出しにしてよい「workflow」は行頭（直前に状態グリフ 1 個だけ許す: `● Workflow(x)`）。
// 散文の途中の語（`…いまは Workflow が裏で動いています`）や、`● ` で始まる発言行は見出しにしない。
pattern!(
    HEADER_LEAD,
    r"(?i)^(?:[^\p{L}\p{N}\t\n\x0c\r ]\s+)?workflows?\b"
);
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
    if TIP.is_match(line.trim()) || (!line.contains('⚙') && !HEADER_LEAD.is_match(line.trim())) {
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
    let mut past_wait = false;
    for (i, raw) in lines.iter().enumerate().skip(start) {
        if i > start && BOUNDARY.is_match(&ansi(raw)) {
            break;
        }
        let line = tree(raw);
        // 「待ち」の行より下は入力欄の上の案内（アップデート通知など）で、ツリーではない。
        // エージェント行・フェーズ見出し・割合はここから先を読まない（集計行だけは読む）。
        if i > start && WAIT.is_match(&line) {
            past_wait = true;
            continue;
        }
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
        if i == start || past_wait {
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
#[cfg(test)]
mod tests {
    use super::parse;
    fn lines(rows: &[&str]) -> Vec<String> {
        rows.iter().map(|row| row.to_string()).collect()
    }
    // 2026-11-06 の背景 Workflow 実行中の画面末尾を、スクリーンショットから起こした再構成
    // （実物の VT ミラーではない）。散文に単語 Workflow があり、「待ち」の行の下に
    // アップデート通知が出る。
    #[test]
    fn prose_and_update_notice_around_the_waiting_line_are_not_a_workflow_tree() {
        let tail = lines(&[
            "● C1（棚卸し）と C3（静的検査）は終わり、いまは Workflow が裏で動いています。",
            "ソースには手を入れていません（tracked ファイルの差分は 0 件）。Workflow が終わったら、通知を受けて反証の結果を集計し、report と HTML",
            "を仕上げます。",
            "",
            "* Waiting for 1 dynamic workflow to finish",
            "                                      ✔ Update installed · Restart to update",
            "──────────────────────────────────── ultracode ─",
            "❯",
        ]);
        let progress = parse(&tail).expect("the waiting line alone is a signal");
        assert_eq!(progress.waiting_dynamic, 1);
        assert_eq!((progress.total, progress.done), (0, 0));
        assert!(progress.phases.is_empty());
    }
    #[test]
    fn rows_below_the_waiting_line_are_not_agents_even_under_a_real_header() {
        let tail = lines(&[
            "⚙ Workflow review",
            "  ● explore-a",
            "  ✔ explore-b",
            "* Waiting for 1 dynamic workflow to finish",
            "  ✔ Update installed · Restart to update",
            "──────────────",
            "❯",
        ]);
        let progress = parse(&tail).unwrap();
        assert_eq!((progress.total, progress.done, progress.running), (2, 1, 1));
        assert_eq!(progress.waiting_dynamic, 1);
    }
    #[test]
    fn a_tool_call_line_with_a_leading_glyph_is_still_a_header() {
        let tail = lines(&["● Workflow(review-changes)", "  ✔ agent-1", "  ● agent-2"]);
        let progress = parse(&tail).unwrap();
        assert_eq!((progress.total, progress.done), (2, 1));
    }
    #[test]
    fn a_prose_line_that_merely_mentions_workflow_is_not_a_header() {
        let tail = lines(&["いまは Workflow が動いています。", "  ✔ Update installed"]);
        assert!(parse(&tail).is_none());
    }
}
