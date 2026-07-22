package hub

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/report"
)

const (
	bugReportLogLines      = 200
	bugReportLogMaxBytes   = 512 * 1024
	bugReportGistTimeout   = 30 * time.Second
	bugReportGistOutputMax = 4096
	bugReportGistFilename  = "many-ai-cli-report-log.md"
	bugReportGistURLPrefix = "https://gist.github.com/"
	bugReportPreviewTTL    = 15 * time.Minute
	bugReportPreviewMax    = 64
)

type bugReportLogPreview struct {
	hash      [sha256.Size]byte
	expiresAt time.Time
}

type bugReportGistRunner interface {
	LookPath(file string) (string, error)
	CreateSecretGist(ctx context.Context, ghPath, markdown string) (string, error)
}

type osBugReportGistRunner struct{}

type cappedBugReportOutput struct {
	buf      bytes.Buffer
	max      int
	overflow bool
}

func (w *cappedBugReportOutput) Write(p []byte) (int, error) {
	originalLen := len(p)
	remaining := w.max - w.buf.Len()
	if remaining <= 0 {
		w.overflow = w.overflow || originalLen > 0
		return originalLen, nil
	}
	if len(p) > remaining {
		p = p[:remaining]
		w.overflow = true
	}
	_, _ = w.buf.Write(p)
	return originalLen, nil
}

func (osBugReportGistRunner) LookPath(file string) (string, error) {
	return exec.LookPath(file)
}

func (osBugReportGistRunner) CreateSecretGist(ctx context.Context, ghPath, markdown string) (string, error) {
	cmd := exec.CommandContext(ctx, ghPath, "gist", "create", "--secret", "--filename", bugReportGistFilename)
	cmd.Stdin = strings.NewReader(markdown)
	stdout := &cappedBugReportOutput{max: bugReportGistOutputMax}
	cmd.Stdout = stdout
	cmd.Stderr = io.Discard
	if err := cmd.Run(); err != nil {
		return "", errors.New("gist command failed")
	}
	if stdout.overflow {
		return "", errors.New("gist command output too large")
	}
	return strings.TrimSpace(stdout.buf.String()), nil
}

type bugReportPreviewRequest struct {
	SessionID             *int   `json:"session_id,omitempty"`
	IncludeRecentLogLines int    `json:"include_recent_log_lines,omitempty"`
	Locale                string `json:"locale,omitempty"`
	UserAgent             string `json:"user_agent,omitempty"`
}

type bugReportPreviewResponse struct {
	Markdown               string   `json:"markdown"`
	EnvironmentMarkdown    string   `json:"environment_markdown"`
	Warnings               []string `json:"warnings"`
	GHAvailable            bool     `json:"gh_available"`
	SessionLogRecorded     bool     `json:"session_log_recorded"`
	LogAttachmentAvailable bool     `json:"log_attachment_available"`
	LogMarkdown            string   `json:"log_markdown,omitempty"`
	LogSavedPath           string   `json:"log_saved_path,omitempty"`
	LogPreviewToken        string   `json:"log_preview_token,omitempty"`
}

type bugReportFinalizeRequest struct {
	Symptom             string `json:"symptom"`
	Reproduction        string `json:"reproduction,omitempty"`
	EnvironmentMarkdown string `json:"environment_markdown"`
	Locale              string `json:"locale,omitempty"`
	IncludeSessionLog   bool   `json:"include_session_log,omitempty"`
	LogMarkdown         string `json:"log_markdown,omitempty"`
	LogPreviewToken     string `json:"log_preview_token,omitempty"`
}

type bugReportFinalizeResponse struct {
	Markdown  string   `json:"markdown"`
	URL       string   `json:"url,omitempty"`
	SavedPath string   `json:"saved_path,omitempty"`
	Warnings  []string `json:"warnings"`
}

