package hub

import (
	"bytes"
	"strings"
	"testing"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/proto"
)

func TestUIPrimingQueuesLiveEventsUntilReplayFinishes(t *testing.T) {
	s := newTestServer()
	conn := &websocket.Conn{}
	var sent []string
	uc := newUIConn(conn)
	uc.sendFunc = func(m any) error {
		msg, ok := m.(proto.Message)
		if !ok {
			t.Fatalf("sent message type = %T, want proto.Message", m)
		}
		sent = append(sent, msg.Type)
		if msg.Type == "first-live" {
			// finishUIPriming is draining the first batch here. This event must
			// join the next batch instead of overtaking the first frame.
			s.broadcast(proto.Message{Type: "during-drain"})
		}
		return nil
	}

	s.sessionsMu.Lock()
	uc.priming = true
	s.uis[conn] = uc
	s.sessionsMu.Unlock()

	s.broadcast(proto.Message{Type: "first-live"})
	if len(sent) != 0 {
		t.Fatalf("live event sent before priming completed: %v", sent)
	}

	if !s.finishUIPriming(uc) {
		t.Fatal("finishUIPriming reported a disconnected UI")
	}
	if got, want := append([]string(nil), sent...), []string{"first-live", "during-drain"}; !equalStrings(got, want) {
		t.Fatalf("priming send order = %v, want %v", got, want)
	}

	s.broadcast(proto.Message{Type: "after-priming"})
	if got, want := sent, []string{"first-live", "during-drain", "after-priming"}; !equalStrings(got, want) {
		t.Fatalf("live send order = %v, want %v", got, want)
	}
}

func equalStrings(got, want []string) bool {
	if len(got) != len(want) {
		return false
	}
	for i := range got {
		if got[i] != want[i] {
			return false
		}
	}
	return true
}

// findPTYData は addUIWithHistoryAtEpoch の戻り値から、指定セッションの
// 最初の pty_data メッセージを返す。
func findPTYData(items []proto.Message, sessionID int) (proto.Message, bool) {
	for _, m := range items {
		if m.Type == "pty_data" && m.SessionID == sessionID {
			return m, true
		}
	}
	return proto.Message{}, false
}

// bugfix_alt-screen-mode-lost-on-ui-replay_2026-09-12.md C1:
// セッションが代替画面バッファ（ESC[?1049h）中なら、UI 接続時の replay の
// 先頭へ ESC[?1049h を前置する。前置しないとブラウザの xterm が通常画面の
// まま復帰し、上へスクロールできなくなる。
func TestAddUIWithHistoryPrependsAltScreenEnterWhenInAltScreen(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	body := []byte("some replayed screen output")
	s.sessionsMu.Lock()
	ses.ptyBuf = append([]byte(nil), body...)
	ses.altScreen = true
	s.sessionsMu.Unlock()

	uiWS := &websocket.Conn{}
	_, items := s.addUIWithHistoryAtEpoch(uiWS, ses.ID, 0)

	msg, ok := findPTYData(items, ses.ID)
	if !ok {
		t.Fatal("no pty_data message for session")
	}
	want := append([]byte(altScreenEnterSeq), body...)
	if !bytes.Equal(msg.Data, want) {
		t.Fatalf("replay data = %q, want ESC[?1049h-prefixed %q", msg.Data, want)
	}
}

// 通常画面のセッションでは何も前置しない（ESC[?1049l を足す必要は無い —
// 通常画面が既定のため）。
func TestAddUIWithHistoryDoesNotPrependAltScreenEnterForNormalScreen(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 1, "claude")
	body := []byte("some replayed screen output")
	s.sessionsMu.Lock()
	ses.ptyBuf = append([]byte(nil), body...)
	ses.altScreen = false
	s.sessionsMu.Unlock()

	uiWS := &websocket.Conn{}
	_, items := s.addUIWithHistoryAtEpoch(uiWS, ses.ID, 0)

	msg, ok := findPTYData(items, ses.ID)
	if !ok {
		t.Fatal("no pty_data message for session")
	}
	if !bytes.Equal(msg.Data, body) {
		t.Fatalf("replay data = %q, want unprefixed %q", msg.Data, body)
	}
	if bytes.Contains(msg.Data, []byte(altScreenEnterSeq)) {
		t.Fatalf("replay data unexpectedly contains ESC[?1049h: %q", msg.Data)
	}
}

// 非アクティブセッションは replayTailForNonActive で窓を切り出すが、前置は
// その窓カットの "後" に行う（切り出し前に足すと窓計算に混ざる）。
func TestAddUIWithHistoryPrependsAltScreenEnterAfterNonActiveWindowCut(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 2, "claude")
	// replayTailForNonActive を超える長さにして窓カットを実際に踏ませる。
	// マーカーを含めない（含めると「最後のマーカー位置まで遡る」経路に入り、
	// 期待する窓の長さが変わってしまう）。
	filler := strings.Repeat("x", replayTailForNonActive+4096)
	s.sessionsMu.Lock()
	ses.ptyBuf = []byte(filler)
	ses.altScreen = true
	s.sessionsMu.Unlock()

	uiWS := &websocket.Conn{}
	// activeSessionID をこのセッションと別の ID にして非アクティブ窓カットへ入れる。
	_, items := s.addUIWithHistoryAtEpoch(uiWS, ses.ID+1, 0)

	msg, ok := findPTYData(items, ses.ID)
	if !ok {
		t.Fatal("no pty_data message for session")
	}
	wantTail := filler[len(filler)-replayTailForNonActive:]
	want := append([]byte(altScreenEnterSeq), []byte(wantTail)...)
	if !bytes.Equal(msg.Data, want) {
		t.Fatalf("replay data length = %d, want %d (ESC[?1049h + %d-byte window)", len(msg.Data), len(want), replayTailForNonActive)
	}
}
