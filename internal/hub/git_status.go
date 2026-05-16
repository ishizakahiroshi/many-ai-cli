package hub

import (
	"context"
	"encoding/json"
	"net/http"
	"path/filepath"
	"strconv"
	"strings"
)

type gitStatusFile struct {
	Status  string `json:"status"`
	Path    string `json:"path"`
	Added   *int   `json:"added"`
	Removed *int   `json:"removed"`
}

type gitStatusSummary struct {
	FilesChanged int `json:"files_changed"`
	Added        int `json:"added"`
	Removed      int `json:"removed"`
}

type gitStatusResp struct {
	OK         bool             `json:"ok"`
	GitRoot    string           `json:"git_root"`
	RepoName   string           `json:"repo_name"`
	Branch     string           `json:"branch"`
	HeadHash   string           `json:"head_hash"`
	HasChanges bool             `json:"has_changes"`
	Files      []gitStatusFile  `json:"files"`
	Summary    gitStatusSummary `json:"summary"`
}

func (s *Server) handleGitStatus(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	sid, ok := parseSessionID(r.URL.Query().Get("session"))
	if !ok {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	gitRoot, cwd, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, err)
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
	applyWorkingTreeNumstat(files, func() string {
		out, nerr := runGit(ctx, cwd, "diff", "--numstat", "HEAD", "--")
		if nerr != nil {
			return ""
		}
		return string(out)
	}())
	summary := summarizeGitStatusFiles(files)

	branch := ""
	if out, berr := runGit(ctx, cwd, "rev-parse", "--abbrev-ref", "HEAD"); berr == nil {
		branch = strings.TrimSpace(string(out))
		if branch == "HEAD" {
			if shortOut, sherr := runGit(ctx, cwd, "rev-parse", "--short", "HEAD"); sherr == nil {
				branch = "detached:" + strings.TrimSpace(string(shortOut))
			}
		}
	}
	headHash := ""
	if out, herr := runGit(ctx, cwd, "rev-parse", "HEAD"); herr == nil {
		headHash = strings.TrimSpace(string(out))
	}

	resp := gitStatusResp{
		OK:         true,
		GitRoot:    gitRoot,
		RepoName:   filepath.Base(gitRoot),
		Branch:     branch,
		HeadHash:   headHash,
		HasChanges: len(files) > 0,
		Files:      files,
		Summary:    summary,
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(resp)
}

func parseGitStatusPorcelainZ(raw string) []gitStatusFile {
	if raw == "" {
		return []gitStatusFile{}
	}
	entries := strings.Split(raw, "\x00")
	files := make([]gitStatusFile, 0, len(entries))
	for i := 0; i < len(entries); i++ {
		entry := entries[i]
		if len(entry) < 4 {
			continue
		}
		status := strings.TrimSpace(entry[:2])
		if status == "" {
			status = strings.TrimSpace(entry[:1])
		}
		path := entry[3:]
		shortStatus := status
		if len(shortStatus) > 1 {
			if shortStatus == "??" {
				shortStatus = "??"
			} else if shortStatus[0] != ' ' && shortStatus[0] != '?' {
				shortStatus = shortStatus[:1]
			} else {
				shortStatus = shortStatus[1:]
			}
		}
		if strings.HasPrefix(status, "R") || strings.Contains(status, "R") || strings.HasPrefix(status, "C") || strings.Contains(status, "C") {
			if i+1 < len(entries) && entries[i+1] != "" {
				// porcelain v1 -z prints rename/copy as "XY to\0from\0".
				// Keep the destination path because it is the path commit/status UIs act on.
				i++
			}
		}
		files = append(files, gitStatusFile{Status: shortStatus, Path: path})
	}
	return files
}

func applyWorkingTreeNumstat(files []gitStatusFile, raw string) {
	type stat struct{ added, removed int }
	stats := map[string]stat{}
	for _, line := range strings.Split(raw, "\n") {
		line = strings.TrimRight(line, "\r")
		if line == "" {
			continue
		}
		fields := strings.SplitN(line, "\t", 3)
		if len(fields) < 3 {
			continue
		}
		added, _ := strconv.Atoi(fields[0])
		removed, _ := strconv.Atoi(fields[1])
		path := normalizeNumstatPath(fields[2])
		stats[path] = stat{added: added, removed: removed}
	}
	for i := range files {
		if st, ok := stats[files[i].Path]; ok {
			files[i].Added = intPtr(st.added)
			files[i].Removed = intPtr(st.removed)
		}
	}
}

func normalizeNumstatPath(path string) string {
	if idx := strings.LastIndex(path, " => "); idx >= 0 {
		path = strings.TrimSpace(path[idx+4:])
		path = strings.TrimRight(path, "}")
	}
	return strings.TrimSpace(path)
}

func summarizeGitStatusFiles(files []gitStatusFile) gitStatusSummary {
	s := gitStatusSummary{FilesChanged: len(files)}
	for _, f := range files {
		if f.Added != nil {
			s.Added += *f.Added
		}
		if f.Removed != nil {
			s.Removed += *f.Removed
		}
	}
	return s
}

func intPtr(v int) *int { return &v }
