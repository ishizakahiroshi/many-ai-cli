package hub

import (
	"strings"
	"sync"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
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

	s.submitInput(1, "user input\r")

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
// TestInjectEchoMarker は「末尾」から marker を作ること（先頭ではない）を確認する。
// TUI の入力欄は長文の末尾しか映さないため、先頭を marker にすると必ず空振りする。
func TestInjectEchoMarker(t *testing.T) {
	cases := []struct {
		name   string
		prompt string
		want   string
	}{
		{"tail not head", "You are an orchestration child session.\nRole: review\n", "sion.Role:review"},
		{"short prompt", "fix bug", "fixbug"},
		{"empty", "", ""},
		{"whitespace collapsed", "a b\tc", "abc"},
	}
	for _, c := range cases {
		if got := injectEchoMarker(c.prompt); got != c.want {
			t.Errorf("%s: injectEchoMarker = %q, want %q", c.name, got, c.want)
		}
	}
	long := "You are an orchestration child session.\nRole: x"
	got := injectEchoMarker(long)
	if len([]rune(got)) > 16 {
		t.Errorf("marker length %d exceeds 16 runes", len([]rune(got)))
	}
	if collapsed := collapseWhitespace(long); !strings.HasSuffix(collapsed, got) {
		t.Errorf("marker %q is not the tail of %q", got, collapsed)
	}
}

// TestWaitForInjectEcho は VT バッファ上のエコー検出（折り返し耐性含む）を確認する。
func TestWaitForInjectEcho(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.vt = newVTBuffer(20, 10)
	// 幅 20 で折り返される長さのテキストを書き込む（改行を挟んで折り返しを模す）
	ses.vt.Write([]byte("You are an orchestr\r\nation child session.\r\n"))

	marker := injectEchoMarker("You are an orchestr\nation child session.")
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

// TestWaitForInjectEchoAcceptsPastePlaceholder は、本文が 1 文字も画面へ出ない
// 「貼り付けプレースホルダへ畳まれた」状態を受理として扱うことを確認する。
// claude は複数行の貼り付けを [Pasted text #N +NN lines] に畳むため、これを受理と
// 見なさないと再注入が必ず撃ち切り、同じ指示が複数回届く（2026-08-31 session #14）。
func TestWaitForInjectEchoAcceptsPastePlaceholder(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	ses.vt = newVTBuffer(60, 10)
	ses.vt.Write([]byte("[Pasted text #2 +33 lines]\r\nPress up to edit queued messages\r\n"))

	if !s.waitForInjectEcho(1, "absent-from-screen", 0) {
		t.Errorf("paste placeholder should count as an accepted injection")
	}
}

// TestSubmitInputBypassGateJumpsPendingQueue は初期プロンプト注入が保留キューを
// 追い越して wrapper へ届き、delivered=true を返すことを確認する。
//
// 追い越さないと: ゲートが積んだ 1 件（board 通知等）の後ろに注入が並び、PTY へ 1 バイトも
// 出ないまま呼び出し側のエコー検証が空振りし、再注入のたびに同じ本文が保留へ積み増される。
// ゲート解除時にまとめて flush され、子が同じ指示を複数回受け取る（2026-08-31 session #14）。
func TestSubmitInputBypassGateJumpsPendingQueue(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	ses.initialInjectPending = true
	ses.initialInjectGateAt = time.Now()

	var sent []string
	s.sessionsMu.Lock()
	s.wrappers[1] = &wrapperConn{sendFunc: func(m any) error {
		if msg, ok := m.(proto.Message); ok && msg.Type == "pty_input" {
			sent = append(sent, string(msg.Data))
		}
		return nil
	}}
	s.sessionsMu.Unlock()

	// ゲート中のユーザー入力（本番では board 通知もこの経路）は保留される。
	s.submitInput(1, "queued by gate")
	if delivered := s.submitInputWithGate(1, "queued by gate 2", false); delivered {
		t.Errorf("gated input must report delivered=false")
	}

	if delivered := s.injectRawBypassGate(1, "initial prompt"); !delivered {
		t.Fatalf("bypass injection must report delivered=true")
	}
	if len(sent) != 1 || sent[0] != "initial prompt" {
		t.Errorf("wrapper received %q, want exactly the injected prompt", sent)
	}
	s.sessionsMu.Lock()
	pending := append([]string(nil), s.pendingInput[1]...)
	s.sessionsMu.Unlock()
	if len(pending) != 2 {
		t.Errorf("pendingInput = %q, want the 2 gated inputs untouched", pending)
	}
	for _, p := range pending {
		if p == "initial prompt" {
			t.Errorf("initial prompt must not be queued when it was delivered")
		}
	}
}

// TestSubmitInputBypassGateReportsUndelivered は wrapper が居ないときに
// delivered=false を返すことを確認する。呼び出し側はこれを見て再注入を止める
// （保留された 1 通はゲート解除時の flush が届けるので、送り直すと二重になる）。
func TestSubmitInputBypassGateReportsUndelivered(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	ses.initialInjectPending = true
	ses.initialInjectGateAt = time.Now()

	if delivered := s.injectRawBypassGate(1, "initial prompt"); delivered {
		t.Fatalf("injection without a wrapper must report delivered=false")
	}
	s.sessionsMu.Lock()
	pending := append([]string(nil), s.pendingInput[1]...)
	s.sessionsMu.Unlock()
	if len(pending) != 1 || pending[0] != "initial prompt" {
		t.Errorf("pendingInput = %q, want the injection held exactly once", pending)
	}
}

// TestNotifyBoardSessionSkipsGatedChild は、初期プロンプト注入前の子へ board 通知を
// 送らないことを確認する。送ると保留キューの 1 件目になり、注入より先に届く。
func TestNotifyBoardSessionSkipsGatedChild(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 2, "claude")
	ses.OrchestrationID = "o1"
	ses.ParentSessionID = 1
	ses.initialInjectPending = true
	ses.initialInjectGateAt = time.Now()

	s.notifyBoardSession("o1", 2, "\n[orchestration] board updated by hub: board.md\n")

	s.sessionsMu.Lock()
	pending := len(s.pendingInput[2])
	s.sessionsMu.Unlock()
	if pending != 0 {
		t.Errorf("pendingInput length = %d, want 0 (notice must be dropped while gated)", pending)
	}

	// ゲートが下りた後は従来どおり届く。
	s.sessionsMu.Lock()
	ses.initialInjectPending = false
	s.sessionsMu.Unlock()
	s.notifyBoardSession("o1", 2, "\n[orchestration] board updated by hub: board.md\n")
	s.sessionsMu.Lock()
	pending = len(s.pendingInput[2])
	s.sessionsMu.Unlock()
	if pending == 0 {
		t.Errorf("board notice must be delivered once the gate is cleared")
	}
}
