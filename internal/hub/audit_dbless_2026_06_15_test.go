package hub

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"unicode/utf8"

	"many-ai-cli/internal/config"
)

// --- 監査 2026-06-15: sanitizeCommitMessage の UTF-8 境界丸め + 制御文字除去 ---

// TestSanitizeCommitMessageUTF8Boundary は maxLen 超過時にマルチバイト文字を
// バイト単位で分断せず、戻り値が常に有効な UTF-8 になることを検証する。
func TestSanitizeCommitMessageUTF8Boundary(t *testing.T) {
	// 日本語 70 文字（各 3 バイト = 210 バイト）。maxLen=200 でバイト切りすると
	// 末尾のルーンが分断され不正 UTF-8 になる。
	in := strings.Repeat("あ", 70)
	out := sanitizeCommitMessage(in, 200)
	if !utf8.ValidString(out) {
		t.Fatalf("result is not valid UTF-8: %q", out)
	}
	if len(out) > 200 {
		t.Fatalf("result %d bytes exceeds maxLen 200", len(out))
	}
	// 上限未満の通常メッセージは不変。
	short := "feat: 通常のコミット"
	if got := sanitizeCommitMessage(short, 200); got != short {
		t.Fatalf("short message changed: got %q want %q", got, short)
	}
}

// TestSanitizeCommitMessageStripsControl は BEL/ESC 等の C0 制御文字と DEL が
// 除去され（\t/\n は保持）、git 履歴・WS 配信へのターミナルエスケープ注入を防ぐことを検証する。
func TestSanitizeCommitMessageStripsControl(t *testing.T) {
	in := "feat: bell\x07 esc\x1b[31mred\x7f tab\there"
	out := sanitizeCommitMessage(in, 0)
	for _, bad := range []string{"\x07", "\x1b", "\x7f"} {
		if strings.Contains(out, bad) {
			t.Fatalf("control char %q not stripped: %q", bad, out)
		}
	}
	if !strings.Contains(out, "\t") {
		t.Fatalf("tab should be preserved: %q", out)
	}
	multiline := "subject\n\nbody line"
	if got := sanitizeCommitMessage(multiline, 0); got != multiline {
		t.Fatalf("newlines should be preserved: got %q", got)
	}
}

// --- 監査 2026-06-15: /api/avatar の任意ファイル読み出し防止 ---

func newAvatarTestServer(t *testing.T) (*Server, string) {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	dir, err := config.Dir()
	if err != nil {
		t.Fatalf("config.Dir: %v", err)
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatalf("mkdir config dir: %v", err)
	}
	s := newSecTestServer(t, t.TempDir())
	return s, dir
}

func avatarReq() *http.Request {
	req := httptest.NewRequest(http.MethodGet, "/api/avatar?token=tok", nil)
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:5000"
	return req
}

// TestHandleAvatarRejectsConfigYaml は avatar 値に config.yaml への絶対パスを設定しても
// /api/avatar がその中身を返さない（404）ことを検証する。config.yaml には Token /
// RemotePINHash / AuthCookieSecret 等の機密が入るため、任意ファイル read プリミティブ化を防ぐ。
func TestHandleAvatarRejectsConfigYaml(t *testing.T) {
	s, dir := newAvatarTestServer(t)
	secretPath := filepath.Join(dir, "config.yaml")
	secret := "token: super-secret-value\nauth_cookie_secret: deadbeef\n"
	if err := os.WriteFile(secretPath, []byte(secret), 0o600); err != nil {
		t.Fatalf("write config.yaml: %v", err)
	}
	s.cfg.UserPrefs.Avatar = secretPath

	w := httptest.NewRecorder()
	s.handleAvatar(w, avatarReq())
	if w.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404 (config.yaml must not be served)", w.Code)
	}
	if strings.Contains(w.Body.String(), "super-secret-value") {
		t.Fatal("config.yaml content leaked via /api/avatar")
	}
}

// TestHandleAvatarServesUploadPath は正規アップロードパス（user_avatar.bin）の avatar は
// 従来どおり配信されることを検証する（修正で正常系を壊していないこと）。
func TestHandleAvatarServesUploadPath(t *testing.T) {
	s, _ := newAvatarTestServer(t)
	uploadPath, err := avatarUploadPath()
	if err != nil {
		t.Fatalf("avatarUploadPath: %v", err)
	}
	png := []byte{0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x01}
	if err := os.WriteFile(uploadPath, png, 0o600); err != nil {
		t.Fatalf("write avatar: %v", err)
	}
	s.cfg.UserPrefs.Avatar = uploadPath

	w := httptest.NewRecorder()
	s.handleAvatar(w, avatarReq())
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, want 200 for legitimate upload avatar", w.Code)
	}
	if w.Body.Len() != len(png) {
		t.Fatalf("body len = %d, want %d", w.Body.Len(), len(png))
	}
}

// TestSanitizeAvatarPref は書き込み側検証（任意ローカルパスを永続化しない）を確認する。
func TestSanitizeAvatarPref(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	uploadPath, err := avatarUploadPath()
	if err != nil {
		t.Fatalf("avatarUploadPath: %v", err)
	}
	cases := map[string]string{
		"":                          "",
		"https://example.com/a.png": "https://example.com/a.png",
		"http://host/a.png":         "http://host/a.png",
		uploadPath:                  uploadPath,
		filepath.Join(home, ".many-ai-cli", "config.yaml"): "", // 任意ローカルパスは破棄
		"/etc/passwd": "",
	}
	for in, want := range cases {
		if got := sanitizeAvatarPref(in); got != want {
			t.Errorf("sanitizeAvatarPref(%q) = %q, want %q", in, got, want)
		}
	}
}
