package hub

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/sessionstore"
)

const (
	workbenchContextFileMaxBytes  = 64 * 1024
	workbenchContextTotalMaxBytes = 256 * 1024
	workbenchDiagnosticsTimeout   = 1500 * time.Millisecond
	workbenchExportFileMaxBytes   = 2 * 1024 * 1024
)

type workbenchItem struct {
	ID        string            `json:"id"`
	Title     string            `json:"title,omitempty"`
	Body      string            `json:"body,omitempty"`
	Status    string            `json:"status,omitempty"`
	Provider  string            `json:"provider,omitempty"`
	CWD       string            `json:"cwd,omitempty"`
	Model     string            `json:"model,omitempty"`
	Branch    string            `json:"branch,omitempty"`
	Tags      []string          `json:"tags,omitempty"`
	Meta      map[string]string `json:"meta,omitempty"`
	CreatedAt string            `json:"created_at,omitempty"`
	UpdatedAt string            `json:"updated_at,omitempty"`
}

type workbenchDoc struct {
	Items []workbenchItem `json:"items"`
}

func (s *Server) registerWorkbenchRoutes(mux *http.ServeMux) {
	mux.HandleFunc("/api/workbench/sessions", s.handleWorkbenchSessions)
	mux.HandleFunc("/api/workbench/session-timeline", s.handleWorkbenchSessionTimeline)
	mux.HandleFunc("/api/workbench/session-meta", s.handleWorkbenchSessionMeta)
	mux.HandleFunc("/api/workbench/session-summary", s.handleWorkbenchSessionSummary)
	mux.HandleFunc("/api/workbench/session-export", s.handleWorkbenchSessionExport)
	mux.HandleFunc("/api/workbench/state", s.handleWorkbenchState)
	mux.HandleFunc("/api/workbench/templates", s.handleWorkbenchTemplates)
	mux.HandleFunc("/api/workbench/tasks", s.handleWorkbenchTasks)
	mux.HandleFunc("/api/workbench/palette", s.handleWorkbenchPalette)
	mux.HandleFunc("/api/workbench/policies", s.handleWorkbenchPolicies)
	mux.HandleFunc("/api/workbench/approval-simulate", s.handleWorkbenchApprovalSimulate)
	mux.HandleFunc("/api/workbench/diagnostics", s.handleWorkbenchDiagnostics)
	mux.HandleFunc("/api/workbench/files-context", s.handleWorkbenchFilesContext)
	mux.HandleFunc("/api/workbench/git-review", s.handleWorkbenchGitReview)
	mux.HandleFunc("/api/workbench/worktrees", s.handleWorkbenchWorktrees)
	mux.HandleFunc("/api/workbench/file-watch", s.handleWorkbenchFileWatch)
	mux.HandleFunc("/api/workbench/test-results", s.handleWorkbenchTestResults)
	mux.HandleFunc("/api/workbench/redaction-preview", s.handleWorkbenchRedactionPreview)
	mux.HandleFunc("/api/workbench/usage", s.handleWorkbenchUsage)
	mux.HandleFunc("/api/workbench/stale-sessions", s.handleWorkbenchStaleSessions)
}

func (s *Server) handleWorkbenchSessions(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.sessionStore == nil {
		writeJSON(w, map[string]any{"ok": true, "sessions": []any{}})
		return
	}
	limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
	includeArchived := r.URL.Query().Get("archived") == "1"
	items, err := s.sessionStore.ListSessions(limit, includeArchived)
	if err != nil {
		s.logger.Warn("workbench sessions failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "session_list_failed", "failed to list sessions")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "sessions": items})
}

func (s *Server) handleWorkbenchSessionTimeline(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.sessionStore == nil {
		writeJSON(w, map[string]any{"ok": true, "events": []any{}})
		return
	}
	id, _ := strconv.Atoi(r.URL.Query().Get("session_id"))
	if id <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "session_id required")
		return
	}
	limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
	events, err := s.sessionStore.TimelineByLiveSession(id, limit)
	if err != nil {
		s.logger.Warn("workbench timeline failed", "session_id", id, "err", err)
		writeJSONError(w, http.StatusInternalServerError, "timeline_failed", "failed to load timeline")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "events": events})
}

