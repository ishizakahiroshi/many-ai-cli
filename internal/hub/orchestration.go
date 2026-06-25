package hub

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

const orchestrationPollInterval = 2 * time.Second

var safeOrchestrationToken = regexp.MustCompile(`[^a-zA-Z0-9._-]+`)

type orchestrationManager struct {
	mu       sync.Mutex
	pending  map[string]pendingChild
	boards   map[string]*orchestrationBoard
	stopOnce sync.Once
	stopCh   chan struct{}
}

type pendingChild struct {
	ParentSessionID int
	Role            string
	Auto            bool
	Depth           int
	OrchestrationID string
	BoardPath       string
	WorktreeBranch  string
}

type orchestrationBoard struct {
	ID       string
	Path     string
	Sessions map[int]string
	Done     map[string]bool
	LastSize int64
	LastMod  time.Time
}

type spawnChildRequest struct {
	Role           string `json:"role"`
	Provider       string `json:"provider"`
	Model          string `json:"model"`
	InitialPrompt  string `json:"initial_prompt"`
	CWD            string `json:"cwd"`
	Auto           bool   `json:"auto"`
	PermissionMode string `json:"permission_mode"`
	Sandbox        string `json:"sandbox"`
	AskForApproval string `json:"ask_for_approval"`
	Route          string `json:"route"`
	ModelSelection string `json:"model_selection_mode"`
	RiskConfirmed  bool   `json:"risk_confirmed"`
}

type injectRequest struct {
	Text          string `json:"text"`
	FromSessionID int    `json:"from_session_id"`
	PressEnter    bool   `json:"press_enter"`
	Interrupt     bool   `json:"interrupt"`
}

func newOrchestrationManager() *orchestrationManager {
	return &orchestrationManager{
		pending: map[string]pendingChild{},
		boards:  map[string]*orchestrationBoard{},
		stopCh:  make(chan struct{}),
	}
}

func (m *orchestrationManager) stop() {
	if m == nil {
		return
	}
	m.stopOnce.Do(func() { close(m.stopCh) })
}

func (s *Server) handleSessionAPI(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	path := strings.TrimPrefix(r.URL.Path, "/api/sessions/")
	parts := strings.Split(strings.Trim(path, "/"), "/")
	if len(parts) != 2 {
		writeJSONError(w, http.StatusNotFound, "not_found", "not found")
		return
	}
	id, err := strconv.Atoi(parts[0])
	if err != nil || id <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session id")
		return
	}
	switch parts[1] {
	case "spawn-child":
		s.handleSpawnChild(w, r, id)
	case "inject":
		s.handleSessionInject(w, r, id)
	case "children":
		s.handleSessionChildren(w, r, id)
	default:
		writeJSONError(w, http.StatusNotFound, "not_found", "not found")
	}
}

