package hub

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
)

const (
	gitCommitSubjectMaxLen = 200
	gitCommitBodyMaxLen    = 8192
	gitCommitDiffMaxBytes  = 48 * 1024
)

type gitCommitAllReq struct {
	Session int    `json:"session"`
	Token   string `json:"token"`
	Subject string `json:"subject"`
	Body    string `json:"body"`
}

type gitCommitAllResp struct {
	OK           bool   `json:"ok"`
	Hash         string `json:"hash"`
	ShortHash    string `json:"short_hash"`
	Subject      string `json:"subject"`
	FilesChanged int    `json:"files_changed"`
}

type gitCommitMessageReq struct {
	Session  int    `json:"session"`
	Token    string `json:"token"`
	Mode     string `json:"mode"`
	Language string `json:"language"`
}

type gitCommitMessageResp struct {
	OK      bool   `json:"ok"`
	Subject string `json:"subject"`
	Body    string `json:"body"`
}

func (s *Server) handleGitCommitAll(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var req gitCommitAllReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeGitError(w, http.StatusBadRequest, "bad_request", "invalid json")
		return
	}
	if req.Token != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	subject := sanitizeCommitMessage(req.Subject, gitCommitSubjectMaxLen)
	body := sanitizeCommitMessage(req.Body, gitCommitBodyMaxLen)
	if subject == "" {
		writeGitError(w, http.StatusBadRequest, "bad_request", "subject is required")
		return
	}
	if req.Session <= 0 {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	gitRoot, cwd, err := s.resolveGitRoot(req.Session)
	if err != nil {
		writeGitErrorFromResolve(w, req.Session, err)
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()

	statusOut, err := runGit(ctx, cwd, "status", "--short", "--porcelain=v1", "-z")
	if err != nil {
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", err.Error())
		return
	}
	filesChanged := len(parseGitStatusPorcelainZ(string(statusOut)))
	if filesChanged == 0 {
		writeGitError(w, http.StatusBadRequest, "no_changes", "working tree has no changes")
		return
	}
	if _, err := runGitCombined(ctx, gitRoot, "add", "-A"); err != nil {
		code, status := classifyGitCommitError(err)
		writeGitError(w, status, code, err.Error())
		return
	}
	if _, err := runGit(ctx, gitRoot, "diff", "--cached", "--quiet"); err == nil {
		writeGitError(w, http.StatusBadRequest, "no_changes", "no staged changes after git add -A")
		return
	}
	args := []string{"commit", "-m", subject}
	if body != "" {
		args = append(args, "-m", body)
	}
	if _, err := runGitCombined(ctx, gitRoot, args...); err != nil {
		code, status := classifyGitCommitError(err)
		writeGitError(w, status, code, err.Error())
		return
	}
	hash := ""
	if out, err := runGit(ctx, gitRoot, "rev-parse", "HEAD"); err == nil {
		hash = strings.TrimSpace(string(out))
	}
	shortHash := ""
	if out, err := runGit(ctx, gitRoot, "rev-parse", "--short", "HEAD"); err == nil {
		shortHash = strings.TrimSpace(string(out))
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(gitCommitAllResp{
		OK:           true,
		Hash:         hash,
		ShortHash:    shortHash,
		Subject:      subject,
		FilesChanged: filesChanged,
	})
}

func (s *Server) handleGitCommitMessage(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var req gitCommitMessageReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeGitError(w, http.StatusBadRequest, "bad_request", "invalid json")
		return
	}
	if req.Token != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if req.Session <= 0 {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	_, cwd, err := s.resolveGitRoot(req.Session)
	if err != nil {
		writeGitErrorFromResolve(w, req.Session, err)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()

	statusOut, err := runGit(ctx, cwd, "status", "--short", "--porcelain=v1", "-z")
	if err != nil {
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", err.Error())
		return
	}
	files := parseGitStatusPorcelainZ(string(statusOut))
	if len(files) == 0 {
		writeGitError(w, http.StatusBadRequest, "no_changes", "working tree has no changes")
		return
	}
	stat := ""
	if out, err := runGit(ctx, cwd, "diff", "--stat", "HEAD", "--"); err == nil {
		stat = strings.TrimSpace(string(out))
	}
	diffNotice := ""
	if out, err := runGit(ctx, cwd, "diff", "--", "."); err == nil {
		diff := string(out)
		if len(diff) > gitCommitDiffMaxBytes {
			diffNotice = fmt.Sprintf("Diff context truncated to %d KiB.", gitCommitDiffMaxBytes/1024)
		}
	}

	subject, body := suggestCommitMessage(files, stat, diffNotice, req.Language)
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(gitCommitMessageResp{
		OK:      true,
		Subject: subject,
		Body:    body,
	})
}

func suggestCommitMessage(files []gitStatusFile, stat, diffNotice, language string) (string, string) {
	prefix := "chore"
	if hasPathPrefix(files, "docs/") || hasPathPrefix(files, "README") || hasPathPrefix(files, "CHANGELOG") {
		prefix = "docs"
	}
	if hasPathPrefix(files, "web/") || hasPathPrefix(files, "internal/") || hasPathPrefix(files, "cmd/") {
		prefix = "feat"
	}
	scope := dominantCommitScope(files)
	ja := strings.EqualFold(language, "ja") || language == ""
	if ja {
		subject := fmt.Sprintf("%s: %sの変更を反映", prefix, scope)
		body := fmt.Sprintf("%s に関する %d 件の変更をまとめて反映しました。", scope, len(files))
		if stat != "" {
			body += "\n\n" + stat
		}
		if diffNotice != "" {
			body += "\n\n" + diffNotice
		}
		return sanitizeCommitMessage(subject, gitCommitSubjectMaxLen), sanitizeCommitMessage(body, gitCommitBodyMaxLen)
	}
	subject := fmt.Sprintf("%s: update %s changes", prefix, scope)
	body := fmt.Sprintf("Apply %d working tree changes related to %s.", len(files), scope)
	if stat != "" {
		body += "\n\n" + stat
	}
	if diffNotice != "" {
		body += "\n\n" + diffNotice
	}
	return sanitizeCommitMessage(subject, gitCommitSubjectMaxLen), sanitizeCommitMessage(body, gitCommitBodyMaxLen)
}

func hasPathPrefix(files []gitStatusFile, prefix string) bool {
	for _, f := range files {
		if strings.HasPrefix(strings.ReplaceAll(f.Path, "\\", "/"), prefix) {
			return true
		}
	}
	return false
}

func dominantCommitScope(files []gitStatusFile) string {
	counts := map[string]int{}
	best := "working tree"
	bestN := 0
	for _, f := range files {
		p := strings.ReplaceAll(f.Path, "\\", "/")
		scope := p
		if idx := strings.Index(p, "/"); idx > 0 {
			scope = p[:idx]
		}
		if scope == "" {
			continue
		}
		counts[scope]++
		if counts[scope] > bestN {
			best = scope
			bestN = counts[scope]
		}
	}
	return best
}
