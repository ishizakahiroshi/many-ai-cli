package hub

import (
	"bufio"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
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
	Provider       string
	CWD            string
	StartedAt      string
	HomeDir        string
	CodexHome      string
	ClaudeDir      string
	AgentSessionID string
	NativeLogPath  string
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
		AgentSessionID: ses.AgentSessionID, NativeLogPath: ses.NativeLogPath,
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
		if snap.AgentSessionID != "" {
			if path, ok := claudeTranscriptPath(root, snap.CWD, snap.AgentSessionID); ok {
				return agentLogLocation{Available: true, Path: path, Label: "Claude Code transcript"}
			}
			return agentLogLocation{Reason: "Claude Code transcript file not found yet"}
		}
		path := filepath.Join(root, "projects", claudeProjectDirName(snap.CWD))
		if isExistingDir(path) {
			return agentLogLocation{Available: true, Path: path, Label: "Claude Code project transcripts"}
		}
		return agentLogLocation{Reason: "Claude Code transcript directory not found yet"}
	case "codex":
		root := strings.TrimSpace(snap.CodexHome)
		if root == "" {
			root = filepath.Join(snap.HomeDir, ".codex")
		}
		if snap.CWD == "" || root == "" {
			return agentLogLocation{Reason: "Codex home directory is unavailable"}
		}
		// Codex writes its rollout JSONL itself (~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl),
		// independent of the Stop hook that feeds NativeLogPath. Locating it by
		// cwd + start time (same approach as findGrokChatHistory) works even when
		// the hook never fires, so try it first.
		if startedAt, err := time.Parse(time.RFC3339, snap.StartedAt); err == nil {
			if path, ok := findCodexRolloutLog(root, snap.CWD, startedAt); ok {
				return agentLogLocation{Available: true, Path: path, Label: "Codex rollout transcript"}
			}
		}
		if snap.NativeLogPath != "" && isExistingFile(snap.NativeLogPath) {
			return agentLogLocation{Available: true, Path: snap.NativeLogPath, Label: "Codex rollout transcript"}
		}
		return agentLogLocation{Reason: "Codex rollout log is available after the first completed turn"}
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
	case "copilot":
		if snap.HomeDir == "" || snap.CWD == "" {
			return agentLogLocation{Reason: "Copilot session directory is unavailable"}
		}
		startedAt, err := time.Parse(time.RFC3339, snap.StartedAt)
		if err != nil {
			return agentLogLocation{Reason: "Copilot session start time is unavailable"}
		}
		path, ok := findCopilotSessionState(filepath.Join(snap.HomeDir, ".copilot"), snap.CWD, startedAt)
		if ok {
			return agentLogLocation{Available: true, Path: path, Label: "GitHub Copilot CLI session state"}
		}
		return agentLogLocation{Reason: "Copilot session state not found yet"}
	case "cursor-agent":
		if snap.HomeDir == "" || snap.CWD == "" {
			return agentLogLocation{Reason: "Cursor Agent session directory is unavailable"}
		}
		startedAt, err := time.Parse(time.RFC3339, snap.StartedAt)
		if err != nil {
			return agentLogLocation{Reason: "Cursor Agent session start time is unavailable"}
		}
		path, ok := findCursorChatDir(filepath.Join(snap.HomeDir, ".cursor"), snap.CWD, startedAt)
		if ok {
			return agentLogLocation{Available: true, Path: path, Label: "Cursor Agent CLI chat"}
		}
		return agentLogLocation{Reason: "Cursor Agent chat history not found yet"}
	case "opencode":
		// opencode は全セッションを単一 SQLite に集約しており、cwd/開始時刻での
		// 個別セッション特定はできない。DB の中身は一切パースしない設計を保つため、
		// ここでは共有ストアのパスをそのまま返す（1セッションに絞り込めない旨を
		// Label に明記する）。
		if snap.HomeDir == "" {
			return agentLogLocation{Reason: "opencode home directory is unavailable"}
		}
		dbPath := filepath.Join(snap.HomeDir, ".local", "share", "opencode", "opencode.db")
		if isExistingFile(dbPath) {
			return agentLogLocation{Available: true, Path: dbPath, Label: "opencode session store (shared across all sessions, not scoped to this one)"}
		}
		return agentLogLocation{Reason: "opencode session store not found"}
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

// codexRolloutMatchWindow は「セッション開始時刻 と rollout の session_meta
// タイムスタンプの近さ」でファイルを対応付ける際の許容差（findGrokChatHistory
// の grokHistoryMatchWindow と同じ考え方）。
const codexRolloutMatchWindow = 10 * time.Minute

// codexSessionMeta は rollout JSONL 先頭行（type: "session_meta"）の抜粋。
type codexSessionMeta struct {
	Type    string `json:"type"`
	Payload struct {
		CWD       string `json:"cwd"`
		Timestamp string `json:"timestamp"`
	} `json:"payload"`
}

// findCodexRolloutLog は many-ai-cli セッション（cwd + 開始時刻）に対応する
// Codex rollout JSONL を ~/.codex/sessions/YYYY/MM/DD/ 配下から特定する。
//
// Codex はこのファイルを Stop フックとは無関係に自分で書き出す（先頭行の
// session_meta に cwd と timestamp を持つ）ため、Stop フックが発火するかに
// 依存せず特定できる。Grok の findGrokChatHistory と同じ「cwd + 開始時刻の
// 近傍一致」方式。
func findCodexRolloutLog(codexHome, cwd string, startedAt time.Time) (string, bool) {
	local := startedAt.Local()
	bestDelta := codexRolloutMatchWindow
	bestPath := ""
	// セッション開始が日跨ぎ直前・直後になり得るため前日・当日・翌日を見る。
	for _, day := range []time.Time{local.AddDate(0, 0, -1), local, local.AddDate(0, 0, 1)} {
		dir := filepath.Join(codexHome, "sessions",
			fmt.Sprintf("%04d", day.Year()), fmt.Sprintf("%02d", day.Month()), fmt.Sprintf("%02d", day.Day()))
		entries, err := os.ReadDir(dir)
		if err != nil {
			continue
		}
		for _, e := range entries {
			if e.IsDir() || !strings.HasSuffix(e.Name(), ".jsonl") {
				continue
			}
			path := filepath.Join(dir, e.Name())
			meta, ok := readCodexSessionMeta(path)
			if !ok || !grokPathsEquivalent(meta.Payload.CWD, cwd) {
				continue
			}
			ts, err := time.Parse(time.RFC3339Nano, meta.Payload.Timestamp)
			if err != nil {
				continue
			}
			if d := ts.Sub(startedAt).Abs(); d <= bestDelta {
				bestDelta = d
				bestPath = path
			}
		}
	}
	if bestPath == "" {
		return "", false
	}
	return bestPath, true
}

// readCodexSessionMeta は rollout JSONL の先頭行だけを読んで session_meta を返す。
// 会話本文（以降の行）は読まない。
func readCodexSessionMeta(path string) (codexSessionMeta, bool) {
	f, err := os.Open(path)
	if err != nil {
		return codexSessionMeta{}, false
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 64*1024), 4*1024*1024)
	if !sc.Scan() {
		return codexSessionMeta{}, false
	}
	var meta codexSessionMeta
	if err := json.Unmarshal(sc.Bytes(), &meta); err != nil || meta.Type != "session_meta" {
		return codexSessionMeta{}, false
	}
	return meta, true
}