func (s *Server) handleSpawnChild(w http.ResponseWriter, r *http.Request, parentID int) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body spawnChildRequest
	if !decodeJSON(w, r, &body) {
		return
	}
	body.Role = sanitizeRole(body.Role)
	if body.Role == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "role is required")
		return
	}
	if body.Provider == "" {
		body.Provider = "codex"
	}
	if !validOrchestrationProvider(body.Provider) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid provider")
		return
	}
	if !spawnValidModelLabel(body.Model) || strings.HasPrefix(body.Model, "-") {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid model value")
		return
	}

	cfg := s.snapshotCfg().Orchestration
	parent, childCount, totalSessions := s.orchestrationParentState(parentID)
	if parent == nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "parent session not found")
		return
	}
	if parent.Depth >= cfg.MaxDepth {
		writeJSONStatus(w, http.StatusTooManyRequests, httpErrorResp{OK: false, Error: "orchestration_limit", Detail: "max orchestration depth reached"})
		return
	}
	if childCount >= cfg.MaxChildrenPerParent {
		writeJSONStatus(w, http.StatusTooManyRequests, httpErrorResp{OK: false, Error: "orchestration_limit", Detail: "max children per parent reached"})
		return
	}
	if totalSessions >= cfg.MaxTotalSessions {
		writeJSONStatus(w, http.StatusTooManyRequests, httpErrorResp{OK: false, Error: "orchestration_limit", Detail: "max total sessions reached"})
		return
	}

	cwd := strings.TrimSpace(body.CWD)
	if cwd == "" {
		cwd = parent.CWD
	}
	if cwd == "" {
		cwd = s.hubCWD
	}
	info, statErr := os.Stat(cwd)
	if statErr != nil || !info.IsDir() {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "cwd does not exist or is not a directory")
		return
	}
	if spawnCwdTooBroad(cwd) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "cwd is too broad (system root or home root)")
		return
	}

	orchestrationID := parent.OrchestrationID
	if orchestrationID == "" {
		orchestrationID = fmt.Sprintf("s%d", parentID)
	}
	boardPath, err := s.ensureOrchestrationBoard(orchestrationID, parent, body)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "board_error", errorDetail("board error", err))
		return
	}
	childCWD, branch, worktreeNote := s.prepareChildWorktree(cwd, orchestrationID, body.Role, cfg)
	if worktreeNote != "" {
		_ = appendBoardSection(boardPath, "hub", fmt.Sprintf("%s\n", worktreeNote))
	}

	label := fmt.Sprintf("orch-%s-%s-%d", safeToken(orchestrationID), body.Role, time.Now().UnixNano())
	meta := pendingChild{
		ParentSessionID: parentID,
		Role:            body.Role,
		Auto:            true,
		Depth:           parent.Depth + 1,
		OrchestrationID: orchestrationID,
		BoardPath:       boardPath,
		WorktreeBranch:  branch,
	}
	s.orchestration.mu.Lock()
	s.orchestration.pending[label] = meta
	s.orchestration.mu.Unlock()
	childID, err := s.spawnWrappedSession(spawnWrappedSpec{
		Provider:       body.Provider,
		CWD:            childCWD,
		Model:          body.Model,
		ModelSelection: body.ModelSelection,
		RiskConfirmed:  body.RiskConfirmed,
		Label:          label,
		PermissionMode: body.PermissionMode,
		Sandbox:        body.Sandbox,
		AskForApproval: body.AskForApproval,
		Route:          body.Route,
	}, 20*time.Second)
	if err != nil {
		s.orchestration.mu.Lock()
		delete(s.orchestration.pending, label)
		s.orchestration.mu.Unlock()
		writeJSONError(w, http.StatusInternalServerError, "spawn_error", errorDetail("spawn error", err))
		return
	}
	s.registerBoardSession(orchestrationID, boardPath, parentID, "conductor")
	s.registerBoardSession(orchestrationID, boardPath, childID, body.Role)
	prompt := buildChildInitialPrompt(body.InitialPrompt, boardPath, body.Role, branch)
	s.injectText(childID, prompt, true, false)
	writeJSON(w, map[string]any{"ok": true, "session_id": childID, "board_path": boardPath, "cwd": childCWD, "worktree_branch": branch})
}

func (s *Server) handleSessionInject(w http.ResponseWriter, r *http.Request, id int) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body injectRequest
	if !decodeJSON(w, r, &body) {
		return
	}
	if body.FromSessionID == id {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "self inject is not allowed")
		return
	}
	if strings.TrimSpace(body.Text) == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "text is required")
		return
	}
	s.sessionsMu.Lock()
	_, ok := s.sessions[id]
	s.sessionsMu.Unlock()
	if !ok {
		writeJSONError(w, http.StatusNotFound, "not_found", "session not found")
		return
	}
	s.injectText(id, body.Text, body.PressEnter, body.Interrupt)
	writeJSON(w, map[string]bool{"ok": true})
}

func (s *Server) handleSessionChildren(w http.ResponseWriter, r *http.Request, id int) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	s.sessionsMu.Lock()
	children := make([]*session, 0)
	for _, ses := range s.sessions {
		if ses.ParentSessionID == id {
			cp := *ses
			children = append(children, &cp)
		}
	}
	s.sessionsMu.Unlock()
	writeJSON(w, map[string]any{"ok": true, "children": children})
}

func (s *Server) orchestrationParentState(parentID int) (*session, int, int) {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	parent := s.sessions[parentID]
	var parentCopy *session
	if parent != nil {
		cp := *parent
		parentCopy = &cp
	}
	childCount := 0
	for _, ses := range s.sessions {
		if ses.ParentSessionID == parentID {
			childCount++
		}
	}
	return parentCopy, childCount, len(s.sessions)
}

