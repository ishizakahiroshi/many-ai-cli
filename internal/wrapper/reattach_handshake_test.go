package wrapper

import (
	"net/http/httptest"
	"net/url"
	"strconv"
	"testing"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

func TestDialAndReattachWithPendingKeepsMessagesBeforeAck(t *testing.T) {
	server := httptest.NewServer(websocket.Handler(func(conn *websocket.Conn) {
		var req proto.Message
		if err := websocket.JSON.Receive(conn, &req); err != nil {
			return
		}
		if req.Type != "reattach" {
			return
		}
		_ = websocket.JSON.Send(conn, proto.Message{
			Type:     "pty_input",
			InputSeq: 4,
			Data:     []byte("answer\r"),
		})
		_ = websocket.JSON.Send(conn, proto.Message{
			Type: "pty_resize",
			Cols: 120,
			Rows: 40,
		})
		_ = websocket.JSON.Send(conn, proto.Message{Type: "reattach_ack", SessionID: 9})
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
	cfg := &config.Config{}
	cfg.Hub.Port = port

	conn, sid, queued, err := dialAndReattachWithPending(
		cfg, 1, "codex", "Codex", "/tmp", "", "", "", "", "", "", 80, 24, nil, 0,
	)
	if err != nil {
		t.Fatalf("dialAndReattachWithPending returned error: %v", err)
	}
	if conn == nil {
		t.Fatal("dialAndReattachWithPending returned nil connection")
	}
	defer conn.Close()
	if sid != 9 {
		t.Fatalf("session id = %d, want 9", sid)
	}
	if len(queued) != 2 {
		t.Fatalf("queued messages = %d, want 2", len(queued))
	}
	if queued[0].Type != "pty_input" || string(queued[0].Data) != "answer\r" || queued[0].InputSeq != 4 {
		t.Fatalf("queued[0] = %+v, want the pty_input frame", queued[0])
	}
	if queued[1].Type != "pty_resize" || queued[1].Cols != 120 || queued[1].Rows != 40 {
		t.Fatalf("queued[1] = %+v, want the pty_resize frame", queued[1])
	}
}