// copilotWorkspaceMeta は ~/.copilot/session-state/<uuid>/workspace.yaml の抜粋。
type copilotWorkspaceMeta struct {
	CWD       string `yaml:"cwd"`
	CreatedAt string `yaml:"created_at"`
}

// findCopilotSessionState は many-ai-cli セッション（cwd + 開始時刻）に対応する
// Copilot CLI のセッションディレクトリを ~/.copilot/session-state/<uuid>/ から
// 特定する。workspace.yaml に cwd/created_at が入っており、Codex の
// findCodexRolloutLog と同じ「cwd + 開始時刻の近傍一致」方式が使える。
func findCopilotSessionState(copilotHome, cwd string, startedAt time.Time) (string, bool) {
	root := filepath.Join(copilotHome, "session-state")
	entries, err := os.ReadDir(root)
	if err != nil {
		return "", false
	}
	bestDelta := codexRolloutMatchWindow
	bestPath := ""
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		dir := filepath.Join(root, e.Name())
		data, err := os.ReadFile(filepath.Join(dir, "workspace.yaml"))
		if err != nil {
			continue
		}
		var meta copilotWorkspaceMeta
		if yaml.Unmarshal(data, &meta) != nil || !grokPathsEquivalent(meta.CWD, cwd) {
			continue
		}
		ts, err := time.Parse(time.RFC3339, meta.CreatedAt)
		if err != nil {
			continue
		}
		if d := ts.Sub(startedAt).Abs(); d <= bestDelta {
			bestDelta = d
			bestPath = dir
		}
	}
	if bestPath == "" {
		return "", false
	}
	return bestPath, true
}

// cursorChatMeta は ~/.cursor/chats/<hash>/<uuid>/meta.json の抜粋。
type cursorChatMeta struct {
	CWD         string `json:"cwd"`
	CreatedAtMs int64  `json:"createdAtMs"`
}

