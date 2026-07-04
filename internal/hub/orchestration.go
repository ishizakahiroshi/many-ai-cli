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
	"unicode"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

const orchestrationPollInterval = 2 * time.Second

// orchestrationInjectQuiet / orchestrationInjectMaxWait は spawn 直後の初期案内文注入前に
// 起動アニメーションの静止を待つ上限。quiet 続けば十分とみなし、静止しなくても maxWait で
// 諦めて注入する（プロバイダ起動が異常に遅い場合でも無期限に待たない）。
const (
	orchestrationInjectQuiet   = 300 * time.Millisecond
	orchestrationInjectMaxWait = 5 * time.Second
)

// injectInitialPrompt のエコー検証パラメータ。静止時間だけの判定は codex 等の
// 起動シーケンス途中の静止窓で誤発火し、readline 未起動の TUI に注入バイトが
// 捨てられる（bugfix_orchestration-codex-child-spawn-failures_2026-07-04.md 根本原因 A）。
// 注入後に PTY 画面へ注入テキストのエコーが現れることを実観測し、現れなければ再注入する。
const (
	orchestrationInjectEchoWait    = 3 * time.Second
	orchestrationInjectMaxAttempts = 4
	orchestrationInjectEchoPoll    = 100 * time.Millisecond
)

var safeOrchestrationToken = regexp.MustCompile(`[^a-zA-Z0-9._-]+`)

type orchestrationManager struct {
	mu       sync.Mutex
	appendMu sync.Mutex
	pending  map[string]pendingChild
	boards   map[string]*orchestrationBoard
	// roles は C1 (plan_orchestration-spawn-ui-exposure.md) で起動時に受け取った
	// 役割マッピングを orchestration_id ごとに保持する。conductor への instruction file
	// 注入（C2）はここから読み出す想定。
	roles    map[string]map[string]orchestrationRoleAssignment
	stopOnce sync.Once
	stopCh   chan struct{}
}

// orchestrationRoleAssignment は起動フォームの「子役割の詳細設定」アコーディオンで
// 役割ごとに指定された CLI + モデルの組。
type orchestrationRoleAssignment struct {
	Provider string `json:"provider"`
	Model    string `json:"model"`
}

type pendingChild struct {
	ParentSessionID int
	Role            string
	Auto            bool
	Depth           int
	OrchestrationID string
	BoardPath       string
	WorktreeBranch  string
	SpawnedAt       time.Time
}

type orchestrationBoard struct {
	ID         string
	Path       string
	Sessions   map[int]string
	Children   map[int]*orchestrationChild
	Done       map[int]bool
	IdleWarned map[int]bool
	TimedOut   map[int]bool
	LastSize   int64
	LastMod    time.Time
	LastWrite  time.Time
}

type orchestrationChild struct {
	ID             int
	ParentID       int
	Role           string
	SpawnedAt      time.Time
	LastBoardWrite time.Time
	Done           bool
}

type boardDoneEvent struct {
	Role      string
	SessionID int
}

type boardWriter struct {
	Role      string
	SessionID int
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
		roles:   map[string]map[string]orchestrationRoleAssignment{},
		stopCh:  make(chan struct{}),
	}
}

// reserveOrchestrationConductor は「オーケストレーション」ボタン経由の起動リクエストを
// 受け取った時点で conductor 用の orchestration_id を予約する（plan_orchestration-spawn-ui-exposure.md
// C1）。board.md の実体生成は最初の spawn-child 呼び出し時（ensureOrchestrationBoard）まで遅延させる
// 軽量な処理にとどめる。予約は spawn 時に決めた label をキーにした pending map 経由で、
// wrapperLoop の WS register 時（reg.Label 一致）に session へ適用される。
func (s *Server) reserveOrchestrationConductor(label string, roles map[string]orchestrationRoleAssignment) string {
	orchestrationID := fmt.Sprintf("o%d", time.Now().UnixNano())
	s.orchestration.mu.Lock()
	s.orchestration.pending[label] = pendingChild{
		OrchestrationID: orchestrationID,
		SpawnedAt:       time.Now(),
	}
	if len(roles) > 0 {
		if s.orchestration.roles == nil {
			s.orchestration.roles = map[string]map[string]orchestrationRoleAssignment{}
		}
		s.orchestration.roles[orchestrationID] = roles
	}
	s.orchestration.mu.Unlock()
	return orchestrationID
}