func (s *Server) handleBugReportPreview(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req bugReportPreviewRequest
	if !decodeJSON(w, r, &req) {
		return
	}
	if req.IncludeRecentLogLines != 0 && req.IncludeRecentLogLines != bugReportLogLines {
		writeJSONError(w, http.StatusBadRequest, "invalid_log_line_count", "include_recent_log_lines must be 0 or 200")
		return
	}

	provider, model, jsonlPath, activeSession, warnings := s.bugReportSessionMetadata(req.SessionID)
	ghPath, ghAvailable := s.bugReportGHPath()
	env := report.Collect(report.CollectOptions{
		Version:   s.version,
		Provider:  provider,
		Model:     model,
		UserAgent: req.UserAgent,
		Config:    s.snapshotCfg(),
	})
	environment := report.RenderEnvironment(env, req.Locale)
	markdown := report.RenderMarkdown(report.TemplateInput{
		Locale:              req.Locale,
		EnvironmentMarkdown: environment,
	})
	response := bugReportPreviewResponse{
		Markdown:               report.Redact(markdown),
		EnvironmentMarkdown:    report.Redact(environment),
		Warnings:               warnings,
		GHAvailable:            ghAvailable,
		SessionLogRecorded:     jsonlPath != "",
		LogAttachmentAvailable: activeSession && ghAvailable && jsonlPath != "",
	}
	if req.IncludeRecentLogLines == 0 {
		writeJSON(w, response)
		return
	}
	// This is the explicit opt-in boundary. No JSONL file is opened on the
	// default (0-line) preview path above.
	if !activeSession {
		writeJSONError(w, http.StatusNotFound, "active_session_required", "active session is required")
		return
	}
	if !ghAvailable {
		writeJSONError(w, http.StatusConflict, "gh_not_available", "gh CLI is required")
		return
	}
	_ = ghPath // Availability is rechecked before the final external action.
	logText, truncated, err := s.readBugReportLogTail(jsonlPath, bugReportLogLines)
	if err != nil {
		writeJSONError(w, http.StatusNotFound, "session_log_unavailable", "session log is not available")
		return
	}
	logMarkdown := canonicalBugReportLog(renderBugReportLogMarkdown(logText))
	logSavedPath, err := s.saveBugReportMarkdown(logMarkdown)
	if err != nil {
		s.logger.Error("bug report log save failed")
		writeJSONError(w, http.StatusInternalServerError, "report_save_failed", "failed to save redacted report log")
		return
	}
	if truncated {
		response.Warnings = append(response.Warnings, "session_log_byte_limit")
	}
	response.LogMarkdown = logMarkdown
	response.LogSavedPath = logSavedPath
	response.LogPreviewToken, err = s.rememberBugReportLogPreview(logMarkdown)
	if err != nil {
		s.logger.Error("bug report log preview token failed")
		writeJSONError(w, http.StatusInternalServerError, "log_preview_failed", "failed to prepare session log preview")
		return
	}
	writeJSON(w, response)
}

func (s *Server) handleBugReportFinalize(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req bugReportFinalizeRequest
	if !decodeJSON(w, r, &req) {
		return
	}
	if strings.TrimSpace(req.Symptom) == "" {
		writeJSONError(w, http.StatusBadRequest, "symptom_required", "symptom is required")
		return
	}

	// RenderMarkdown and BuildIssueURL both scrub. Keeping the second pass in
	// BuildIssueURL is intentional: it is the final boundary before browser data
	// can leave the local Hub.
	markdown := report.RenderMarkdown(report.TemplateInput{
		Locale:              req.Locale,
		Symptom:             req.Symptom,
		Reproduction:        req.Reproduction,
		EnvironmentMarkdown: req.EnvironmentMarkdown,
	})
	response := bugReportFinalizeResponse{Warnings: []string{}}
	if req.IncludeSessionLog {
		logMarkdown := canonicalBugReportLog(req.LogMarkdown)
		if logMarkdown == "" {
			writeJSONError(w, http.StatusBadRequest, "log_preview_required", "previewed session log is required")
			return
		}
		if !s.validateBugReportLogPreview(req.LogPreviewToken, logMarkdown) {
			writeJSONError(w, http.StatusBadRequest, "log_preview_required", "valid session log preview is required")
			return
		}
		ghPath, ok := s.bugReportGHPath()
		if !ok {
			s.writeBugReportGistFallback(w, req.Symptom, markdown, logMarkdown, "gh_not_available")
			return
		}
		ctx, cancel := context.WithTimeout(r.Context(), bugReportGistTimeout)
		gistURL, err := s.gistRunner().CreateSecretGist(ctx, ghPath, logMarkdown)
		cancel()
		if err != nil {
			s.writeBugReportGistFallback(w, req.Symptom, markdown, logMarkdown, "gist_create_failed")
			return
		}
		gistURL, ok = validatedGistURL(gistURL)
		if !ok {
			s.writeBugReportGistFallback(w, req.Symptom, markdown, logMarkdown, "gist_url_rejected")
			return
		}
		markdown = report.Redact(strings.TrimRight(markdown, "\n") + "\n\n[log-attachment](" + gistURL + ")\n")
	}

	issueURL, tooLong := report.BuildIssueURL(report.DefaultTitle(req.Symptom), markdown)
	response.Markdown = report.Redact(markdown)
	if !tooLong {
		response.URL = issueURL
		writeJSON(w, response)
		return
	}

	savedPath, err := s.saveBugReportMarkdown(markdown)
	if err != nil {
		s.logger.Error("bug report fallback save failed")
		writeJSONError(w, http.StatusInternalServerError, "report_save_failed", "failed to save redacted report")
		return
	}
	response.SavedPath = savedPath
	// Keep the user flow moving: open the fixed GitHub form with only the
	// scrubbed title, then let the user paste the locally saved markdown.
	response.URL, _ = report.BuildIssueURL(report.DefaultTitle(req.Symptom), "")
	response.Warnings = append(response.Warnings, "issue_url_too_long")
	writeJSON(w, response)
}

