package hub

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

// 子 plan: docs/local/plan_v0.9-release-readiness_c10_answer-fixes.md C2（判断保留ログ #5 の (a)〜(c)）。
// 起動引数で指示を渡した子が、まだ指示を受け取らずに起動画面で待っている間の扱い。
// 画面・指示の本文・設定はすべて合成。

// claudeExternalImportsScreen は Claude Code の外部 CLAUDE.md の読み込みの確認
// （usage_probe_test.go の同じ画面と同じ文言）。フォルダの信頼の確認ではない起動画面。
const claudeExternalImportsScreen = "Allow external CLAUDE.md file imports?\r\n" +
	"This project's CLAUDE.md imports files outside the current working directory.\r\n\r\n" +
	"❯ 1. Yes, allow external imports\r\n  2. No, disable external imports\r\n\r\nEnter to confirm · Esc to cancel\r\n"

// codexUnknownStartupScreen は Hub が文言を登録していない起動画面の代わり（合成）。
// 入力欄の合図（Ask Codex to do anything）が無い。
const codexUnknownStartupScreen = "Synthetic onboarding step\r\n\r\n" +
	"› 1. Continue\r\n  2. Quit\r\n\r\nenter continue · esc quit\r\n"

// claudeREPLQuotingTrustWords は、指示を受け取って動き始めた Claude の画面。指示の本文が
// フォルダの信頼の確認の文言を引用しているので、トランスクリプトの欄にその文言が出る。
// 入力欄と footer（shift+tab to cycle）がある。
const claudeREPLQuotingTrustWords = "❯ Check how the CLI words its startup question: \"Is this a project you created or one you trust?\" " +
	"and \"Yes, I trust this folder\". Report the exact text.\r\n\r\n" +
	"● Reading the source.\r\n\r\n" +
	"──────────────────────────────\r\n❯ \r\n──────────────────────────────\r\n" +
	"⏵⏵ bypass permissions on (shift+tab to cycle)\r\n"

// codexREPLQuotingTrustWords は codex の同じ場面（入力欄の合図は Ask Codex to do anything）。
const codexREPLQuotingTrustWords = "› Find where the text \"Trust this folder? Codex can read, edit, and run files here\" is defined.\r\n\r\n" +
	"• Searching the repository.\r\n\r\n" +
	"› Ask Codex to do anything\r\n"