func (s *Server) handleWorkbenchSessionMeta(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPut, http.MethodPost) {
		return
	}
	if s.sessionStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "session_store_disabled", "sqlite session store is disabled")
		return
	}
	if r.Method == http.MethodGet {
		id, _ := strconv.Atoi(r.URL.Query().Get("session_id"))
		if id <= 0 {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "session_id required")
			return
		}
		item, err := s.sessionStore.SessionOverviewByLiveSession(id)
		if err != nil {
			writeJSONError(w, http.StatusInternalServerError, "session_meta_failed", "failed to load session meta")
			return
		}
		writeJSON(w, map[string]any{"ok": true, "session": item})
		return
	}
	var body struct {
		SessionID int      `json:"session_id"`
		Title     string   `json:"title"`
		Tags      []string `json:"tags"`
		Summary   string   `json:"summary"`
		Archived  bool     `json:"archived"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.SessionID <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "session_id required")
		return
	}
	item, err := s.sessionStore.UpdateSessionMeta(body.SessionID, body.Title, body.Tags, body.Summary, body.Archived)
	if err != nil {
		s.logger.Warn("workbench session meta update failed", "session_id", body.SessionID, "err", err)
		writeJSONError(w, http.StatusInternalServerError, "session_meta_failed", "failed to save session meta")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "session": item})
}

func (s *Server) handleWorkbenchSessionSummary(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if s.sessionStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "session_store_disabled", "sqlite session store is disabled")
		return
	}
	var body struct {
		SessionID int `json:"session_id"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.SessionID <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "session_id required")
		return
	}
	msgs, err := s.sessionStore.ChatMessagesByLiveSession(body.SessionID, 120)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "summary_failed", "failed to load messages")
		return
	}
	current, _ := s.sessionStore.SessionOverviewByLiveSession(body.SessionID)
	title, summary, tags := summarizeMessages(current, msgs)
	if current.Title != "" {
		title = current.Title
	}
	if len(current.Tags) > 0 {
		tags = current.Tags
	}
	item, err := s.sessionStore.UpdateSessionMeta(body.SessionID, title, tags, summary, current.Archived)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "summary_failed", "failed to save summary")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "session": item, "summary": summary, "title": title, "tags": tags})
}

func (s *Server) handleWorkbenchSessionExport(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.sessionStore == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "session_store_disabled", "sqlite session store is disabled")
		return
	}
	id, _ := strconv.Atoi(r.URL.Query().Get("session_id"))
	if id <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "session_id required")
		return
	}
	redact := r.URL.Query().Get("redact") != "0"
	meta, err := s.sessionStore.SessionOverviewByLiveSession(id)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "export_failed", "failed to load session")
		return
	}
	messages, _ := s.sessionStore.ChatMessagesByLiveSession(id, 1000)
	events, _ := s.sessionStore.TimelineByLiveSession(id, 1000)
	name := fmt.Sprintf("many-ai-cli-session-%d.zip", id)
	w.Header().Set("Content-Type", "application/zip")
	w.Header().Set("Content-Disposition", fmt.Sprintf(`attachment; filename="%s"`, name))
	zw := zip.NewWriter(w)
	defer zw.Close()
	addZipJSON(zw, "session.json", map[string]any{"session": meta, "events": events})
	var md strings.Builder
	fmt.Fprintf(&md, "# many-ai-cli session #%d\n\n", id)
	if meta.Title != "" {
		fmt.Fprintf(&md, "Title: %s\n\n", meta.Title)
	}
	if meta.Summary != "" {
		fmt.Fprintf(&md, "Summary:\n%s\n\n", meta.Summary)
	}
	for _, msg := range messages {
		text := msg.RawText
		if redact {
			text = sessionlog.MaskSecrets(text)
		}
		fmt.Fprintf(&md, "## %s %s\n\n%s\n\n", msg.Role, msg.TS, text)
	}
	addZipText(zw, "messages.md", md.String())
	if meta.JSONLPath != "" {
		addZipFile(zw, "raw/session.jsonl", meta.JSONLPath, redact)
	}
	if meta.LogPath != "" {
		addZipFile(zw, "raw/session.log", meta.LogPath, redact)
	}
}

func (s *Server) handleWorkbenchState(w http.ResponseWriter, r *http.Request) {
	s.handleWorkbenchJSONMap(w, r, "workspace-state.json")
}

