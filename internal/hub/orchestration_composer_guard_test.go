package hub

// orchestration_composer_guard_test.go: waitForComposerReady / containsAnySignal の
// 分岐カバレッジと、reportInjectFailure / injectInitialPromptNotify の統合テスト。
// plan_spawn-orchestration-backlog-closeout_c1_codex-inject-guard.md C2/C3。
//
// 「モーダルが安定して出続けたら deadline を待たず即座に composerBlocked を返す」という
// C1 の修正を検証する意図で書いている。経過時間のアサーションを外すと、C1 実装前の
// コード（maxWait 満了まで待ってから composerBlocked を返す）でも値の一致だけは通って
// しまい、無意味なテストになる。

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

// TestWaitForComposerReady_NoSignalForUnknownProvider は、providerComposerSignals に
// 登録の無い provider（claude）では、ポーリングせず即座に composerNoSignal を返すことを
// 確認する。
func TestWaitForComposerReady_NoSignalForUnknownProvider(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	ses.vt = newVTBuffer(80, 24)

	start := time.Now()
	res, blocker := s.waitForComposerReady(1, 5*time.Second)
	elapsed := time.Since(start)

	if res != composerNoSignal {
		t.Fatalf("result = %v, want composerNoSignal", res)
	}
	if blocker != "" {
		t.Fatalf("blocker = %q, want empty", blocker)
	}
	if elapsed >= time.Second {
		t.Fatalf("took %v, want to return immediately (no polling for a provider with no registered signals)", elapsed)
	}
}

// TestWaitForComposerReady_MissingSessionReturnsUnknown は、未登録の sessionID に対して
// composerUnknown を返すことを確認する。
func TestWaitForComposerReady_MissingSessionReturnsUnknown(t *testing.T) {
	s := newTestServer()

	res, blocker := s.waitForComposerReady(999, 5*time.Second)

	if res != composerUnknown {
		t.Fatalf("result = %v, want composerUnknown", res)
	}
	if blocker != "" {
		t.Fatalf("blocker = %q, want empty", blocker)
	}
}

// TestWaitForComposerReady_ReadySignalStableReturnsReady は、陽性シグナルが最初から
// 安定して出ている画面に対し、orchestrationComposerStableFor 程度で composerReady を
// 返すこと（maxWait まで待たないこと）を確認する。
func TestWaitForComposerReady_ReadySignalStableReturnsReady(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.vt = newVTBuffer(80, 24)
	ses.vt.Write([]byte("Ask Codex to do anything\r\n? for shortcuts\r\n"))

	const maxWait = 5 * time.Second
	start := time.Now()
	res, blocker := s.waitForComposerReady(1, maxWait)
	elapsed := time.Since(start)

	if res != composerReady {
		t.Fatalf("result = %v, want composerReady", res)
	}
	if blocker != "" {
		t.Fatalf("blocker = %q, want empty", blocker)
	}
	if elapsed < orchestrationComposerStableFor {
		t.Fatalf("took %v, want >= stableFor %v (must confirm stability before declaring ready)", elapsed, orchestrationComposerStableFor)
	}
	if elapsed > 5*orchestrationComposerStableFor {
		t.Fatalf("took %v, want close to stableFor %v, not maxWait %v", elapsed, orchestrationComposerStableFor, maxWait)
	}
}

// TestWaitForComposerReady_BlockedSignalReturnsPromptly は C1 の修正そのものを検証する。
// モーダルが起動直後から一度も揺らがず出ている画面に対し、maxWait（ここでは 5 秒）の
// 満了を待たず orchestrationComposerStableFor 程度で composerBlocked を返すことを確認する。
// C1 が無い状態（blocked 側に即時リターン分岐が無い実装）でこのテストを走らせると、
// elapsed がほぼ maxWait に張り付いて down 側のアサーションで落ちる。
func TestWaitForComposerReady_BlockedSignalReturnsPromptly(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.vt = newVTBuffer(80, 24)
	ses.vt.Write([]byte("Do you trust the contents of this directory?\r\nPress enter to continue\r\n"))

	const maxWait = 5 * time.Second
	start := time.Now()
	res, blocker := s.waitForComposerReady(1, maxWait)
	elapsed := time.Since(start)

	if res != composerBlocked {
		t.Fatalf("result = %v, want composerBlocked", res)
	}
	if blocker == "" {
		t.Fatalf("blocker is empty, want a detected modal signal")
	}
	if elapsed >= maxWait {
		t.Fatalf("took %v, want well under maxWait %v (C1 regression: blocked must return promptly once stable)", elapsed, maxWait)
	}
	if elapsed > 5*orchestrationComposerStableFor {
		t.Fatalf("took %v, want close to stableFor %v, not maxWait %v", elapsed, orchestrationComposerStableFor, maxWait)
	}
}