func startupScreenBoard(t *testing.T, boardPath string) string {
	t.Helper()
	data, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

func startupScreenParentEvents(s *Server) []boardEvent {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	return append([]boardEvent(nil), s.orchestration.boards[launchPromptBoardID].PendingEvents...)
}

// (a) 信頼確認ではない起動画面（Hub が文言を知っているもの）でも、親と board へ 1 回知らせる。
func TestStartupScreenNoticeCoversScreensOtherThanFolderTrust(t *testing.T) {
	s, child, boardPath := launchArgChildServer(t, "claude")
	s.markChildPromptViaLaunchArg(launchPromptBoardID, 7)
	child.vt = newVTBuffer(130, 35)
	child.vt.Write([]byte(claudeExternalImportsScreen))
	child.lastOutputAt = time.Now().Add(-time.Minute)

	s.checkOrchestrationChildTimers(launchPromptBoardID, time.Now(), config.OrchestrationConfig{})
	s.checkOrchestrationChildTimers(launchPromptBoardID, time.Now(), config.OrchestrationConfig{})

	board := startupScreenBoard(t, boardPath)
	if n := strings.Count(board, "child waiting on a startup screen"); n != 1 {
		t.Fatalf("board carries the startup-screen notice %d times, want 1:\n%s", n, board)
	}
	for _, want := range []string{"AllowexternalCLAUDE.mdfileimports?", "do not send them again"} {
		if !strings.Contains(board, want) {
			t.Errorf("notice is missing %q:\n%s", want, board)
		}
	}
	if strings.Contains(board, "folder-trust prompt") {
		t.Errorf("a screen that is not the folder-trust question was reported as one:\n%s", board)
	}
	found := 0
	for _, e := range startupScreenParentEvents(s) {
		if e.SessionID == 1 && strings.Contains(e.Text, "startup screen") {
			found++
		}
	}
	if found != 1 {
		t.Fatalf("parent received the notice %d times, want 1", found)
	}
}

// (a) Hub が文言を知らない起動画面でも、入力欄が出ないまま起動から十分たち、画面が
// 止まっていれば知らせる。起動直後や、画面がまだ動いている間は知らせない。
func TestStartupScreenNoticeCoversScreensHubDoesNotRecognise(t *testing.T) {
	for _, tc := range []struct {
		name       string
		spawnedAgo time.Duration
		quietFor   time.Duration
		want       bool
	}{
		{"settled on an unknown screen", 10 * time.Minute, time.Minute, true},
		{"just started", 5 * time.Second, time.Minute, false},
		{"screen still moving", 10 * time.Minute, time.Second, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s, child, boardPath := launchArgChildServer(t, "codex")
			s.markChildPromptViaLaunchArg(launchPromptBoardID, 7)
			s.orchestration.mu.Lock()
			s.orchestration.boards[launchPromptBoardID].Children[7].SpawnedAt = time.Now().Add(-tc.spawnedAgo)
			s.orchestration.mu.Unlock()
			child.vt = newVTBuffer(130, 35)
			child.vt.Write([]byte(codexUnknownStartupScreen))
			child.lastOutputAt = time.Now().Add(-tc.quietFor)

			s.checkOrchestrationChildTimers(launchPromptBoardID, time.Now(), config.OrchestrationConfig{})

			board := startupScreenBoard(t, boardPath)
			got := strings.Contains(board, "child waiting on a startup screen")
			if got != tc.want {
				t.Fatalf("notice sent = %v, want %v:\n%s", got, tc.want, board)
			}
		})
	}
}

// (a) 指示の本文が信頼確認の文言を引用していても、入力欄が出ている（＝指示を受け取って
// 動いている）子については知らせない。トランスクリプトの読み取りがまだ追いついていない
// 間（PromptDeliveredAt がゼロ）でも同じ。
func TestStartupScreenNoticeIgnoresTrustWordsInTheInstruction(t *testing.T) {
	for _, tc := range []struct{ provider, screen string }{
		{"claude", claudeREPLQuotingTrustWords},
		{"codex", codexREPLQuotingTrustWords},
	} {
		t.Run(tc.provider, func(t *testing.T) {
			s, child, boardPath := launchArgChildServer(t, tc.provider)
			s.markChildPromptViaLaunchArg(launchPromptBoardID, 7)
			child.vt = newVTBuffer(130, 35)
			child.vt.Write([]byte(tc.screen))
			child.lastOutputAt = time.Now().Add(-time.Minute)

			s.checkOrchestrationChildTimers(launchPromptBoardID, time.Now(), config.OrchestrationConfig{})

			board := startupScreenBoard(t, boardPath)
			if strings.Contains(board, "child waiting on") {
				t.Fatalf("a child at its input box was reported as waiting on a startup screen:\n%s", board)
			}
			if n := len(startupScreenParentEvents(s)); n != 0 {
				t.Fatalf("parent received %d events, want 0", n)
			}
		})
	}
}

