package hub

// handoff_turn_summary_test.go: 子 plan
// docs/local/plan_session-handoff-board_c3_intent-layer.md 内部 C3（案 3
// turn-summary）の完了条件を固定する。
//
// - 既定（done-only）ではターンが終わっても何も注入されず、kind=turn_summary
//   が 1 行も増えない
// - intent_mode=turn-summary を選ぶと、ターンの終わりへ要約プロンプトが注入され、
//   マーカーを拾うと kind=turn_summary が 1 行増える
//
// startAICommitMessage 系のテスト（git_commit_test.go）と同じ方針で、
// submitInput の配送メカニズムそのもの（bracketed paste・確定 CR 再送）は
// 別のテストが担保済みの前提とし、ここでは「注入されたか／マーカーを拾えたか」
// だけを固定する。

import (
	"os"
	"strings"
	"testing"

	"many-ai-cli/internal/handoff"
	"many-ai-cli/internal/proto"
)

// fakeInputCapture は s.wrappers[id] に挿す最小のフェイク。pty_input で
// 送られた本文だけを蓄積する。
type fakeInputCapture struct {
	frames []string
}

func (f *fakeInputCapture) conn() *wrapperConn {
	return &wrapperConn{sendFunc: func(m any) error {
		if msg, ok := m.(proto.Message); ok && msg.Type == "pty_input" {
			f.frames = append(f.frames, string(msg.Data))
		}
		return nil
	}}
}

func TestMaybeInjectHandoffTurnSummarySkipsWhenModeIsDefault(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	capture := &fakeInputCapture{}
	s.sessionsMu.Lock()
	s.wrappers[1] = capture.conn()
	s.sessionsMu.Unlock()

	s.maybeInjectHandoffTurnSummary(1, 1)

	if ses.turnSummaryAwait {
		t.Fatal("turnSummaryAwait must stay false when intent_mode is the default (done-only)")
	}
	if len(capture.frames) != 0 {
		t.Fatalf("no prompt should be injected in done-only mode, got %d frame(s)", len(capture.frames))
	}
}

func TestMaybeInjectHandoffTurnSummaryInjectsWhenEnabled(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()
	// submitInput の確定 CR 待ち（本番値は秒オーダー）をテスト用の短い値に
	// 差し替える。fakeInputCapture は PTY 出力の再描画を再現しないため、
	// 確定 CR は「効果を確認できず 1 回だけ再送」の経路を通る（本文 1 +
	// enter 1 + resend 1 = 3 フレーム）。これは submitInput 自体の挙動で、
	// この C の対象ではないので件数を固定値で縛らない。
	s.submitEnter = submitEnterTestTiming()
	s.cfg.Handoff.IntentMode = "turn-summary"
	ses := registerTestSession(s, 1, "claude")
	capture := &fakeInputCapture{}
	s.sessionsMu.Lock()
	s.wrappers[1] = capture.conn()
	s.sessionsMu.Unlock()

	s.maybeInjectHandoffTurnSummary(1, 3)

	if !ses.turnSummaryAwait {
		t.Fatal("turnSummaryAwait must be set once a prompt is injected")
	}
	if ses.turnSummaryTurn != 3 {
		t.Fatalf("turnSummaryTurn = %d, want 3", ses.turnSummaryTurn)
	}
	if len(capture.frames) == 0 {
		t.Fatal("expected at least one injected frame")
	}
	if !strings.Contains(capture.frames[0], handoffTurnSummaryMarkerOpen) {
		t.Fatalf("injected prompt does not ask for the turn-summary marker: %q", capture.frames[0])
	}

	// 既に待ち受け中なら、同じターンへの再呼び出しで二重注入しない。
	sent := len(capture.frames)
	s.maybeInjectHandoffTurnSummary(1, 3)
	if len(capture.frames) != sent {
		t.Fatalf("got %d frame(s) after a duplicate call, want still %d (no pile-up while awaiting)", len(capture.frames), sent)
	}
}