// findCursorChatDir は many-ai-cli セッション（cwd + 開始時刻）に対応する
// Cursor Agent CLI のチャットディレクトリを ~/.cursor/chats/<hash>/<uuid>/ から
// 特定する。meta.json に cwd/createdAtMs が入っている。
func findCursorChatDir(cursorHome, cwd string, startedAt time.Time) (string, bool) {
	root := filepath.Join(cursorHome, "chats")
	hashDirs, err := os.ReadDir(root)
	if err != nil {
		return "", false
	}
	bestDelta := codexRolloutMatchWindow
	bestPath := ""
	for _, hd := range hashDirs {
		if !hd.IsDir() {
			continue
		}
		chatDirs, err := os.ReadDir(filepath.Join(root, hd.Name()))
		if err != nil {
			continue
		}
		for _, cd := range chatDirs {
			if !cd.IsDir() {
				continue
			}
			chatDir := filepath.Join(root, hd.Name(), cd.Name())
			data, err := os.ReadFile(filepath.Join(chatDir, "meta.json"))
			if err != nil {
				continue
			}
			var meta cursorChatMeta
			if json.Unmarshal(data, &meta) != nil || !grokPathsEquivalent(meta.CWD, cwd) {
				continue
			}
			ts := time.UnixMilli(meta.CreatedAtMs)
			if d := ts.Sub(startedAt).Abs(); d <= bestDelta {
				bestDelta = d
				bestPath = chatDir
			}
		}
	}
	if bestPath == "" {
		return "", false
	}
	return bestPath, true
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

type claudeTranscriptMeta struct {
	SessionID string `json:"sessionId"`
	CWD       string `json:"cwd"`
	Timestamp string `json:"timestamp"`
}

// claudeTranscriptPath resolves only a generated UUID. Keeping the ID as a
// filename component is safe for both Windows and Unix and prevents a wrapper
// from turning the path resolver into an arbitrary-file reader.
func claudeTranscriptPath(claudeDir, cwd, sessionID string) (string, bool) {
	if claudeDir == "" || cwd == "" || !isUUIDv4Like(sessionID) {
		return "", false
	}
	path := filepath.Join(claudeDir, "projects", claudeProjectDirName(cwd), sessionID+".jsonl")
	if !isExistingFile(path) {
		return "", false
	}
	return path, true
}

func isUUIDv4Like(value string) bool {
	if len(value) != 36 {
		return false
	}
	for i, r := range value {
		if i == 8 || i == 13 || i == 18 || i == 23 {
			if r != '-' {
				return false
			}
			continue
		}
		if !((r >= '0' && r <= '9') || (r >= 'a' && r <= 'f') || (r >= 'A' && r <= 'F')) {
			return false
		}
	}
	return true
}

// findClaudeTranscript is the fallback for sessions started without the
// wrapper-generated --session-id. It refuses an equal-time tie rather than
// guessing between two Claude sessions in the same cwd.
func findClaudeTranscript(claudeDir, cwd string, startedAt time.Time) (string, bool) {
	projectDir := filepath.Join(claudeDir, "projects", claudeProjectDirName(cwd))
	entries, err := os.ReadDir(projectDir)
	if err != nil {
		return "", false
	}
	bestDelta := codexRolloutMatchWindow
	bestPath := ""
	tie := false
	for _, entry := range entries {
		if entry.IsDir() || !strings.HasSuffix(entry.Name(), ".jsonl") {
			continue
		}
		path := filepath.Join(projectDir, entry.Name())
		meta, ok := readClaudeTranscriptMeta(path)
		if !ok || !grokPathsEquivalent(meta.CWD, cwd) {
			continue
		}
		stamp, err := time.Parse(time.RFC3339Nano, meta.Timestamp)
		if err != nil {
			continue
		}
		delta := stamp.Sub(startedAt).Abs()
		if delta > codexRolloutMatchWindow {
			continue
		}
		if bestPath == "" || delta < bestDelta {
			bestPath = path
			bestDelta = delta
			tie = false
		} else if delta == bestDelta {
			tie = true
		}
	}
	if bestPath == "" || tie {
		return "", false
	}
	return bestPath, true
}

func readClaudeTranscriptMeta(path string) (claudeTranscriptMeta, bool) {
	f, err := os.Open(path)
	if err != nil {
		return claudeTranscriptMeta{}, false
	}
	defer f.Close()
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 64*1024), 4*1024*1024)
	for i := 0; i < 8 && sc.Scan(); i++ {
		var meta claudeTranscriptMeta
		if json.Unmarshal(sc.Bytes(), &meta) != nil {
			continue
		}
		if meta.CWD != "" && meta.Timestamp != "" {
			return meta, true
		}
	}
	return claudeTranscriptMeta{}, false
}

func isExistingDir(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}

func isExistingFile(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}
