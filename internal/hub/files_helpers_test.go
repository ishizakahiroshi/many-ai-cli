package hub

import (
	"fmt"
	"net/http/httptest"
	"testing"

	"any-ai-cli/internal/config"
)

// TestCwdForRequest_NoSession は ?session パラメータ無しのとき hubCWD を返すことを確認する。
func TestCwdForRequest_NoSession(t *testing.T) {
	s := &Server{
		cfg:      &config.Config{},
		hubCWD:   "/hub/cwd",
		sessions: map[int]*session{},
	}
	req := httptest.NewRequest("GET", "/api/files-list", nil)
	got := s.cwdForRequest(req)
	if got != "/hub/cwd" {
		t.Fatalf("cwdForRequest = %q, want %q", got, "/hub/cwd")
	}
}

// TestCwdForRequest_WithSession は ?session=<id> がセッション CWD を返すことを確認する。
func TestCwdForRequest_WithSession(t *testing.T) {
	s := &Server{
		cfg:    &config.Config{},
		hubCWD: "/hub/cwd",
		sessions: map[int]*session{
			42: {ID: 42, CWD: "/project/abc"},
		},
	}
	req := httptest.NewRequest("GET", fmt.Sprintf("/api/files-list?session=%d", 42), nil)
	got := s.cwdForRequest(req)
	if got != "/project/abc" {
		t.Fatalf("cwdForRequest = %q, want %q", got, "/project/abc")
	}
}

// TestCwdForRequest_UnknownSession は存在しないセッション ID のとき hubCWD に fallback することを確認する。
func TestCwdForRequest_UnknownSession(t *testing.T) {
	s := &Server{
		cfg:      &config.Config{},
		hubCWD:   "/hub/cwd",
		sessions: map[int]*session{},
	}
	req := httptest.NewRequest("GET", "/api/files-list?session=999", nil)
	got := s.cwdForRequest(req)
	if got != "/hub/cwd" {
		t.Fatalf("cwdForRequest = %q, want %q", got, "/hub/cwd")
	}
}

// TestCwdForRequest_InvalidSession は session パラメータが数値でない場合に hubCWD を返すことを確認する。
func TestCwdForRequest_InvalidSession(t *testing.T) {
	s := &Server{
		cfg:      &config.Config{},
		hubCWD:   "/hub/cwd",
		sessions: map[int]*session{},
	}
	req := httptest.NewRequest("GET", "/api/files-list?session=abc", nil)
	got := s.cwdForRequest(req)
	if got != "/hub/cwd" {
		t.Fatalf("cwdForRequest = %q, want %q", got, "/hub/cwd")
	}
}
