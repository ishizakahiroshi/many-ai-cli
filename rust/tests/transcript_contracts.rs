use many_ai_cli::approval::transcript::reader::*;
use std::{
    fs,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
fn budget(bytes: usize, records: usize) -> ReadBudget {
    ReadBudget {
        max_bytes: bytes,
        max_records: records,
        deadline: Instant::now() + Duration::from_secs(20),
        clock: Arc::new(Instant::now),
    }
}
#[test]
fn forward_reader_retains_partial_record_and_committed_safe_cursor() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.jsonl");
    fs::write(&path, b"one\ntw").unwrap();
    let mut state = ReadState::default();
    let mut out = Vec::new();
    let s = read_range(&path, 0, &mut state, &budget(100, 100), |l, _, _| {
        out.push(l.to_vec());
        Ok(())
    })
    .unwrap();
    assert_eq!(out, vec![b"one".to_vec()]);
    assert_eq!(s.offset, 6);
    assert_eq!(s.safe_offset, 4);
    assert!(!s.complete);
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(b"o\nthree\n").unwrap();
    let s = read_range(&path, s.offset, &mut state, &budget(100, 100), |l, _, _| {
        out.push(l.to_vec());
        Ok(())
    })
    .unwrap();
    assert_eq!(
        out,
        vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
    );
    assert_eq!(s.offset, s.safe_offset);
    assert!(s.complete);
}
#[test]
fn physical_byte_and_record_budgets_do_not_lose_buffered_tail() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.jsonl");
    fs::write(&path, b"one\ntwo\nthree\n").unwrap();
    let mut state = ReadState::default();
    let mut out = Vec::new();
    let a = read_range(&path, 0, &mut state, &budget(100, 1), |l, _, _| {
        out.push(l.to_vec());
        Ok(())
    })
    .unwrap();
    assert_eq!(a.offset, 4);
    assert_eq!(a.bytes_read, 14);
    assert!(a.hit_budget);
    let b = read_range(&path, a.offset, &mut state, &budget(2, 100), |l, _, _| {
        out.push(l.to_vec());
        Ok(())
    })
    .unwrap();
    assert_eq!(b.bytes_read, 2);
    assert_eq!(b.offset, 6);
    assert_eq!(b.safe_offset, 4);
    read_range(&path, b.offset, &mut state, &budget(100, 100), |l, _, _| {
        out.push(l.to_vec());
        Ok(())
    })
    .unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(out[1], b"two");
}
#[test]
fn oversize_record_is_drained_across_polls_then_recovers() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.jsonl");
    let mut file = fs::File::create(&path).unwrap();
    file.write_all(&vec![b'x'; MAX_RECORD + 5]).unwrap();
    file.write_all(b"\nok\n").unwrap();
    let mut state = ReadState::default();
    let mut out = Vec::new();
    loop {
        let offset = state.next_offset;
        let s = read_range(
            &path,
            offset,
            &mut state,
            &budget(777_777, 100),
            |l, _, _| {
                out.push(l.to_vec());
                Ok(())
            },
        )
        .unwrap();
        assert!(state.retained_bytes() <= MAX_RECORD);
        assert!(s.bytes_read <= 777_777);
        if s.complete {
            break;
        }
    }
    assert_eq!(out, vec![b"ok".to_vec()]);
}
#[test]
fn tail_snapshot_never_adopts_file_size_past_incomplete_last_line() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.jsonl");
    fs::write(&path, b"old\nnew\nincomplete").unwrap();
    let (records, s) = read_tail_page(&path, 100, None, &budget(100, 100)).unwrap();
    assert_eq!(s.safe_offset, 8);
    assert_eq!(s.snapshot_end, 18);
    assert!(s.tail_page_ready);
    assert!(!s.complete);
    assert_eq!(
        records.iter().map(|r| r.line.clone()).collect::<Vec<_>>(),
        vec![b"new".to_vec(), b"old".to_vec()]
    );
    let (records, s) = read_tail_page(&path, 1, Some(8), &budget(100, 100)).unwrap();
    assert_eq!(records[0].start, 4);
    assert_eq!(s.offset, 4);
    assert!(s.hit_budget);
}
#[test]
fn tail_page_window_drops_partial_front_record() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.jsonl");
    fs::write(&path, b"long-record\nnew\n").unwrap();
    let (records, s) = read_tail_page(&path, 100, None, &budget(8, 100)).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].line, b"new");
    assert_eq!(records[0].start, 12);
    assert!(s.hit_budget);
}
#[test]
fn tail_budget_interruption_does_not_commit_partial_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.jsonl");
    fs::write(&path, vec![b'x'; READ_BUFFER * 3]).unwrap();
    let start = Instant::now();
    let ticks = Arc::new(AtomicUsize::new(0));
    let counter = ticks.clone();
    let b = ReadBudget {
        max_bytes: READ_BUFFER * 3,
        max_records: 100,
        deadline: start + Duration::from_millis(2),
        clock: Arc::new(move || {
            start + Duration::from_millis(counter.fetch_add(1, Ordering::SeqCst) as u64)
        }),
    };
    let (records, s) = read_tail_page(&path, 100, None, &b).unwrap();
    assert!(records.is_empty());
    assert!(s.hit_budget);
    assert!(!s.tail_page_ready);
    assert_eq!(s.offset, (READ_BUFFER * 3) as u64);
    assert_eq!(s.safe_offset, 0);
}