func (s *Server) handleWorkbenchTemplates(w http.ResponseWriter, r *http.Request) {
	s.handleWorkbenchItems(w, r, "templates.json", defaultWorkbenchTemplates())
}

func (s *Server) handleWorkbenchTasks(w http.ResponseWriter, r *http.Request) {
	s.handleWorkbenchItems(w, r, "tasks.json", workbenchDoc{})
}

func (s *Server) handleWorkbenchPalette(w http.ResponseWriter, r *http.Request) {
	s.handleWorkbenchItems(w, r, "prompt-palette.json", defaultWorkbenchPalette())
}

func (s *Server) handleWorkbenchPolicies(w http.ResponseWriter, r *http.Request) {
	s.handleWorkbenchItems(w, r, "policy-profiles.json", defaultWorkbenchPolicies())
}

func (s *Server) handleWorkbenchApprovalSimulate(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Provider string `json:"provider"`
		Text     string `json:"text"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	lines := strings.Split(sessionlog.CleanVisibleText(body.Text), "\n")
	approval := detectNativeApproval(strings.ToLower(strings.TrimSpace(body.Provider)), lines)
	if approval == nil {
		writeJSON(w, map[string]any{"ok": true, "matched": false})
		return
	}
	writeJSON(w, map[string]any{
		"ok":       true,
		"matched":  true,
		"sig":      approval.Sig,
		"kind":     approval.Kind,
		"question": approval.Question,
		"context":  approval.Context,
		"options":  approval.Options,
	})
}

func (s *Server) handleWorkbenchDiagnostics(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	type check struct {
		Name    string `json:"name"`
		Found   bool   `json:"found"`
		Path    string `json:"path,omitempty"`
		Version string `json:"version,omitempty"`
		Error   string `json:"error,omitempty"`
	}
	commands := []struct {
		name string
		args []string
	}{
		{"git", []string{"--version"}},
		{"claude", []string{"--version"}},
		{"codex", []string{"--version"}},
		{"copilot", []string{"--version"}},
		{"cursor-agent", []string{"--version"}},
		{"ollama", []string{"--version"}},
	}
	out := make([]check, 0, len(commands))
	for _, cmd := range commands {
		item := check{Name: cmd.name}
		path, err := exec.LookPath(cmd.name)
		if err != nil {
			item.Error = "not found in PATH"
			out = append(out, item)
			continue
		}
		item.Found = true
		item.Path = path
		ctx, cancel := context.WithTimeout(r.Context(), workbenchDiagnosticsTimeout)
		b, runErr := exec.CommandContext(ctx, path, cmd.args...).CombinedOutput()
		cancel()
		if runErr != nil {
			item.Error = sanitizeOneLine(runErr.Error())
		}
		item.Version = trimRunes(strings.TrimSpace(string(b)), 240)
		out = append(out, item)
	}
	writeJSON(w, map[string]any{"ok": true, "checks": out, "hub_cwd": s.hubCWD, "parent_shell": s.parentShell})
}

func (s *Server) handleWorkbenchFilesContext(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		SessionID int      `json:"session_id"`
		Paths     []string `json:"paths"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	if len(body.Paths) == 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "paths required")
		return
	}
	cwd := s.cwdForSessionOrHub(body.SessionID)
	gitRoot := findGitRoot(cwd)
	var total int
	var items []map[string]any
	var prompt strings.Builder
	for _, p := range body.Paths {
		p = strings.TrimSpace(p)
		if p == "" || !filepath.IsAbs(p) {
			continue
		}
		if ok, _ := isPathUnderAllowedRoots(p, cwd, gitRoot); !ok {
			items = append(items, map[string]any{"path": p, "ok": false, "error": "outside allowed roots"})
			continue
		}
		if !isTextFile(p) {
			items = append(items, map[string]any{"path": p, "ok": false, "error": "not a text file"})
			continue
		}
		info, err := os.Stat(p)
		if err != nil || info.IsDir() {
			items = append(items, map[string]any{"path": p, "ok": false, "error": "not readable"})
			continue
		}
		remaining := workbenchContextTotalMaxBytes - total
		if remaining <= 0 {
			items = append(items, map[string]any{"path": p, "ok": false, "error": "total context limit reached"})
			continue
		}
		limit := min(workbenchContextFileMaxBytes, remaining)
		b, truncated, err := readLimitedFile(p, limit)
		if err != nil {
			items = append(items, map[string]any{"path": p, "ok": false, "error": "read failed"})
			continue
		}
		total += len(b)
		rel := p
		if r, err := filepath.Rel(gitRoot, p); err == nil && !strings.HasPrefix(r, "..") {
			rel = filepath.ToSlash(r)
		}
		fmt.Fprintf(&prompt, "\n### %s\n\n```%s\n%s\n```\n", rel, strings.TrimPrefix(filepath.Ext(p), "."), string(b))
		items = append(items, map[string]any{"path": p, "rel": rel, "ok": true, "size": info.Size(), "truncated": truncated})
	}
	writeJSON(w, map[string]any{"ok": true, "items": items, "prompt": strings.TrimSpace(prompt.String()), "bytes": total})
}

