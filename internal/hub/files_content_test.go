package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/sessionstore"
)

func newTestFilesContentServer(tmpDir string) *Server {
	return &Server{
		cfg:      &config.Config{Token: "tok"},
		hubCWD:   tmpDir,
		sessions: map[int]*session{},
	}
}

// Files 読み取り API のスコープ判定は呼び出し元で変わる（internal/hub/files_scope.go の
// filesScopeRestricted）。どちらを検証しているかを取り違えないよう、ヘルパは RemoteAddr を
// 必ず明示して組み立てる。
//   - testRemoteAddrRemote:   論理リモート。許可ルート + チャット言及フォールバックに閉じる
//   - testRemoteAddrLoopback: 直 loopback。許可ルート制限なし（秘密情報 denylist のみ）
const (
	testRemoteAddrRemote   = "203.0.113.9:54321"
	testRemoteAddrLoopback = "127.0.0.1:54321"
)

func newFilesReq(t *testing.T, target, remoteAddr string) *http.Request {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, target, nil)
	req.RemoteAddr = remoteAddr
	req.Host = "127.0.0.1:47777"
	return req
}

func callFilesContentFrom(t *testing.T, s *Server, path, remoteAddr string) (int, string, filesContentResp) {
	t.Helper()
	req := newFilesReq(t, "/api/files-content?token=tok&path="+url.QueryEscape(path), remoteAddr)
	w := httptest.NewRecorder()
	s.handleFilesContent(w, req)

	var resp filesContentResp
	body := w.Body.String()
	_ = json.Unmarshal([]byte(body), &resp)
	return w.Code, body, resp
}

func callFilesContent(t *testing.T, s *Server, path string) (int, string, filesContentResp) {
	t.Helper()
	return callFilesContentFrom(t, s, path, testRemoteAddrRemote)
}

func callFilesAsset(t *testing.T, s *Server, path string) (int, string) {
	t.Helper()
	req := newFilesReq(t, "/api/files-asset?token=tok&path="+url.QueryEscape(path), testRemoteAddrRemote)
	w := httptest.NewRecorder()
	s.handleFilesAsset(w, req)
	return w.Code, w.Body.String()
}

func callFilesDownloadFrom(t *testing.T, s *Server, path, remoteAddr string) (int, string, http.Header) {
	t.Helper()
	req := newFilesReq(t, "/api/files-download?token=tok&path="+url.QueryEscape(path), remoteAddr)
	w := httptest.NewRecorder()
	s.handleFilesDownload(w, req)
	return w.Code, w.Body.String(), w.Header()
}

func callFilesDownload(t *testing.T, s *Server, path string) (int, string, http.Header) {
	t.Helper()
	return callFilesDownloadFrom(t, s, path, testRemoteAddrRemote)
}

func callFilesDownloadWithSession(t *testing.T, s *Server, path string, sessionID int) (int, string, http.Header) {
	t.Helper()
	req := newFilesReq(t, "/api/files-download?token=tok&session="+strconv.Itoa(sessionID)+"&path="+url.QueryEscape(path), testRemoteAddrRemote)
	w := httptest.NewRecorder()
	s.handleFilesDownload(w, req)
	return w.Code, w.Body.String(), w.Header()
}

func TestIsTextFile(t *testing.T) {
	cases := map[string]bool{
		"main.go":        true,
		"app.js":         true,
		"config.yaml":    true,
		"Dockerfile":     true,
		"dockerfile":     true,
		"Makefile":       true,
		"README":         true,
		".gitignore":     true,
		"image.png":      false,
		"program.exe":    false,
		"archive.tar.gz": false,
	}
	for name, want := range cases {
		if got := isTextFile(filepath.Join("root", name)); got != want {
			t.Fatalf("isTextFile(%q) = %v, want %v", name, got, want)
		}
	}
}

