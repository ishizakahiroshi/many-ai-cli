package hub

import (
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// 抑止イベントのたびに、原因調査に必要な 3 点（Hub が描いた結果 / 入力の生バイト /
// 描画時の寸法）が 1 レコードとして残ること。
func TestDumpCorruptApprovalBlockWritesRecord(t *testing.T) {
	s := newTestServer()
	dir := t.TempDir()
	s.cfg.Hub.LogDir = dir
	s.cfg.Log.SessionEnabled = true

	ses := registerTestSession(s, 1, "claude")
	ses.vt = newVTBuffer(120, 40)
	ses.lastCols, ses.lastRows = 120, 40
	ses.ptyBuf = []byte("前の再描画\x1b[2Kここまでが入力\n")

	// 罫線が本文へ重なった形（box_rule）。
	corrupt := &approvalMarkerBlock{Block: markerBlock(
		"最初の質問ですか？",
		"1. 案 A ────────────",
		"2. 案 B",
	)}
	corrupt.Sig = approvalMarkerSignature(corrupt.Block)

	now := time.Date(2026, 8, 4, 7, 47, 15, 0, time.UTC)
	if s.maybeBroadcastApprovalMarker(1, corrupt, now) {
		t.Fatal("corrupt marker was broadcast")
	}

	path := filepath.Join(dir, "approval-corrupt", "2026-08-04.jsonl")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("dump file not written: %v", err)
	}
	lines := strings.Split(strings.TrimSpace(string(data)), "\n")
	if len(lines) != 1 {
		t.Fatalf("dump lines = %d, want 1", len(lines))
	}
	var rec approvalCorruptRecord
	if err := json.Unmarshal([]byte(lines[0]), &rec); err != nil {
		t.Fatalf("unmarshal dump: %v", err)
	}
	if rec.Reason != "box_rule" {
		t.Fatalf("reason = %q, want box_rule", rec.Reason)
	}
	if rec.SessionID != 1 || rec.Provider != "claude" || rec.Sig != corrupt.Sig {
		t.Fatalf("identity mismatch: %+v", rec)
	}
	if rec.VTCols != 120 || rec.VTRows != 40 || rec.LastSentCols != 120 || rec.LastSentRows != 40 {
		t.Fatalf("size fields = %d x %d / %d x %d, want 120x40 both", rec.VTCols, rec.VTRows, rec.LastSentCols, rec.LastSentRows)
	}
	// 生バイトが原形のまま復元できること（これが崩れると再現実験ができない）。
	block, err := base64.StdEncoding.DecodeString(rec.BlockB64)
	if err != nil {
		t.Fatalf("decode block: %v", err)
	}
	if string(block) != corrupt.Block {
		t.Fatalf("block roundtrip mismatch:\n got %q\nwant %q", block, corrupt.Block)
	}
	tail, err := base64.StdEncoding.DecodeString(rec.PTYTailB64)
	if err != nil {
		t.Fatalf("decode pty tail: %v", err)
	}
	if string(tail) != string(ses.ptyBuf) {
		t.Fatalf("pty tail roundtrip mismatch:\n got %q\nwant %q", tail, ses.ptyBuf)
	}
	if rec.PTYTailBytes != len(ses.ptyBuf) {
		t.Fatalf("pty_tail_bytes = %d, want %d", rec.PTYTailBytes, len(ses.ptyBuf))
	}
	if rec.Masked {
		t.Fatal("masked = true for a block with no secrets")
	}
	// resize 未発生のセッションは -1（0 と区別する。0 は「resize 直後」を意味する）。
	if rec.SinceResizeMS != -1 {
		t.Fatalf("ms_since_last_resize = %d, want -1 (no resize happened)", rec.SinceResizeMS)
	}
}

// 直近 resize からの経過が記録されること。ミラーは resize で reflow しないので、
// この値が小さいレコードに破損が偏るかどうかが原因切り分けの主要な手がかりになる。
func TestDumpCorruptApprovalBlockRecordsTimeSinceResize(t *testing.T) {
	s := newTestServer()
	dir := t.TempDir()
	s.cfg.Hub.LogDir = dir
	s.cfg.Log.SessionEnabled = true
	ses := registerTestSession(s, 1, "claude")
	ses.vt = newVTBuffer(100, 30)

	now := time.Date(2026, 8, 4, 7, 47, 15, 0, time.UTC)
	// 検出の 500ms 前に resize があった状態を作る。
	ses.vtResizeDebounceUntil = now.Add(-500 * time.Millisecond).Add(vtResizeDebounce)

	blk := &approvalMarkerBlock{Block: markerBlock("最初の質問ですか？", "1. 案 A ────────────", "2. 案 B")}
	blk.Sig = approvalMarkerSignature(blk.Block)
	if s.maybeBroadcastApprovalMarker(1, blk, now) {
		t.Fatal("corrupt marker was broadcast")
	}

	data, err := os.ReadFile(filepath.Join(dir, "approval-corrupt", "2026-08-04.jsonl"))
	if err != nil {
		t.Fatalf("dump file not written: %v", err)
	}
	var rec approvalCorruptRecord
	if err := json.Unmarshal([]byte(strings.TrimSpace(string(data))), &rec); err != nil {
		t.Fatalf("unmarshal dump: %v", err)
	}
	if rec.SinceResizeMS != 500 {
		t.Fatalf("ms_since_last_resize = %d, want 500", rec.SinceResizeMS)
	}
}

