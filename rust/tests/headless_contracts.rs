//! Source → Rust: internal/headless/{format_test,runner_test}.go → orchestration/headless*.
//! Actual process fixtures are this synthetic test binary; no provider executable/config.
use many_ai_cli::{
    config::{HeadlessDef, RuntimePaths},
    orchestration::{headless::*, headless_formats::*},
    process::{Cancellation, ProcessPlan},
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Deserialize)]
struct Golden {
    format: String,
    input: String,
    events: Option<Vec<GoldenEvent>>,
    lines: Option<Vec<String>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GoldenEvent {
    #[serde(rename = "Type")]
    kind: String,
    tool: String,
    text: String,
    is_error: bool,
}
#[test]
fn actual_go_parser_and_render_corpus() {
    let cases: Vec<Golden> =
        serde_json::from_str(include_str!("fixtures/core/headless/go-golden.json")).unwrap();
    assert_eq!(cases.len(), 60);
    for c in cases {
        let events = parser_for(&c.format).unwrap().parse(c.input.as_bytes());
        let expected: Vec<_> = c
            .events
            .unwrap_or_default()
            .into_iter()
            .map(|e| Event {
                kind: e.kind,
                tool: e.tool,
                text: e.text,
                is_error: e.is_error,
            })
            .collect();
        assert_eq!(events, expected, "{} {:?}", c.format, c.input);
        assert_eq!(
            events.iter().map(Event::line).collect::<Vec<_>>(),
            c.lines.unwrap_or_default()
        );
    }
}
#[test]
fn registry_argv_and_placeholder_contract() {
    let formats = many_ai_cli::config::known_headless_formats();
    assert_eq!(
        formats,
        KNOWN_FORMATS
            .iter()
            .map(|s| (*s).to_string())
            .collect::<Vec<_>>()
    );
    for format in formats {
        assert!(parser_for(&format).is_some());
    }
    assert!(parser_for("made-up").is_none());
    let mut def = HeadlessDef {
        args: vec!["-p".into()],
        format: "text".into(),
        prompt_via: "stdin".into(),
    };
    let launch = vec!["--allowedTools".into(), "Read".into(), "Edit".into()];
    assert_eq!(
        build_argv(&def, &launch, "P"),
        ["-p", "--allowedTools", "Read", "Edit"]
    );
    def.prompt_via = " arg ".into();
    assert_eq!(
        build_argv(&def, &launch, "P"),
        ["-p", "P", "--allowedTools", "Read", "Edit"]
    );
    assert_eq!(
        expand_prompt(
            &format!("{SESSION_ID_PLACEHOLDER} {SESSION_ID_PLACEHOLDER}"),
            42
        ),
        "42 42"
    );
    assert_eq!(expand_prompt("unchanged", 42), "unchanged");
}
#[test]
fn framing_drains_oversized_and_preserves_blank_final_cr() {
    let mut bytes = vec![b'x'; MAX_LINE_BYTES * 2 + 19];
    bytes.extend(b"\r\n\nlast\r");
    let mut f = LineFramer::default();
    let mut lines = vec![];
    for chunk in bytes.chunks(137) {
        f.push(chunk, |line| lines.push(line.to_vec()));
        assert!(f.pending_bytes() < MAX_LINE_BYTES);
    }
    f.finish(|line| lines.push(line.to_vec()));
    assert_eq!(
        lines.iter().map(Vec::len).collect::<Vec<_>>(),
        [MAX_LINE_BYTES, MAX_LINE_BYTES, 19, 0, 5]
    );
    assert_eq!(lines.last().unwrap(), b"last\r");
}
#[test]
fn synthetic_provider_helper() {
    let Ok(mode) = std::env::var("MANY_SYNTHETIC_HEADLESS") else {
        return;
    };
    match mode.as_str() {
        "duplex" => {
            std::io::stdout()
                .write_all(&vec![b'x'; 128 * 1024])
                .unwrap();
            println!();
            std::io::stdout().flush().unwrap();
            let mut input = vec![];
            std::io::stdin().read_to_end(&mut input).unwrap();
            println!("INPUT_BYTES={}", input.len());
        }
        "eof" => {
            let mut input = vec![];
            std::io::stdin().read_to_end(&mut input).unwrap();
            println!("INPUT_BYTES={}", input.len());
        }
        "fail" => {
            println!(
                "{{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"model says success\"}}"
            );
            eprintln!("synthetic failure");
            std::process::exit(7);
        }
        "model_fail" => {
            println!("{{\"type\":\"result\",\"subtype\":\"failure\",\"is_error\":true}}")
        }
        "oversize" => {
            std::io::stdout()
                .write_all(&vec![b'x'; MAX_LINE_BYTES * 3 + 500])
                .unwrap();
            println!("\nAFTER_OVERSIZE");
        }
        "sleep" => {
            println!("READY");
            std::io::stdout().flush().unwrap();
            std::thread::sleep(Duration::from_secs(30));
        }
        "closed_pipes" => {
            #[cfg(unix)]
            unsafe {
                libc::close(1);
                libc::close(2);
            }
            std::thread::sleep(Duration::from_secs(30));
        }
        _ => panic!("unexpected synthetic mode"),
    }
    std::process::exit(0);
}
fn spec(mode: &str, dir: &std::path::Path) -> Spec {
    let mut env = BTreeMap::new();
    env.insert("MANY_SYNTHETIC_HEADLESS".into(), Some(mode.into()));
    Spec {
        process: ProcessPlan {
            executable: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "synthetic_provider_helper".into(),
                "--nocapture".into(),
            ],
            cwd: dir.into(),
            env,
            stdin: vec![],
            timeout: Duration::from_secs(5),
            output_cap: 1024,
            pipe_drain_timeout: Duration::from_millis(100),
        },
        prompt: String::new(),
        prompt_via: "stdin".into(),
        format: "text".into(),
        raw_logs: None,
        event_capacity: 1024,
    }
}
#[tokio::test]
async fn real_duplex_child_reads_large_prompt_without_deadlock() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = spec("duplex", dir.path());
    s.prompt = "p".repeat(256 * 1024);
    let mut output = vec![];
    let r = run(s, &Cancellation::default(), |b| output.extend_from_slice(b))
        .await
        .unwrap();
    assert_eq!(r.state, "completed");
    assert_eq!(r.dropped_chunks, 0);
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("INPUT_BYTES=262144\r\n")
    );
    assert!(r.process_output.stdout_truncated);
}
#[tokio::test]
async fn arg_prompt_closes_stdin_and_zero_timeout_has_no_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = spec("eof", dir.path());
    s.prompt = "not stdin".into();
    s.prompt_via = "arg".into();
    s.process.timeout = Duration::ZERO;
    let mut out = vec![];
    let r = run(s, &Cancellation::default(), |b| out.extend_from_slice(b))
        .await
        .unwrap();
    assert_eq!(r.exit_code, 0);
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("INPUT_BYTES=0\r\n")
    );
}
#[tokio::test]
async fn exit_code_is_authoritative_over_model_verdict() {
    let dir = tempfile::tempdir().unwrap();
    for (mode, expected) in [("fail", 7), ("model_fail", 0)] {
        let mut s = spec(mode, dir.path());
        s.format = "claude-stream-json".into();
        let mut out = vec![];
        let r = run(s, &Cancellation::default(), |b| out.extend_from_slice(b))
            .await
            .unwrap();
        assert_eq!(r.exit_code, expected);
        assert_eq!(r.state, if expected == 0 { "completed" } else { "error" });
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains(if expected == 0 { "[error]" } else { "[result]" }));
        if expected != 0 {
            assert!(text.contains("[stderr] synthetic failure\r\n"));
        }
    }
}
#[tokio::test]
async fn oversized_process_output_drains_to_real_exit() {
    let dir = tempfile::tempdir().unwrap();
    let s = spec("oversize", dir.path());
    let mut out = vec![];
    let r = run(s, &Cancellation::default(), |b| out.extend_from_slice(b))
        .await
        .unwrap();
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.dropped_chunks, 0);
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("AFTER_OVERSIZE\r\n")
    );
}
#[tokio::test]
async fn cancellation_and_timeout_are_distinct_and_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = spec("sleep", dir.path());
    s.process.timeout = Duration::from_millis(150);
    let now = Instant::now();
    let r = run(s, &Cancellation::default(), |_| {}).await.unwrap();
    assert!(r.timed_out && !r.canceled);
    assert!(now.elapsed() < Duration::from_secs(3));
    let s = spec("sleep", dir.path());
    let cancel = Cancellation::default();
    let trigger = cancel.clone();
    let r = run(s, &cancel, move |b| {
        if String::from_utf8_lossy(b).contains("READY") {
            trigger.cancel();
        }
    })
    .await
    .unwrap();
    assert!(r.canceled && !r.timed_out);
}
#[cfg(unix)]
#[tokio::test]
async fn cancellation_after_both_output_pipes_close_still_reaches_child() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = spec("closed_pipes", dir.path());
    s.process.timeout = Duration::from_secs(30);
    let cancel = Cancellation::default();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        trigger.cancel();
    });
    let now = Instant::now();
    let r = run(s, &cancel, |_| {}).await.unwrap();
    assert!(r.canceled);
    assert!(now.elapsed() < Duration::from_secs(3));
}
#[tokio::test]
async fn invalid_format_and_spawn_errors_are_real_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = spec("eof", dir.path());
    s.format = "not-a-format".into();
    s.process.executable = dir.path().join("missing");
    assert_eq!(
        run(s.clone(), &Cancellation::default(), |_| {})
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    s.format = "text".into();
    assert_eq!(
        run(s, &Cancellation::default(), |_| {})
            .await
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
}
#[tokio::test]
async fn raw_logs_are_opt_in_private_and_root_checked() {
    let trial = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(trial.path(), 49231, installed.path()).unwrap();
    let mut s = spec("eof", trial.path());
    run(s.clone(), &Cancellation::default(), |_| {})
        .await
        .unwrap();
    assert!(!paths.resource(many_ai_cli::config::Resource::Logs).exists());
    s.raw_logs = Some(RawLogs {
        paths: paths.clone(),
        prefix: "sessions/synthetic".into(),
    });
    let result = run(s.clone(), &Cancellation::default(), |_| {})
        .await
        .unwrap();
    assert!(result.raw_log_errors.is_empty());
    let file = paths
        .resource(many_ai_cli::config::Resource::Logs)
        .join("sessions/synthetic.stdout.log");
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("INPUT_BYTES=0\n")
    );
    let mut existing = std::fs::File::open(&file).unwrap();
    let prior_len = existing.metadata().unwrap().len();
    let appended = run(s.clone(), &Cancellation::default(), |_| {})
        .await
        .unwrap();
    assert!(appended.raw_log_errors.is_empty());
    assert!(existing.metadata().unwrap().len() > prior_len);
    {
        use std::io::{Read, Seek, SeekFrom};
        existing.seek(SeekFrom::Start(prior_len)).unwrap();
        let mut new_bytes = String::new();
        existing.read_to_string(&mut new_bytes).unwrap();
        assert!(new_bytes.contains("INPUT_BYTES=0\n"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    s.raw_logs.as_mut().unwrap().prefix = "../../escape".into();
    assert!(
        !run(s, &Cancellation::default(), |_| {})
            .await
            .unwrap()
            .raw_log_errors
            .is_empty()
    );
}
#[cfg(unix)]
#[tokio::test]
async fn inherited_descendant_pipes_cannot_hang_runner() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = spec("eof", dir.path());
    s.process.executable = "/bin/sh".into();
    s.process.args = vec!["-c".into(), "sleep 30 & echo $!; exit 0".into()];
    let now = Instant::now();
    let r = run(s, &Cancellation::default(), |_| {}).await.unwrap();
    assert_eq!(r.exit_code, 0);
    assert!(r.process_output.pipes_forced_closed);
    assert!(now.elapsed() < Duration::from_secs(3));
    #[cfg(target_os = "linux")]
    {
        let pid: u32 = String::from_utf8(r.process_output.stdout.clone())
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // A bounded return alone is not containment proof: inspect only the
        // synthetic descendant PID emitted by our own shell fixture.
        let until = Instant::now() + Duration::from_secs(1);
        loop {
            let alive = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|s| {
                    s.rsplit_once(") ")
                        .map(|(_, fields)| fields.starts_with(['R', 'S', 'D', 'T', 't', 'I']))
                })
                .unwrap_or(false);
            if !alive {
                break;
            }
            assert!(
                Instant::now() < until,
                "owned synthetic descendant is still running"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_lag_is_explicit_and_cancels_owned_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = spec("oversize", dir.path());
    s.event_capacity = 1;
    let seen = Arc::new(Mutex::new(false));
    let seen_callback = seen.clone();
    let r = run(s, &Cancellation::default(), move |_| {
        let mut first = seen_callback.lock().unwrap();
        if !*first {
            *first = true;
            std::thread::sleep(Duration::from_millis(150));
        }
    })
    .await
    .unwrap();
    assert!(r.dropped_chunks > 0);
    assert_eq!(r.state, "error");
}

#[derive(Deserialize)]
struct FramingGolden {
    length: usize,
    suffix: String,
    lines: Option<Vec<FramedLine>>,
}
#[derive(Deserialize, Debug, PartialEq, Eq)]
struct FramedLine {
    length: usize,
    sha256: String,
}
#[test]
fn actual_go_line_framing_corpus_all_chunk_divisions() {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let cases: Vec<FramingGolden> =
        serde_json::from_str(include_str!("fixtures/core/headless/framing-golden.json")).unwrap();
    assert_eq!(cases.len(), 40);
    for case in cases {
        let mut input = vec![b'x'; case.length];
        input.extend_from_slice(case.suffix.as_bytes());
        for chunk_size in [1, 137, 65536, 1048576] {
            let mut reader = LineFramer::default();
            let mut lines = vec![];
            let mut accept = |line: &[u8]| {
                let digest = Sha256::digest(line);
                let mut sha256 = String::with_capacity(digest.len() * 2);
                for byte in digest.iter() {
                    write!(sha256, "{byte:02x}").unwrap();
                }
                lines.push(FramedLine {
                    length: line.len(),
                    sha256,
                })
            };
            for chunk in input.chunks(chunk_size) {
                reader.push(chunk, &mut accept);
                assert!(reader.pending_bytes() < MAX_LINE_BYTES + 65536);
            }
            reader.finish(&mut accept);
            assert_eq!(
                &lines,
                case.lines.as_ref().unwrap_or(&vec![]),
                "len={} suffix={:?} chunks={chunk_size}",
                case.length,
                case.suffix
            );
        }
    }
}
