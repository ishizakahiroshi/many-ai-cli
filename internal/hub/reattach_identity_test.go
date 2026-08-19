package hub

import (
	"testing"

	"many-ai-cli/internal/proto"
)

// TestReattachIdentityMatches は「出力キュー溢れで wrapper が自分から張り直した
// 再接続」を別 wrapper との ID 衝突と誤判定しないことを確認する。
//
// 誤判定すると同一プロセスに 2 つ目のセッション番号が振られ、UI 上でカードが
// 2 枚に割れる（2026-08-05 #15 → #17 / 同一 pid=6224）。逆に緩すぎると他人の
// セッションを乗っ取るので、PID 単独では一致とみなさない。
func TestReattachIdentityMatches(t *testing.T) {
	const pid = 6224
	cwd := `C:\work\sample-project`
	prev := &wrapperConn{pid: pid}
	ses := &session{ID: 15, Provider: "codex", CWD: cwd}
	req := proto.Message{SessionID: 15, PID: pid, Provider: "codex", CWD: cwd}

	tests := []struct {
		name string
		prev *wrapperConn
		ses  *session
		req  proto.Message
		want bool
	}{
		{"same wrapper reconnects", prev, ses, req, true},
		{
			"different pid is a real collision",
			&wrapperConn{pid: 21696}, ses, req, false,
		},
		{
			"same pid but different cwd",
			prev, &session{ID: 15, Provider: "codex", CWD: `C:\work\other-project`}, req, false,
		},
		{
			"same pid but different provider",
			prev, &session{ID: 15, Provider: "claude", CWD: cwd}, req, false,
		},
		{
			"wrapper without pid falls back to renumber",
			prev, ses, proto.Message{SessionID: 15, Provider: "codex", CWD: cwd}, false,
		},
		{
			"existing conn without pid falls back to renumber",
			&wrapperConn{}, ses, req, false,
		},
		{"no existing conn", nil, ses, req, false},
		{"no existing session", prev, nil, req, false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := reattachIdentityMatches(tt.prev, tt.ses, tt.req); got != tt.want {
				t.Fatalf("reattachIdentityMatches = %v, want %v", got, tt.want)
			}
		})
	}
}

// TestReattachKeepsSessionIDForSameWrapper は衝突判定の結果として ID が維持され、
// 別 wrapper のときだけ採番が進むことを、reattachLoop と同じ手順で確認する。
// reattachLoop 本体は websocket.Conn を要求するため、ID 決定部分だけを同じ順序で
// 再現している。
func TestReattachKeepsSessionIDForSameWrapper(t *testing.T) {
	cwd := `C:\work\sample-project`
	newServerWithSession := func(pid int) *Server {
		s := newTestServer()
		s.sessions[15] = &session{ID: 15, Provider: "codex", CWD: cwd, State: "running"}
		s.wrappers[15] = &wrapperConn{pid: pid}
		s.nextID = 16
		return s
	}

	decide := func(s *Server, req proto.Message) (int, *wrapperConn) {
		s.sessionsMu.Lock()
		defer s.sessionsMu.Unlock()
		acceptedID := req.SessionID
		var stale *wrapperConn
		if prev := s.wrappers[acceptedID]; prev != nil {
			if reattachIdentityMatches(prev, s.sessions[acceptedID], req) {
				stale = prev
			} else {
				s.nextID++
				acceptedID = s.nextID
			}
		}
		return acceptedID, stale
	}

	t.Run("same wrapper keeps its number", func(t *testing.T) {
		s := newServerWithSession(6224)
		id, stale := decide(s, proto.Message{SessionID: 15, PID: 6224, Provider: "codex", CWD: cwd})
		if id != 15 {
			t.Fatalf("acceptedID = %d, want 15 (card must not split)", id)
		}
		if stale == nil {
			t.Fatal("stale conn = nil, want the previous conn so it gets closed")
		}
		if s.nextID != 16 {
			t.Fatalf("nextID = %d, want 16 (no number consumed)", s.nextID)
		}
	})

	t.Run("different wrapper still renumbers", func(t *testing.T) {
		s := newServerWithSession(21696)
		id, stale := decide(s, proto.Message{SessionID: 15, PID: 6224, Provider: "codex", CWD: cwd})
		if id != 17 {
			t.Fatalf("acceptedID = %d, want 17 (real collision must renumber)", id)
		}
		if stale != nil {
			t.Fatal("stale conn != nil; a foreign wrapper's conn must not be closed")
		}
	})
}

func TestReattachAnnounceMessageSkipsDismissedSession(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 15, "codex")
	ses.State = "running"
	wc := &wrapperConn{}
	s.sessionsMu.Lock()
	s.wrappers[15] = wc
	s.sessionsMu.Unlock()

	if _, ok := s.reattachAnnounceMessage(15, wc); !ok {
		t.Fatal("active session should produce an announce message")
	}

	s.sessionsMu.Lock()
	delete(s.sessions, 15)
	s.sessionsMu.Unlock()
	if _, ok := s.reattachAnnounceMessage(15, wc); ok {
		t.Fatal("dismissed session must not produce an announce message")
	}
}