func (s *Server) handleWorkbenchGitReview(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	sid, ok := parseSessionID(r.URL.Query().Get("session"))
	if !ok {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	gitRoot, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()
	statusOut, _ := runGit(ctx, cwd, "status", "--short")
	statOut, _ := runGit(ctx, cwd, "diff", "--stat", "HEAD", "--")
	nameOut, _ := runGit(ctx, cwd, "diff", "--name-status", "HEAD", "--")
	checkOut, checkErr := runGitCombined(ctx, cwd, "diff", "--check")
	files := parseNameStatusLines(string(nameOut))
	risks := gitReviewRisks(files, string(checkOut), checkErr)
	suggestions := gitCommitSplitSuggestions(files)
	writeJSON(w, map[string]any{
		"ok":          true,
		"git_root":    gitRoot,
		"status":      strings.TrimSpace(string(statusOut)),
		"stat":        strings.TrimSpace(string(statOut)),
		"files":       files,
		"risks":       risks,
		"suggestions": suggestions,
	})
}

func (s *Server) handleWorkbenchWorktrees(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	sid, ok := parseSessionID(r.URL.Query().Get("session"))
	if r.Method == http.MethodPost {
		var body struct {
			SessionID int    `json:"session_id"`
			Branch    string `json:"branch"`
			Path      string `json:"path"`
			Base      string `json:"base"`
		}
		if !decodeJSON(w, r, &body) {
			return
		}
		sid = body.SessionID
		ok = sid > 0
		if !ok {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "session_id required")
			return
		}
		gitRoot, cwd, err := s.resolveGitRoot(sid)
		if err != nil {
			writeGitErrorFromResolve(w, sid, err)
			return
		}
		branch := strings.TrimSpace(body.Branch)
		if !validRevision(branch) {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "safe branch name required")
			return
		}
		target := strings.TrimSpace(body.Path)
		if target == "" || !filepath.IsAbs(target) {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "absolute worktree path required")
			return
		}
		if ok, _ := isPathUnderAllowedRoots(target, filepath.Dir(gitRoot)); !ok {
			writeJSONError(w, http.StatusForbidden, "forbidden", "worktree path must stay under the repository parent")
			return
		}
		base := strings.TrimSpace(body.Base)
		if base == "" {
			base = "HEAD"
		}
		if !validRevision(base) {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "safe base revision required")
			return
		}
		ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
		defer cancel()
		if _, err := runGitCombined(ctx, cwd, "worktree", "add", "-b", branch, target, base); err != nil {
			writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
			return
		}
		writeJSON(w, map[string]any{"ok": true, "path": target, "branch": branch})
		return
	}
	if !ok {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	_, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()
	out, err := runGit(ctx, cwd, "worktree", "list", "--porcelain")
	if err != nil {
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
		return
	}
	writeJSON(w, map[string]any{"ok": true, "worktrees": parseWorktreePorcelain(string(out))})
}

