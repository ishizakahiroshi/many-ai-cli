//! internal/sessionlog/transcript.go conversion; malformed crash-tail records are skipped.
use crate::proto::wire::{Field, GoWire, Schema};
use base64::{Engine, engine::general_purpose::STANDARD};
use regex::Regex;
use std::{
    io::{self, BufRead, Write},
    sync::LazyLock,
};
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct Event {
    ts: String,
    #[serde(rename = "type")]
    kind: String,
    session_id: i64,
    provider: String,
    cwd: String,
    branch: String,
    model: String,
    shell: String,
    pid: i64,
    filename: String,
    path: String,
    text: String,
    data_b64: String,
    state: String,
    exit_code: i64,
    combined_has_inject: bool,
}
impl GoWire for Event {
    const GO_TYPE: &'static str = "TranscriptEvent";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "TranscriptEvent",
        fields: &[
            Field {
                name: "ts",
                kind: "string",
            },
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "session_id",
                kind: "int",
            },
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "cwd",
                kind: "string",
            },
            Field {
                name: "branch",
                kind: "string",
            },
            Field {
                name: "model",
                kind: "string",
            },
            Field {
                name: "shell",
                kind: "string",
            },
            Field {
                name: "pid",
                kind: "int",
            },
            Field {
                name: "filename",
                kind: "string",
            },
            Field {
                name: "path",
                kind: "string",
            },
            Field {
                name: "text",
                kind: "string",
            },
            Field {
                name: "data_b64",
                kind: "string",
            },
            Field {
                name: "state",
                kind: "string",
            },
            Field {
                name: "exit_code",
                kind: "int",
            },
            Field {
                name: "combined_has_inject",
                kind: "bool",
            },
        ],
    }];
}
pub fn clean_visible_text(text: &str) -> String {
    crate::approval::identity::strip_ansi(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|&c| !matches!(c,'\0'..='\u{8}'|'\u{b}'|'\u{c}'|'\u{e}'..='\u{1f}'|'\u{7f}'))
        .collect()
}
pub fn is_thinking_noise_line(line: &str) -> bool {
    let line = line.trim();
    let lower = line.to_lowercase();
    if line.is_empty() {
        return false;
    }
    if lower.contains("esc to interrupt")
        || lower.contains("shift+tab to cycle")
        || lower.contains("shift + tab to cycle")
        || (line.contains('↑') && line.contains('↓'))
    {
        return true;
    }
    let count = line
        .chars()
        .filter(|c| matches!(*c,'\u{2722}'..='\u{273f}'|'\u{2800}'..='\u{28ff}'))
        .take(2)
        .count();
    count >= 2 || (lower.contains("thinking") && count > 0)
}
fn spinner(line: &str) -> bool {
    static LETTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\pL\pN]").unwrap());
    matches!(line, "Boot" | "Boo" | "Bo" | "Thinking" | "Working")
        || (line.chars().count() <= 2 && !LETTER.is_match(line))
        || is_thinking_noise_line(line)
}
fn normalize(text: &str) -> String {
    let cleaned = clean_visible_text(text);
    let mut lines = vec![];
    let mut previous = "";
    for line in cleaned.split('\n') {
        let line = line.trim_end_matches([' ', '\t']);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !previous.is_empty() {
                lines.push("");
                previous = "";
            }
            continue;
        }
        if spinner(trimmed) || trimmed == previous {
            continue;
        }
        lines.push(line);
        previous = trimmed;
    }
    lines.join("\n").trim().to_owned()
}
fn flush(writer: &mut impl Write, output: &mut String, last: &mut String) -> io::Result<()> {
    let text = normalize(output);
    output.clear();
    if text.is_empty() || text == *last {
        return Ok(());
    }
    last.clone_from(&text);
    write!(writer, "\n[output]\n{text}\n")
}
pub fn write_transcript(reader: &mut impl BufRead, writer: &mut impl Write) -> io::Result<()> {
    let (mut output, mut last) = (String::new(), String::new());
    let mut line = vec![];
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let Ok(event) = crate::proto::decode_wire::<Event>(&line) else {
            continue;
        };
        if event.kind == "pty_output" {
            let text = if event.text.is_empty() && !event.data_b64.is_empty() {
                STANDARD
                    .decode(&event.data_b64)
                    .map(|bytes| crate::proto::wire::go_utf8_lossy(&bytes))
                    .unwrap_or_default()
            } else {
                event.text
            };
            output.push_str(&clean_visible_text(&text));
            continue;
        }
        flush(writer, &mut output, &mut last)?;
        match event.kind.as_str() {
            "session_start" => {
                writeln!(
                    writer,
                    "[{}] session_start #{} {} pid={}",
                    event.ts, event.session_id, event.provider, event.pid
                )?;
                for (label, value) in [
                    ("cwd", event.cwd),
                    ("branch", event.branch),
                    ("model", event.model),
                    ("shell", event.shell),
                ] {
                    if !value.is_empty() {
                        writeln!(writer, "{label}: {value}")?;
                    }
                }
            }
            "attach" => write!(
                writer,
                "\n[{}] attach {}\n{}\n",
                event.ts, event.filename, event.path
            )?,
            "user_input" => write!(
                writer,
                "\n[{}] user_input{}\n> {}\n",
                event.ts,
                if event.combined_has_inject {
                    " + attachment"
                } else {
                    ""
                },
                clean_visible_text(&event.text).trim()
            )?,
            "session_end" => write!(
                writer,
                "\n[{}] session_end state={} exit_code={}\n",
                event.ts, event.state, event.exit_code
            )?,
            _ => write!(writer, "\n[{}] {}\n", event.ts, event.kind)?,
        }
    }
    flush(writer, &mut output, &mut last)
}
