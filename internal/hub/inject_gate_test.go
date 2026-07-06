package hub

import (
	"sync"
	"testing"
	"time"
)

// TestSessionInjectGated はゲートの有効・期限切れ・解除の判定を確認する。
func TestSessionInjectGated(t *testing.T) {
	now := time.Now()
	ses := &session{initialInjectPending: true, initialInjectGateAt: now}
	if !sessionInjectGated(ses, now) {
		t.Errorf("fresh gate should be active")
	}
	if sessionInjectGated(ses, now.Add(initialInjectGateMaxAge+time.Second)) {
		t.Errorf("expired gate should be inactive (safety valve)")
	}
	ses.initialInjectPending = false
	if sessionInjectGated(ses, now) {
		t.Errorf("cleared gate should be inactive")
	}
}

// TestClearInitialInjectGate はゲート解除でフラグが落ちることを確認する。
func TestClearInitialInjectGate(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.initialInjectPending = true
	ses.initialInjectGateAt = time.Now()

	s.clearInitialInjectGate(1)

	s.sessionsMu.Lock()
	pending := ses.initialInjectPending
	s.sessionsMu.Unlock()
	if pending {
		t.Errorf("initialInjectPending should be false after clearInitialInjectGate")
	}
	// 存在しないセッションでもパニックしない
	s.clearInitialInjectGate(99)
}

// TestSubmitInputGateDefersToPending はゲート中のユーザー入力が pendingInput へ
// 保留されること、bypassGate=true の注入経路は保留判定をスキップすることを確認する。
// wrapper は nil のため bypass 経路も最終的には pending へ落ちるが、ゲート経路との
// 違いは flushPendingInput のゲート中スキップ（下のテスト）とあわせて担保する。
func TestSubmitInputGateDefersToPending(t *testing.T) {
	s := newTestServer()
	ses := &session{ID: 1, State: "standby", inputMu: new(sync.Mutex),
		initialInjectPending: true, initialInjectGateAt: time.Now()}
	s.sessionsMu.Lock()
	s.sessions[1] = ses
	s.sessionsMu.Unlock()

	s.submitInput(nil, 1, "user input\r")

	s.sessionsMu.Lock()
	got := len(s.pendingInput[1])
	s.sessionsMu.Unlock()
	if got != 1 {
		t.Errorf("pendingInput length = %d, want 1 (gated input must be deferred)", got)
	}
}

// TestFlushPendingInputSkipsWhileGated はゲート中の flushPendingInput が
// キューを消費しない（注入前にユーザー入力が流れない）ことを確認する。
func TestFlushPendingInputSkipsWhileGated(t *testing.T) {
	s := newTestServer()
	ses := &session{ID: 1, State: "standby", inputMu: new(sync.Mutex),
		initialInjectPending: true, initialInjectGateAt: time.Now()}
	s.sessionsMu.Lock()
	s.sessions[1] = ses
	s.pendingInput[1] = []string{"queued\r"}
	s.sessionsMu.Unlock()

	s.flushPendingInput(1)

	s.sessionsMu.Lock()
	got := len(s.pendingInput[1])
	s.sessionsMu.Unlock()
	if got != 1 {
		t.Errorf("pendingInput length = %d, want 1 (flush must be skipped while gated)", got)
	}
}

// TestInjectEchoMarker は先頭行抽出・空白除去・長さ制限を確認する。
func TestInjectEchoMarker(t *testing.T) {
	cases := []struct {
		name   string
		prompt string
		want   string
	}{
		{"first line only", "You are an orchestration child session.\nRole: review\n", "Youareanorchestrationchildsessio"},
		{"short prompt", "fix bug", "fixbug"},
		{"empty", "", ""},
		{"whitespace collapsed", "a b\tc", "abc"},
	}
	for _, c := range cases {
		if got := injectEchoMarker(c.prompt); got != c.want {
			t.Errorf("%s: injectEchoMarker = %q, want %q", c.name, got, c.want)
		}
	}
	if got := injectEchoMarker("You are an orchestration child session.\nRole: x"); len([]rune(got)) > 32 {
		t.Errorf("marker length %d exceeds 32 runes", len([]rune(got)))
	}
}

// TestWaitForInjectEcho は VT バッファ上のエコー検出（折り返し耐性含む）を確認する。
func TestWaitForInjectEcho(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.vt = newVTBuffer(20, 10)
	// 幅 20 で折り返される長さのテキストを書き込む（改行を挟んで折り返しを模す）
	ses.vt.Write([]byte("You are an orchestr\r\nation child session.\r\n"))

	marker := injectEchoMarker("You are an orchestration child session.\nRole: x")
	if !s.waitForInjectEcho(1, marker, 200*time.Millisecond) {
		t.Errorf("echo should be detected across wrapped lines")
	}
	if s.waitForInjectEcho(1, "notonscreenanywhere", 0) {
		t.Errorf("absent marker should not be detected")
	}
	if s.waitForInjectEcho(99, "x", 0) {
		t.Errorf("missing session should return false")
	}
	if !s.waitForInjectEcho(1, "", 0) {
		t.Errorf("empty marker should be treated as success")
	}
}