func (s *Server) ensureOrchestrationBoard(id string, parent *session, body spawnChildRequest) (string, error) {
	base, err := config.Dir()
	if err != nil {
		return "", err
	}
	dir := filepath.Join(base, "orchestration", safeToken(id))
	if err := os.MkdirAll(dir, sessionlog.PrivateDirMode); err != nil {
		return "", err
	}
	path := filepath.Join(dir, "board.md")
	if _, err := os.Stat(path); os.IsNotExist(err) {
		content := fmt.Sprintf("# Orchestration %s\n\n- conductor: session #%d provider=%s model=%s\n- purpose: %s\n\n## conductor %s\nCreated board. Children must append progress sections and finish with `## DONE <role>`.\n",
			id, parent.ID, parent.Provider, parent.Model, strings.TrimSpace(body.InitialPrompt), time.Now().Format(time.RFC3339))
		if err := os.WriteFile(path, []byte(content), sessionlog.PrivateFileMode); err != nil {
			return "", err
		}
	}
	return path, nil
}

func (s *Server) registerBoardSession(id, path string, sessionID int, role string) {
	s.orchestration.mu.Lock()
	b := s.orchestration.boards[id]
	if b == nil {
		b = &orchestrationBoard{ID: id, Path: path, Sessions: map[int]string{}, Done: map[string]bool{}}
		s.orchestration.boards[id] = b
	}
	b.Sessions[sessionID] = role
	s.orchestration.mu.Unlock()
}

func (s *Server) orchestrationBoardLoop(ctx context.Context) {
	t := time.NewTicker(orchestrationPollInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-s.orchestration.stopCh:
			return
		case <-t.C:
			s.scanOrchestrationBoards()
		}
	}
}

func (s *Server) scanOrchestrationBoards() {
	s.orchestration.mu.Lock()
	boards := make([]*orchestrationBoard, 0, len(s.orchestration.boards))
	for _, b := range s.orchestration.boards {
		boards = append(boards, b)
	}
	s.orchestration.mu.Unlock()
	for _, b := range boards {
		info, err := os.Stat(b.Path)
		if err != nil {
			continue
		}
		if info.Size() == b.LastSize && info.ModTime().Equal(b.LastMod) {
			continue
		}
		data, err := os.ReadFile(b.Path)
		if err != nil {
			continue
		}
		text := string(data)
		dones := detectBoardDoneRoles(text)
		updatedBy := detectLastBoardRole(text)
		s.orchestration.mu.Lock()
		stored := s.orchestration.boards[b.ID]
		if stored != nil {
			stored.LastSize = info.Size()
			stored.LastMod = info.ModTime()
			for _, role := range dones {
				stored.Done[role] = true
			}
		}
		sessions := map[int]string{}
		if stored != nil {
			for id, role := range stored.Sessions {
				sessions[id] = role
			}
		}
		s.orchestration.mu.Unlock()
		for sessionID := range sessions {
			s.injectText(sessionID, fmt.Sprintf("\n[orchestration] board updated by %s: %s\n", updatedBy, b.Path), false, false)
		}
		for _, role := range dones {
			s.markChildDoneByRole(b.ID, role)
		}
	}
}

func detectBoardDoneRoles(text string) []string {
	var roles []string
	seen := map[string]bool{}
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if !strings.HasPrefix(line, "## DONE ") {
			continue
		}
		fields := strings.Fields(strings.TrimSpace(strings.TrimPrefix(line, "## DONE ")))
		if len(fields) == 0 {
			continue
		}
		role := sanitizeRole(fields[0])
		if role != "" && !seen[role] {
			roles = append(roles, role)
			seen[role] = true
		}
	}
	return roles
}

func detectLastBoardRole(text string) string {
	role := "unknown"
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if strings.HasPrefix(line, "## ") {
			fields := strings.Fields(strings.TrimPrefix(line, "## "))
			if len(fields) > 0 {
				role = fields[0]
			}
		}
	}
	return role
}

