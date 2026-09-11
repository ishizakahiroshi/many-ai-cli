package hub

import (
	"net/http/httptest"
	"net/url"
	"strconv"
	"strings"
	"testing"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/proto"
)

// TestReattachPreservesAltScreenAcrossPTYSizeChange は
// bugfix_alt-screen-mode-lost-on-ui-replay_2026-09-12.md の追加分。
//
// reattachLoop は PTY サイズが変わった reattach では既存の vtBuffer を温存できず、
// wrapper 側 64KB リング（replay）だけからミラーを作り直す。その replay には
// セッション開始直後の一度きりの ESC[?1049h が含まれないことが多いため、
// 何もしなければ新しいセッションの altScreen は常に false から始まってしまう。
// wrapper の意図的な再接続（出力キュー溢れ時など）は珍しくないので、これが起きる
// たびに「Hub は通常画面だと思っているが CLI は代替画面のまま」という、この
// bugfix が塞ごうとした食い違いが再発する。
//
// 直前セッションの altScreen を種として引き継ぐことで、サイズが変わっても
// 代替画面状態が保たれることを確認する。
func TestReattachPreservesAltScreenAcrossPTYSizeChange(t *testing.T) {
	s := newTestServer()
	tokenStatusbarDisabled := false
	s.cfg.UserPrefs.TokenStatusbar.Enabled = &tokenStatusbarDisabled

	ses := registerTestSession(s, 1, "codex")
	ses.CWD = "/tmp"
	s.sessionsMu.Lock()
	ses.lastCols, ses.lastRows = 80, 24
	ses.vt = newVTBuffer(80, 24)
	ses.vt.Write([]byte("\x1b[?1049h"))
	ses.altScreen = true
	s.sessionsMu.Unlock()

	server := httptest.NewServer(websocket.Handler(func(conn *websocket.Conn) {
		var req proto.Message
		if err := websocket.JSON.Receive(conn, &req); err != nil {
			return
		}
		s.reattachLoop(conn, req)
	}))
	defer server.Close()

	parsed, err := url.Parse(server.URL)
	if err != nil {
		t.Fatalf("parse test server URL: %v", err)
	}
	port, err := strconv.Atoi(parsed.Port())
	if err != nil {
		t.Fatalf("parse test server port: %v", err)
	}
	conn, err := websocket.Dial("ws"+strings.TrimPrefix(server.URL, "http"), "", "http://127.0.0.1:"+strconv.Itoa(port))
	if err != nil {
		t.Fatalf("dial test server: %v", err)
	}
	defer conn.Close()

	if err := websocket.JSON.Send(conn, proto.Message{
		Type:       "reattach",
		SessionID:  1,
		Provider:   "codex",
		CWD:        "/tmp",
		PID:        999,
		Cols:       120, // 80x24 -> 120x40: PTY サイズを変えて、既存ミラー温存分岐を避ける
		Rows:       40,
		UsageProbe: true,
	}); err != nil {
		t.Fatalf("send reattach request: %v", err)
	}

	var ack proto.Message
	if err := websocket.JSON.Receive(conn, &ack); err != nil {
		t.Fatalf("receive reattach_ack: %v", err)
	}
	if ack.Type != "reattach_ack" || ack.SessionID != 1 {
		t.Fatalf("first frame = %+v, want reattach_ack for session 1", ack)
	}

	s.sessionsMu.Lock()
	newSes := s.sessions[1]
	s.sessionsMu.Unlock()
	if newSes == nil {
		t.Fatal("session 1 missing after reattach")
	}
	if !newSes.altScreen {
		t.Fatal("session.altScreen = false after a PTY-size-changed reattach, want true (must be seeded from the prior session)")
	}
	if newSes.vt == nil || !newSes.vt.AltScreen() {
		t.Fatal("vt.AltScreen() = false after a PTY-size-changed reattach, want true")
	}
}
