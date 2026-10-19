package hub

import (
	"bytes"
	"encoding/base64"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/proto"
)

func TestReattachRestoresInputSequenceAndReplay(t *testing.T) {
	for _, tc := range []struct {
		name                string
		previous, processed int64
		replaySize          int
	}{
		{"cold restart", 0, 42, proto.PTYReplayBufferLimit},
		{"warm reconnect with unacked input", 50, 42, 0},
		{"legacy wrapper", 50, 0, 0},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s := newTestServer()
			disabled := false
			s.cfg.UserPrefs.TokenStatusbar.Enabled = &disabled
			if tc.previous > 0 {
				ses := registerTestSession(s, 1, "codex")
				ses.CWD = "/tmp"
				ses.inputSeq = tc.previous
			}
			done := make(chan struct{})
			server := httptest.NewServer(websocket.Handler(func(conn *websocket.Conn) {
				defer close(done)
				limitWSReceive(conn)
				var req proto.Message
				if err := websocket.JSON.Receive(conn, &req); err != nil {
					return
				}
				s.reattachLoop(conn, req)
			}))
			defer server.Close()
			conn, err := websocket.Dial("ws"+strings.TrimPrefix(server.URL, "http"), "", server.URL)
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				conn.Close()
				select {
				case <-done:
				case <-time.After(5 * time.Second):
					t.Error("reattach handler did not stop")
				}
			}()
			conn.SetDeadline(time.Now().Add(10 * time.Second))
			replay := bytes.Repeat([]byte("x"), tc.replaySize)
			err = websocket.JSON.Send(conn, proto.Message{
				Type: "reattach", SessionID: 1, Provider: "codex", CWD: "/tmp", PID: 999,
				Cols: 80, Rows: 24, UsageProbe: true,
				InputSeqHighWatermark: tc.processed, ReplayB64: base64.StdEncoding.EncodeToString(replay), PTYBytes: int64(len(replay)),
			})
			if err != nil {
				t.Fatal(err)
			}
			var ack proto.Message
			if err := websocket.JSON.Receive(conn, &ack); err != nil {
				t.Fatal(err)
			}
			if ack.Type != "reattach_ack" {
				t.Fatalf("first frame = %s", ack.Type)
			}
			s.sessionsMu.Lock()
			wc := s.wrappers[ack.SessionID]
			restored := bytes.Clone(s.sessions[ack.SessionID].ptyBuf)
			s.sessionsMu.Unlock()
			if !bytes.Equal(restored, replay) {
				t.Fatalf("replay shrank: %d -> %d", len(replay), len(restored))
			}
			next := s.reserveInflightInput(wc, ack.SessionID, "new instruction\r")
			if want := max(tc.previous, tc.processed) + 1; next != want {
				t.Fatalf("new input sequence = %d, want %d; wrapper would discard it as already processed", next, want)
			}
		})
	}
}