func (s *Server) handleWorkbenchFileWatch(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	sid, ok := parseSessionID(r.URL.Query().Get("session"))
	if !ok {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	_, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()
	out, err := runGit(ctx, cwd, "status", "--short", "--porcelain=v1")
	if err != nil {
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
		return
	}
	files := parseShortStatusLines(string(out))
	writeJSON(w, map[string]any{"ok": true, "files": files, "count": len(files), "checked_at": time.Now().Format(time.RFC3339)})
}

func (s *Server) handleWorkbenchTestResults(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.sessionStore == nil {
		writeJSON(w, map[string]any{"ok": true, "results": []any{}})
		return
	}
	queries := []string{"FAIL", "PASS", "go test", "npm test", "pytest", "vitest"}
	seen := map[int64]bool{}
	var all []any
	for _, q := range queries {
		results, err := s.sessionStore.SearchMessages(q, 10)
		if err != nil {
			continue
		}
		for _, item := range results {
			if seen[item.MessageID] {
				continue
			}
			seen[item.MessageID] = true
			all = append(all, item)
		}
	}
	writeJSON(w, map[string]any{"ok": true, "results": all})
}

func (s *Server) handleWorkbenchRedactionPreview(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body struct {
		Text string `json:"text"`
	}
	if !decodeJSON(w, r, &body) {
		return
	}
	masked := sessionlog.MaskSecrets(body.Text)
	writeJSON(w, map[string]any{"ok": true, "masked": masked, "changed": masked != body.Text})
}

func (s *Server) handleWorkbenchUsage(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.sessionStore == nil {
		writeJSON(w, map[string]any{"ok": true, "usage": map[string]any{}})
		return
	}
	usage, err := s.sessionStore.UsageSummary()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "usage_failed", "failed to summarize usage")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "usage": usage})
}

func (s *Server) handleWorkbenchStaleSessions(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	if s.sessionStore == nil {
		writeJSON(w, map[string]any{"ok": true, "sessions": []any{}})
		return
	}
	hours, _ := strconv.Atoi(r.URL.Query().Get("hours"))
	if hours <= 0 || hours > 24*365 {
		hours = 24
	}
	limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
	items, err := s.sessionStore.StaleSessions(time.Now().Add(-time.Duration(hours)*time.Hour), limit)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "stale_failed", "failed to list stale sessions")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "sessions": items, "hours": hours})
}

func (s *Server) handleWorkbenchItems(w http.ResponseWriter, r *http.Request, filename string, fallback workbenchDoc) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost, http.MethodDelete) {
		return
	}
	var doc workbenchDoc
	if err := readWorkbenchJSON(filename, &doc); err != nil {
		doc = fallback
	}
	if doc.Items == nil {
		doc.Items = fallback.Items
	}
	switch r.Method {
	case http.MethodGet:
		writeJSON(w, map[string]any{"ok": true, "items": doc.Items})
	case http.MethodPost:
		var item workbenchItem
		if !decodeJSON(w, r, &item) {
			return
		}
		now := time.Now().Format(time.RFC3339)
		if item.ID == "" {
			item.ID = fmt.Sprintf("wb-%d", time.Now().UnixNano())
			item.CreatedAt = now
		}
		item.UpdatedAt = now
		doc.Items = upsertWorkbenchItem(doc.Items, item)
		if err := writeWorkbenchJSON(filename, doc); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", "failed to save item")
			return
		}
		writeJSON(w, map[string]any{"ok": true, "item": item, "items": doc.Items})
	case http.MethodDelete:
		id := strings.TrimSpace(r.URL.Query().Get("id"))
		doc.Items = deleteWorkbenchItem(doc.Items, id)
		if err := writeWorkbenchJSON(filename, doc); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", "failed to delete item")
			return
		}
		writeJSON(w, map[string]any{"ok": true, "items": doc.Items})
	}
}

func (s *Server) handleWorkbenchJSONMap(w http.ResponseWriter, r *http.Request, filename string) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost, http.MethodPut) {
		return
	}
	var state map[string]any
	if err := readWorkbenchJSON(filename, &state); err != nil || state == nil {
		state = map[string]any{}
	}
	if r.Method == http.MethodGet {
		writeJSON(w, map[string]any{"ok": true, "state": state})
		return
	}
	if !decodeJSON(w, r, &state) {
		return
	}
	if err := writeWorkbenchJSON(filename, state); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "save_failed", "failed to save state")
		return
	}
	writeJSON(w, map[string]any{"ok": true, "state": state})
}