// TestWaitForComposerReady_NeitherSignalTimesOutAsUnknown は、ready にも blocked にも
// 一致しない画面が続く場合、deadline 経由でのみ composerUnknown を返すことを確認する
// （この分岐に即時リターンは無い。安定した確定シグナルが無いのだから待つほかない）。
func TestWaitForComposerReady_NeitherSignalTimesOutAsUnknown(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "codex")
	ses.vt = newVTBuffer(80, 24)
	ses.vt.Write([]byte("Booting...\r\n"))

	const maxWait = 200 * time.Millisecond
	start := time.Now()
	res, blocker := s.waitForComposerReady(1, maxWait)
	elapsed := time.Since(start)

	if res != composerUnknown {
		t.Fatalf("result = %v, want composerUnknown", res)
	}
	if blocker != "" {
		t.Fatalf("blocker = %q, want empty", blocker)
	}
	if elapsed < maxWait {
		t.Fatalf("took %v, want >= maxWait %v (this branch only resolves via deadline)", elapsed, maxWait)
	}
}

// TestContainsAnySignal は containsAnySignal 単体の分岐（一致あり・一致なし・
// シグナル一覧が空）を確認する。
func TestContainsAnySignal(t *testing.T) {
	if !containsAnySignal("fooBARbaz", []string{"qux", "BAR"}) {
		t.Errorf("expected true when one of several signals matches")
	}
	if containsAnySignal("foobaz", []string{"qux", "BAR"}) {
		t.Errorf("expected false when no signal matches")
	}
	if containsAnySignal("anything", nil) {
		t.Errorf("expected false for an empty signal list")
	}
}

// TestReportInjectFailure_AppendsBoardAndNotifiesParent は、reportInjectFailure が
// board への追記と親セッションへの ORCHESTRATION-ERROR 注入の両方を行うことを確認する。
func TestReportInjectFailure_AppendsBoardAndNotifiesParent(t *testing.T) {
	s := newTestServer()
	registerTestSession(s, 1, "codex") // 親。wrapper 未接続なので pendingInput で受ける。
	boardPath := filepath.Join(t.TempDir(), "board.md")

	s.reportInjectFailure(2, injectNotice{ParentID: 1, BoardPath: boardPath, Role: "implementation"}, "detail text")

	data, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatalf("board file not written: %v", err)
	}
	board := string(data)
	for _, want := range []string{"initial prompt NOT delivered", "role=implementation session=2", "detail text"} {
		if !strings.Contains(board, want) {
			t.Errorf("board missing %q: %s", want, board)
		}
	}

	s.sessionsMu.Lock()
	pending := append([]string(nil), s.pendingInput[1]...)
	s.sessionsMu.Unlock()
	joined := strings.Join(pending, "")
	for _, want := range []string{"[MANY-AI-CLI-ORCHESTRATION-ERROR]", "limit=child_input_blocked", "role=implementation id=2: detail text"} {
		if !strings.Contains(joined, want) {
			t.Errorf("parent pendingInput missing %q: %q", want, pending)
		}
	}
}

// TestReportInjectFailure_EmptyNoticeIsNoop は、conductor 自身への注入相当の空 notice で
// 呼ばれたとき、board も親通知も一切発生しないことを確認する。
func TestReportInjectFailure_EmptyNoticeIsNoop(t *testing.T) {
	s := newTestServer()
	boardPath := filepath.Join(t.TempDir(), "board.md")

	s.reportInjectFailure(2, injectNotice{}, "detail text")

	if _, err := os.Stat(boardPath); !os.IsNotExist(err) {
		t.Errorf("board file should not be created for an empty notice, stat err = %v", err)
	}
	s.sessionsMu.Lock()
	total := 0
	for _, q := range s.pendingInput {
		total += len(q)
	}
	s.sessionsMu.Unlock()
	if total != 0 {
		t.Errorf("pendingInput should be untouched for an empty notice, got %d entries", total)
	}
}