// orchestrationRolesFor は C1 で予約された役割マッピングの取得口。
// conductor への instruction file 注入（C2）から参照される想定で、C1 時点では未使用。
func (s *Server) orchestrationRolesFor(id string) map[string]orchestrationRoleAssignment {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	return s.orchestration.roles[id]
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
	if strings.TrimSpace(body.Role) == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "role is required")
		return
	}
	body.Role = sanitizeRole(body.Role)
	if body.Role == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "role is required")
		return
	}

	cfg := s.snapshotCfg().Orchestration
	parent, childCount, totalSessions := s.orchestrationParentState(parentID)
	if parent == nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "parent session not found")
		return
	}

	// C2 (plan_orchestration-spawn-ui-exposure.md): `many-ai-cli orchestrate spawn`
	// は provider/model を省略できる。省略時は起動フォームの詳細設定で決めた
	// role→provider/model マッピングから自動解決する（AI に provider/model を
	// 判断させたくない「詳細設定あり」ケース向け）。明示指定があれば常に優先する。
	if body.Provider == "" || body.Model == "" {
		if roles := s.orchestrationRolesFor(parent.OrchestrationID); roles != nil {
			if ra, ok := roles[body.Role]; ok {
				if body.Provider == "" {
					body.Provider = ra.Provider
				}
				if body.Model == "" {
					body.Model = ra.Model
				}
			}
		}
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
	applyChildApprovalDefaults(&body)
	if parent.Depth >= cfg.MaxDepth {
		s.notifyOrchestrationError(parentID, "depth", "max orchestration depth reached")
		writeJSONStatus(w, http.StatusTooManyRequests, httpErrorResp{OK: false, Error: "orchestration_limit", Detail: "max orchestration depth reached"})
		return
	}
	if childCount >= cfg.MaxChildrenPerParent {
		s.notifyOrchestrationError(parentID, "children_per_parent", "max children per parent reached")
		writeJSONStatus(w, http.StatusTooManyRequests, httpErrorResp{OK: false, Error: "orchestration_limit", Detail: "max children per parent reached"})
		return
	}
	if totalSessions >= cfg.MaxTotalSessions {
		s.notifyOrchestrationError(parentID, "total_sessions", "max total sessions reached")
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
	s.markConductor(parentID, orchestrationID, boardPath)
	childCWD, branch, worktreeNote := s.prepareChildWorktree(cwd, orchestrationID, body.Role, cfg)
	if worktreeNote != "" {
		_ = s.appendBoardSection(boardPath, "hub", fmt.Sprintf("%s\n", worktreeNote))
	}

	label := fmt.Sprintf("orch-%s-%s-%d", safeToken(orchestrationID), body.Role, time.Now().UnixNano())
	spawnedAt := time.Now()
	meta := pendingChild{
		ParentSessionID: parentID,
		Role:            body.Role,
		Auto:            true,
		Depth:           parent.Depth + 1,
		OrchestrationID: orchestrationID,
		BoardPath:       boardPath,
		WorktreeBranch:  branch,
		SpawnedAt:       spawnedAt,
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
	s.registerBoardChild(orchestrationID, boardPath, childID, parentID, body.Role, spawnedAt)
	prompt := buildChildInitialPrompt(body.InitialPrompt, boardPath, body.Role, branch, childID)
	// goroutine で注入する: エコー検証リトライは数十秒かかりうるため、conductor の
	// spawn 呼び出し（HTTP レスポンス）をブロックしない。注入完了までのユーザー入力は
	// initialInjectPending ゲート（registerLoop で設定）が pendingInput へ保留する。
	go s.injectInitialPrompt(childID, prompt)
	writeJSON(w, map[string]any{"ok": true, "session_id": childID, "board_path": boardPath, "cwd": childCWD, "worktree_branch": branch})
}

// applyChildApprovalDefaults は orchestration 子セッションの承認モード既定値を埋める。
// 子は自走が前提のため、呼び出し側（conductor）が指定しない場合はプロバイダごとの
// 全許可（承認バイパス相当）を既定にする。子の承認プロンプトを人間が張り付いて
// 処理する運用は自走目的と矛盾するため。危険操作の抑制は承認ではなく worktree 隔離と
// board の禁止事項で行う。承認バイパスは spawn リスクゲート（evaluateClaudeRisk /
// evaluateCodexRisk）で HighRisk 扱いになるが、人間の同意はオーケストレーション開始時に
// 済んでいるため子 spawn では確認済みとして扱う
// （docs/local/bugfix_orchestration-codex-child-spawn-failures_2026-07-04.md）。
func applyChildApprovalDefaults(body *spawnChildRequest) {
	switch body.Provider {
	case "shell":
		return
	case "codex":
		if body.AskForApproval == "" {
			body.AskForApproval = "never"
		}
		if body.Sandbox == "" {
			body.Sandbox = "danger-full-access"
		}
	default:
		// claude / grok は --permission-mode をネイティブサポート。
		// copilot / cursor-agent / opencode は wrapper 側で各 CLI の全許可指定
		// （--allow-all / --force / opencode.json permission "*":"allow"）に変換される。
		if body.PermissionMode == "" {
			body.PermissionMode = "bypassPermissions"
		}
	}
	body.RiskConfirmed = true
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

// orchestrationDir は board.md 一式を格納する `~/.many-ai-cli/orchestration` を返す。
// files-list / files-content の許可ルート拡張（board.md 閲覧導線）でも参照する。
func orchestrationDir() (string, error) {
	base, err := config.Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(base, "orchestration"), nil
}

func (s *Server) ensureOrchestrationBoard(id string, parent *session, body spawnChildRequest) (string, error) {
	base, err := orchestrationDir()
	if err != nil {
		return "", err
	}
	dir := filepath.Join(base, safeToken(id))
	if err := os.MkdirAll(dir, sessionlog.PrivateDirMode); err != nil {
		return "", err
	}
	path := filepath.Join(dir, "board.md")
	if _, err := os.Stat(path); os.IsNotExist(err) {
		content := fmt.Sprintf("# Orchestration %s\n\n- conductor: session #%d provider=%s model=%s\n- purpose: %s\n\n## conductor %s\nCreated board. Children must append progress sections and finish with `## DONE <role> session=<child_id>`.\n",
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
		b = newOrchestrationBoard(id, path)
		s.orchestration.boards[id] = b
	}
	b.Sessions[sessionID] = role
	s.orchestration.mu.Unlock()
}

func (s *Server) registerBoardChild(id, path string, sessionID, parentID int, role string, spawnedAt time.Time) {
	s.orchestration.mu.Lock()
	b := s.orchestration.boards[id]
	if b == nil {
		b = newOrchestrationBoard(id, path)
		s.orchestration.boards[id] = b
	}
	if spawnedAt.IsZero() {
		spawnedAt = time.Now()
	}
	b.Sessions[sessionID] = role
	b.Children[sessionID] = &orchestrationChild{
		ID:             sessionID,
		ParentID:       parentID,
		Role:           role,
		SpawnedAt:      spawnedAt,
		LastBoardWrite: spawnedAt,
	}
	s.orchestration.mu.Unlock()
}

func newOrchestrationBoard(id, path string) *orchestrationBoard {
	now := time.Now()
	return &orchestrationBoard{
		ID:         id,
		Path:       path,
		Sessions:   map[int]string{},
		Children:   map[int]*orchestrationChild{},
		Done:       map[int]bool{},
		IdleWarned: map[int]bool{},
		TimedOut:   map[int]bool{},
		LastWrite:  now,
	}
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
	now := time.Now()
	cfg := s.snapshotCfg().Orchestration
	s.orchestration.mu.Lock()
	boards := make([]*orchestrationBoard, 0, len(s.orchestration.boards))
	for _, b := range s.orchestration.boards {
		boards = append(boards, b)
	}
	s.orchestration.mu.Unlock()
	for _, b := range boards {
		info, err := os.Stat(b.Path)
		if err != nil {
			s.checkOrchestrationChildTimers(b.ID, now, cfg)
			continue
		}
		changed := true
		if info.Size() == b.LastSize && info.ModTime().Equal(b.LastMod) {
			changed = false
		}
		if changed {
			data, err := os.ReadFile(b.Path)
			if err == nil {
				s.handleBoardChange(b.ID, b.Path, info, string(data), now)
			}
		}
		s.checkOrchestrationChildTimers(b.ID, now, cfg)
	}
}

func (s *Server) handleBoardChange(boardID, boardPath string, info os.FileInfo, text string, now time.Time) {
	s.orchestration.mu.Lock()
	stored := s.orchestration.boards[boardID]
	if stored == nil {
		s.orchestration.mu.Unlock()
		return
	}
	dones := detectBoardDoneEvents(text)
	writer := detectLastBoardWriter(text)
	writerID := s.resolveBoardWriterLocked(stored, writer)
	stored.LastSize = info.Size()
	stored.LastMod = info.ModTime()
	stored.LastWrite = now
	for _, child := range stored.Children {
		if writerID == child.ID || (writerID == 0 && writer.Role == child.Role) {
			child.LastBoardWrite = now
		}
	}
	for _, ev := range dones {
		sessionID := ev.SessionID
		if sessionID == 0 {
			sessionID = uniqueChildSessionForRole(stored, ev.Role)
		}
		if sessionID != 0 {
			stored.Done[sessionID] = true
			if child := stored.Children[sessionID]; child != nil {
				child.Done = true
				child.LastBoardWrite = now
			}
		}
	}
	sessions := map[int]string{}
	for id, role := range stored.Sessions {
		sessions[id] = role
	}
	s.orchestration.mu.Unlock()

	updatedBy := writer.Role
	if updatedBy == "" {
		updatedBy = "unknown"
	}
	for sessionID := range sessions {
		if writerID != 0 && sessionID == writerID {
			continue
		}
		s.injectText(sessionID, fmt.Sprintf("\n[orchestration] board updated by %s: %s\n", updatedBy, boardPath), true, false)
	}
	for _, ev := range dones {
		sessionID := ev.SessionID
		if sessionID == 0 {
			s.orchestration.mu.Lock()
			stored := s.orchestration.boards[boardID]
			if stored != nil {
				sessionID = uniqueChildSessionForRole(stored, ev.Role)
			}
			s.orchestration.mu.Unlock()
		}
		if sessionID != 0 {
			s.markChildState(sessionID, "done")
		}
	}
}

func (s *Server) resolveBoardWriterLocked(b *orchestrationBoard, writer boardWriter) int {
	if writer.SessionID != 0 {
		if _, ok := b.Sessions[writer.SessionID]; ok {
			return writer.SessionID
		}
	}
	return uniqueChildSessionForRole(b, writer.Role)
}

func uniqueChildSessionForRole(b *orchestrationBoard, role string) int {
	if b == nil || role == "" {
		return 0
	}
	found := 0
	for id, child := range b.Children {
		if child.Role != role {
			continue
		}
		if found != 0 {
			return 0
		}
		found = id
	}
	return found
}

func (s *Server) checkOrchestrationChildTimers(boardID string, now time.Time, cfg config.OrchestrationConfig) {
	type notice struct {
		parentID  int
		childID   int
		role      string
		kind      string
		state     string
		threshold int
	}
	var notices []notice
	s.orchestration.mu.Lock()
	b := s.orchestration.boards[boardID]
	if b != nil {
		for id, child := range b.Children {
			if child.Done || b.Done[id] || b.TimedOut[id] {
				continue
			}
			if cfg.ChildTimeoutSeconds > 0 && now.Sub(child.SpawnedAt) > time.Duration(cfg.ChildTimeoutSeconds)*time.Second {
				b.TimedOut[id] = true
				notices = append(notices, notice{parentID: child.ParentID, childID: id, role: child.Role, kind: "timeout", state: "timeout", threshold: cfg.ChildTimeoutSeconds})
				continue
			}
			if cfg.IdleDoneThresholdSec > 0 && !b.IdleWarned[id] && now.Sub(child.LastBoardWrite) > time.Duration(cfg.IdleDoneThresholdSec)*time.Second {
				b.IdleWarned[id] = true
				notices = append(notices, notice{parentID: child.ParentID, childID: id, role: child.Role, kind: "idle", threshold: cfg.IdleDoneThresholdSec})
			}
		}
	}
	s.orchestration.mu.Unlock()
	for _, n := range notices {
		switch n.kind {
		case "timeout":
			s.notifyOrchestrationError(n.parentID, "timeout", fmt.Sprintf("role=%s id=%d threshold=%ds", n.role, n.childID, n.threshold))
			s.markChildState(n.childID, n.state)
		case "idle":
			s.injectText(n.parentID, fmt.Sprintf("\n[orchestration] idle warning role=%s id=%d no board update for %ds\n", n.role, n.childID, n.threshold), true, false)
		}
	}
}

func detectBoardDoneRoles(text string) []string {
	events := detectBoardDoneEvents(text)
	var roles []string
	seen := map[string]bool{}
	for _, ev := range events {
		if ev.Role != "" && !seen[ev.Role] {
			roles = append(roles, ev.Role)
			seen[ev.Role] = true
		}
	}
	return roles
}

func detectBoardDoneEvents(text string) []boardDoneEvent {
	var events []boardDoneEvent
	seen := map[boardDoneEvent]bool{}
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
		if role == "" {
			continue
		}
		ev := boardDoneEvent{Role: role, SessionID: parseBoardSessionID(fields[1:])}
		if !seen[ev] {
			events = append(events, ev)
			seen[ev] = true
		}
	}
	return events
}

func detectLastBoardRole(text string) string {
	writer := detectLastBoardWriter(text)
	if writer.Role == "" {
		return "unknown"
	}
	return writer.Role
}

func detectLastBoardWriter(text string) boardWriter {
	var writer boardWriter
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if !strings.HasPrefix(line, "## ") || strings.HasPrefix(line, "## DONE ") {
			continue
		}
		fields := strings.Fields(strings.TrimPrefix(line, "## "))
		if len(fields) > 0 {
			writer = boardWriter{Role: sanitizeRole(fields[0]), SessionID: parseBoardSessionID(fields[1:])}
		}
	}
	return writer
}

func parseBoardSessionID(fields []string) int {
	for _, field := range fields {
		if !strings.HasPrefix(field, "session=") {
			continue
		}
		id, err := strconv.Atoi(strings.TrimPrefix(field, "session="))
		if err == nil && id > 0 {
			return id
		}
	}
	return 0
}

// markConductor は spawn-child の親セッションに orchestration_id / board_path を
// 記録し、conductor カード用の情報をフロントへ配信する。子セッションと異なり
// 親は spawnWrappedSession を経由しないため、ここで初めて自身が conductor である
// ことを session_update として通知する必要がある。
func (s *Server) markConductor(parentID int, orchestrationID, boardPath string) {
	s.sessionsMu.Lock()
	ses := s.sessions[parentID]
	if ses == nil || (ses.OrchestrationID == orchestrationID && ses.BoardPath == boardPath) {
		s.sessionsMu.Unlock()
		return
	}
	ses.OrchestrationID = orchestrationID
	ses.BoardPath = boardPath
	msg := sessionUpdateMessage(ses)
	s.sessionsMu.Unlock()
	s.broadcast(msg)
}

func (s *Server) markChildState(sessionID int, state string) {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	if ses == nil || ses.State == state {
		s.sessionsMu.Unlock()
		return
	}
	ses.State = state
	msg := sessionUpdateMessage(ses)
	s.sessionsMu.Unlock()
	s.broadcast(msg)
	if s.sessionStore != nil {
		s.sessionStore.UpdateSessionState(msg.SessionID, state, "")
	}
}

func sessionUpdateMessage(ses *session) proto.Message {
	return proto.Message{
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
	}
}

func (s *Server) notifyOrchestrationError(parentID int, limit, detail string) {
	s.injectText(parentID, fmt.Sprintf("\n[MANY-AI-CLI-ORCHESTRATION-ERROR] limit=%s detail=%s\n", safeToken(limit), strings.ReplaceAll(detail, "\n", " ")), true, false)
}

// waitForInputReady は spawn 直後の起動アニメーション（スプラッシュ・Tips バナー等）が
// 描画し終わるまで待つ。この静止を待たずに注入すると、readline がまだ起動しきっていない
// タイミングで Enter が飲み込まれ、案内文だけが入力欄に残って未送信のまま停止する
// （conductor セッションが起動時に何も実行しない不具合の原因）。
func (s *Server) waitForInputReady(sessionID int, quiet, maxWait time.Duration) {
	deadline := time.Now().Add(maxWait)
	for {
		s.sessionsMu.Lock()
		ses := s.sessions[sessionID]
		var last time.Time
		if ses != nil {
			last = ses.lastOutputAt
		}
		s.sessionsMu.Unlock()
		if ses == nil {
			return
		}
		if !last.IsZero() && time.Since(last) >= quiet {
			return
		}
		if time.Now().After(deadline) {
			return
		}
		time.Sleep(50 * time.Millisecond)
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

// injectRawBypassGate は初期プロンプト注入（injectInitialPrompt）専用の送信経路。
// initialInjectPending ゲート中でも wrapper へ直接届ける（通常経路だと注入自体が
// pendingInput へ回ってデッドロックするため）。
func (s *Server) injectRawBypassGate(sessionID int, text string) {
	s.sessionsMu.Lock()
	wc := s.wrappers[sessionID]
	s.sessionsMu.Unlock()
	s.submitInputWithGate(wc, sessionID, text, true)
}

// injectInitialPrompt は orchestration セッション（conductor / 子）への初期プロンプトを、
// CLI の入力受付開始を実観測しながら注入する。手順:
//  1. waitForInputReady で出力静止を待つ（従来判定・早期注入の目安）
//  2. 注入し、PTY 画面（VT バッファ）に注入テキストのエコーが現れるかを確認
//  3. 現れなければ CLI 起動途中で入力が捨てられたとみなし再注入（最大 MaxAttempts 回）
//
// 完了・断念にかかわらず initialInjectPending ゲートを解除し、保留中のユーザー入力を
// 順番どおり flush する。goroutine で呼ぶこと（エコー検証で数十秒ブロックしうる）。
func (s *Server) injectInitialPrompt(sessionID int, prompt string) {
	defer s.clearInitialInjectGate(sessionID)
	marker := injectEchoMarker(prompt)
	text := prompt
	if !strings.HasSuffix(text, "\r") {
		text += "\r"
	}
	for attempt := 1; attempt <= orchestrationInjectMaxAttempts; attempt++ {
		s.waitForInputReady(sessionID, orchestrationInjectQuiet, orchestrationInjectMaxWait)
		if attempt > 1 && s.waitForInjectEcho(sessionID, marker, 0) {
			// 前回注入分のエコーが遅れて描画された場合は再注入しない（二重送信防止）
			s.logger.Info("initial prompt echo observed late", "session_id", sessionID, "attempt", attempt)
			return
		}
		s.injectRawBypassGate(sessionID, text)
		if s.waitForInjectEcho(sessionID, marker, orchestrationInjectEchoWait) {
			if attempt > 1 {
				s.logger.Info("initial prompt injected after retry", "session_id", sessionID, "attempt", attempt)
			}
			return
		}
		s.logger.Warn("initial prompt echo not observed; retrying", "session_id", sessionID, "attempt", attempt)
	}
	s.logger.Warn("initial prompt injection gave up", "session_id", sessionID, "attempts", orchestrationInjectMaxAttempts)
}

// clearInitialInjectGate は初期注入ゲートを解除し、ゲート中に溜まったユーザー入力を flush する。
func (s *Server) clearInitialInjectGate(sessionID int) {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	if ses != nil {
		ses.initialInjectPending = false
	}
	s.sessionsMu.Unlock()
	if ses == nil {
		return
	}
	s.flushPendingInput(sessionID)
}

// waitForInjectEcho は注入テキストのエコー（marker）が PTY 画面に現れるまで待つ。
// TUI の折り返しに影響されないよう、画面テキストと marker の双方から空白を除いて比較する。
func (s *Server) waitForInjectEcho(sessionID int, marker string, maxWait time.Duration) bool {
	if marker == "" {
		return true
	}
	deadline := time.Now().Add(maxWait)
	for {
		s.sessionsMu.Lock()
		ses := s.sessions[sessionID]
		var screen string
		if ses != nil && ses.vt != nil {
			screen = collapseWhitespace(strings.Join(ses.vt.Lines(), ""))
		}
		s.sessionsMu.Unlock()
		if ses == nil {
			return false
		}
		if strings.Contains(screen, marker) {
			return true
		}
		if time.Now().After(deadline) {
			return false
		}
		time.Sleep(orchestrationInjectEchoPoll)
	}
}

// injectEchoMarker は注入テキストの先頭行から、エコー検出用の空白除去済み部分文字列を作る。
func injectEchoMarker(prompt string) string {
	line := prompt
	if i := strings.IndexByte(line, '\n'); i >= 0 {
		line = line[:i]
	}
	line = collapseWhitespace(line)
	const maxMarkerRunes = 32
	runes := []rune(line)
	if len(runes) > maxMarkerRunes {
		runes = runes[:maxMarkerRunes]
	}
	return string(runes)
}

// collapseWhitespace は全空白文字（改行含む）を除去する。
func collapseWhitespace(text string) string {
	var b strings.Builder
	b.Grow(len(text))
	for _, r := range text {
		if !unicode.IsSpace(r) {
			b.WriteRune(r)
		}
	}
	return b.String()
}

func buildChildInitialPrompt(base, boardPath, role, branch string, sessionID int) string {
	var b strings.Builder
	b.WriteString("You are an orchestration child session.\n")
	b.WriteString("Role: " + role + "\n")
	b.WriteString("Session ID: " + strconv.Itoa(sessionID) + "\n")
	b.WriteString("Shared board: " + boardPath + "\n")
	if branch != "" {
		b.WriteString("Worktree branch: " + branch + "\n")
	}
	b.WriteString("Read the board before acting. Append progress as `## " + role + " session=" + strconv.Itoa(sessionID) + " <RFC3339 time>` sections. When complete, append `## DONE " + role + " session=" + strconv.Itoa(sessionID) + "` and a concise summary.\n\n")
	b.WriteString(base)
	return b.String()
}

// buildConductorInitialPrompt は plan_orchestration-spawn-ui-exposure.md C2 の
// conductor 向け起動時案内。詳細設定（roles）の有無で内容を分岐する:
//   - あり: role→provider/model の対応表をそのまま提示し、
//     `many-ai-cli orchestrate spawn --role <role> "<prompt>"` だけで済むことを案内する
//   - なし: 子が必要になった時点で provider/model を明示指定する自律運用を案内する
func buildConductorInitialPrompt(orchestrationID string, roles map[string]orchestrationRoleAssignment) string {
	var b strings.Builder
	b.WriteString("You are an orchestration conductor session (many-ai-cli).\n")
	b.WriteString("Orchestration ID: " + orchestrationID + "\n")
	if len(roles) > 0 {
		b.WriteString("Configured child roles (provider/model already decided by the user):\n")
		for role, ra := range roles {
			b.WriteString(fmt.Sprintf("- %s: provider=%s model=%s\n", role, ra.Provider, ra.Model))
		}
		b.WriteString("To spawn a child for a role above, run:\n")
		b.WriteString("  many-ai-cli orchestrate spawn --role <role> \"<prompt>\"\n")
		b.WriteString("(provider/model are resolved automatically from the mapping above; pass --provider/--model to override a specific spawn.)\n")
	} else {
		b.WriteString("No child role mapping was configured. Decide provider/model yourself whenever a child is needed and run:\n")
		b.WriteString("  many-ai-cli orchestrate spawn --role <role> --provider <provider> --model <model> \"<prompt>\"\n")
	}
	b.WriteString("Do not call the Hub HTTP API or handle any auth token directly; this subcommand does it for you.\n")
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

func (s *Server) appendBoardSection(path, role, text string) error {
	s.orchestration.appendMu.Lock()
	defer s.orchestration.appendMu.Unlock()
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
