package hub

// orchestration_inject_submit_test.go: 初期プロンプト注入の「確定まで確かめる」経路。
// docs/local/bugfix_claude-child-initial-prompt-enter-lost_2026-09-07.md
//
// 2026-09-07 の #29 / #32 / #34 では、本文は入力欄に載ったのに確定 CR だけが効かず、
// エコー検証がそれを配送成功と記録した。ここでは Claude Code の入力欄まわりだけを真似た
// 偽 wrapper で「\r が効かない」を再現し、Enter を 1 回だけ送り直すこと・それでも残れば
// 配送失敗として親へ出すこと・成立していれば余計な \r を送らないことを固定する。

import (
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

const testClaudeRule = "──────────────────────────────────────────────────"

// fakeClaudeTUI は Claude Code の入力欄・区切り線・footer だけを描く偽 wrapper。
// enterTakesOn は何回目の "\r" で送信が成立するか。0 は「何回押しても効かない」。
type fakeClaudeTUI struct {
	s            *Server
	id           int
	prompt       string
	enterTakesOn int

	mu     sync.Mutex
	bodies int
	enters int
	frames []string
}

// draw は画面全体を描き直す。transcript は入力欄より上の行、composer は入力欄の中身。
func (f *fakeClaudeTUI) draw(transcript []string, composer string) {
	var b strings.Builder
	b.WriteString("\x1b[2J\x1b[H")
	b.WriteString(" ▐▛███▛█   Claude Code v2.1.263\r\n")
	b.WriteString("▝▜██████▀  Sonnet 5 · Claude Max\r\n")
	b.WriteString("  ▝▝ ▝▝    C:\\work\\repo\r\n")
	for _, line := range transcript {
		b.WriteString(line + "\r\n")
	}
	b.WriteString(testClaudeRule + "\r\n")
	b.WriteString("❯ " + composer + "\r\n")
	b.WriteString(testClaudeRule + "\r\n")
	b.WriteString("$0.0000  Sonnet 5  ↑0 ↓0\r\n")
	b.WriteString("⏵⏵ bypass permissions on (shift+tab to cycle)\r\n")
	f.s.sessionsMu.Lock()
	if cur := f.s.sessions[f.id]; cur != nil && cur.vt != nil {
		cur.vt.Write([]byte(b.String()))
	}
	f.s.sessionsMu.Unlock()
}

func (f *fakeClaudeTUI) conn() *wrapperConn {
	return &wrapperConn{sendFunc: func(m any) error {
		msg, ok := m.(proto.Message)
		if !ok || msg.Type != "pty_input" {
			return nil
		}
		data := string(msg.Data)
		var redraw func()
		f.mu.Lock()
		f.frames = append(f.frames, data)
		switch {
		case strings.Contains(data, bracketedPasteStart):
			// 本文は起動後にまとめて読まれ、入力欄に載る（#32 の 12:57:55 の初回フレーム）。
			f.bodies++
			redraw = func() { f.draw(nil, f.prompt) }
		case data == "\r":
			f.enters++
			if f.enterTakesOn > 0 && f.enters >= f.enterTakesOn {
				// 送信成立: 入力欄は空になり、本文は transcript 側へ `❯ ` 付きで流れる。
				// marker は transcript に残る（＝画面のどこかには居続ける）ので、
				// 「入力欄の中だけを見る」判定でないと成立を検出できない。
				redraw = func() { f.draw([]string{"❯ " + f.prompt, "✻ Thinking…"}, "") }
			}
		}
		f.mu.Unlock()
		if redraw != nil {
			redraw()
		}
		// どの入力にも CLI の再描画（PTY 出力）が続いたことにする。確定 CR の再送判定
		// （confirmOrResendSubmitEnter）が「出力なし」で独自に \r を足すと、この
		// テストで数える \r の回数がぶれるため。#32 でも起動時の描画が同じ役を演じた。
		go func() {
			time.Sleep(5 * time.Millisecond)
			f.s.setTestLastOutput(f.id, time.Now())
		}()
		return nil
	}}
}

func (f *fakeClaudeTUI) counts() (bodies, enters int) {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.bodies, f.enters
}

// newClaudeInjectTestServer は親 #1（wrapper 未接続・pendingInput で通知を受ける）と
// 子 #2（claude・偽 TUI 接続・入力欄は空で footer あり）を持つ Server を返す。
func newClaudeInjectTestServer(t *testing.T, enterTakesOn int) (*Server, *fakeClaudeTUI, string) {
	t.Helper()
	s := newTestServer()
	s.logger = slog.New(slog.NewTextHandler(io.Discard, nil))
	s.submitEnter = submitEnterTestTiming()
	s.injectSubmitConfirm = 150 * time.Millisecond
	registerTestSession(s, 1, "claude")
	child := registerTestSession(s, 2, "claude")
	child.vt = newVTBuffer(130, 40)
	f := &fakeClaudeTUI{
		s:            s,
		id:           2,
		prompt:       "Read docs/plan.md and write your progress file when done.",
		enterTakesOn: enterTakesOn,
	}
	f.draw(nil, "")
	s.sessionsMu.Lock()
	s.wrappers[2] = f.conn()
	s.sessionsMu.Unlock()
	boardPath := filepath.Join(t.TempDir(), "board.md")
	s.registerBoardChild("o1", boardPath, 2, 1, "implementation", time.Now())
	return s, f, boardPath
}

func testBoardChild(t *testing.T, s *Server, boardID string, childID int) orchestrationChild {
	t.Helper()
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	board := s.orchestration.boards[boardID]
	if board == nil || board.Children[childID] == nil {
		t.Fatalf("board child %s/%d not registered", boardID, childID)
	}
	return *board.Children[childID]
}

// TestClaudeComposerText は入力欄の切り出しそのもの。transcript 側の `❯ ` 行と入力欄の
// `❯ ` 行を取り違えないこと、折り返した入力欄を 1 つに繋ぐこと、入力欄が無ければ
// 判定不能を返すことを確認する。
func TestClaudeComposerText(t *testing.T) {
	cases := []struct {
		name   string
		screen []string
		want   string
		wantOK bool
	}{
		{
			name: "prompt still in composer",
			screen: []string{
				"Claude Code v2.1.263",
				testClaudeRule,
				"❯ Read docs/plan.md and write your progress",
				"file when done.",
				testClaudeRule,
				"$0.0000  Sonnet 5  ↑0 ↓0",
				"⏵⏵ bypass permissions on (shift+tab to cycle)",
			},
			want:   "Readdocs/plan.mdandwriteyourprogressfilewhendone.",
			wantOK: true,
		},
		{
			name: "submitted: echo in transcript, composer empty",
			screen: []string{
				"❯ Read docs/plan.md and write your progress file when done.",
				"✻ Thinking…",
				testClaudeRule,
				"❯ ",
				testClaudeRule,
				"⏵⏵ auto mode on (shift+tab to cycle)",
			},
			want:   "",
			wantOK: true,
		},
		{
			name: "footer without bottom rule still terminates the composer",
			screen: []string{
				testClaudeRule,
				"❯ hello",
				"⏵⏵ bypass permissions on (shift+tab to cycle)",
			},
			want:   "hello",
			wantOK: true,
		},
		{
			name:   "no composer on screen",
			screen: []string{"Claude Code v2.1.263", "Sonnet 5"},
			want:   "",
			wantOK: false,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, ok := claudeComposerText(tc.screen)
			if ok != tc.wantOK {
				t.Fatalf("ok = %v, want %v", ok, tc.wantOK)
			}
			if got != tc.want {
				t.Fatalf("text = %q, want %q", got, tc.want)
			}
		})
	}
}

