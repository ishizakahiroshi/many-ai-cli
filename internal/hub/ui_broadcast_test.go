package hub

import (
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
