package doctor

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"many-ai-cli/internal/config"
)

func TestOllamaHTTPChecks(t *testing.T) {
	for _, tc := range []struct {
		name       string
		status     int
		timeout    bool
		wantLevel  Level
		invalidURL string
	}{
		{name: "ok", status: http.StatusOK, wantLevel: OK},
		{name: "not found", status: http.StatusNotFound, wantLevel: Warn},
		{name: "timeout", timeout: true, wantLevel: Warn},
		{name: "invalid url", invalidURL: "http://foo\x00", wantLevel: Warn},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cfg := &config.Config{}
			if tc.invalidURL != "" {
				cfg.Ollama.BaseURL = tc.invalidURL
				check := ollama(context.Background(), cfg)
				if check.Level != tc.wantLevel || check.Message == "" {
					t.Fatalf("ollama invalid URL check = %+v, want level %s with message", check, tc.wantLevel)
				}
				return
			}
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if tc.timeout {
					<-r.Context().Done()
					return
				}
				w.WriteHeader(tc.status)
			}))
			defer server.Close()
			cfg.Ollama.BaseURL = server.URL
			check := ollama(context.Background(), cfg)
			if check.Level != tc.wantLevel {
				t.Fatalf("ollama check = %+v, want level %s", check, tc.wantLevel)
			}
		})
	}
}

func TestWhisperHTTPChecks(t *testing.T) {
	for _, tc := range []struct {
		name      string
		status    int
		timeout   bool
		wantLevel Level
	}{
		{name: "ok", status: http.StatusOK, wantLevel: OK},
		{name: "not found", status: http.StatusNotFound, wantLevel: OK},
		{name: "timeout", timeout: true, wantLevel: Warn},
	} {
		t.Run(tc.name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if tc.timeout {
					<-r.Context().Done()
					return
				}
				w.WriteHeader(tc.status)
			}))
			defer server.Close()
			cfg := &config.Config{}
			cfg.Voice.Whisper.ServerURL = server.URL
			check := whisper(context.Background(), cfg)
			if check.Level != tc.wantLevel {
				t.Fatalf("whisper check = %+v, want level %s", check, tc.wantLevel)
			}
		})
	}
}

func TestTokenAndACLPermissions(t *testing.T) {
	for _, tc := range []struct {
		name string
		mode os.FileMode
		want Level
	}{
		{name: "private", mode: 0o600, want: OK},
		{name: "group readable", mode: 0o640, want: Warn},
	} {
		t.Run(tc.name, func(t *testing.T) {
			home := t.TempDir()
			t.Setenv("HOME", home)
			t.Setenv("USERPROFILE", home)
			dir := filepath.Join(home, ".many-ai-cli")
			if err := os.MkdirAll(dir, 0o700); err != nil {
				t.Fatal(err)
			}
			path := filepath.Join(dir, "config.yaml")
			if err := os.WriteFile(path, []byte("token: test\n"), tc.mode); err != nil {
				t.Fatal(err)
			}
			if err := os.Chmod(path, tc.mode); err != nil {
				t.Fatal(err)
			}
			// Directory needs owner execute bit (+x) so os.Stat on files inside works.
			// 0o600 on dir would prevent traversal on Unix.
			if err := os.Chmod(dir, tc.mode|0o100); err != nil {
				t.Fatal(err)
			}
			// Restore 0o700 on cleanup so t.TempDir removal succeeds.
			t.Cleanup(func() { _ = os.Chmod(dir, 0o700) })
			cfg := &config.Config{Token: "test"}
			gotToken, gotACL := token(cfg), acl()
			if runtime.GOOS == "windows" {
				if gotToken.Level != OK || gotACL.Level != OK {
					t.Fatalf("Windows permission checks = token=%+v acl=%+v, want OK", gotToken, gotACL)
				}
				return
			}
			if gotToken.Level != tc.want || gotACL.Level != tc.want {
				t.Fatalf("permission checks = token=%+v acl=%+v, want level %s", gotToken, gotACL, tc.want)
			}
		})
	}
}
