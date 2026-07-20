package hub

import (
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// agentLogLocation deliberately returns a path only, never transcript content.
// Provider transcripts may contain prompts, source snippets, and credentials.
type agentLogLocation struct {
	Available bool   `json:"available"`
	Path      string `json:"path,omitempty"`
	Label     string `json:"label,omitempty"`
	Reason    string `json:"reason,omitempty"`
}

type agentLogSession struct {
	Provider      string
	CWD           string
	StartedAt     string
	HomeDir       string
	CodexHome     string
	ClaudeDir     string
	NativeLogPath string
}

// handleAgentLog locates the provider-owned transcript for one active session.
// It intentionally does not fall back to many-ai-cli's optional PTY log.
func (s *Server) handleAgentLog(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	id, err := strconv.Atoi(r.URL.Query().Get("session_id"))
	if err != nil || id <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session_id")
		return
	}
	location := s.agentLogForSession(id)
	writeJSON(w, location)
}

// handleOpenAgentLog opens the resolved file's parent directory on the Hub
// host. The path is derived server-side from a session ID so a browser cannot
// use this endpoint as an arbitrary local-path opener.
func (s *Server) handleOpenAgentLog(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) || !s.requireLoopbackRemote(w, r) {
		return
	}
	id, err := strconv.Atoi(r.URL.Query().Get("session_id"))
	if err != nil || id <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session_id")
		return
	}
	location := s.agentLogForSession(id)
	if !location.Available {
		writeJSONError(w, http.StatusNotFound, "not_found", location.Reason)
		return
	}
	dir := location.Path
	if info, err := os.Stat(location.Path); err != nil || !info.IsDir() {
		dir = filepath.Dir(location.Path)
	}
	if err := openDirNative(dir); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "open_failed", errorDetail("open failed", err))
		return
	}
	writeJSON(w, map[string]any{"ok": true})
}

func (s *Server) agentLogForSession(id int) agentLogLocation {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return agentLogLocation{Reason: "session not found"}
	}
	snap := agentLogSession{
		Provider: ses.Provider, CWD: ses.CWD, StartedAt: ses.StartedAt,
		HomeDir: ses.HomeDir, CodexHome: ses.CodexHome, ClaudeDir: ses.ClaudeDir,
		NativeLogPath: ses.NativeLogPath,
	}
	s.sessionsMu.Unlock()

	switch snap.Provider {
	case "claude":
		root := strings.TrimSpace(snap.ClaudeDir)
		if root == "" {
			root = filepath.Join(snap.HomeDir, ".claude")
		}
		if snap.CWD == "" || root == "" {
			return agentLogLocation{Reason: "Claude project directory is unavailable"}
		}
		path := filepath.Join(root, "projects", claudeProjectDirName(snap.CWD))
		if isExistingDir(path) {
			return agentLogLocation{Available: true, Path: path, Label: "Claude Code project transcripts"}
		}
		return agentLogLocation{Reason: "Claude Code transcript directory not found yet"}
	case "codex":
		if snap.NativeLogPath == "" {
			return agentLogLocation{Reason: "Codex rollout log is available after the first completed turn"}
		}
		if isExistingFile(snap.NativeLogPath) {
			return agentLogLocation{Available: true, Path: snap.NativeLogPath, Label: "Codex rollout transcript"}
		}
		return agentLogLocation{Reason: "Codex rollout log is no longer readable"}
	case "grok":
		if snap.HomeDir == "" || snap.CWD == "" {
			return agentLogLocation{Reason: "Grok session directory is unavailable"}
		}
		startedAt, err := time.Parse(time.RFC3339, snap.StartedAt)
		if err != nil {
			return agentLogLocation{Reason: "Grok session start time is unavailable"}
		}
		path, ok := findGrokChatHistory(filepath.Join(snap.HomeDir, ".grok"), snap.CWD, startedAt)
		if ok {
			return agentLogLocation{Available: true, Path: path, Label: "Grok Build chat history"}
		}
		return agentLogLocation{Reason: "Grok Build chat history not found yet"}
	default:
		return agentLogLocation{Reason: "This provider's native transcript location is not supported yet"}
	}
}

func (s *Server) setCodexNativeLogPath(id int, path string) {
	clean := filepath.Clean(strings.TrimSpace(path))
	if clean == "." || !filepath.IsAbs(clean) || !strings.EqualFold(filepath.Ext(clean), ".jsonl") {
		return
	}
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[id]
	if ses == nil || ses.Provider != "codex" || !codexTranscriptPathAllowed(ses, clean) {
		return
	}
	ses.NativeLogPath = clean
}

func codexTranscriptPathAllowed(ses *session, path string) bool {
	root := strings.TrimSpace(ses.CodexHome)
	if root == "" {
		root = filepath.Join(ses.HomeDir, ".codex")
	}
	if root == "" {
		return false
	}
	return isAncestorOrEqual(filepath.Join(root, "sessions"), path)
}

func claudeProjectDirName(cwd string) string {
	clean := filepath.Clean(cwd)
	return strings.NewReplacer("\\", "-", "/", "-", ":", "-").Replace(clean)
}

func isExistingDir(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}

func isExistingFile(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}