func (s *Server) bugReportSessionMetadata(sessionID *int) (provider, model, jsonlPath string, active bool, warnings []string) {
	if sessionID == nil {
		return "", "", "", false, []string{}
	}
	s.sessionsMu.Lock()
	ses := s.sessions[*sessionID]
	if ses != nil {
		provider, model, jsonlPath, active = ses.Provider, ses.Model, ses.JSONLPath, true
	}
	s.sessionsMu.Unlock()
	if ses == nil {
		warnings = append(warnings, "session_not_found")
	}
	return provider, model, jsonlPath, active, warnings
}

func (s *Server) gistRunner() bugReportGistRunner {
	if s.bugReportGistRunner != nil {
		return s.bugReportGistRunner
	}
	return osBugReportGistRunner{}
}

func (s *Server) rememberBugReportLogPreview(logMarkdown string) (string, error) {
	random := make([]byte, 32)
	if _, err := rand.Read(random); err != nil {
		return "", errors.New("generate preview token")
	}
	token := hex.EncodeToString(random)
	now := time.Now()
	s.bugReportLogPreviewMu.Lock()
	if s.bugReportLogPreviews == nil {
		s.bugReportLogPreviews = make(map[string]bugReportLogPreview)
	}
	for key, preview := range s.bugReportLogPreviews {
		if !preview.expiresAt.After(now) {
			delete(s.bugReportLogPreviews, key)
		}
	}
	if len(s.bugReportLogPreviews) >= bugReportPreviewMax {
		var oldestKey string
		var oldestExpiry time.Time
		for key, preview := range s.bugReportLogPreviews {
			if oldestKey == "" || preview.expiresAt.Before(oldestExpiry) {
				oldestKey, oldestExpiry = key, preview.expiresAt
			}
		}
		delete(s.bugReportLogPreviews, oldestKey)
	}
	s.bugReportLogPreviews[token] = bugReportLogPreview{
		hash: sha256.Sum256([]byte(canonicalBugReportLog(logMarkdown))), expiresAt: now.Add(bugReportPreviewTTL),
	}
	s.bugReportLogPreviewMu.Unlock()
	return token, nil
}

func (s *Server) validateBugReportLogPreview(token, logMarkdown string) bool {
	if len(token) != 64 {
		return false
	}
	now := time.Now()
	s.bugReportLogPreviewMu.Lock()
	preview, ok := s.bugReportLogPreviews[token]
	if ok {
		// A displayed preview authorizes exactly one external gist attempt.
		delete(s.bugReportLogPreviews, token)
	}
	if ok && !preview.expiresAt.After(now) {
		ok = false
	}
	s.bugReportLogPreviewMu.Unlock()
	if !ok {
		return false
	}
	got := sha256.Sum256([]byte(canonicalBugReportLog(logMarkdown)))
	return subtle.ConstantTimeCompare(got[:], preview.hash[:]) == 1
}

func canonicalBugReportLog(logMarkdown string) string {
	return strings.TrimSpace(report.Redact(logMarkdown))
}