func summarizeMessages(meta sessionstore.SessionOverview, msgs []sessionstore.ChatMessage) (title string, summary string, tags []string) {
	provider := strings.TrimSpace(meta.Provider)
	project := strings.TrimSpace(filepath.Base(meta.CWD))
	if project == "." || project == string(filepath.Separator) {
		project = ""
	}
	for _, msg := range msgs {
		if msg.Role == "user" && strings.TrimSpace(msg.RawText) != "" {
			title = trimRunes(strings.TrimSpace(msg.RawText), 70)
			break
		}
	}
	if title == "" {
		switch {
		case project != "" && provider != "":
			title = provider + " / " + project
		case project != "":
			title = project
		default:
			title = fmt.Sprintf("session #%d", meta.LiveSessionID)
		}
	}

	counts := map[string]int{}
	var firstUser, lastUser, lastAI string
	for _, msg := range msgs {
		counts[msg.Role]++
		text := strings.TrimSpace(msg.RawText)
		if text == "" {
			continue
		}
		switch msg.Role {
		case "user":
			if firstUser == "" {
				firstUser = text
			}
			lastUser = text
		case "ai":
			lastAI = text
		}
	}
	var b strings.Builder
	fmt.Fprintf(&b, "messages: user=%d ai=%d system=%d\n", counts["user"], counts["ai"], counts["system"])
	if meta.StartedAt != "" {
		fmt.Fprintf(&b, "started: %s\n", meta.StartedAt)
	}
	if firstUser != "" {
		fmt.Fprintf(&b, "\nfirst request:\n%s\n", trimRunes(firstUser, 600))
	}
	if lastUser != "" && lastUser != firstUser {
		fmt.Fprintf(&b, "\nlatest request:\n%s\n", trimRunes(lastUser, 600))
	}
	if lastAI != "" {
		fmt.Fprintf(&b, "\nlatest output:\n%s\n", trimRunes(lastAI, 900))
	}
	summary = strings.TrimSpace(b.String())

	if provider != "" {
		tags = append(tags, provider)
	}
	if project != "" {
		tags = append(tags, project)
	}
	if meta.Branch != "" {
		tags = append(tags, meta.Branch)
	}
	return title, summary, normalizeWorkbenchTags(tags)
}

func workbenchPath(filename string) (string, error) {
	base, err := config.Dir()
	if err != nil {
		return "", err
	}
	dir := filepath.Join(base, "workbench")
	if err := os.MkdirAll(dir, sessionlog.PrivateDirMode); err != nil {
		return "", err
	}
	return filepath.Join(dir, filepath.Base(filename)), nil
}

func readWorkbenchJSON(filename string, dst any) error {
	path, err := workbenchPath(filename)
	if err != nil {
		return err
	}
	b, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	return json.Unmarshal(b, dst)
}

func writeWorkbenchJSON(filename string, value any) error {
	path, err := workbenchPath(filename)
	if err != nil {
		return err
	}
	b, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(b, '\n'), sessionlog.PrivateFileMode)
}

func upsertWorkbenchItem(items []workbenchItem, item workbenchItem) []workbenchItem {
	item.Title = trimRunes(strings.TrimSpace(item.Title), 120)
	item.Body = trimRunes(strings.TrimSpace(item.Body), 8000)
	item.Status = trimRunes(strings.TrimSpace(item.Status), 40)
	item.Tags = normalizeWorkbenchTags(item.Tags)
	for i := range items {
		if items[i].ID == item.ID {
			if item.CreatedAt == "" {
				item.CreatedAt = items[i].CreatedAt
			}
			items[i] = item
			return items
		}
	}
	return append([]workbenchItem{item}, items...)
}

func deleteWorkbenchItem(items []workbenchItem, id string) []workbenchItem {
	id = strings.TrimSpace(id)
	if id == "" {
		return items
	}
	out := items[:0]
	for _, item := range items {
		if item.ID != id {
			out = append(out, item)
		}
	}
	return out
}

func defaultWorkbenchTemplates() workbenchDoc {
	now := time.Now().Format(time.RFC3339)
	return workbenchDoc{Items: []workbenchItem{
		{ID: "tpl-review", Title: "Git review", Body: "この変更のリスク、テスト不足、コミット分割案をレビューして。", Tags: []string{"review", "git"}, CreatedAt: now, UpdatedAt: now},
		{ID: "tpl-investigate", Title: "Investigation", Body: "現象を再現条件、原因候補、確認コマンド、修正案に分けて調査して。", Tags: []string{"investigation"}, CreatedAt: now, UpdatedAt: now},
	}}
}

