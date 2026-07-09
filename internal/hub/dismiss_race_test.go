package hub

import (
	"sync"
	"testing"

	"many-ai-cli/internal/proto"
)

// TestWrapperStillRegistered は inject 後の生存判定ヘルパを固定する。
// dismiss で sessions/wrappers から落ちた ID は false になる。
func TestWrapperStillRegistered(t *testing.T) {
	s := newTestServer()
	registerTestSession(s, 13, "grok")
	wc := &wrapperConn{}
	s.sessionsMu.Lock()
	s.wrappers[13] = wc
	s.sessionsMu.Unlock()

	if !s.wrapperStillRegistered(13, wc) {
		t.Fatal("want registered while session+wrapper present")
	}
	if s.wrapperStillRegistered(13, &wrapperConn{}) {
		t.Fatal("want false for different wrapperConn pointer")
	}

	// handleDismiss 相当: map から削除
	s.sessionsMu.Lock()
	delete(s.sessions, 13)
	delete(s.wrappers, 13)
	s.sessionsMu.Unlock()

	if s.wrapperStillRegistered(13, wc) {
		t.Fatal("want false after dismiss removed session")
	}
}

// TestHandleDismissRemovesSession は通常 dismiss で map から消えることを確認する。
func TestHandleDismissRemovesSession(t *testing.T) {
	s := newTestServer()
	registerTestSession(s, 2, "grok")
	wc := &wrapperConn{}
	s.sessionsMu.Lock()
	s.wrappers[2] = wc
	s.sessionsMu.Unlock()

	s.handleDismiss(proto.Message{Type: "session_dismiss", SessionID: 2})

	s.sessionsMu.Lock()
	_, exists := s.sessions[2]
	_, wrapExists := s.wrappers[2]
	s.sessionsMu.Unlock()
	if exists {
		t.Fatal("session should be deleted after dismiss")
	}
	if wrapExists {
		t.Fatal("wrapper should be deleted after dismiss")
	}
}

// TestHandleDismissIdempotentNoPanic は既に無い ID への再 dismiss が panic せず、
// 冪等に処理されることを確認する（幽霊カード掃除経路）。
func TestHandleDismissIdempotentNoPanic(t *testing.T) {
	s := newTestServer()
	// 最初から存在しない
	s.handleDismiss(proto.Message{Type: "session_dismiss", SessionID: 99})

	// 存在 → 削除 → 再 dismiss
	registerTestSession(s, 14, "grok")
	s.handleDismiss(proto.Message{Type: "session_dismiss", SessionID: 14})
	s.handleDismiss(proto.Message{Type: "session_dismiss", SessionID: 14})

	s.sessionsMu.Lock()
	_, exists := s.sessions[14]
	s.sessionsMu.Unlock()
	if exists {
		t.Fatal("session should remain absent after double dismiss")
	}
}

// TestHandleDismissDuringRegisterWindow は「register 中に map から消す」操作後に
// wrapperStillRegistered が false になること（A 修正の前提条件）を固定する。
func TestHandleDismissDuringRegisterWindow(t *testing.T) {
	s := newTestServer()
	ses := registerTestSession(s, 13, "grok")
	ses.State = "standby"
	wc := &wrapperConn{}
	s.sessionsMu.Lock()
	s.wrappers[13] = wc
	s.sessionsMu.Unlock()

	// inject 相当の窓: まだ announce していないが map には載っている
	if !s.wrapperStillRegistered(13, wc) {
		t.Fatal("precondition: session should be registered")
	}

	// UI × 相当
	var wg sync.WaitGroup
	wg.Add(1)
	go func() {
		defer wg.Done()
		s.handleDismiss(proto.Message{Type: "session_dismiss", SessionID: 13})
	}()
	wg.Wait()

	if s.wrapperStillRegistered(13, wc) {
		t.Fatal("after dismiss during register window, announce must be skipped")
	}
}
