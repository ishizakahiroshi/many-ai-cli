package hub

import (
	"os"
	"path/filepath"
	"testing"
)

func TestEnsureSubscriptionLoginRootIsUnderConfigDir(t *testing.T) {
	hubCWD := t.TempDir()
	configDir := t.TempDir()
	if err := os.MkdirAll(filepath.Join(hubCWD, "dist"), 0o755); err != nil {
		t.Fatal(err)
	}
	got, err := ensureSubscriptionLoginRoot(configDir)
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(configDir, subscriptionLoginDirName)
	if got != want {
		t.Fatalf("login cwd = %q, want %q", got, want)
	}
	if filepath.Base(got) == "dist" {
		t.Fatalf("login cwd used the Hub exe directory name: %q", got)
	}
	info, err := os.Stat(got)
	if err != nil || !info.IsDir() {
		t.Fatalf("login workspace was not created: %v", err)
	}
}

func TestEnsureSubscriptionLoginRootRejectsEmptyConfigDir(t *testing.T) {
	if _, err := ensureSubscriptionLoginRoot(""); err == nil {
		t.Fatal("empty config dir should fail")
	}
	if _, err := ensureSubscriptionLoginRoot("   "); err == nil {
		t.Fatal("blank config dir should fail")
	}
}

func TestDismissSubscriptionLoginSessionLeavesOrdinarySessions(t *testing.T) {
	s, _ := subsTestServer(t)
	s.sessionsMu.Lock()
	s.sessions[1] = &session{ID: 1, Label: "login-grok-dev", SubscriptionLogin: true}
	s.sessions[2] = &session{ID: 2, Label: "work"}
	s.sessionsMu.Unlock()

	s.dismissSubscriptionLoginSession(1)
	s.dismissSubscriptionLoginSession(2)

	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if _, ok := s.sessions[1]; ok {
		t.Fatal("login session should be dismissed")
	}
	if _, ok := s.sessions[2]; !ok {
		t.Fatal("ordinary session should remain")
	}
}