func defaultWorkbenchPalette() workbenchDoc {
	now := time.Now().Format(time.RFC3339)
	return workbenchDoc{Items: []workbenchItem{
		{ID: "pal-risk", Title: "Risk checklist", Body: "破壊的変更、セキュリティ、互換性、テスト漏れの観点で確認して。", Tags: []string{"review"}, CreatedAt: now, UpdatedAt: now},
		{ID: "pal-summary", Title: "Session summary", Body: "ここまでの作業を、完了事項、未完了事項、次の一手に整理して。", Tags: []string{"summary"}, CreatedAt: now, UpdatedAt: now},
	}}
}

func defaultWorkbenchPolicies() workbenchDoc {
	now := time.Now().Format(time.RFC3339)
	return workbenchDoc{Items: []workbenchItem{
		{ID: "policy-normal", Title: "normal", Body: "通常の編集とテストは許可。削除、外部送信、権限変更は確認する。", Status: "active", Tags: []string{"default"}, CreatedAt: now, UpdatedAt: now},
		{ID: "policy-readonly", Title: "read-only", Body: "調査と読み取りのみ。ファイル編集、削除、git commit は確認する。", Status: "draft", Tags: []string{"safe"}, CreatedAt: now, UpdatedAt: now},
		{ID: "policy-risky", Title: "risky", Body: "worktree 作成、依存追加、外部コマンド、ログ export は明示承認する。", Status: "draft", Tags: []string{"strict"}, CreatedAt: now, UpdatedAt: now},
	}}
}

func (s *Server) cwdForSessionOrHub(id int) string {
	if id > 0 {
		s.sessionsMu.Lock()
		if ses := s.sessions[id]; ses != nil && strings.TrimSpace(ses.CWD) != "" {
			cwd := ses.CWD
			s.sessionsMu.Unlock()
			return cwd
		}
		s.sessionsMu.Unlock()
	}
	return s.hubCWD
}

func readLimitedFile(path string, maxBytes int) ([]byte, bool, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, false, err
	}
	defer f.Close()
	b, err := io.ReadAll(io.LimitReader(f, int64(maxBytes)+1))
	if err != nil {
		return nil, false, err
	}
	truncated := len(b) > maxBytes
	if truncated {
		b = b[:maxBytes]
	}
	return b, truncated, nil
}

func addZipJSON(zw *zip.Writer, name string, value any) {
	b, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return
	}
	addZipText(zw, name, string(append(b, '\n')))
}

func addZipText(zw *zip.Writer, name, text string) {
	f, err := zw.Create(name)
	if err != nil {
		return
	}
	_, _ = io.Copy(f, strings.NewReader(text))
}

func addZipFile(zw *zip.Writer, name, path string, redact bool) {
	b, truncated, err := readLimitedFile(path, workbenchExportFileMaxBytes)
	if err != nil {
		return
	}
	if redact {
		b = []byte(sessionlog.MaskSecrets(string(b)))
	}
	if truncated {
		b = append(b, []byte("\n\n[truncated by many-ai-cli export]\n")...)
	}
	f, err := zw.Create(name)
	if err != nil {
		return
	}
	_, _ = io.Copy(f, bytes.NewReader(b))
}

func sanitizeOneLine(s string) string {
	s = strings.ReplaceAll(s, "\r\n", " ")
	s = strings.ReplaceAll(s, "\r", " ")
	s = strings.ReplaceAll(s, "\n", " ")
	s = strings.Join(strings.Fields(s), " ")
	return trimRunes(s, 240)
}

func parseNameStatusLines(raw string) []map[string]string {
	var out []map[string]string
	for _, line := range strings.Split(raw, "\n") {
		fields := strings.Fields(line)
		if len(fields) < 2 {
			continue
		}
		item := map[string]string{"status": fields[0], "path": fields[len(fields)-1]}
		if len(fields) > 2 {
			item["from"] = fields[1]
		}
		out = append(out, item)
	}
	return out
}

func parseShortStatusLines(raw string) []map[string]string {
	var out []map[string]string
	for _, line := range strings.Split(raw, "\n") {
		if len(line) < 4 {
			continue
		}
		out = append(out, map[string]string{
			"status": strings.TrimSpace(line[:2]),
			"path":   strings.TrimSpace(line[3:]),
		})
	}
	return out
}