// TestInjectInitialPromptNotify_BlockedModalNeverInjects は C1+C2 の統合確認。
// 子 TUI がモーダルに固着している間は一切注入せず、board と親の両方へ失敗を通知し、
// かつ orchestrationComposerMaxWait（45秒）に張り付かず数秒以内に返ることを確認する。
func TestInjectInitialPromptNotify_BlockedModalNeverInjects(t *testing.T) {
	s := newTestServer()
	registerTestSession(s, 1, "codex") // 親。wrapper 未接続。
	child := registerTestSession(s, 2, "codex")
	child.vt = newVTBuffer(80, 24)
	child.vt.Write([]byte("Do you trust the contents of this directory?\r\nPress enter to continue\r\n"))

	var frames []string
	s.sessionsMu.Lock()
	s.wrappers[2] = &wrapperConn{sendFunc: func(m any) error {
		if msg, ok := m.(proto.Message); ok && msg.Type == "pty_input" {
			frames = append(frames, string(msg.Data))
		}
		return nil
	}}
	s.sessionsMu.Unlock()

	boardPath := filepath.Join(t.TempDir(), "board.md")
	notice := injectNotice{ParentID: 1, BoardPath: boardPath, Role: "implementation"}

	start := time.Now()
	// goroutine 越しにしない: injectInitialPromptNotify はテスト対象そのものなので同期呼び出しする。
	s.injectInitialPromptNotify(2, "some prompt", notice)
	elapsed := time.Since(start)

	if len(frames) != 0 {
		t.Fatalf("wrapper received pty_input while the child was on a modal: %q", frames)
	}
	if elapsed >= 5*time.Second {
		t.Fatalf("injectInitialPromptNotify took %v, want a few seconds at most (not stuck near maxWait=%v)", elapsed, orchestrationComposerMaxWait)
	}

	data, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatalf("board file not written: %v", err)
	}
	if !strings.Contains(string(data), "initial prompt NOT delivered") {
		t.Errorf("board missing failure notice: %s", data)
	}

	s.sessionsMu.Lock()
	pending := append([]string(nil), s.pendingInput[1]...)
	s.sessionsMu.Unlock()
	if !strings.Contains(strings.Join(pending, ""), "[MANY-AI-CLI-ORCHESTRATION-ERROR]") {
		t.Errorf("parent did not receive an orchestration error notice: %q", pending)
	}
}

// TestInjectInitialPromptNotify_ReadySignalInjectsOnce は、入力欄が最初から安定して
// 出ている通常経路で、初回プロンプトが 1 回だけ届く（再試行しない）ことを確認する。
//
// fake wrapper は受け取った pty_input をそのまま vt へエコーする。これが無いと
// waitForInjectEcho が orchestrationInjectEchoWait（20秒）待ってから諦め、
// injectInitialPromptNotify が 2 回目の attempt に入ってテストが遅く・不安定になる。
//
// 確定 \r 送出後の再送判定（confirmOrResendSubmitEnter）が働くよう、"\r" を受け取った
// 直後は少し遅らせて lastOutputAt を更新する（submit_enter_test.go の
// streamTestOutput / TestSubmitEnterDoesNotResendWhenOutputFollows と同じ手法）。
// これが無いと確定 \r 自体は毎回 1 回だけ再送されうるが、それは本文の二重送信では
// ないので下のアサーション（本文の出現回数）には影響しない。
func TestInjectInitialPromptNotify_ReadySignalInjectsOnce(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 2, "codex")
	ses.vt = newVTBuffer(80, 24)
	ses.vt.Write([]byte("Ask Codex to do anything\r\n? for shortcuts\r\n"))

	var frames []string
	s.sessionsMu.Lock()
	s.wrappers[2] = &wrapperConn{sendFunc: func(m any) error {
		msg, ok := m.(proto.Message)
		if !ok || msg.Type != "pty_input" {
			return nil
		}
		data := string(msg.Data)
		frames = append(frames, data)
		s.sessionsMu.Lock()
		if cur := s.sessions[2]; cur != nil && cur.vt != nil {
			cur.vt.Write(msg.Data)
		}
		s.sessionsMu.Unlock()
		if data == "\r" {
			go func() {
				time.Sleep(5 * time.Millisecond)
				s.sessionsMu.Lock()
				if cur := s.sessions[2]; cur != nil {
					cur.lastOutputAt = time.Now()
				}
				s.sessionsMu.Unlock()
			}()
		}
		return nil
	}}
	s.sessionsMu.Unlock()

	s.injectInitialPromptNotify(2, "hello", injectNotice{})

	bodyFrames := 0
	for _, f := range frames {
		if strings.Contains(f, "hello") {
			bodyFrames++
		}
	}
	if bodyFrames != 1 {
		t.Fatalf("prompt body was sent %d times, want exactly 1 (no retry): %q", bodyFrames, frames)
	}
}
