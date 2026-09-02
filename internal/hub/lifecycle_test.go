package hub

import (
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/proto"
)

func TestLocalHubURLEscapesToken(t *testing.T) {
	got := localHubURL(47777, "/", "a+b&x=1")
	want := "http://127.0.0.1:47777/?token=a%2Bb%26x%3D1"
	if got != want {
		t.Errorf("localHubURL = %q, want %q", got, want)
	}
}

func TestKillAllWrappersSendsDismissBeforeClose(t *testing.T) {
	s := newTestServer()
	ready := make(chan struct{})
	wsServer := httptest.NewServer(websocket.Handler(func(conn *websocket.Conn) {
		wc := newWrapperConn(conn)
		s.sessionsMu.Lock()
		s.wrappers[1] = wc
		s.sessionsMu.Unlock()
		close(ready)
		var ignored proto.Message
		_ = websocket.JSON.Receive(conn, &ignored)
	}))
	defer wsServer.Close()

	wsURL := "ws" + strings.TrimPrefix(wsServer.URL, "http")
	client, err := websocket.Dial(wsURL, "", "http://127.0.0.1/")
	if err != nil {
		t.Fatalf("dial wrapper websocket: %v", err)
	}
	defer client.Close()
	select {
	case <-ready:
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for wrapper registration")
	}

	s.killAllWrappers("idle_timeout")
	var got proto.Message
	if err := websocket.JSON.Receive(client, &got); err != nil {
		t.Fatalf("receive session_dismissed: %v", err)
	}
	if got.Type != proto.TypeSessionDismissed || got.Reason != "idle_timeout" {
		t.Fatalf("dismiss message = %+v, want session_dismissed/idle_timeout", got)
	}
}

// TestKillWrapper_SendsDismissWithSessionIDAndCloses covers C3
// (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md): killWrapper
// targets exactly one session's wrapper (unlike killAllWrappers' broadcast) and
// stamps SessionID on the dismiss message, matching handleDismiss's own usage
// for log/JSONL traceability.
func TestKillWrapper_SendsDismissWithSessionIDAndCloses(t *testing.T) {
	s := newTestServer()
	var mu sync.Mutex
	var received []proto.Message
	done := make(chan struct{})
	wsServer := httptest.NewServer(websocket.Handler(func(conn *websocket.Conn) {
		var m proto.Message
		if err := websocket.JSON.Receive(conn, &m); err == nil {
			mu.Lock()
			received = append(received, m)
			mu.Unlock()
		}
		close(done)
	}))
	defer wsServer.Close()

	wsURL := "ws" + strings.TrimPrefix(wsServer.URL, "http")
	client, err := websocket.Dial(wsURL, "", "http://127.0.0.1/")
	if err != nil {
		t.Fatalf("dial wrapper websocket: %v", err)
	}
	defer client.Close()
	wc := newWrapperConn(client)
	s.sessionsMu.Lock()
	s.wrappers[42] = wc
	s.wrappers[43] = newWrapperConn(&websocket.Conn{}) // 別セッション: 触れられないことを確認する対象
	s.sessionsMu.Unlock()

	if ok := s.killWrapper(42, "startup_failed"); !ok {
		t.Fatal("killWrapper(42) = false, want true (wrapper was connected)")
	}

	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("timed out waiting for the dismiss message")
	}
	mu.Lock()
	got := append([]proto.Message(nil), received...)
	mu.Unlock()
	if len(got) != 1 {
		t.Fatalf("received = %+v, want exactly 1 message", got)
	}
	if got[0].Type != proto.TypeSessionDismissed || got[0].SessionID != 42 || got[0].Reason != "startup_failed" {
		t.Fatalf("dismiss message = %+v, want session_dismissed/startup_failed for session 42", got[0])
	}
}

// TestKillWrapper_NoWrapperReturnsFalse covers the no-op case: a session with
// no connected wrapper is reported as "nothing to stop" rather than panicking
// or silently pretending to have stopped something.
func TestKillWrapper_NoWrapperReturnsFalse(t *testing.T) {
	s := newTestServer()
	if s.killWrapper(999, "startup_failed") {
		t.Fatal("killWrapper for an unregistered session = true, want false")
	}
}

// TestKillAllWrappers_TargetsEveryRegisteredSession covers the killAllWrappers
// refactor onto killWrapper (C3): every registered wrapper still receives its
// own dismiss message with its own SessionID and the shared reason, and
// wrappers that were never registered are left alone.
func TestKillAllWrappers_TargetsEveryRegisteredSession(t *testing.T) {
	s := newTestServer()
	var mu sync.Mutex
	received := map[int]proto.Message{}
	ids := []int{1, 2, 3}
	for _, id := range ids {
		id := id
		s.sessionsMu.Lock()
		s.wrappers[id] = &wrapperConn{sendFunc: func(m any) error {
			if msg, ok := m.(proto.Message); ok {
				mu.Lock()
				received[id] = msg
				mu.Unlock()
			}
			return nil
		}}
		s.sessionsMu.Unlock()
	}

	s.killAllWrappers("kill_all")

	mu.Lock()
	defer mu.Unlock()
	if len(received) != len(ids) {
		t.Fatalf("received messages for %d sessions, want %d: %+v", len(received), len(ids), received)
	}
	for _, id := range ids {
		msg, ok := received[id]
		if !ok {
			t.Fatalf("session %d never received a dismiss message", id)
		}
		if msg.Type != proto.TypeSessionDismissed || msg.SessionID != id || msg.Reason != "kill_all" {
			t.Fatalf("session %d dismiss message = %+v, want session_dismissed/kill_all with SessionID=%d", id, msg, id)
		}
	}
}