// (b) まだ指示を受け取っていない子へ、orchestrate send は 1 バイトも打ち込まない。
// 送った側（conductor）には、打ち込まなかったことと理由がエラーで返る。受け取った後は
// 今までどおり届く。
func TestSendChildDoesNotTypeIntoAChildBeforeItTakesItsLaunchInstruction(t *testing.T) {
	s, child, boardPath := launchArgChildServer(t, "claude")
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	s.markChildPromptViaLaunchArg(launchPromptBoardID, 7)
	s.sessionsMu.Lock()
	s.sessions[1].BoardPath = boardPath
	s.sessionsMu.Unlock()
	child.ParentSessionID = 1
	child.Role = "impl"
	child.vt = newVTBuffer(130, 35)
	child.vt.Write([]byte(claudeFolderTrustScreen))
	var framesMu sync.Mutex
	var frames []string
	typedFrames := func() []string {
		framesMu.Lock()
		defer framesMu.Unlock()
		return append([]string(nil), frames...)
	}
	s.sessionsMu.Lock()
	s.wrappers[7] = &wrapperConn{sendFunc: func(m any) error {
		if msg, ok := m.(proto.Message); ok && msg.Type == "pty_input" {
			framesMu.Lock()
			frames = append(frames, string(msg.Data))
			framesMu.Unlock()
		}
		return nil
	}}
	s.sessionsMu.Unlock()

	rr := httptest.NewRecorder()
	s.handleSessionAPI(rr, orchestrationRequest(http.MethodPost, "/api/sessions/1/send-child", sendChildRequest{Role: "impl", Text: "synthetic follow-up"}))
	if rr.Code != http.StatusConflict {
		t.Fatalf("status = %d, want 409, body=%s", rr.Code, rr.Body.String())
	}
	for _, want := range []string{"child_not_ready", "has not taken its launch instructions", "nothing was typed"} {
		if !strings.Contains(rr.Body.String(), want) {
			t.Errorf("response is missing %q: %s", want, rr.Body.String())
		}
	}
	typed := typedFrames()
	s.sessionsMu.Lock()
	queued := append([]string(nil), s.pendingInput[7]...)
	s.sessionsMu.Unlock()
	if len(typed) != 0 || len(queued) != 0 {
		t.Fatalf("typed %q / queued %q into a child on its startup screen", typed, queued)
	}
	if strings.Contains(startupScreenBoard(t, boardPath), "synthetic follow-up") {
		t.Fatal("an instruction that was not typed was recorded on the board as sent")
	}

	// 受け取った後（トランスクリプトに最初の発言が出た後）は今までどおり届く。
	s.noteLaunchPromptAccepted(7, []agentChatMessage{{Role: "user", Text: "synthetic instruction"}}, time.Now())
	rr = httptest.NewRecorder()
	s.handleSessionAPI(rr, orchestrationRequest(http.MethodPost, "/api/sessions/1/send-child", sendChildRequest{Role: "impl", Text: "synthetic follow-up"}))
	if rr.Code != http.StatusOK {
		t.Fatalf("after the instruction was taken: status = %d, want 200, body=%s", rr.Code, rr.Body.String())
	}
	if typed = typedFrames(); !strings.Contains(strings.Join(typed, ""), "synthetic follow-up") {
		t.Fatalf("after the instruction was taken the follow-up was not typed: %q", typed)
	}
}

// (b) relay の催促も、まだ指示を受け取っていない子へは打ち込まない。打ち込まなかったことは
// board に残り、催促の 1 回の枠は使わない（受け取った後に黙れば催促できる）。
func TestRelayReminderIsNotTypedIntoAChildBeforeItTakesItsLaunchInstruction(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	h.s.markChildPromptViaLaunchArg(id, impl)
	old := time.Now().Add(-10 * time.Minute)
	h.s.orchestration.mu.Lock()
	c := h.s.orchestration.boards[id].Children[impl]
	c.SpawnedAt, c.LastBoardWrite = old, old
	h.s.orchestration.mu.Unlock()
	ses := h.s.sessions[impl]
	ses.vt = newVTBuffer(130, 35)
	ses.vt.Write([]byte(codexFolderTrustScreen))
	ses.lastOutputAt = old
	cfg := config.OrchestrationConfig{IdleDoneThresholdSec: 60}

	h.s.checkOrchestrationChildTimers(id, time.Now(), cfg)

	if len(h.injects) != 0 {
		t.Fatalf("injects = %+v, want none while the child has not taken its launch instructions", h.injects)
	}
	data, err := os.ReadFile(h.run(id).boardPath)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), fmt.Sprintf("reminder not typed: implementation #%d", impl)) {
		t.Fatalf("board does not say the reminder was held back:\n%s", data)
	}
	run := h.run(id)
	run.mu.Lock()
	nudged := run.nudged[impl]
	run.mu.Unlock()
	if nudged {
		t.Fatal("the one reminder per work unit was used up without being typed")
	}

	// 受け取って動いた後に再び黙ったら、催促は今までどおり打ち込まれる。
	h.s.noteLaunchPromptAccepted(impl, []agentChatMessage{{Role: "user", Text: "synthetic instruction"}}, time.Now())
	ses.vt = newVTBuffer(130, 35)
	ses.vt.Write([]byte("› Ask Codex to do anything\r\n"))
	h.s.orchestration.mu.Lock()
	delete(h.s.orchestration.boards[id].IdleWarned, impl)
	h.s.orchestration.mu.Unlock()
	h.s.checkOrchestrationChildTimers(id, time.Now(), cfg)
	if len(h.injects) != 1 || h.injects[0].id != impl {
		t.Fatalf("injects = %+v, want one reminder after the instruction was taken", h.injects)
	}
}