// TestInjectInitialPromptNotify_ClaudeSubmittedOnFirstEnterSendsNoExtraEnter は正常系。
// 確定 CR が 1 回で効いたら、余計な \r を送らず配送成功を記録する。
func TestInjectInitialPromptNotify_ClaudeSubmittedOnFirstEnterSendsNoExtraEnter(t *testing.T) {
	s, f, boardPath := newClaudeInjectTestServer(t, 1)

	s.injectInitialPromptNotify(2, f.prompt, injectNotice{ParentID: 1, BoardPath: boardPath, Role: "implementation"})

	bodies, enters := f.counts()
	if bodies != 1 {
		t.Fatalf("prompt body sent %d times, want 1", bodies)
	}
	if enters != 1 {
		t.Fatalf("\\r sent %d times, want exactly 1 (no resend when the composer cleared)", enters)
	}
	child := testBoardChild(t, s, "o1", 2)
	if child.PromptDeliveredAt.IsZero() || child.PromptFailed {
		t.Fatalf("delivery not recorded: deliveredAt=%v failed=%v", child.PromptDeliveredAt, child.PromptFailed)
	}
	if _, err := os.Stat(boardPath); !os.IsNotExist(err) {
		t.Fatalf("board must not receive a failure note on success (stat err = %v)", err)
	}
}