func TestMaybeInjectHandoffTurnSummarySkipsNonAIProvider(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()
	s.cfg.Handoff.IntentMode = "turn-summary"
	ses := registerTestSession(s, 1, "shell")
	capture := &fakeInputCapture{}
	s.sessionsMu.Lock()
	s.wrappers[1] = capture.conn()
	s.sessionsMu.Unlock()

	s.maybeInjectHandoffTurnSummary(1, 1)

	if ses.turnSummaryAwait || len(capture.frames) != 0 {
		t.Fatal("a shell session must never receive a turn-summary prompt")
	}
}

// TestHandleHandoffTurnSummaryChunkWritesIntentRecord is the C3 completion
// criterion's positive case: once the marker arrives, a kind=turn_summary
// line is appended and the await state clears.
func TestHandleHandoffTurnSummaryChunkWritesIntentRecord(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()
	s.submitEnter = submitEnterTestTiming() // 理由は TestMaybeInjectHandoffTurnSummaryInjectsWhenEnabled 参照
	s.cfg.Handoff.IntentMode = "turn-summary"
	ses := registerTestSession(s, 9, "claude")
	capture := &fakeInputCapture{}
	s.sessionsMu.Lock()
	s.wrappers[9] = capture.conn()
	s.sessionsMu.Unlock()

	s.maybeInjectHandoffTurnSummary(9, 5)
	s.handleHandoffTurnSummaryChunk(9, "noise\n"+handoffTurnSummaryMarkerOpen+"files-view.ts の検索バグを直した"+handoffTurnSummaryMarkerClose+"\ntrailing")

	if ses.turnSummaryAwait {
		t.Fatal("turnSummaryAwait must clear once the marker is picked up")
	}
	got := readHandoffRecords(t, 9)
	var found bool
	for _, r := range got {
		if r.Kind == handoff.KindTurnSummary {
			found = true
			if r.Turn != 5 {
				t.Errorf("turn_summary Turn = %d, want 5", r.Turn)
			}
			if r.Text != "files-view.ts の検索バグを直した" {
				t.Errorf("turn_summary Text = %q, want %q", r.Text, "files-view.ts の検索バグを直した")
			}
		}
	}
	if !found {
		t.Fatalf("no kind=turn_summary record written: %+v", got)
	}
}

// TestHandleHandoffTurnSummaryChunkIgnoredWhenNotAwaiting is the C3
// completion criterion's negative case (既定 done-only では kind=turn_summary
// が 1 行も出ない): a chunk arriving for a session that never had a prompt
// injected must not write anything, even if it happens to contain the marker.
func TestHandleHandoffTurnSummaryChunkIgnoredWhenNotAwaiting(t *testing.T) {
	withTempHandoffHome(t)
	s := newTestServer()
	registerTestSession(s, 9, "claude") // intent_mode は既定（done-only）のまま

	s.handleHandoffTurnSummaryChunk(9, handoffTurnSummaryMarkerOpen+"何か"+handoffTurnSummaryMarkerClose)

	// 一度も await していないセッションなので、そもそも看板ファイル自体が作られない
	// （readHandoffRecords は存在前提のヘルパーなので、ここでは直接 stat する）。
	path, err := handoff.PathFor(9)
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("expected no handoff file to be created, stat err = %v", err)
	}
}

func TestExtractHandoffTurnSummaryLine(t *testing.T) {
	t.Run("基本抽出", func(t *testing.T) {
		buf := "noise\n" + handoffTurnSummaryMarkerOpen + "作業を完了した" + handoffTurnSummaryMarkerClose + "\ntrailing"
		line, ok := extractHandoffTurnSummaryLine(buf)
		if !ok || line != "作業を完了した" {
			t.Fatalf("got ok=%v line=%q", ok, line)
		}
	})

	t.Run("複数行に割れても 1 行へ結合する", func(t *testing.T) {
		buf := handoffTurnSummaryMarkerOpen + "\nfiles-view.ts の\n検索バグを直した\n" + handoffTurnSummaryMarkerClose
		line, ok := extractHandoffTurnSummaryLine(buf)
		if !ok || line != "files-view.ts の 検索バグを直した" {
			t.Fatalf("got ok=%v line=%q", ok, line)
		}
	})

	t.Run("close 未到達なら未確定", func(t *testing.T) {
		if _, ok := extractHandoffTurnSummaryLine(handoffTurnSummaryMarkerOpen + "\n途中"); ok {
			t.Fatal("expected ok=false while the close marker is absent")
		}
	})
}