func gitReviewRisks(files []map[string]string, diffCheck string, diffCheckErr error) []string {
	var risks []string
	if diffCheckErr != nil && strings.TrimSpace(diffCheck) != "" {
		risks = append(risks, "git diff --check detected whitespace or conflict-marker issues")
	}
	for _, f := range files {
		p := strings.ToLower(f["path"])
		switch {
		case strings.Contains(p, "config") || strings.HasSuffix(p, ".yaml") || strings.HasSuffix(p, ".yml"):
			risks = append(risks, "configuration file changed: "+f["path"])
		case strings.Contains(p, "approval") || strings.Contains(p, "security"):
			risks = append(risks, "approval/security surface changed: "+f["path"])
		case strings.HasSuffix(p, ".go") && !hasNearbyTest(files, p):
			risks = append(risks, "Go source changed without nearby *_test.go in this diff: "+f["path"])
		case strings.HasSuffix(p, ".js") && strings.Contains(p, "/app/"):
			risks = append(risks, "frontend app behavior changed: "+f["path"])
		}
	}
	if len(risks) == 0 {
		risks = append(risks, "no obvious high-risk pattern detected")
	}
	return uniqueStrings(risks)
}

func gitCommitSplitSuggestions(files []map[string]string) []string {
	groups := map[string]int{}
	for _, f := range files {
		p := filepath.ToSlash(f["path"])
		switch {
		case strings.HasPrefix(p, "internal/"):
			groups["backend"]++
		case strings.HasPrefix(p, "web/"):
			groups["frontend"]++
		case strings.HasPrefix(p, "docs/"):
			groups["docs"]++
		case p == "go.mod" || p == "go.sum" || strings.Contains(p, "package"):
			groups["dependencies"]++
		default:
			groups["misc"]++
		}
	}
	keys := make([]string, 0, len(groups))
	for k := range groups {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	out := make([]string, 0, len(keys))
	for _, k := range keys {
		out = append(out, fmt.Sprintf("%s: %d files", k, groups[k]))
	}
	if len(out) == 0 {
		out = append(out, "working tree has no diff against HEAD")
	}
	return out
}

func hasNearbyTest(files []map[string]string, path string) bool {
	dir := filepath.ToSlash(filepath.Dir(path))
	for _, f := range files {
		p := strings.ToLower(filepath.ToSlash(f["path"]))
		if strings.HasSuffix(p, "_test.go") && filepath.ToSlash(filepath.Dir(p)) == dir {
			return true
		}
	}
	return false
}

func parseWorktreePorcelain(raw string) []map[string]string {
	var out []map[string]string
	cur := map[string]string{}
	flush := func() {
		if len(cur) > 0 {
			out = append(out, cur)
			cur = map[string]string{}
		}
	}
	for _, line := range strings.Split(raw, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			flush()
			continue
		}
		if v, ok := strings.CutPrefix(line, "worktree "); ok {
			flush()
			cur["path"] = v
		} else if v, ok := strings.CutPrefix(line, "HEAD "); ok {
			cur["head"] = v
		} else if v, ok := strings.CutPrefix(line, "branch "); ok {
			cur["branch"] = strings.TrimPrefix(v, "refs/heads/")
		} else if line == "bare" || line == "detached" {
			cur[line] = "true"
		}
	}
	flush()
	return out
}

func normalizeWorkbenchTags(tags []string) []string {
	seen := map[string]bool{}
	out := make([]string, 0, len(tags))
	re := regexp.MustCompile(`\s+`)
	for _, tag := range tags {
		tag = strings.TrimSpace(re.ReplaceAllString(tag, "-"))
		tag = strings.Trim(tag, ",#-")
		if tag == "" || seen[tag] {
			continue
		}
		seen[tag] = true
		out = append(out, trimRunes(tag, 32))
		if len(out) >= 12 {
			break
		}
	}
	return out
}

func uniqueStrings(in []string) []string {
	seen := map[string]bool{}
	out := make([]string, 0, len(in))
	for _, item := range in {
		item = strings.TrimSpace(item)
		if item == "" || seen[item] {
			continue
		}
		seen[item] = true
		out = append(out, item)
	}
	return out
}

func trimRunes(s string, max int) string {
	if max <= 0 {
		return s
	}
	runes := []rune(s)
	if len(runes) <= max {
		return s
	}
	return string(runes[:max])
}