func (s *Server) bugReportGHPath() (string, bool) {
	path, err := s.gistRunner().LookPath("gh")
	return path, err == nil && strings.TrimSpace(path) != ""
}

func (s *Server) saveBugReportMarkdown(markdown string) (string, error) {
	if s.bugReportSaveMarkdown != nil {
		return s.bugReportSaveMarkdown(report.Redact(markdown))
	}
	return report.SaveMarkdown(markdown)
}

func (s *Server) readBugReportLogTail(jsonlPath string, lineLimit int) (string, bool, error) {
	cfg := s.snapshotCfg()
	if cfg.Hub.LogDir == "" || jsonlPath == "" || !strings.EqualFold(filepath.Ext(jsonlPath), ".jsonl") {
		return "", false, errors.New("invalid session log path")
	}
	allowedDir, err := filepath.EvalSymlinks(filepath.Join(cfg.Hub.LogDir, "sessions"))
	if err != nil {
		return "", false, errors.New("resolve session log directory")
	}
	resolvedPath, err := filepath.EvalSymlinks(filepath.Clean(jsonlPath))
	if err != nil {
		return "", false, errors.New("resolve session log path")
	}
	if ok, _ := isPathUnderAllowedRoots(resolvedPath, allowedDir); !ok {
		return "", false, errors.New("session log outside log directory")
	}
	f, err := os.Open(resolvedPath) // #nosec G304 -- resolved path is constrained to Hub logs/sessions.
	if err != nil {
		return "", false, errors.New("open session log")
	}
	defer f.Close()
	info, err := f.Stat()
	if err != nil || !info.Mode().IsRegular() {
		return "", false, errors.New("stat session log")
	}
	readBytes := min(info.Size(), int64(bugReportLogMaxBytes))
	start := info.Size() - readBytes
	buf := make([]byte, readBytes)
	if _, err := f.ReadAt(buf, start); err != nil && err != io.EOF {
		return "", false, errors.New("read session log")
	}
	truncated := start > 0
	if truncated {
		if firstNewline := bytes.IndexByte(buf, '\n'); firstNewline >= 0 {
			buf = buf[firstNewline+1:]
		}
	}
	lines := strings.Split(strings.TrimRight(string(buf), "\r\n"), "\n")
	if len(lines) > lineLimit {
		lines = lines[len(lines)-lineLimit:]
		truncated = true
	}
	return report.Redact(strings.Join(lines, "\n")), truncated, nil
}

func renderBugReportLogMarkdown(logText string) string {
	lines := strings.Split(report.Redact(logText), "\n")
	for i := range lines {
		lines[i] = "    " + lines[i]
	}
	return report.Redact(fmt.Sprintf("## many-ai-cli session log (last %d lines, redacted)\n\n%s\n",
		bugReportLogLines, strings.Join(lines, "\n")))
}

func validatedGistURL(raw string) (string, bool) {
	if len(raw) == 0 || len(raw) > 2048 || !strings.HasPrefix(raw, bugReportGistURLPrefix) ||
		strings.ContainsAny(raw, "[]()<> \t\r\n") {
		return "", false
	}
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "https" || u.Host != "gist.github.com" || u.User != nil ||
		u.RawQuery != "" || u.Fragment != "" || u.Path == "" || u.Path == "/" {
		return "", false
	}
	return u.String(), true
}

func (s *Server) writeBugReportGistFallback(w http.ResponseWriter, symptom, markdown, logMarkdown, warning string) {
	fallback := report.Redact(strings.TrimRight(markdown, "\n") +
		"\n\n## Scrubbed session log (local fallback)\n\n" + strings.TrimSpace(logMarkdown) + "\n")
	savedPath, err := s.saveBugReportMarkdown(fallback)
	if err != nil {
		s.logger.Error("bug report gist fallback save failed")
		writeJSONError(w, http.StatusInternalServerError, "report_save_failed", "failed to save redacted report")
		return
	}
	issueURL, tooLong := report.BuildIssueURL(report.DefaultTitle(symptom), markdown)
	if tooLong {
		issueURL, _ = report.BuildIssueURL(report.DefaultTitle(symptom), "")
	}
	writeJSON(w, bugReportFinalizeResponse{
		Markdown:  report.Redact(markdown),
		URL:       issueURL,
		SavedPath: savedPath,
		Warnings:  []string{warning},
	})
}