// (c) 起動画面で待っている子は timeout に数えない。TimeoutRespawn が有効でも 2 体目を
// 起動しない（同じ作業フォルダで同じ指示を 2 体が実行しない）。受け取った後は今までどおり
// timeout の対象になる。
func TestChildOnStartupScreenIsNotTimedOutNorRespawned(t *testing.T) {
	s, child, _ := launchArgChildServer(t, "claude")
	s.markChildPromptViaLaunchArg(launchPromptBoardID, 7)
	old := time.Now().Add(-30 * time.Minute)
	// 直す前のコードでは respawnTimedOutChild の goroutine が走る。model を不正な値にして
	// おき、spawnWrappedSession がプロセスを起こす前にエラーで返るようにする（テストで
	// 本物の CLI を起動しないための安全網。直した後はそこまで行かない）。
	s.setChildRestartData(launchPromptBoardID, 7, spawnWrappedSpec{Provider: "claude", Model: "-synthetic-invalid"}, "synthetic instruction", "", 0)
	s.orchestration.mu.Lock()
	c := s.orchestration.boards[launchPromptBoardID].Children[7]
	c.SpawnedAt, c.LastBoardWrite = old, old
	s.orchestration.mu.Unlock()
	child.vt = newVTBuffer(130, 35)
	child.vt.Write([]byte(claudeFolderTrustScreen))
	child.lastOutputAt = old
	cfg := config.OrchestrationConfig{ChildTimeoutSeconds: 60, TimeoutRespawn: true, MaxTimeoutRespawns: 1}

	s.checkOrchestrationChildTimers(launchPromptBoardID, time.Now(), cfg)

	s.orchestration.mu.Lock()
	b := s.orchestration.boards[launchPromptBoardID]
	timedOut, retries := b.TimedOut[7], b.Children[7].TimeoutRetries
	s.orchestration.mu.Unlock()
	if timedOut {
		t.Fatal("a child waiting on its startup screen was marked timed out")
	}
	if retries != 0 {
		t.Fatalf("TimeoutRetries = %d, want 0 (no second child for the same instruction)", retries)
	}
	s.sessionsMu.Lock()
	parentInput := strings.Join(s.pendingInput[1], "")
	state := s.sessions[7].State
	s.sessionsMu.Unlock()
	if strings.Contains(parentInput, "limit=timeout") {
		t.Fatalf("parent was told the waiting child timed out: %q", parentInput)
	}
	if state == "timeout" {
		t.Fatal("the waiting child's session was marked timeout")
	}

	// 受け取って動いた後に黙り込んだ子は、今までどおり timeout になる。
	s.noteLaunchPromptAccepted(7, []agentChatMessage{{Role: "user", Text: "synthetic instruction"}}, old)
	child.vt = newVTBuffer(130, 35)
	child.vt.Write([]byte(claudeREPLQuotingTrustWords))
	s.checkOrchestrationChildTimers(launchPromptBoardID, time.Now(), config.OrchestrationConfig{ChildTimeoutSeconds: 60})
	s.orchestration.mu.Lock()
	timedOut = s.orchestration.boards[launchPromptBoardID].TimedOut[7]
	s.orchestration.mu.Unlock()
	if !timedOut {
		t.Fatal("once the instruction was taken, the usual timeout must apply again")
	}
}
