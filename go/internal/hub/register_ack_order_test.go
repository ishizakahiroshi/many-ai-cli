package hub

import (
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/proto"
)

// TestRegisterAckIsFirstFrameEvenIfResizeArrivesEarly は、ラッパーが受け取る最初の
// フレームが必ず registered であることを固定する。
//
// ラッパー（wrapper.dialAndRegister）は register の応答として最初に届いたフレームが
// registered でなければ即終了する。Hub は s.wrappers へ接続を載せたあと registered を
// 送るまでの間に承認ルール・Codex の Stop hook の書き込みを挟むので、その窓で UI の
// pty_resize が来ると、registered より先に pty_resize がラッパーへ届いて起動に失敗する
// （観測: logs/spawn/codex-20261002-1143*.log
// `unexpected register response type "pty_resize" (want registered)`）。
func TestRegisterAckIsFirstFrameEvenIfResizeArrivesEarly(t *testing.T) {
	s := newTestServer()
	server := httptest.NewServer(websocket.Handler(func(conn *websocket.Conn) {
		var reg proto.Message
		if err := websocket.JSON.Receive(conn, &reg); err != nil {
			return
		}
		s.wrapperLoop(conn, reg)
	}))
	defer server.Close()

	stop := make(chan struct{})
	defer close(stop)
	go func() {
		// 新しいセッションが Hub に載った瞬間から resize を撃ち続ける（UI の再描画を模す）。
		for {
			select {
			case <-stop:
				return
			default:
			}
			s.sessionsMu.Lock()
			ids := make([]int, 0, len(s.sessions))
			for id := range s.sessions {
				ids = append(ids, id)
			}
			s.sessionsMu.Unlock()
			for _, id := range ids {
				s.handleResizeFromUI(nil, proto.Message{Type: "pty_resize", SessionID: id, Cols: 77, Rows: 33})
			}
		}
	}()

	wsURL := "ws" + strings.TrimPrefix(server.URL, "http")
	client, err := websocket.Dial(wsURL, "", "http://127.0.0.1/")
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	defer client.Close()
	if err := websocket.JSON.Send(client, proto.Message{Type: "register", Role: "wrapper", Provider: "codex", CWD: t.TempDir(), PID: 1}); err != nil {
		t.Fatalf("send register: %v", err)
	}
	_ = client.SetReadDeadline(time.Now().Add(3 * time.Second))
	var first proto.Message
	if err := websocket.JSON.Receive(client, &first); err != nil {
		t.Fatalf("receive first frame: %v", err)
	}
	if first.Type != "registered" {
		t.Fatalf("first frame after register = %q, want registered", first.Type)
	}
}