use many_ai_cli::approval::transcript::parser::*;
fn parse(format: ProviderFormat, lines: &[&str]) -> ParseState {
    let mut state = ParseState::default();
    state.begin_batch();
    for line in lines {
        state.parse_record(format, line.as_bytes());
    }
    state
}
#[test]
fn claude_tools_results_and_partial_batch_identity() {
    let mut state = parse(
        ProviderFormat::Claude,
        &[
            r#"{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"role":"assistant","content":[{"type":"text","text":"Working"},{"type":"thinking","thinking":"Inspect first"},{"type":"tool_use","id":"t1","name":"Read","input":{"path":"/synthetic/a","n":1}}]}}"#,
        ],
    );
    let first = state.output_messages();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].message_id, "tool:t1");
    assert_eq!(first[0].tools[0].input, r#"{"n":1,"path":"/synthetic/a"}"#);
    assert_eq!(state.pending_count(), 1);
    state.begin_batch();
    state.parse_record(ProviderFormat::Claude,br#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"result"}]}]}}"#);
    let update = state.output_messages();
    assert_eq!(update.len(), 1);
    assert_eq!(update[0].message_id, "tool:t1");
    assert_eq!(update[0].tools[0].result, "result");
    assert_eq!(state.pending_count(), 0);
}
#[test]
fn sidechain_and_synthetic_turn_filter_and_command_code_envelope() {
    let state = parse(
        ProviderFormat::Claude,
        &[
            r#"{"type":"user","message":{"content":"<system-reminder>injected"}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":"private reasoning"}}"#,
        ],
    );
    let out = state.output_messages();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, "sidechain");
    assert_eq!(out[0].text, "");
    assert_eq!(out[0].thinking, vec!["private reasoning"]);
    let state = parse(
        ProviderFormat::CommandCode,
        &[
            r#"{"type":"session","message":{"role":"assistant","content":"ignored"}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":"visible"}}"#,
        ],
    );
    assert_eq!(state.output_messages()[0].text, "visible");
}
#[test]
fn codex_metadata_filters_user_injection_and_preserves_classification() {
    let state = parse(
        ProviderFormat::Codex,
        &[
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"injected"},{"type":"input_text","text":"real request"}],"internal_chat_message_metadata_passthrough":{"content_item_kinds":["developer.agents","user.text"]}}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"final","ignored":1e1000}]}}"#,
            r#"{"type":"event_msg","payload":{"type":"agent_message","channel":"analysis","message":"hidden"}}"#,
        ],
    );
    let out = state.output_messages();
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].text, "real request");
    assert_eq!(out[1].presentation, "answer");
    assert_eq!(out[2].presentation, "hidden");
}
#[test]
fn codex_tool_results_work_across_batches_and_raw_null_does_not_fallback() {
    let mut state = parse(
        ProviderFormat::Codex,
        &[
            r#"{"type":"response_item","payload":{"type":"function_call","call_id":"c1","name":"shell","arguments":null,"input":"must not become arguments"}}"#,
        ],
    );
    assert_eq!(state.output_messages()[0].tools[0].input, "");
    state.begin_batch();
    state.parse_record(ProviderFormat::Codex,br#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"done"}}"#);
    assert_eq!(state.output_messages()[0].tools[0].result, "done");
}
#[test]
fn go_field_folding_nested_merge_and_invalid_unicode() {
    let mut state = ParseState::default();
    state.begin_batch();
    state.parse_record(ProviderFormat::Claude,br#"{"TYPE":"assistant","Message":{"role":"assistant","content":"old"},"message":{"CONTENT":"new"}}"#);
    assert_eq!(state.output_messages()[0].text, "new");
    state.begin_batch();
    let mut bytes = br#"{"type":"assistant","message":{"content":"bad "#.to_vec();
    bytes.push(0xff);
    bytes.extend_from_slice(br#" \ud800"}}"#);
    state.parse_record(ProviderFormat::Claude, &bytes);
    assert_eq!(state.output_messages()[0].text, "bad � �");
    state.begin_batch();
    state.parse_record(
        ProviderFormat::Claude,
        br#"{"type":5,"type":"assistant","message":{"content":"invalid earlier type"}}"#,
    );
    assert!(state.output_messages().is_empty());
    state.parse_record(ProviderFormat::Codex,br#"{"type":"event_msg","payload":{"type":"agent_message","message":"old"},"PAYLOAD":{"type":"agent_message","message":"new"}}"#);
    assert_eq!(state.output_messages()[0].text, "new");
}
#[test]
fn transcript_masks_synthetic_secrets_and_bounds_text_tools_pending() {
    let mut state = ParseState::default();
    state.begin_batch();
    let long = "x".repeat(70 * 1024);
    state.parse_record(
        ProviderFormat::Claude,
        serde_json::to_string(&serde_json::json!({"type":"assistant","message":{"content":long}}))
            .unwrap()
            .as_bytes(),
    );
    assert_eq!(state.output_messages()[0].text.len(), 64 * 1024 + 3);
    for n in 0..300 {
        state.parse_record(ProviderFormat::Claude,serde_json::to_string(&serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":format!("t{n}"),"name":"read","input":{"auth_token":"syntheticlongsecret"}}]}})).unwrap().as_bytes());
    }
    assert!(state.output_messages().len() <= 200);
    assert!(state.pending_count() <= 256);
    assert!(state.retained_pending_bytes() <= 4 * 1024 * 1024);
    assert!(
        state.output_messages().last().unwrap().tools[0]
            .input
            .contains("***")
    );
}
#[test]
fn prime_restores_tail_and_never_promotes_partial_line_or_old_completion() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.jsonl");
    let first = r#"{"type":"event_msg","payload":{"type":"agent_message","message":"question?"}}"#;
    let completion = r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"t1","completed_at":1767225600}}"#;
    let partial = r#"{"type":"event_msg","payload":{"type":"user_message","message":"an"#;
    fs::write(&path, format!("{first}\n{completion}\n{partial}")).unwrap();
    let mut state = ParseState::default();
    let out = state
        .read_tail(
            &path,
            ProviderFormat::Codex,
            None,
            &budget(PAGE_BYTES, PAGE_RECORDS),
        )
        .unwrap();
    assert_eq!(out[0].text, "question?");
    assert!(state.last_read.decode_committed);
    let safe = state.last_read.safe_offset;
    assert!(safe < fs::metadata(&path).unwrap().len());
    assert!(state.adopt_prime_cursor());
    assert!(state.take_completions().is_empty());
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"swer\"}}\n")
        .unwrap();
    let next = state
        .read_forward(
            &path,
            ProviderFormat::Codex,
            &budget(READ_BYTES, READ_RECORDS),
        )
        .unwrap();
    assert_eq!(next[0].text, "answer");
    assert_eq!(
        state.read_state.safe_offset,
        fs::metadata(&path).unwrap().len()
    );
}

#[test]
fn codex_turn_activity_survives_prime_without_replaying_completions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.jsonl");
    fs::write(&path, concat!(
        "{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"one\"}}\n",
        "{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:00:10Z\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"one\"}}\n",
        "{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:00:20Z\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"two\"}}\n"
    )).unwrap();
    let mut parser = ParseState::default();
    parser
        .read_tail(
            &path,
            ProviderFormat::Codex,
            None,
            &budget(PAGE_BYTES, PAGE_RECORDS),
        )
        .unwrap();
    assert!(parser.adopt_prime_cursor());
    assert!(parser.take_completions().is_empty());
    assert_eq!(parser.codex_turn_active(), Some(true));
    fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(
        b"{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:00:30Z\",\"payload\":{\"type\":\"turn_aborted\"}}\n"
    ).unwrap();
    parser
        .read_forward(
            &path,
            ProviderFormat::Codex,
            &budget(READ_BYTES, READ_RECORDS),
        )
        .unwrap();
    assert_eq!(parser.codex_turn_active(), Some(false));
    assert!(parser.take_completions().is_empty());
}

fn long_codex_log(prefix: &str) -> String {
    let mut log = prefix.to_owned();
    for n in 0..1024 {
        log.push_str(&format!(
            "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"count\":{n}}}}}\n"
        ));
    }
    log.push_str("{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"visible tail\"}}\n");
    log
}
fn finish_codex_worker_prime(
    state: &mut ParseState,
    path: &std::path::Path,
) -> Vec<many_ai_cli::approval::transcript::parser::AgentChatMessage> {
    let mut previous = 0;
    for _ in 0..16 {
        let messages = state
            .read_prime_tail_file(
                fs::File::open(path).unwrap(),
                ProviderFormat::Codex,
                &budget(PAGE_BYTES, PAGE_RECORDS),
            )
            .unwrap();
        if state.adopt_prime_cursor() {
            return messages;
        }
        assert!(messages.is_empty());
        assert!(
            state.last_read.offset > previous,
            "lifecycle replay must continue instead of restarting at byte zero"
        );
        assert!(state.last_read.records <= READ_RECORDS);
        assert!(state.last_read.bytes_read <= READ_BYTES);
        assert_eq!(
            state.parsed_messages, 0,
            "state-only replay must not build chat messages"
        );
        previous = state.last_read.offset;
    }
    panic!("bounded lifecycle replay did not finish");
}
#[test]
fn worker_prime_recovers_start_outside_chat_tail_without_replaying_cards() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic-long.jsonl");
    fs::write(&path, long_codex_log(concat!(
        "{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"old\"}}\n",
        "{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:00:10Z\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"old\"}}\n",
        "{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:01:00Z\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"current\"}}\n"
    ))).unwrap();
    // The history API must continue returning a bounded page in one call.
    let mut history = ParseState::default();
    assert_eq!(
        history
            .read_tail(
                &path,
                ProviderFormat::Codex,
                None,
                &budget(PAGE_BYTES, PAGE_RECORDS)
            )
            .unwrap()[0]
            .text,
        "visible tail"
    );
    assert_eq!(history.codex_turn_active(), None);
    let mut worker = ParseState::default();
    let messages = finish_codex_worker_prime(&mut worker, &path);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "visible tail");
    assert_eq!(worker.parsed_messages, 1);
    assert_eq!(worker.codex_turn_active(), Some(true));
    assert!(worker.take_completions().is_empty());
    assert!(
        worker
            .read_forward(
                &path,
                ProviderFormat::Codex,
                &budget(READ_BYTES, READ_RECORDS)
            )
            .unwrap()
            .is_empty()
    );
    assert!(worker.take_completions().is_empty());
}
#[test]
fn worker_prime_uses_the_same_turn_identity_and_completion_time_rules_as_live_reads() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic-lifecycle.jsonl");
    for (ending, expected) in [
        (
            "{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:02:00Z\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"old\"}}\n",
            true,
        ),
        (
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"current\",\"completed_at\":1767225720}}\n",
            false,
        ),
        (
            "{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:02:00Z\",\"payload\":{\"type\":\"turn_aborted\",\"turn_id\":\"current\"}}\n",
            false,
        ),
    ] {
        let prefix = format!(
            "{{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:01:00Z\",\"payload\":{{\"type\":\"task_started\",\"turn_id\":\"current\"}}}}\n{ending}"
        );
        fs::write(&path, long_codex_log(&prefix)).unwrap();
        let mut state = ParseState::default();
        finish_codex_worker_prime(&mut state, &path);
        assert_eq!(state.codex_turn_active(), Some(expected));
        assert!(state.take_completions().is_empty());
    }
}
#[test]
fn worker_prime_preserves_partial_line_for_the_live_cursor() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic-partial.jsonl");
    let mut log = long_codex_log(
        "{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:01:00Z\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"current\"}}\n",
    );
    log.push_str("{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:02:00Z\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"curr");
    fs::write(&path, log).unwrap();
    let mut state = ParseState::default();
    finish_codex_worker_prime(&mut state, &path);
    assert_eq!(state.codex_turn_active(), Some(true));
    assert!(state.read_state.safe_offset < fs::metadata(&path).unwrap().len());
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"ent\"}}\n")
        .unwrap();
    state
        .read_forward(
            &path,
            ProviderFormat::Codex,
            &budget(READ_BYTES, READ_RECORDS),
        )
        .unwrap();
    assert_eq!(state.codex_turn_active(), Some(false));
    assert_eq!(state.take_completions().len(), 1);
}
#[test]
fn worker_prime_does_not_scan_or_delay_other_provider_history() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic-anthropic.jsonl");
    for format in [ProviderFormat::Claude, ProviderFormat::CommandCode] {
        let mut log = String::new();
        let kind = if format == ProviderFormat::Claude {
            "assistant"
        } else {
            "message"
        };
        for n in 0..1024 {
            log.push_str(&format!("{{\"type\":\"{kind}\",\"message\":{{\"role\":\"assistant\",\"content\":\"message {n}\"}}}}\n"));
        }
        fs::write(&path, log).unwrap();
        let mut state = ParseState::new(2, PAGE_BYTES);
        let messages = state
            .read_prime_tail_file(
                fs::File::open(&path).unwrap(),
                format,
                &budget(PAGE_BYTES, PAGE_RECORDS),
            )
            .unwrap();
        assert!(state.adopt_prime_cursor());
        assert_eq!(messages.len(), 2);
        assert_eq!(messages.last().unwrap().text, "message 1023");
        assert_eq!(state.codex_turn_active(), None);
    }
}
#[test]
fn worker_prime_waits_for_lifecycle_when_the_chat_snapshot_grows_after_eof() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic-growing.jsonl");
    fs::write(&path,"{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:01:00Z\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"current\"}}\n").unwrap();
    let before = fs::metadata(&path).unwrap().len();
    let append_path = path.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let clock_calls = calls.clone();
    let fixed = Instant::now();
    let racing = ReadBudget {
        max_bytes: PAGE_BYTES,
        max_records: PAGE_RECORDS,
        deadline: fixed + Duration::from_secs(60),
        clock: Arc::new(move || {
            // The fifth budget check follows the lifecycle reader's EOF read,
            // before the separately authorized chat snapshot reads metadata.
            if clock_calls.fetch_add(1, Ordering::SeqCst) == 4 {
                fs::OpenOptions::new().append(true).open(&append_path).unwrap().write_all(b"{\"type\":\"event_msg\",\"timestamp\":\"2026-01-01T00:02:00Z\",\"payload\":{\"type\":\"task_complete\",\"turn_id\":\"current\"}}\n").unwrap();
            }
            fixed
        }),
    };
    let mut state = ParseState::default();
    state
        .read_prime_tail_file(
            fs::File::open(&path).unwrap(),
            ProviderFormat::Codex,
            &racing,
        )
        .unwrap();
    assert!(calls.load(Ordering::SeqCst) >= 5);
    assert!(
        state.last_read.safe_offset > before,
        "the chat snapshot must include the raced append"
    );
    assert!(
        !state.adopt_prime_cursor(),
        "a newer chat cursor cannot skip unreplayed lifecycle events"
    );
    assert!(state.take_completions().is_empty());
    state
        .read_prime_tail_file(
            fs::File::open(&path).unwrap(),
            ProviderFormat::Codex,
            &budget(PAGE_BYTES, PAGE_RECORDS),
        )
        .unwrap();
    assert!(state.adopt_prime_cursor());
    assert_eq!(state.codex_turn_active(), Some(false));
    assert!(state.take_completions().is_empty());
}