func TestHandleFilesContent_AllowsPreviewableText(t *testing.T) {
	tmp := t.TempDir()
	files := map[string]string{
		"main.go":    "package main\n",
		"app.js":     "console.log('ok');\n",
		"config.yml": "ok: true\n",
		"Dockerfile": "FROM scratch\n",
		"Makefile":   "test:\n\tgo test ./...\n",
	}

	s := newTestFilesContentServer(tmp)
	for name, content := range files {
		path := filepath.Join(tmp, name)
		if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
		code, _, resp := callFilesContent(t, s, path)
		if code != http.StatusOK {
			t.Fatalf("%s: expected 200, got %d", name, code)
		}
		if resp.Content != content {
			t.Fatalf("%s: content = %q, want %q", name, resp.Content, content)
		}
	}
}

func TestHandleFilesDownload_AllowsArbitraryExtension(t *testing.T) {
	tmp := t.TempDir()
	path := filepath.Join(tmp, "payload.custombin")
	content := "binary-ish\x00payload"
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestFilesContentServer(tmp)
	code, body, header := callFilesDownload(t, s, path)
	if code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", code, body)
	}
	if body != content {
		t.Fatalf("body = %q, want %q", body, content)
	}
	disposition := header.Get("Content-Disposition")
	if !strings.Contains(disposition, "attachment") || !strings.Contains(disposition, `filename="payload.custombin"`) || !strings.Contains(disposition, "filename*=UTF-8''payload.custombin") {
		t.Fatalf("unexpected Content-Disposition: %q", disposition)
	}
	if got := header.Get("Content-Type"); got != "application/octet-stream" {
		t.Fatalf("Content-Type = %q, want application/octet-stream", got)
	}
	if got := header.Get("X-Content-Type-Options"); got != "nosniff" {
		t.Fatalf("X-Content-Type-Options = %q, want nosniff", got)
	}
}

