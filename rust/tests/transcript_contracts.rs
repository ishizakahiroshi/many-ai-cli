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