// TestInjectInitialPromptNotify_ClaudeResendsEnterOnceWhenPromptStaysInComposer は #32 の
// 再現。1 回目の \r が捨てられて本文が入力欄に残ったら、Enter を 1 回だけ送り直し、
// それで成立したら配送成功として記録する（失敗通知は出さない）。
func TestInjectInitialPromptNotify_ClaudeResendsEnterOnceWhenPromptStaysInComposer(t *testing.T) {
	s, f, boardPath := newClaudeInjectTestServer(t, 2)

	s.injectInitialPromptNotify(2, f.prompt, injectNotice{ParentID: 1, BoardPath: boardPath, Role: "implementation"})

	bodies, enters := f.counts()
	if bodies != 1 {
		t.Fatalf("prompt body sent %d times, want 1 (the body must never be re-sent)", bodies)
	}
	if enters != 2 {
		t.Fatalf("\\r sent %d times, want exactly 2 (initial + one resend)", enters)
	}
	child := testBoardChild(t, s, "o1", 2)
	if child.PromptDeliveredAt.IsZero() || child.PromptFailed {
		t.Fatalf("delivery not recorded after the resend: deliveredAt=%v failed=%v", child.PromptDeliveredAt, child.PromptFailed)
	}
	if _, err := os.Stat(boardPath); !os.IsNotExist(err) {
		t.Fatalf("board must not receive a failure note when the resend succeeded (stat err = %v)", err)
	}
}

// TestInjectInitialPromptNotify_ClaudeReportsWhenEnterNeverSubmits は、送り直しても入力欄に
// 残るとき、\r を 2 回で打ち切り、配送失敗として board と親へ出し、起動失敗の見張りが
// 重ねて発火しないよう PromptFailed を立てることを確認する。
func TestInjectInitialPromptNotify_ClaudeReportsWhenEnterNeverSubmits(t *testing.T) {
	s, f, boardPath := newClaudeInjectTestServer(t, 0)

	s.injectInitialPromptNotify(2, f.prompt, injectNotice{ParentID: 1, BoardPath: boardPath, Role: "implementation"})

	bodies, enters := f.counts()
	if bodies != 1 {
		t.Fatalf("prompt body sent %d times, want 1", bodies)
	}
	if enters != 2 {
		t.Fatalf("\\r sent %d times, want exactly 2 (never more than one resend)", enters)
	}
	child := testBoardChild(t, s, "o1", 2)
	if !child.PromptFailed || !child.PromptDeliveredAt.IsZero() {
		t.Fatalf("expected PromptFailed with no delivery time, got failed=%v deliveredAt=%v", child.PromptFailed, child.PromptDeliveredAt)
	}
	data, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatalf("board not written: %v", err)
	}
	for _, want := range []string{"initial prompt NOT delivered", "role=implementation session=2", "still sitting in the child's composer"} {
		if !strings.Contains(string(data), want) {
			t.Errorf("board missing %q: %s", want, data)
		}
	}
	s.sessionsMu.Lock()
	pending := strings.Join(s.pendingInput[1], "")
	s.sessionsMu.Unlock()
	for _, want := range []string{"[MANY-AI-CLI-ORCHESTRATION-ERROR]", "limit=child_input_blocked", "role=implementation id=2"} {
		if !strings.Contains(pending, want) {
			t.Errorf("parent notice missing %q: %q", want, pending)
		}
	}
}

// TestConfirmInitialPromptSubmitted_UnknownProviderKeepsEchoAsDelivered は、入力欄の
// 切り出しが未登録の provider では従来どおりエコー＝配送成功に倒し、\r を送らないことを
// 確認する（codex / opencode 等の挙動を変えないための固定）。
func TestConfirmInitialPromptSubmitted_UnknownProviderKeepsEchoAsDelivered(t *testing.T) {
	s := newTestServer()
	s.logger = slog.New(slog.NewTextHandler(io.Discard, nil))
	s.injectSubmitConfirm = 50 * time.Millisecond
	ses := registerTestSession(s, 2, "grok")
	ses.vt = newVTBuffer(80, 24)
	ses.vt.Write([]byte("> hello there world\r\n"))
	var frames []string
	s.sessionsMu.Lock()
	s.wrappers[2] = &wrapperConn{sendFunc: func(m any) error {
		if msg, ok := m.(proto.Message); ok && msg.Type == "pty_input" {
			frames = append(frames, string(msg.Data))
		}
		return nil
	}}
	s.sessionsMu.Unlock()

	if !s.confirmInitialPromptSubmitted(2, injectEchoMarker("hello there world"), injectNotice{}) {
		t.Fatalf("unknown provider must fall back to echo-as-delivered")
	}
	if len(frames) != 0 {
		t.Fatalf("no Enter must be sent for a provider without a composer extractor, got %q", frames)
	}
}
