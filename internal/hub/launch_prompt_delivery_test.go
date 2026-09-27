package hub

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
)

// 子 plan: docs/local/plan_child-launch-prompt-and-trust_c4_launch-arg-prompt.md 内部 C4。
// 起動引数で指示を渡した子は、打ち込みのエコーが無いので、トランスクリプトの最初の
// user メッセージで「受け取った」とする。受け取る前に信頼確認で待っている子は、
// 起動失敗と見なさず、親と board へ 1 回だけ知らせる。画面・トランスクリプトは合成。

const launchPromptBoardID = "board-launch-arg"

func launchArgChildServer(t *testing.T, childProvider string) (*Server, *session, string) {
	t.Helper()
	s := newTestServer()
	boardPath := filepath.Join(t.TempDir(), "board.md")
	if err := os.WriteFile(boardPath, []byte("# Orchestration (synthetic)\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	registerTestSession(s, 1, "claude")
	child := registerTestSession(s, 7, childProvider)
	s.registerBoardSession(launchPromptBoardID, boardPath, 1, "conductor")
	s.registerBoardChild(launchPromptBoardID, boardPath, 7, 1, "impl", time.Now().Add(-10*time.Minute))
	return s, child, boardPath
}

func launchArgChild(t *testing.T, s *Server) *orchestrationChild {
	t.Helper()
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	child := s.orchestration.boards[launchPromptBoardID].Children[7]
	if child == nil {
		t.Fatal("child not registered on the board")
	}
	return child
}

// 引数経路の子が standby のまま猶予を超えても、トランスクリプトに発言が無い間は
// 起動失敗と判定しない。user の発言が現れたら PromptDeliveredAt が入り、以降は
// これまでどおりの見張りが効く。
func TestLaunchArgChildIsNotAStartupFailureBeforeItAcceptsTheInstruction(t *testing.T) {
	s, _, _ := launchArgChildServer(t, "claude")
	s.markChildPromptViaLaunchArg(launchPromptBoardID, 7)
	s.orchestration.mu.Lock()
	s.orchestration.boards[launchPromptBoardID].Children[7].StandbySince = time.Now().Add(-5 * time.Minute)
	s.orchestration.mu.Unlock()
	cfg := config.OrchestrationConfig{ChildStartupGraceSeconds: 60}

	if s.childStartupFailed(7, time.Now(), cfg) {
		t.Fatal("a child waiting (standby) before accepting its launch instruction was judged a failed startup")
	}
	s.noteLaunchPromptAccepted(7, []agentChatMessage{{Role: "assistant", Text: "synthetic greeting"}}, time.Now())
	if !launchArgChild(t, s).PromptDeliveredAt.IsZero() {
		t.Fatal("an assistant-only batch was taken as the instruction being accepted")
	}

	at := time.Now()
	s.noteLaunchPromptAccepted(7, []agentChatMessage{{Role: "user", Text: "synthetic instruction"}, {Role: "assistant", Text: "ok"}}, at)
	if got := launchArgChild(t, s).PromptDeliveredAt; !got.Equal(at) {
		t.Fatalf("PromptDeliveredAt = %v, want %v", got, at)
	}
	later := at.Add(time.Hour)
	s.noteLaunchPromptAccepted(7, []agentChatMessage{{Role: "user", Text: "a later turn"}}, later)
	if got := launchArgChild(t, s).PromptDeliveredAt; !got.Equal(at) {
		t.Fatalf("a later user turn moved PromptDeliveredAt to %v", got)
	}
	if !s.childStartupFailed(7, time.Now(), cfg) {
		t.Fatal("once the instruction was accepted, the usual startup watchdog must apply again")
	}
}

// 打ち込みの子（印が無い）は、トランスクリプトの発言では PromptDeliveredAt を
// 動かさない（エコー観測が正本のまま）。
func TestTypedChildDeliveryIsNotTakenFromTheTranscript(t *testing.T) {
	s, _, _ := launchArgChildServer(t, "copilot")
	s.noteLaunchPromptAccepted(7, []agentChatMessage{{Role: "user", Text: "synthetic"}}, time.Now())
	if !launchArgChild(t, s).PromptDeliveredAt.IsZero() {
		t.Fatal("a typed child's delivery was stamped from the transcript")
	}
}

// 信頼確認の画面が出ている引数経路の子について、知らせは親と board に 1 回だけ出る
// （2 回目の見回りでは出ない）。
func TestLaunchArgChildWaitingOnFolderTrustIsReportedOnce(t *testing.T) {
	s, child, boardPath := launchArgChildServer(t, "claude")
	s.markChildPromptViaLaunchArg(launchPromptBoardID, 7)
	child.vt = newVTBuffer(130, 35)
	child.vt.Write([]byte(claudeFolderTrustScreen))
	cfg := config.OrchestrationConfig{}

	s.checkOrchestrationChildTimers(launchPromptBoardID, time.Now(), cfg)
	s.checkOrchestrationChildTimers(launchPromptBoardID, time.Now(), cfg)

	board, err := os.ReadFile(boardPath)
	if err != nil {
		t.Fatal(err)
	}
	if n := strings.Count(string(board), "child waiting on its folder-trust prompt"); n != 1 {
		t.Fatalf("board carries the notice %d times, want 1:\n%s", n, board)
	}
	if !strings.Contains(string(board), "do not send them again") {
		t.Errorf("the notice does not tell the conductor to leave the instructions alone:\n%s", board)
	}
	s.orchestration.mu.Lock()
	events := append([]boardEvent(nil), s.orchestration.boards[launchPromptBoardID].PendingEvents...)
	s.orchestration.mu.Unlock()
	found := 0
	for _, e := range events {
		if e.SessionID == 1 && strings.Contains(e.Text, "folder-trust prompt") {
			found++
		}
	}
	if found != 1 {
		t.Fatalf("parent received the notice %d times, want 1 (events %+v)", found, events)
	}
}

// 受け取り済みの子と、打ち込みの子には知らせない（打ち込みの子は注入側の
// composerBlocked が別の文面で知らせる）。
func TestFolderTrustNoticeSkipsAcceptedAndTypedChildren(t *testing.T) {
	for _, tc := range []struct {
		name     string
		viaArg   bool
		accepted bool
	}{
		{"accepted launch-arg child", true, true},
		{"typed child", false, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s, child, boardPath := launchArgChildServer(t, "claude")
			if tc.viaArg {
				s.markChildPromptViaLaunchArg(launchPromptBoardID, 7)
			}
			if tc.accepted {
				s.noteLaunchPromptAccepted(7, []agentChatMessage{{Role: "user", Text: "synthetic"}}, time.Now())
			}
			child.vt = newVTBuffer(130, 35)
			child.vt.Write([]byte(claudeFolderTrustScreen))
			s.checkOrchestrationChildTimers(launchPromptBoardID, time.Now(), config.OrchestrationConfig{})
			board, err := os.ReadFile(boardPath)
			if err != nil {
				t.Fatal(err)
			}
			if strings.Contains(string(board), "folder-trust prompt") {
				t.Fatalf("notice was sent:\n%s", board)
			}
		})
	}
}