func TestHandleFilesDownload_ContentDispositionNonASCII(t *testing.T) {
	tmp := t.TempDir()
	name := "日本語.txt"
	path := filepath.Join(tmp, name)
	if err := os.WriteFile(path, []byte("hello\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestFilesContentServer(tmp)
	code, body, header := callFilesDownload(t, s, path)
	if code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", code, body)
	}
	disposition := header.Get("Content-Disposition")
	if !strings.Contains(disposition, `filename="___.txt"`) {
		t.Fatalf("missing ASCII fallback in Content-Disposition: %q", disposition)
	}
	if !strings.Contains(disposition, "filename*=UTF-8''"+url.PathEscape(name)) {
		t.Fatalf("missing filename* in Content-Disposition: %q", disposition)
	}
	if got := header.Get("Content-Type"); !strings.HasPrefix(got, "text/plain") {
		t.Fatalf("Content-Type = %q, want text/plain", got)
	}
}

func TestHandleFilesDownload_RejectsDirectory(t *testing.T) {
	tmp := t.TempDir()
	s := newTestFilesContentServer(tmp)

	code, body, _ := callFilesDownload(t, s, tmp)
	if code != http.StatusBadRequest {
		t.Fatalf("expected 400, got %d: %s", code, body)
	}
	if !strings.Contains(body, "path is a directory") {
		t.Fatalf("unexpected body: %q", body)
	}
}

func TestHandleFilesContent_RejectsBinaryExtension(t *testing.T) {
	tmp := t.TempDir()
	path := filepath.Join(tmp, "image.png")
	if err := os.WriteFile(path, []byte{0x89, 'P', 'N', 'G'}, 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestFilesContentServer(tmp)
	code, body, _ := callFilesContent(t, s, path)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d", code)
	}
	if !strings.Contains(body, "not a previewable text file") {
		t.Fatalf("unexpected body: %q", body)
	}
}

// newMentionTestServer は「チャットに言及されたスコープ外パス」テスト用の
// Server（sessionStore + live セッション付き）と、言及登録用ヘルパを返す。
func newMentionTestServer(t *testing.T, projDir string, sessionID int) (*Server, func(text string)) {
	t.Helper()
	store, err := sessionstore.OpenForLogDir(filepath.Join(t.TempDir(), "logs"))
	if err != nil {
		t.Fatalf("OpenForLogDir: %v", err)
	}
	t.Cleanup(func() { store.Close() })
	if _, err := store.StartSession(sessionstore.SessionStart{
		LiveSessionID: sessionID,
		Provider:      "claude",
		CWD:           projDir,
		State:         "standby",
		StartedAt:     time.Now().Format(time.RFC3339),
	}); err != nil {
		t.Fatalf("StartSession: %v", err)
	}
	s := newTestFilesContentServer(projDir)
	s.sessionStore = store
	s.sessions[sessionID] = &session{ID: sessionID, CWD: projDir}
	// mention はユーザー入力（user_input → role='user'）として保存する。
	// read-only バイパスは role='user' 言及のみで許可される仕様のため、
	// 正規 UX（ユーザーがチャットで言及したファイルを開く）はこの経路で検証する。
	mention := func(text string) {
		t.Helper()
		ev := map[string]any{"ts": time.Now().Format(time.RFC3339), "type": "user_input", "session_id": sessionID, "text": text}
		if err := store.StoreEvent(sessionID, ev); err != nil {
			t.Fatalf("StoreEvent: %v", err)
		}
	}
	return s, mention
}

// newMentionTestServerAI は AI 出力（pty_output → role='ai'）言及を登録できる
// ヘルパ付きの Server を返す。AI 出力一致では read-only バイパスが効かないこと
// （インジェクション経路の遮断）を検証するために使う。
func newMentionTestServerAI(t *testing.T, projDir string, sessionID int) (*Server, func(text string)) {
	t.Helper()
	s, _ := newMentionTestServer(t, projDir, sessionID)
	mentionAI := func(text string) {
		t.Helper()
		ev := map[string]any{"ts": time.Now().Format(time.RFC3339), "type": "pty_output", "session_id": sessionID, "text": text}
		if err := s.sessionStore.StoreEvent(sessionID, ev); err != nil {
			t.Fatalf("StoreEvent: %v", err)
		}
	}
	return s, mentionAI
}

func callFilesContentWithSession(t *testing.T, s *Server, path string, sessionID int) (int, string, filesContentResp) {
	t.Helper()
	req := newFilesReq(t, "/api/files-content?token=tok&session="+strconv.Itoa(sessionID)+"&path="+url.QueryEscape(path), testRemoteAddrRemote)
	w := httptest.NewRecorder()
	s.handleFilesContent(w, req)
	var resp filesContentResp
	body := w.Body.String()
	_ = json.Unmarshal([]byte(body), &resp)
	return w.Code, body, resp
}

// TestHandleFilesContent_MentionedOutsidePathReadOnly は、論理リモートからの要求でも
// チャットに言及されたスコープ外パスが読み取り専用（readOnly=true）で 200 になることを確認する。
func TestHandleFilesContent_MentionedOutsidePathReadOnly(t *testing.T) {
	projDir := t.TempDir()
	outsideDir := t.TempDir()
	outsideFile := filepath.Join(outsideDir, "plan_outside.md")
	if err := os.WriteFile(outsideFile, []byte("# outside\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("変更ファイル: " + outsideFile + "\n")

	code, body, resp := callFilesContentWithSession(t, s, outsideFile, 7)
	if code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", code, body)
	}
	if !resp.ReadOnly {
		t.Fatalf("expected readOnly=true, got %+v", resp)
	}
	if resp.Content != "# outside\n" {
		t.Fatalf("content = %q", resp.Content)
	}

	// スコープ内のファイルは従来どおり readOnly=false
	insideFile := filepath.Join(projDir, "inside.md")
	if err := os.WriteFile(insideFile, []byte("inside\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	code, body, resp = callFilesContentWithSession(t, s, insideFile, 7)
	if code != http.StatusOK || resp.ReadOnly {
		t.Fatalf("inside file: code=%d readOnly=%v body=%s", code, resp.ReadOnly, body)
	}
}

// TestHandleFilesContent_UnmentionedOutsidePathForbidden は、論理リモートからの要求では
// 言及のないスコープ外パスが引き続き 403 のままであることを確認する。
// （直 loopback は TestHandleFilesContent_LoopbackOutsidePathReadOnly のとおり許可される）
func TestHandleFilesContent_UnmentionedOutsidePathForbidden(t *testing.T) {
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "secret.md")
	if err := os.WriteFile(outsideFile, []byte("secret\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("関係ない出力\n")

	// session 指定あり・言及なし → 403
	code, body, _ := callFilesContentWithSession(t, s, outsideFile, 7)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d: %s", code, body)
	}
	// session 指定なし → 403
	code, body, _ = callFilesContent(t, s, outsideFile)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403 without session, got %d: %s", code, body)
	}
}

func TestHandleFilesDownload_UnmentionedOutsidePathForbidden(t *testing.T) {
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "artifact.bin")
	if err := os.WriteFile(outsideFile, []byte("secret\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("関係ない出力\n")

	code, body, _ := callFilesDownloadWithSession(t, s, outsideFile, 7)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d: %s", code, body)
	}
}

// TestHandleFilesDownload_MentionedOutsideTextAllowed は、ユーザーがチャットで言及した
// スコープ外の「テキストファイル」は read-only バイパスでダウンロードできることを確認する。
func TestHandleFilesDownload_MentionedOutsideTextAllowed(t *testing.T) {
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "plan_outside.md")
	if err := os.WriteFile(outsideFile, []byte("outside\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("plan: " + outsideFile + "\n")

	code, body, _ := callFilesDownloadWithSession(t, s, outsideFile, 7)
	if code != http.StatusOK {
		t.Fatalf("expected 200, got %d: %s", code, body)
	}
	if body != "outside\n" {
		t.Fatalf("body = %q", body)
	}
}

// TestHandleFilesDownload_MentionedOutsideBinaryForbidden は、ユーザーが言及していても
// スコープ外の「テキスト/メディア以外のバイナリ」は read-only バイパス経由では
// ダウンロードできない（type ゲート）ことを確認する。
func TestHandleFilesDownload_MentionedOutsideBinaryForbidden(t *testing.T) {
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "artifact.bin")
	if err := os.WriteFile(outsideFile, []byte("outside\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("artifact: " + outsideFile + "\n")

	code, body, _ := callFilesDownloadWithSession(t, s, outsideFile, 7)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d: %s", code, body)
	}
}

// TestHandleFilesContent_AIMentionedOutsidePathForbidden は、AI 出力（pty_output / role='ai'）
// にだけスコープ外パスが現れた場合は read-only バイパスが効かず 403 になることを確認する
// （プロンプトインジェクション等で AI に任意パスを出力させても開けない）。
func TestHandleFilesContent_AIMentionedOutsidePathForbidden(t *testing.T) {
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "plan_outside.md")
	if err := os.WriteFile(outsideFile, []byte("# outside\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mentionAI := newMentionTestServerAI(t, projDir, 8)
	mentionAI("変更ファイル: " + outsideFile + "\n")

	code, body, _ := callFilesContentWithSession(t, s, outsideFile, 8)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403 (AI-only mention must not bypass), got %d: %s", code, body)
	}

	// 同じパスを download でも 403 であることを確認する。
	code, body, _ = callFilesDownloadWithSession(t, s, outsideFile, 8)
	if code != http.StatusForbidden {
		t.Fatalf("download: expected 403, got %d: %s", code, body)
	}
}

// TestHandleFilesSave_MentionedOutsidePathStillForbidden は、言及があっても
// 書き込み系（files-save）は 403 のままであることを確認する（読み取り専用の担保）。
func TestHandleFilesSave_MentionedOutsidePathStillForbidden(t *testing.T) {
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "plan_outside.md")
	if err := os.WriteFile(outsideFile, []byte("# outside\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	s, mention := newMentionTestServer(t, projDir, 7)
	mention("変更ファイル: " + outsideFile + "\n")

	bodyJSON := `{"path":` + mustJSONString(t, outsideFile) + `,"content":"overwrite"}`
	req := httptest.NewRequest(http.MethodPost, "/api/files-save?token=tok&session=7", strings.NewReader(bodyJSON))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	s.handleFilesSave(w, req)
	if w.Code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d: %s", w.Code, w.Body.String())
	}
}

func mustJSONString(t *testing.T, v string) string {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

func TestHandleFilesAsset_RejectsSVG(t *testing.T) {
	tmp := t.TempDir()
	path := filepath.Join(tmp, "preview.svg")
	if err := os.WriteFile(path, []byte(`<svg><script>alert(1)</script></svg>`), 0o644); err != nil {
		t.Fatal(err)
	}

	s := newTestFilesContentServer(tmp)
	code, body := callFilesAsset(t, s, path)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403, got %d", code)
	}
	if !strings.Contains(body, "not a previewable media file") {
		t.Fatalf("unexpected body: %q", body)
	}
}

// --- 直 loopback のスコープ開放（files_scope.go / filesScopeRestricted） ---

func TestIsSecretReadDenied(t *testing.T) {
	home := setSecTestHome(t)
	cases := map[string]bool{
		filepath.Join("proj", "docs", "plan.md"):   false,
		filepath.Join("proj", "server.pem"):        true,
		filepath.Join("proj", "tls.key"):           true,
		filepath.Join("proj", "id_rsa"):            true,
		filepath.Join("proj", "id_rsa.pub"):        true,
		filepath.Join("proj", ".credentials.json"): true, // secrets-scan: allow
		filepath.Join("proj", "aws_credentials"):   true,
		// 環境変数ファイル。direnv の .envrc は対象外。
		filepath.Join("proj", ".env"):            true,
		filepath.Join("proj", ".env.local"):      true,
		filepath.Join("proj", ".env.production"): true,
		filepath.Join("proj", ".envrc"):          false,
		// Hub のセッション履歴 DB は log_dir を移設した構成でも拾えるよう名前で拒否する。
		filepath.Join("elsewhere", "any-ai-cli.db"):     true,
		filepath.Join("elsewhere", "any-ai-cli.db-wal"): true, // secrets-scan: allow
		filepath.Join("elsewhere", "any-ai-cli.db-shm"): true, // secrets-scan: allow
		// Hub 設定と、その横に残りうる手動バックアップ（同じ token を含む）。
		filepath.Join(home, ".many-ai-cli", "config.yaml"):     true,
		filepath.Join(home, ".many-ai-cli", "config.yaml.bak"): true,
		filepath.Join(home, ".many-ai-cli", "config.yaml.old"): true,
		filepath.Join(home, ".many-ai-cli", "logs", "hub.log"): false,
		// プロジェクト側の config.yaml は ~/.many-ai-cli 配下ではないので対象外。
		filepath.Join("proj", "config.yaml"): false,
	}
	for path, want := range cases {
		if got := isSecretReadDenied(path); got != want {
			t.Fatalf("isSecretReadDenied(%q) = %v, want %v", path, got, want)
		}
	}
}

// TestHandleFilesContent_LoopbackOutsidePathReadOnly は、直 loopback からの要求では
// チャット言及が無くてもスコープ外ファイルを読めること、ただし readOnly=true が付いて
// 書き込み系が拒否され続けることを確認する。
func TestHandleFilesContent_LoopbackOutsidePathReadOnly(t *testing.T) {
	setSecTestHome(t)
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "recap_outside.md")
	if err := os.WriteFile(outsideFile, []byte("# outside\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	s := newTestFilesContentServer(projDir)

	code, body, resp := callFilesContentFrom(t, s, outsideFile, testRemoteAddrLoopback)
	if code != http.StatusOK {
		t.Fatalf("expected 200 from loopback, got %d: %s", code, body)
	}
	if !resp.ReadOnly {
		t.Fatalf("expected readOnly=true for outside path, got %+v", resp)
	}
	if resp.Content != "# outside\n" {
		t.Fatalf("content = %q", resp.Content)
	}

	// 同じパスを論理リモートから要求すると従来どおり 403。
	code, body, _ = callFilesContentFrom(t, s, outsideFile, testRemoteAddrRemote)
	if code != http.StatusForbidden {
		t.Fatalf("expected 403 from remote, got %d: %s", code, body)
	}
}

// TestHandleFilesContent_SecretPathForbiddenFromLoopback は、スコープを開放した直 loopback でも
// 秘密情報 denylist 該当ファイルは 403 のままであることを確認する。
func TestHandleFilesContent_SecretPathForbiddenFromLoopback(t *testing.T) {
	home := setSecTestHome(t)
	projDir := t.TempDir()
	outsideDir := t.TempDir()

	cfgPath := filepath.Join(home, ".many-ai-cli", "config.yaml")
	if err := os.MkdirAll(filepath.Dir(cfgPath), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(cfgPath, []byte("token: secret\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	cfgBakPath := filepath.Join(home, ".many-ai-cli", "config.yaml.bak")
	if err := os.WriteFile(cfgBakPath, []byte("token: secret\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	credPath := filepath.Join(outsideDir, ".credentials.json") // secrets-scan: allow
	if err := os.WriteFile(credPath, []byte("{}\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	// .env は previewableTextExtensions に入っているため、denylist が無ければ本文が返る。
	envPath := filepath.Join(outsideDir, ".env")
	if err := os.WriteFile(envPath, []byte("API_KEY=super-secret\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	s := newTestFilesContentServer(projDir)
	for _, p := range []string{cfgPath, cfgBakPath, credPath, envPath} {
		code, body, _ := callFilesContentFrom(t, s, p, testRemoteAddrLoopback)
		if code != http.StatusForbidden {
			t.Fatalf("%s: expected 403, got %d: %s", p, code, body)
		}
		if !strings.Contains(body, "secret-like file") {
			t.Fatalf("%s: unexpected body: %q", p, body)
		}
	}
}

// TestHandleFilesDownload_LoopbackOutsideArbitraryExtensionAllowed は、直 loopback の
// スコープ外ダウンロードには言及フォールバック用の type ゲートを課さないことを確認する
// （type ゲートは viaMention の経路にだけ残る）。
func TestHandleFilesDownload_LoopbackOutsideArbitraryExtensionAllowed(t *testing.T) {
	setSecTestHome(t)
	projDir := t.TempDir()
	outsideFile := filepath.Join(t.TempDir(), "artifact.bin")
	if err := os.WriteFile(outsideFile, []byte("payload"), 0o644); err != nil {
		t.Fatal(err)
	}
	s := newTestFilesContentServer(projDir)

	code, body, _ := callFilesDownloadFrom(t, s, outsideFile, testRemoteAddrLoopback)
	if code != http.StatusOK {
		t.Fatalf("expected 200 from loopback, got %d: %s", code, body)
	}
	if body != "payload" {
		t.Fatalf("body = %q", body)
	}

	// 鍵ファイルは denylist で拒否される。
	keyFile := filepath.Join(filepath.Dir(outsideFile), "server.pem")
	if err := os.WriteFile(keyFile, []byte("-----BEGIN PRIVATE KEY-----\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	code, body, _ = callFilesDownloadFrom(t, s, keyFile, testRemoteAddrLoopback)
	if code != http.StatusForbidden {
		t.Fatalf("pem: expected 403, got %d: %s", code, body)
	}
}
