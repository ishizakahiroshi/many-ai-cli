package hub

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"testing"

	"many-ai-cli/internal/config"
)

type fakeBugReportGistRunner struct {
	ghPath      string
	lookPathErr error
	gistURL     string
	gistErr     error
	lookups     int
	creates     int
	markdown    string
}

func (f *fakeBugReportGistRunner) LookPath(file string) (string, error) {
	f.lookups++
	if file != "gh" {
		return "", errors.New("unexpected executable")
	}
	if f.lookPathErr != nil {
		return "", f.lookPathErr
	}
	if f.ghPath == "" {
		return "C:\\synthetic\\gh.exe", nil
	}
	return f.ghPath, nil
}

func (f *fakeBugReportGistRunner) CreateSecretGist(_ context.Context, _ string, markdown string) (string, error) {
	f.creates++
	f.markdown = markdown
	if f.gistErr != nil {
		return "", f.gistErr
	}
	if f.gistURL == "" {
		return "https://gist.github.com/example/synthetic123", nil
	}
	return f.gistURL, nil
}

func newBugReportTestServer() *Server {
	cfg := &config.Config{}
	cfg.Hub.Port = 47777
	cfg.Token = "synthetic-test-token"
	runner := &fakeBugReportGistRunner{}
	return &Server{
		cfg:     cfg,
		version: "0.synthetic",
		logger:  slog.New(slog.NewTextHandler(io.Discard, nil)),
		sessions: map[int]*session{
			7: {ID: 7, Provider: "codex", Model: "gpt-synthetic", inputMu: new(sync.Mutex)},
		},
		bugReportGistRunner: runner,
		bugReportSaveMarkdown: func(markdown string) (string, error) {
			return filepath.Join("~", ".many-ai-cli", "reports", "synthetic-report.md"), nil
		},
	}
}

func bugReportRunner(s *Server) *fakeBugReportGistRunner {
	return s.bugReportGistRunner.(*fakeBugReportGistRunner)
}

func authorizeBugReportLog(t *testing.T, s *Server, markdown string) string {
	t.Helper()
	token, err := s.rememberBugReportLogPreview(markdown)
	if err != nil {
		t.Fatal(err)
	}
	return token
}

func bugReportRequest(path string, body any) *http.Request {
	b, _ := json.Marshal(body)
	r := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:47777"+path, bytes.NewReader(b))
	r.Header.Set("Content-Type", "application/json")
	r.Header.Set("Origin", "http://127.0.0.1:47777")
	r.AddCookie(&http.Cookie{Name: tokenCookieName, Value: "synthetic-test-token"})
	return r
}