func (s *Server) markChildDoneByRole(orchestrationID, role string) {
	s.sessionsMu.Lock()
	var updates []proto.Message
	for _, ses := range s.sessions {
		if ses.OrchestrationID == orchestrationID && ses.Role == role && ses.ParentSessionID != 0 && ses.State != "done" {
			ses.State = "done"
			updates = append(updates, proto.Message{
				Type:            "session_update",
				SessionID:       ses.ID,
				Provider:        ses.Provider,
				Display:         ses.Display,
				CWD:             ses.CWD,
				Branch:          ses.Branch,
				Label:           ses.Label,
				Model:           ses.Model,
				Route:           ses.Route,
				State:           ses.State,
				ParentSessionID: ses.ParentSessionID,
				Role:            ses.Role,
				Auto:            ses.Auto,
				Depth:           ses.Depth,
				OrchestrationID: ses.OrchestrationID,
				BoardPath:       ses.BoardPath,
				WorktreeBranch:  ses.WorktreeBranch,
			})
		}
	}
	s.sessionsMu.Unlock()
	for _, msg := range updates {
		s.broadcast(msg)
		if s.sessionStore != nil {
			s.sessionStore.UpdateSessionState(msg.SessionID, "done", "")
		}
	}
}

func (s *Server) injectText(sessionID int, text string, pressEnter bool, interrupt bool) {
	if interrupt {
		s.injectRaw(sessionID, "\x1b")
	}
	if pressEnter && !strings.HasSuffix(text, "\r") {
		text += "\r"
	}
	s.injectRaw(sessionID, text)
}

func (s *Server) injectRaw(sessionID int, text string) {
	s.sessionsMu.Lock()
	wc := s.wrappers[sessionID]
	s.sessionsMu.Unlock()
	s.submitInput(wc, sessionID, text)
}

func buildChildInitialPrompt(base, boardPath, role, branch string) string {
	var b strings.Builder
	b.WriteString("You are an orchestration child session.\n")
	b.WriteString("Role: " + role + "\n")
	b.WriteString("Shared board: " + boardPath + "\n")
	if branch != "" {
		b.WriteString("Worktree branch: " + branch + "\n")
	}
	b.WriteString("Read the board before acting. Append progress as `## " + role + " <RFC3339 time>` sections. When complete, append `## DONE " + role + "` and a concise summary.\n\n")
	b.WriteString(base)
	return b.String()
}

func (s *Server) prepareChildWorktree(cwd, orchestrationID, role string, cfg config.OrchestrationConfig) (string, string, string) {
	if !cfg.WorktreeEnabled() {
		return cwd, "", "worktree skip: disabled by config"
	}
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := exec.CommandContext(ctx, "git", "-C", cwd, "rev-parse", "--show-toplevel").Run(); err != nil {
		return cwd, "", "worktree skip: parent cwd is not a git repository"
	}
	branch := "orch/" + safeToken(orchestrationID) + "/" + safeToken(role)
	root := cfg.WorktreeDirRoot
	if !filepath.IsAbs(root) {
		root = filepath.Join(cwd, root)
	}
	childDir := filepath.Join(root, safeToken(orchestrationID), safeToken(role))
	if err := os.MkdirAll(filepath.Dir(childDir), sessionlog.PrivateDirMode); err != nil {
		return cwd, "", "worktree skip: " + err.Error()
	}
	if _, err := os.Stat(childDir); err == nil {
		return childDir, branch, "worktree reuse: " + childDir + " branch=" + branch
	}
	ctx2, cancel2 := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel2()
	cmd := exec.CommandContext(ctx2, "git", "-C", cwd, "worktree", "add", "-b", branch, childDir)
	if out, err := cmd.CombinedOutput(); err != nil {
		return cwd, "", "worktree skip: " + strings.TrimSpace(string(out)) + " " + err.Error()
	}
	return childDir, branch, "worktree created: " + childDir + " branch=" + branch
}

func appendBoardSection(path, role, text string) error {
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, sessionlog.PrivateFileMode)
	if err != nil {
		return err
	}
	defer f.Close()
	_, err = fmt.Fprintf(f, "\n## %s %s\n%s\n", role, time.Now().Format(time.RFC3339), text)
	return err
}

func sanitizeRole(role string) string {
	role = strings.ToLower(strings.TrimSpace(role))
	role = safeToken(role)
	return strings.Trim(role, "-_.")
}

func safeToken(value string) string {
	value = safeOrchestrationToken.ReplaceAllString(strings.TrimSpace(value), "-")
	value = strings.Trim(value, "-_.")
	if value == "" {
		return "item"
	}
	if len(value) > 80 {
		value = value[:80]
	}
	return value
}

func validOrchestrationProvider(provider string) bool {
	switch provider {
	case "claude", "codex", "copilot", "cursor-agent", "opencode", "grok":
		return true
	default:
		return false
	}
}
