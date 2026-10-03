use many_ai_cli::proto::time::Timestamp;
use many_ai_cli::{
    proto::core::*,
    terminal::{input::*, replay::*, vt::*, width::cell_width},
};
use std::time::Duration;

fn partitions(bytes: &[u8], cols: usize, rows: usize) -> Vec<String> {
    let mut whole = VtBuffer::new(cols, rows);
    whole.write(bytes);
    let expected = whole.tail_lines_with_scrollback(0);
    for step in 1..=bytes.len().min(17) {
        let mut chunked = VtBuffer::new(cols, rows);
        for chunk in bytes.chunks(step) {
            chunked.write(chunk);
        }
        assert_eq!(
            chunked.tail_lines_with_scrollback(0),
            expected,
            "step={step}"
        );
        assert_eq!(chunked.cursor(), whole.cursor());
        assert_eq!(chunked.alt_screen(), whole.alt_screen());
    }
    for cut in 0..=bytes.len() {
        let mut chunked = VtBuffer::new(cols, rows);
        chunked.write(&bytes[..cut]);
        chunked.write(&bytes[cut..]);
        assert_eq!(chunked.tail_lines_with_scrollback(0), expected, "cut={cut}");
    }
    expected
}
#[test]
fn terminal_wide_and_zero_width_redraw_match_go() {
    let bytes = format!("{}\x1b[1;1H実行します", "─".repeat(20));
    assert_eq!(
        partitions(bytes.as_bytes(), 20, 3)[0],
        format!("実行します{}", "─".repeat(10))
    );
    assert_eq!(
        partitions("ÁB\u{fe0f}\u{200d}C".as_bytes(), 20, 2)[0],
        "ABC"
    );
    for c in ['✓', '✻', '⏺', '─', '│'] {
        assert_eq!(cell_width(c), 1);
    }
    for c in ['日', '本', '😀', '⌚', '𠀀'] {
        assert_eq!(cell_width(c), 2);
    }
    for c in ['\u{0300}', '\u{1aff}', '\u{e01ef}'] {
        assert_eq!(cell_width(c), 0);
    }
}
#[test]
fn terminal_cursor_save_erase_and_deferred_wrap() {
    assert_eq!(
        partitions(b"hello\nworld\x1b[2;1Hoverwrite\x1b[K", 20, 3),
        vec!["hello", "overwrite", ""]
    );
    assert_eq!(
        partitions(b"a\x1b7\x1b[2;1Hb\x1b8c", 20, 3),
        vec!["ac", "b", ""]
    );
    let mut vt = VtBuffer::new(5, 2);
    vt.write(b"12345");
    assert_eq!(vt.cursor(), (0, 4));
    vt.write(b"6");
    assert_eq!(vt.lines(), vec!["12345", "6"]);
    vt.write(b"\x1b[1;2H\x1b[2XZ");
    assert_eq!(vt.lines()[0], "1Z 45");
}
#[test]
fn terminal_wide_erasure_invalidates_both_halves() {
    assert_eq!(
        partitions("日本\x1b[1;2H\x1b[X".as_bytes(), 10, 1),
        vec!["  本"]
    );
    assert_eq!(partitions("日本\x1b[1;2HZ".as_bytes(), 10, 1), vec![" Z本"]);
    assert_eq!(
        partitions("a日\x1b[1;2H\x1b[X".as_bytes(), 10, 1),
        vec!["a"]
    );
}
#[test]
fn terminal_string_sequences_are_chunk_independent_strengthening() {
    for introducer in [b']', b'P', b'X', b'^', b'_'] {
        for end in [&b"\x1b\\"[..], &b"\x07"[..]] {
            let mut bytes = b"before\x1b".to_vec();
            bytes.push(introducer);
            bytes.extend_from_slice(b"untrusted-hidden");
            bytes.extend_from_slice(end);
            bytes.extend_from_slice(b"after");
            assert_eq!(partitions(&bytes, 40, 2), vec!["beforeafter", ""]);
        }
    }
    assert_eq!(partitions(b"ok\x1b(B!", 20, 1), vec!["ok!"]);
}
#[test]
fn terminal_string_and_escape_caps_recover() {
    for intro in [b']', b'P', b'X', b'^', b'_'] {
        let mut vt = VtBuffer::new(20, 3);
        vt.write(&[27, intro]);
        for chunk in vec![b'x'; MAX_STRING_SKIP_BYTES + 1].chunks(4093) {
            vt.write(chunk);
        }
        assert!(!vt.skipping_string());
        vt.write(b"recovered");
        assert_eq!(vt.lines()[0], "recovered");
    }
    let mut vt = VtBuffer::new(20, 3);
    vt.write(b"\x1b[");
    for _ in 0..100 {
        vt.write(b"1234567890");
        assert!(vt.pending_bytes() <= MAX_ESCAPE_BYTES + 3);
    }
    vt.write(b"\r\nrecovered");
    assert!(vt.lines().iter().any(|s| s == "recovered"));
}
#[test]
fn terminal_resize_scrollback_and_reset_preserve_mode() {
    let mut vt = VtBuffer::new(20, 2);
    vt.write("承認しますか\r\n次の行\r\nさらに\r\n".as_bytes());
    let before = vt.tail_lines_with_scrollback(0);
    assert!(before.iter().any(|s| s == "承認しますか"));
    assert!(!before.join("").contains('\0'));
    vt.resize(20, 2);
    assert_eq!(vt.tail_lines_with_scrollback(0), before);
    vt.resize(20, 3);
    assert_eq!(vt.tail_lines_with_scrollback(0).len(), 3);
    vt.write(b"\x1b[?1049hcontent");
    vt.reset();
    assert!(vt.alt_screen());
    assert_eq!(vt.lines(), vec!["", "", ""]);
    vt.write(b"\x1b[?1049l");
    assert!(!vt.alt_screen());
}
#[test]
fn terminal_invalid_utf8_and_split_valid_unicode() {
    let mut vt = VtBuffer::new(20, 2);
    vt.write(&[0xe3, 0x81]);
    assert_eq!(vt.lines()[0], "");
    vt.write(&[0x82]);
    assert_eq!(vt.lines()[0], "あ");
    assert_eq!(partitions(&[0xf0, 0x28, 0x8c, 0xbc], 20, 1), vec!["�(��"]);
}
#[test]
fn terminal_scrollback_cap_and_blank_dedupe() {
    let mut vt = VtBuffer::new(20, 2);
    for i in 0..800 {
        vt.write(format!("line{i}\r\n").as_bytes());
    }
    assert_eq!(vt.tail_lines_with_scrollback(0).len(), 502);
    let mut vt = VtBuffer::new(20, 2);
    vt.write(b"\r\n\r\n\r\n\r\n");
    assert_eq!(vt.tail_lines_with_scrollback(0).len(), 3);
}
#[test]
fn replay_raw_retention_offsets_and_alt_prefix() {
    let mut replay = ReplayBuffer::new(5);
    replay.append(b"abc");
    replay.append(b"defg");
    assert_eq!(replay.snapshot(), (b"cdefg".to_vec(), 7));
    replay.append(b"0123456789");
    assert_eq!(replay.snapshot(), (b"56789".to_vec(), 17));
    replay.reset();
    assert_eq!(replay.total(), 17);
    let mut replay = ReplayBuffer::default();
    let marker = b"TURN";
    replay.append(marker);
    replay.append(&vec![b'x'; INACTIVE_REPLAY_TAIL + 10]);
    assert_eq!(
        replay.ui_snapshot(false, true, marker).len(),
        ALT_SCREEN_ENTER.len() + marker.len() + INACTIVE_REPLAY_TAIL + 10
    );
    assert!(
        replay
            .ui_snapshot(false, true, marker)
            .starts_with(ALT_SCREEN_ENTER)
    );
    assert_eq!(
        replay.ui_snapshot(false, false, b"ABSENT").len(),
        INACTIVE_REPLAY_TAIL
    );
    assert_eq!(next_replay_epoch(ReplayEpoch(u64::MAX)), ReplayEpoch(1));
}
#[test]
fn input_ack_tracks_connections_and_resends_original_sequence() {
    let mut input = InputState::default();
    let old = WrapperConnectionId(1);
    let new = WrapperConnectionId(2);
    let a = input.reserve(old, b"a".to_vec());
    let b = input.reserve(old, b"b".to_vec());
    assert_eq!(
        input.acknowledge(new, a.seq),
        AckDisposition::WrongConnection
    );
    assert_eq!(input.disconnected(old, false), 2);
    let resend = input.take_resend();
    assert_eq!(resend, vec![a.clone(), b]);
    assert!(input.readmit(new, &a));
    assert_eq!(
        input.acknowledge(old, a.seq),
        AckDisposition::WrongConnection
    );
    assert_eq!(input.acknowledge(new, a.seq), AckDisposition::Removed);
    assert_eq!(
        input.acknowledge(new, a.seq),
        AckDisposition::CapabilityObserved
    );
    assert_eq!(
        input.acknowledge(new, InputSeq(0)),
        AckDisposition::CapabilityObserved
    );
}
#[test]
fn input_old_wrappers_never_resend_and_queues_are_bounded() {
    let mut input = InputState::default();
    let conn = WrapperConnectionId(1);
    input.reserve(conn, b"legacy".to_vec());
    assert_eq!(input.disconnected(conn, false), 0);
    for n in 0..101 {
        input.enqueue(vec![n]);
        input.reserve(conn, vec![n]);
    }
    assert_eq!(input.pending_len(), 100);
    assert_eq!(input.inflight_len(), 100);
    assert_eq!(input.take_pending()[0], vec![1]);
    input.acknowledge(conn, InputSeq(0));
    assert_eq!(input.disconnected(conn, false), 100);
    assert_eq!(input.take_resend().len(), 100);
    assert_eq!(input.reserve(conn, Vec::new()).seq, InputSeq(0));
}
#[test]
fn input_gate_and_paste_failure_boundary() {
    let mut input = InputState::default();
    let now = Timestamp::UNIX_EPOCH;
    input.set_initial_gate(now);
    assert!(input.gated(now + Duration::from_secs(89)));
    assert!(!input.gated(now + Duration::from_secs(90)));
    assert!(input.initial_prompt_phase());
    input.clear_initial_gate();
    assert!(!input.initial_prompt_phase());
    assert_eq!(
        split_bracketed_paste_submit(b"\x1b[200~body\x1b[201~\r"),
        (&b"\x1b[200~body\x1b[201~"[..], &b"\r"[..])
    );
}
#[test]
fn received_unfinished_watermark_does_not_suppress_retry() {
    let p = ProcessedInputState::default();
    p.received(InputSeq(4));
    assert!(!p.already_processed(InputSeq(4)));
    p.mark_processed(InputSeq(3));
    assert!(p.already_processed(InputSeq(3)));
    assert!(!p.already_processed(InputSeq(0)));
    assert_eq!(
        p.watermarks(),
        InputHighWatermarks {
            processed: InputSeq(3),
            received: InputSeq(4)
        }
    );
    p.mark_processed(InputSeq(4));
    assert!(p.already_processed(InputSeq(4)));
}
#[test]
fn priming_order_capacity_and_invalidation() {
    let mut q = PrimingQueue::new(2);
    q.enqueue(1).unwrap();
    q.enqueue(2).unwrap();
    assert_eq!(q.enqueue(3), Err(3));
    assert_eq!(q.finish(), vec![1, 2]);
    assert_eq!(q.enqueue(4), Err(4));
    let mut q = PrimingQueue::new(2);
    q.enqueue(1).unwrap();
    q.invalidate();
    assert!(q.finish().is_empty());
}

#[test]
fn go_oracle_synthetic_snapshots_and_all_byte_partitions() {
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        cols: usize,
        rows: usize,
        bytes: String,
        lines: Vec<String>,
        row: usize,
        col: usize,
        alt: bool,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "fixtures/core/terminal/vt-oracle-21d0bc7.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 200);
    for c in cases {
        let mut vt = VtBuffer::new(c.cols, c.rows);
        vt.write(c.bytes.as_bytes());
        assert_eq!(vt.tail_lines_with_scrollback(0), c.lines, "{}", c.name);
        assert_eq!(vt.cursor(), (c.row, c.col), "{}", c.name);
        assert_eq!(vt.alt_screen(), c.alt, "{}", c.name);
        assert_eq!(
            partitions(c.bytes.as_bytes(), c.cols, c.rows),
            c.lines,
            "{}",
            c.name
        );
    }
}