func TestHandleBugReportPreviewCollectsAllowlistedSession(t *testing.T) {
	s := newBugReportTestServer()
	id := 7
	w := httptest.NewRecorder()
	s.handleBugReportPreview(w, bugReportRequest("/api/bug-report/preview", bugReportPreviewRequest{
		SessionID: &id, Locale: "en", UserAgent: `Synthetic C:\Users\example-user\Browser`,
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	var got bugReportPreviewResponse
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{"gpt-synthetic", "codex", "0.synthetic", "~/Browser"} {
		if !strings.Contains(got.EnvironmentMarkdown, want) {
			t.Errorf("environment missing %q: %q", want, got.EnvironmentMarkdown)
		}
	}
	if strings.Contains(got.EnvironmentMarkdown, "example-user") {
		t.Fatalf("environment leaked home account: %q", got.EnvironmentMarkdown)
	}
}

func TestHandleBugReportPreviewRejectsLogCollection(t *testing.T) {
	s := newBugReportTestServer()
	w := httptest.NewRecorder()
	s.handleBugReportPreview(w, bugReportRequest("/api/bug-report/preview", bugReportPreviewRequest{IncludeRecentLogLines: 1}))
	if w.Code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400", w.Code)
	}
}

func TestHandleBugReportPreviewDefaultDoesNotReadOrUploadLog(t *testing.T) {
	s := newBugReportTestServer()
	root := t.TempDir()
	s.cfg.Hub.LogDir = root
	sessionsDir := filepath.Join(root, "sessions")
	if err := os.MkdirAll(sessionsDir, 0o700); err != nil {
		t.Fatal(err)
	}
	logPath := filepath.Join(sessionsDir, "synthetic.jsonl")
	if err := os.WriteFile(logPath, []byte("do-not-read"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.sessions[7].JSONLPath = logPath

	// Make any accidental read observable without relying on timing: remove the
	// file after metadata is configured. The default preview must still succeed.
	if err := os.Remove(logPath); err != nil {
		t.Fatal(err)
	}
	id := 7
	w := httptest.NewRecorder()
	s.handleBugReportPreview(w, bugReportRequest("/api/bug-report/preview", bugReportPreviewRequest{SessionID: &id}))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	var got bugReportPreviewResponse
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if !got.LogAttachmentAvailable || got.LogMarkdown != "" || got.LogSavedPath != "" {
		t.Fatalf("unexpected default log response: %#v", got)
	}
	if bugReportRunner(s).creates != 0 {
		t.Fatal("default preview executed gist creation")
	}
}

func TestHandleBugReportPreviewLogRequiresGHAndActiveSession(t *testing.T) {
	s := newBugReportTestServer()
	bugReportRunner(s).lookPathErr = execSyntheticNotFoundError()
	id := 7
	w := httptest.NewRecorder()
	s.handleBugReportPreview(w, bugReportRequest("/api/bug-report/preview", bugReportPreviewRequest{SessionID: &id}))
	var got bugReportPreviewResponse
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.GHAvailable || got.LogAttachmentAvailable {
		t.Fatalf("attachment should be disabled without gh: %#v", got)
	}

	s = newBugReportTestServer()
	w = httptest.NewRecorder()
	s.handleBugReportPreview(w, bugReportRequest("/api/bug-report/preview", bugReportPreviewRequest{}))
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if !got.GHAvailable || got.LogAttachmentAvailable {
		t.Fatalf("attachment should be disabled without an active session: %#v", got)
	}
}

func TestHandleBugReportPreviewDisablesAttachmentWhenSessionLoggingIsOff(t *testing.T) {
	s := newBugReportTestServer()
	id := 7
	w := httptest.NewRecorder()
	s.handleBugReportPreview(w, bugReportRequest("/api/bug-report/preview", bugReportPreviewRequest{SessionID: &id}))
	var got bugReportPreviewResponse
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if !got.GHAvailable || got.SessionLogRecorded || got.LogAttachmentAvailable {
		t.Fatalf("attachment should be disabled when JSONL logging is off: %#v", got)
	}
}

func execSyntheticNotFoundError() error {
	return errors.New("synthetic executable not found")
}

func TestHandleBugReportPreviewShowsRedactedLast200LinesAndSavesSameContent(t *testing.T) {
	s := newBugReportTestServer()
	root := t.TempDir()
	s.cfg.Hub.LogDir = root
	sessionsDir := filepath.Join(root, "sessions")
	if err := os.MkdirAll(sessionsDir, 0o700); err != nil {
		t.Fatal(err)
	}
	lines := make([]string, 205)
	for i := range lines {
		lines[i] = `{"type":"synthetic","line":` + strconv.Itoa(i) + `}`
	}
	secret := "sk-syntheticSecret123456789"
	lines[204] = `{"type":"synthetic","secret":"` + secret + `","path":"C:\\dev\\github\\private\\example"}`
	logPath := filepath.Join(sessionsDir, "codex_s7.jsonl")
	if err := os.WriteFile(logPath, []byte(strings.Join(lines, "\n")+"\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	s.sessions[7].JSONLPath = logPath
	var saved string
	s.bugReportSaveMarkdown = func(markdown string) (string, error) {
		saved = markdown
		return filepath.Join("~", ".many-ai-cli", "reports", "synthetic-log.md"), nil
	}

	id := 7
	w := httptest.NewRecorder()
	s.handleBugReportPreview(w, bugReportRequest("/api/bug-report/preview", bugReportPreviewRequest{
		SessionID: &id, IncludeRecentLogLines: bugReportLogLines,
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	var got bugReportPreviewResponse
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(got.LogMarkdown, "\n    {\"type\":\"synthetic\",\"line\":4}\n") ||
		!strings.Contains(got.LogMarkdown, "\n    {\"type\":\"synthetic\",\"line\":5}\n") {
		t.Fatalf("response did not contain exactly the last 200 lines: %s", got.LogMarkdown[:min(len(got.LogMarkdown), 500)])
	}
	for _, forbidden := range []string{secret, `C:\\dev\\github\\private`} {
		if strings.Contains(got.LogMarkdown, forbidden) {
			t.Fatalf("response leaked %q", forbidden)
		}
	}
	if got.LogMarkdown != saved || got.LogSavedPath == "" || got.LogPreviewToken == "" {
		t.Fatalf("saved log differs from preview: saved=%q response=%#v", saved, got)
	}
	if bugReportRunner(s).creates != 0 {
		t.Fatal("preview must not create a gist")
	}
}

func TestHandleBugReportPreviewRejectsEscapedJSONLPath(t *testing.T) {
	s := newBugReportTestServer()
	root := t.TempDir()
	s.cfg.Hub.LogDir = root
	if err := os.MkdirAll(filepath.Join(root, "sessions"), 0o700); err != nil {
		t.Fatal(err)
	}
	outside := filepath.Join(t.TempDir(), "outside.jsonl")
	if err := os.WriteFile(outside, []byte(`{"secret":"synthetic"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	s.sessions[7].JSONLPath = outside
	id := 7
	w := httptest.NewRecorder()
	s.handleBugReportPreview(w, bugReportRequest("/api/bug-report/preview", bugReportPreviewRequest{
		SessionID: &id, IncludeRecentLogLines: bugReportLogLines,
	}))
	if w.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404: %s", w.Code, w.Body.String())
	}
	if bugReportRunner(s).creates != 0 {
		t.Fatal("rejected path must not create a gist")
	}
}

func TestHandleBugReportPreviewRejectsSymlinkEscape(t *testing.T) {
	s := newBugReportTestServer()
	root := t.TempDir()
	s.cfg.Hub.LogDir = root
	sessionsDir := filepath.Join(root, "sessions")
	if err := os.MkdirAll(sessionsDir, 0o700); err != nil {
		t.Fatal(err)
	}
	outside := filepath.Join(t.TempDir(), "outside.jsonl")
	if err := os.WriteFile(outside, []byte(`{"secret":"synthetic"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	link := filepath.Join(sessionsDir, "linked.jsonl")
	if err := os.Symlink(outside, link); err != nil {
		t.Skipf("symlinks unavailable: %v", err)
	}
	s.sessions[7].JSONLPath = link
	id := 7
	w := httptest.NewRecorder()
	s.handleBugReportPreview(w, bugReportRequest("/api/bug-report/preview", bugReportPreviewRequest{
		SessionID: &id, IncludeRecentLogLines: bugReportLogLines,
	}))
	if w.Code != http.StatusNotFound {
		t.Fatalf("status = %d, want 404: %s", w.Code, w.Body.String())
	}
}

func TestHandleBugReportFinalizeRequiresSymptomAndScrubs(t *testing.T) {
	s := newBugReportTestServer()
	w := httptest.NewRecorder()
	s.handleBugReportFinalize(w, bugReportRequest("/api/bug-report/finalize", bugReportFinalizeRequest{}))
	if w.Code != http.StatusBadRequest {
		t.Fatalf("empty symptom status = %d, want 400", w.Code)
	}

	w = httptest.NewRecorder()
	s.handleBugReportFinalize(w, bugReportRequest("/api/bug-report/finalize", bugReportFinalizeRequest{
		Symptom: "screen stalls sk-syntheticSecret12345", EnvironmentMarkdown: "- OS: windows",
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	var got bugReportFinalizeResponse
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.URL == "" || strings.Contains(got.URL, "syntheticSecret12345") || strings.Contains(got.Markdown, "syntheticSecret12345") {
		t.Fatalf("finalize response was not safely scrubbed: %#v", got)
	}
	if bugReportRunner(s).creates != 0 || bugReportRunner(s).lookups != 0 {
		t.Fatal("default finalize must not inspect or execute gh")
	}
}

func TestHandleBugReportFinalizeOffIgnoresSuppliedLogData(t *testing.T) {
	s := newBugReportTestServer()
	secret := "sk-syntheticIgnoredSecret123456"
	saved := false
	s.bugReportSaveMarkdown = func(string) (string, error) {
		saved = true
		return "", errors.New("must not save")
	}
	w := httptest.NewRecorder()
	s.handleBugReportFinalize(w, bugReportRequest("/api/bug-report/finalize", bugReportFinalizeRequest{
		Symptom: "synthetic failure", IncludeSessionLog: false, LogMarkdown: "ignored=" + secret,
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	if saved || bugReportRunner(s).lookups != 0 || bugReportRunner(s).creates != 0 || strings.Contains(w.Body.String(), secret) {
		t.Fatalf("OFF path touched log attachment state: saved=%v runner=%#v body=%s", saved, bugReportRunner(s), w.Body.String())
	}
}

func TestHandleBugReportFinalizeCreatesGistOnlyAfterOptInAndFinalRedact(t *testing.T) {
	s := newBugReportTestServer()
	secret := "github_pat_syntheticSecret123456789"
	logMarkdown := "log token=" + secret + "\n"
	w := httptest.NewRecorder()
	s.handleBugReportFinalize(w, bugReportRequest("/api/bug-report/finalize", bugReportFinalizeRequest{
		Symptom:             "synthetic failure",
		EnvironmentMarkdown: "- OS: synthetic",
		IncludeSessionLog:   true,
		LogMarkdown:         logMarkdown,
		LogPreviewToken:     authorizeBugReportLog(t, s, logMarkdown),
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	var got bugReportFinalizeResponse
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	runner := bugReportRunner(s)
	if runner.creates != 1 || strings.Contains(runner.markdown, secret) {
		t.Fatalf("gist call was not safely scrubbed: creates=%d markdown=%q", runner.creates, runner.markdown)
	}
	if !strings.Contains(got.Markdown, "[log-attachment](https://gist.github.com/example/synthetic123)") {
		t.Fatalf("issue markdown missing fixed gist link: %q", got.Markdown)
	}
}

func TestHandleBugReportFinalizeRejectsUntrustedGistURLAndSavesRedactedFallback(t *testing.T) {
	s := newBugReportTestServer()
	runner := bugReportRunner(s)
	runner.gistURL = "https://example.invalid/gist/synthetic"
	secret := "sk-syntheticSecret123456789"
	logMarkdown := "log=" + secret
	var saved string
	s.bugReportSaveMarkdown = func(markdown string) (string, error) {
		saved = markdown
		return filepath.Join("~", ".many-ai-cli", "reports", "fallback.md"), nil
	}
	w := httptest.NewRecorder()
	s.handleBugReportFinalize(w, bugReportRequest("/api/bug-report/finalize", bugReportFinalizeRequest{
		Symptom: "synthetic failure", IncludeSessionLog: true, LogMarkdown: logMarkdown,
		LogPreviewToken: authorizeBugReportLog(t, s, logMarkdown),
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	var got bugReportFinalizeResponse
	if err := json.Unmarshal(w.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.SavedPath == "" || len(got.Warnings) != 1 || got.Warnings[0] != "gist_url_rejected" {
		t.Fatalf("unexpected fallback response: %#v", got)
	}
	if strings.Contains(saved, secret) || strings.Contains(w.Body.String(), "example.invalid") {
		t.Fatalf("fallback leaked untrusted data: saved=%q response=%s", saved, w.Body.String())
	}
}

func TestHandleBugReportFinalizeGistFailureSavesRedactedFallback(t *testing.T) {
	s := newBugReportTestServer()
	bugReportRunner(s).gistErr = errors.New("synthetic network failure with untrusted detail")
	secret := "sk-syntheticNetworkSecret123456"
	logMarkdown := "log=" + secret
	var saved string
	s.bugReportSaveMarkdown = func(markdown string) (string, error) {
		saved = markdown
		return filepath.Join("~", ".many-ai-cli", "reports", "network-fallback.md"), nil
	}
	w := httptest.NewRecorder()
	s.handleBugReportFinalize(w, bugReportRequest("/api/bug-report/finalize", bugReportFinalizeRequest{
		Symptom: "synthetic failure", IncludeSessionLog: true, LogMarkdown: logMarkdown,
		LogPreviewToken: authorizeBugReportLog(t, s, logMarkdown),
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d, body = %s", w.Code, w.Body.String())
	}
	if strings.Contains(saved, secret) || strings.Contains(w.Body.String(), secret) ||
		strings.Contains(w.Body.String(), "untrusted detail") || !strings.Contains(w.Body.String(), "gist_create_failed") {
		t.Fatalf("gist failure fallback was unsafe: saved=%q response=%s", saved, w.Body.String())
	}
}

func TestHandleBugReportFinalizeRequiresMatchingOneTimePreviewToken(t *testing.T) {
	s := newBugReportTestServer()
	logMarkdown := "synthetic redacted preview"
	token := authorizeBugReportLog(t, s, logMarkdown)

	w := httptest.NewRecorder()
	s.handleBugReportFinalize(w, bugReportRequest("/api/bug-report/finalize", bugReportFinalizeRequest{
		Symptom: "synthetic failure", IncludeSessionLog: true, LogMarkdown: logMarkdown + " changed",
		LogPreviewToken: token,
	}))
	if w.Code != http.StatusBadRequest || bugReportRunner(s).creates != 0 {
		t.Fatalf("mismatched preview status=%d creates=%d body=%s", w.Code, bugReportRunner(s).creates, w.Body.String())
	}

	// A mismatched attempt consumes the token; it cannot be replayed with the
	// original body to create a gist later.
	w = httptest.NewRecorder()
	s.handleBugReportFinalize(w, bugReportRequest("/api/bug-report/finalize", bugReportFinalizeRequest{
		Symptom: "synthetic failure", IncludeSessionLog: true, LogMarkdown: logMarkdown,
		LogPreviewToken: token,
	}))
	if w.Code != http.StatusBadRequest || bugReportRunner(s).creates != 0 {
		t.Fatalf("replayed preview status=%d creates=%d body=%s", w.Code, bugReportRunner(s).creates, w.Body.String())
	}
}

func TestValidatedGistURLAcceptsOnlyFixedHTTPSHost(t *testing.T) {
	tests := []struct {
		raw  string
		want bool
	}{
		{"https://gist.github.com/example/synthetic", true},
		{"http://gist.github.com/example/synthetic", false},
		{"https://gist.github.com.evil.invalid/example/synthetic", false},
		{"https://gist.github.com@example.invalid/example/synthetic", false},
		{"https://gist.github.com/", false},
		{"https://gist.github.com/example/synthetic?token=synthetic", false},
		{"https://gist.github.com/example/synthetic)injected", false},
	}
	for _, tt := range tests {
		_, got := validatedGistURL(tt.raw)
		if got != tt.want {
			t.Errorf("validatedGistURL(%q) = %v, want %v", tt.raw, got, tt.want)
		}
	}
}