#[test]
fn codex_turn_activity_ignores_invalid_time_and_late_abort_of_another_turn() {
    let mut parser = ParseState::default();
    for (kind, id, at) in [
        ("task_started", "one", "invalid"),
        ("task_complete", "one", "invalid"),
    ] {
        parser.parse_record(
            ProviderFormat::Codex,
            serde_json::json!({"type":"event_msg", "timestamp":at,
            "payload":{"type":kind, "turn_id":id}})
            .to_string()
            .as_bytes(),
        );
        assert_eq!(parser.codex_turn_active(), None);
    }
    for (kind, id, at) in [
        ("task_started", "new", "2026-01-01T00:00:10Z"),
        ("task_started", "old", "2026-01-01T00:00:00Z"),
        ("turn_aborted", "old", "2026-01-01T00:00:20Z"),
    ] {
        parser.parse_record(
            ProviderFormat::Codex,
            serde_json::json!({"type":"event_msg", "timestamp":at,
            "payload":{"type":kind, "turn_id":id}})
            .to_string()
            .as_bytes(),
        );
        assert_eq!(parser.codex_turn_active(), Some(true));
    }
}
#[test]
fn incomplete_prime_retries_same_snapshot_without_forward_adoption() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.jsonl");
    fs::write(
        &path,
        b"{\"type\":\"assistant\",\"message\":{\"content\":\"hi\"}}\n",
    )
    .unwrap();
    let now = Instant::now();
    let expired = ReadBudget {
        max_bytes: PAGE_BYTES,
        max_records: PAGE_RECORDS,
        deadline: now,
        clock: Arc::new(move || now),
    };
    let mut state = ParseState::default();
    assert!(
        state
            .read_tail(&path, ProviderFormat::Claude, None, &expired)
            .unwrap()
            .is_empty()
    );
    assert!(!state.adopt_prime_cursor());
    assert_eq!(state.read_state.next_offset, 0);
    assert_eq!(
        state
            .read_tail(
                &path,
                ProviderFormat::Claude,
                None,
                &budget(PAGE_BYTES, PAGE_RECORDS)
            )
            .unwrap()[0]
            .text,
        "hi"
    );
    assert!(state.adopt_prime_cursor());
}

#[test]
fn out_of_range_codex_completion_timestamp_does_not_panic_or_drop_turn_identity() {
    let mut state = parse(
        ProviderFormat::Codex,
        &[
            r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"large","completed_at":1.8446744073709552e22}}"#,
        ],
    );
    let completions = state.take_completions();
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].turn_id, "large");
    assert_eq!(completions[0].at, "");
}