// ptyBuf はリングバッファで呼び出し後も書き換わる。ロック内で複製していないと
// ダンプの入力側が後続チャンクに汚染されて再現実験に使えなくなる。
func TestSnapshotApprovalCorruptCopiesPTYBuf(t *testing.T) {
	ses := &session{ptyBuf: []byte("original")}
	snap := snapshotApprovalCorrupt(ses, time.Now())
	ses.ptyBuf[0] = 'X'
	if string(snap.ptyTail) != "original" {
		t.Fatalf("snapshot aliases ptyBuf: %q", snap.ptyTail)
	}
}

// 抑止が連続しても告知スロットル（30s）と同じ間隔でしか記録しない＝ログが膨らまない。
func TestDumpCorruptApprovalBlockThrottledWithNotify(t *testing.T) {
	s := newTestServer()
	dir := t.TempDir()
	s.cfg.Hub.LogDir = dir
	s.cfg.Log.SessionEnabled = true
	ses := registerTestSession(s, 1, "claude")
	ses.vt = newVTBuffer(80, 24)

	now := time.Date(2026, 8, 4, 7, 47, 15, 0, time.UTC)
	for i, inner := range []string{"1. 案 A ────────────", "1. 案 A' ────────────"} {
		blk := &approvalMarkerBlock{Block: markerBlock("最初の質問ですか？", inner, "2. 案 B")}
		blk.Sig = approvalMarkerSignature(blk.Block)
		// sig は毎回変わるが、30 秒以内なので 2 件目は記録されない。
		if s.maybeBroadcastApprovalMarker(1, blk, now.Add(time.Duration(i)*time.Second)) {
			t.Fatalf("corrupt marker %d was broadcast", i)
		}
	}

	data, err := os.ReadFile(filepath.Join(dir, "approval-corrupt", "2026-08-04.jsonl"))
	if err != nil {
		t.Fatalf("dump file not written: %v", err)
	}
	if got := len(strings.Split(strings.TrimSpace(string(data)), "\n")); got != 1 {
		t.Fatalf("dump lines = %d, want 1 (throttled)", got)
	}
}

// 保持日数を超えた日次ファイルだけが消えること。
func TestPruneApprovalCorruptDumps(t *testing.T) {
	dir := t.TempDir()
	now := time.Date(2026, 8, 4, 0, 0, 0, 0, time.UTC)
	old := filepath.Join(dir, "2026-07-20.jsonl")
	recent := filepath.Join(dir, "2026-08-03.jsonl")
	other := filepath.Join(dir, "notes.txt")
	for _, p := range []string{old, recent, other} {
		if err := os.WriteFile(p, []byte("x\n"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	pruneApprovalCorruptDumps(dir, now)
	if _, err := os.Stat(old); !os.IsNotExist(err) {
		t.Fatal("old dump was not pruned")
	}
	for _, p := range []string{recent, other} {
		if _, err := os.Stat(p); err != nil {
			t.Fatalf("%s was pruned unexpectedly: %v", filepath.Base(p), err)
		}
	}
}

// セッションログが opt-in されていない既定状態では、破損ブロックのダンプを
// 一切書かないこと。MaskSecrets は通しているが保存対象は PTY の生バイト列なので、
// v0.6.0 リリース前監査 A-01 と同じ「ログ opt-in と無関係に入力由来のバイトを
// 永続化しない」原則に従う。
func TestDumpCorruptApprovalBlockSkippedWhenSessionLogDisabled(t *testing.T) {
	s := newTestServer()
	dir := t.TempDir()
	s.cfg.Hub.LogDir = dir
	s.cfg.Log.SessionEnabled = false

	ses := registerTestSession(s, 1, "claude")
	ses.vt = newVTBuffer(120, 40)
	ses.lastCols, ses.lastRows = 120, 40
	ses.ptyBuf = []byte("前の再描画\x1b[2Kここまでが入力\n")

	corrupt := &approvalMarkerBlock{Block: markerBlock(
		"最初の質問ですか？",
		"1. 案 A ────────────",
		"2. 案 B",
	)}
	corrupt.Sig = approvalMarkerSignature(corrupt.Block)

	now := time.Date(2026, 8, 4, 7, 47, 15, 0, time.UTC)
	if s.maybeBroadcastApprovalMarker(1, corrupt, now) {
		t.Fatal("corrupt marker was broadcast")
	}

	// 抑止そのものは opt-in と無関係に効き続ける。書き出しだけが止まる。
	if entries, err := os.ReadDir(filepath.Join(dir, "approval-corrupt")); err == nil && len(entries) > 0 {
		t.Fatalf("dump written while session logging is disabled: %d entries", len(entries))
	}
}
