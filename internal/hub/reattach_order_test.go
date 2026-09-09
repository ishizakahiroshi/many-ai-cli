package hub

import (
	"context"
	"io"
	"log/slog"
	"net/http/httptest"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/proto"
)

type reattachOrderGateHandler struct {
	next     slog.Handler
	cfgMu    *sync.Mutex
	locked   chan<- struct{}
	lockOnce *sync.Once
}

func (h *reattachOrderGateHandler) Enabled(ctx context.Context, level slog.Level) bool {
	return h.next.Enabled(ctx, level)
}

func (h *reattachOrderGateHandler) Handle(ctx context.Context, record slog.Record) error {
	if record.Message == "requeued in-flight pty_input" {
		h.lockOnce.Do(func() {
			h.cfgMu.Lock()
			close(h.locked)
		})
	}
	return h.next.Handle(ctx, record)
}

func (h *reattachOrderGateHandler) WithAttrs(attrs []slog.Attr) slog.Handler {
	return &reattachOrderGateHandler{
		next:     h.next.WithAttrs(attrs),
		cfgMu:    h.cfgMu,
		locked:   h.locked,
		lockOnce: h.lockOnce,
	}
}

func (h *reattachOrderGateHandler) WithGroup(name string) slog.Handler {
	return &reattachOrderGateHandler{
		next:     h.next.WithGroup(name),
		cfgMu:    h.cfgMu,
		locked:   h.locked,
		lockOnce: h.lockOnce,
	}
}

func TestReattachSendsAckBeforePendingInput(t *testing.T) {
	s := newTestServer()
	tokenStatusbarDisabled := false
	s.cfg.UserPrefs.TokenStatusbar.Enabled = &tokenStatusbarDisabled
	ses := registerTestSession(s, 1, "codex")
	ses.CWD = "/tmp"
	oldWC := &wrapperConn{pid: 123}
	oldWC.inputAckSeen.Store(true)
	s.sessionsMu.Lock()
	s.wrappers[1] = oldWC
	ses.inflightInput = map[int64]inflightInput{
		1: {data: "already-sent\r", conn: oldWC},
	}
	s.pendingInput[1] = []string{"answer\r"}
	s.sessionsMu.Unlock()

	// reattachLoop logs the requeued frame after its initial cfg snapshot and
	// before the ack/flush ordering. Hold cfgMu at that point: the old ordering
	// starts flush before the injection path waits on cfgMu, while the fixed
	// ordering sends the ack before waiting there. This is deterministic and
	// avoids relying on scheduler timing or an arbitrary sleep.
	locked := make(chan struct{})
	s.logger = slog.New(&reattachOrderGateHandler{
		next:     slog.NewTextHandler(io.Discard, nil),
		cfgMu:    &s.cfgMu,
		locked:   locked,
		lockOnce: new(sync.Once),
	})

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
		PID:        123,
		Cols:       80,
		Rows:       24,
		UsageProbe: true,
	}); err != nil {
		t.Fatalf("send reattach request: %v", err)
	}

	select {
	case <-locked:
	case <-time.After(time.Second):
		t.Fatal("reattach did not reach the injection ordering gate")
	}
	configLocked := true
	t.Cleanup(func() {
		if configLocked {
			s.cfgMu.Unlock()
		}
	})

	var first proto.Message
	if err := websocket.JSON.Receive(conn, &first); err != nil {
		t.Fatalf("receive first reattach frame: %v", err)
	}
	if first.Type != "reattach_ack" || first.SessionID != 1 {
		t.Fatalf("first reattach frame = %+v, want reattach_ack", first)
	}
	s.cfgMu.Unlock()
	configLocked = false

	_ = conn.SetReadDeadline(time.Now().Add(time.Second))
	for {
		var frame proto.Message
		if err := websocket.JSON.Receive(conn, &frame); err != nil {
			t.Fatalf("receive pending input frame: %v", err)
		}
		if frame.Type == "pty_input" && string(frame.Data) == "answer\r" {
			return
		}
	}
}
