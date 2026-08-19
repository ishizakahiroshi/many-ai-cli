package hub

import (
	"net/http/httptest"
	"strings"
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
