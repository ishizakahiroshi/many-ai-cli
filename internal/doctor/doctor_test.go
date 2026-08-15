package doctor

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
)

func TestProvidersUsesThreeSecondVersionTimeout(t *testing.T) {
	oldLookPath := providerLookPath
	oldVersionOutput := providerVersionOutput
	t.Cleanup(func() {
		providerLookPath = oldLookPath
		providerVersionOutput = oldVersionOutput
	})
	providerLookPath = func(name string) (string, error) {
		if name == "claude" {
			return "test-claude", nil
		}
		return "", errors.New("not found")
	}
	var remaining time.Duration
	providerVersionOutput = func(ctx context.Context, path string) ([]byte, error) {
		if path != "test-claude" {
			t.Fatalf("provider path = %q, want test-claude", path)
		}
		deadline, ok := ctx.Deadline()
		if !ok {
			t.Fatal("provider version context has no deadline")
		}
		remaining = time.Until(deadline)
		return []byte("claude test-version\n"), nil
	}

	check := providers(context.Background())

	if check.Level != OK || !strings.Contains(check.Message, "test-version") {
		t.Fatalf("providers check = %+v", check)
	}
	if remaining < 2500*time.Millisecond || remaining > providerVersionTimeout {
		t.Fatalf("provider timeout remaining = %s, want approximately %s", remaining, providerVersionTimeout)
	}
}

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

func TestSessionLogCheck(t *testing.T) {
	for _, tc := range []struct {
		name        string
		enabled     bool
		makeDir     bool
		files       map[string]string
		wantLevel   Level
		wantMessage string
	}{
		{name: "disabled", enabled: false, wantLevel: OK, wantMessage: "セッションログは無効です"},
		{name: "no sessions dir", enabled: true, wantLevel: Warn, wantMessage: "まだ 1 本も作られていません"},
		{name: "empty sessions dir", enabled: true, makeDir: true, wantLevel: Warn, wantMessage: "まだ 1 本も作られていません"},
		{
			name:        "zero byte log",
			enabled:     true,
			makeDir:     true,
			files:       map[string]string{"claude_2026-08-15_120000_proj_s1.log": ""},
			wantLevel:   Warn,
			wantMessage: "0 バイトです",
		},
		{
			name:        "written log",
			enabled:     true,
			makeDir:     true,
			files:       map[string]string{"claude_2026-08-15_120000_proj_s1.log": strings.Repeat("x", 2048)},
			wantLevel:   OK,
			wantMessage: "書き込まれています",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			base := t.TempDir()
			if tc.makeDir {
				if err := os.MkdirAll(filepath.Join(base, "sessions"), 0o700); err != nil {
					t.Fatal(err)
				}
			}
			for name, body := range tc.files {
				if err := os.WriteFile(filepath.Join(base, "sessions", name), []byte(body), 0o600); err != nil {
					t.Fatal(err)
				}
			}
			cfg := &config.Config{}
			cfg.Hub.LogDir = base
			cfg.Log.SessionEnabled = tc.enabled

			check := sessionLog(cfg)

			if check.Level != tc.wantLevel || !strings.Contains(check.Message, tc.wantMessage) {
				t.Fatalf("sessionLog check = %+v, want level %s containing %q", check, tc.wantLevel, tc.wantMessage)
			}
		})
	}
}

// TestSessionLogCheckIgnoresNonLogFiles は .jsonl / .txt を「最新の .log」に選ばないことを固定する。
// .jsonl は Hub、.log は wrapper が別々に書くので、更新時刻は .jsonl の方が新しくなりやすい。
func TestSessionLogCheckIgnoresNonLogFiles(t *testing.T) {
	base := t.TempDir()
	dir := filepath.Join(base, "sessions")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "claude_2026-08-15_120000_proj_s1.log"), []byte("payload"), 0o600); err != nil {
		t.Fatal(err)
	}
	jsonlPath := filepath.Join(dir, "claude_2026-08-15_120000_proj_s1.jsonl")
	if err := os.WriteFile(jsonlPath, []byte("{}\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	// .jsonl を .log より新しくして、選択が拡張子で決まることを確かめる。
	later := time.Now().Add(time.Hour)
	if err := os.Chtimes(jsonlPath, later, later); err != nil {
		t.Fatal(err)
	}

	cfg := &config.Config{}
	cfg.Hub.LogDir = base
	cfg.Log.SessionEnabled = true

	check := sessionLog(cfg)

	if check.Level != OK || !strings.Contains(check.Message, ".log") || strings.Contains(check.Message, ".jsonl") {
		t.Fatalf("sessionLog check = %+v, want OK naming the .log file", check)
	}
}

// TestSessionLogCheckExplainsStaleDirectoryEntry は、ディレクトリ一覧のサイズと
// 実サイズが食い違うとき（Windows で書き込み中のメタデータが未反映のとき）に、
// それが故障ではないと明示することを固定する。
func TestSessionLogCheckExplainsStaleDirectoryEntry(t *testing.T) {
	base := t.TempDir()
	dir := filepath.Join(base, "sessions")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	// ディレクトリエントリ上は 0 バイト。
	if err := os.WriteFile(filepath.Join(dir, "claude_2026-08-15_120000_proj_s1.log"), nil, 0o600); err != nil {
		t.Fatal(err)
	}
	old := sessionLogSizeOnDisk
	t.Cleanup(func() { sessionLogSizeOnDisk = old })
	// ハンドル経由では書き込み済み、という Windows の食い違いを再現する。
	sessionLogSizeOnDisk = func(string) (int64, error) { return 3 * 1024 * 1024, nil }

	check := sessionLog(cfgWithSessionLog(base))

	if check.Level != OK {
		t.Fatalf("sessionLog check = %+v, want OK", check)
	}
	if !strings.Contains(check.Message, "3.0 MB") || !strings.Contains(check.Message, "0 B") {
		t.Fatalf("sessionLog message = %q, want both the real size and the directory-entry size", check.Message)
	}
	if !strings.Contains(check.Message, "故障ではありません") {
		t.Fatalf("sessionLog message = %q, want an explicit note that this is not a failure", check.Message)
	}
}

func TestSessionLogCheckOpenError(t *testing.T) {
	base := t.TempDir()
	dir := filepath.Join(base, "sessions")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "claude_2026-08-15_120000_proj_s1.log"), []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	old := sessionLogSizeOnDisk
	t.Cleanup(func() { sessionLogSizeOnDisk = old })
	sessionLogSizeOnDisk = func(string) (int64, error) { return 0, errors.New("locked") }

	check := sessionLog(cfgWithSessionLog(base))

	if check.Level != Warn || !strings.Contains(check.Message, "開けません") || check.Fix == "" {
		t.Fatalf("sessionLog check = %+v, want Warn with a fix hint", check)
	}
}

func cfgWithSessionLog(logDir string) *config.Config {
	cfg := &config.Config{}
	cfg.Hub.LogDir = logDir
	cfg.Log.SessionEnabled = true
	return cfg
}

func TestHumanBytes(t *testing.T) {
	for _, tc := range []struct {
		in   int64
		want string
	}{
		{in: 0, want: "0 B"},
		{in: 512, want: "512 B"},
		{in: 2048, want: "2.0 KB"},
		{in: 3 * 1024 * 1024, want: "3.0 MB"},
	} {
		if got := humanBytes(tc.in); got != tc.want {
			t.Fatalf("humanBytes(%d) = %q, want %q", tc.in, got, tc.want)
		}
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
