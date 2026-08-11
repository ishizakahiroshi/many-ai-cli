package hub

import (
	"context"
	"errors"
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

const orchestrationSpawnConfirmTimeout = 2 * time.Minute

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
	roles              map[string]map[string]orchestrationRoleAssignment
	spawnConfirmations map[string]chan spawnConfirmationDecision
	stopOnce           sync.Once
	stopCh             chan struct{}
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
	NormalWorktree  normalWorktree
	WorktreeCleanup string
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
	// PendingNotices は queue-until-idle の conductor ごとの最新通知。連続した
	// board 更新はここで上書きし、idle 復帰時に Enter を 1 回だけ送る。
	PendingNotices map[int]string
}

type orchestrationChild struct {
	ID             int
	ParentID       int
	Role           string
	SpawnedAt      time.Time
	LastBoardWrite time.Time
	Done           bool
	// FilePath は子専用の進捗ファイル（board と同じディレクトリの child-<ID>.md）。
	// 子の進捗・DONE 記帳を board.md から分離し、共有ファイルの追記競合と
	// 記帳名義ゆれを構造的に解消する（plan_orchestration-conductor-improvements.md C4）。
	// 旧プロンプトで動く子（board.md へ直接記帳）は従来の writer 検出で追従する。
	FilePath string
	FileSize int64
	FileMod  time.Time
	// Restart data is retained only in memory for opt-in timeout recovery.
	RestartSpec    spawnWrappedSpec
	InitialPrompt  string
	WorktreeBranch string
	TimeoutRetries int
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
	// Force は同 role の生存子がいても新規 spawn を許可する（重複 spawn ガードの明示迂回。
	// plan_orchestration-conductor-improvements.md C2）。
	Force bool `json:"force"`
}

type spawnConfirmationDecision struct {
	Approved bool
	Provider string
	Model    string
}

type spawnConfirmationResponse struct {
	ConfirmationID string `json:"confirmation_id"`
	Approved       bool   `json:"approved"`
	Provider       string `json:"provider"`
	Model          string `json:"model"`
}

// sendChildRequest は POST /api/sessions/:id/send-child のリクエスト。
// conductor が既存の子へ追加指示を送る（spawn 枠を消費しない指示経路）。
type sendChildRequest struct {
	Role string `json:"role"`
	Text string `json:"text"`
}

type injectRequest struct {
	Text          string `json:"text"`
	FromSessionID int    `json:"from_session_id"`
	PressEnter    bool   `json:"press_enter"`
	Interrupt     bool   `json:"interrupt"`
}

func newOrchestrationManager() *orchestrationManager {
	return &orchestrationManager{
		pending:            map[string]pendingChild{},
		boards:             map[string]*orchestrationBoard{},
		roles:              map[string]map[string]orchestrationRoleAssignment{},
		spawnConfirmations: map[string]chan spawnConfirmationDecision{},
		stopCh:             make(chan struct{}),
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

func (s *Server) spawnConfirmationRequired(provider string) bool {
	cfg := s.snapshotCfg().Orchestration
	switch config.EffectiveSpawnConfirmMode(cfg.SpawnConfirmMode) {
	case config.SpawnConfirmOff:
		return false
	case config.SpawnConfirmProviders:
		for _, allowed := range cfg.SpawnConfirmProviders {
			if strings.EqualFold(strings.TrimSpace(allowed), provider) {
				return true
			}
		}
		return false
	default:
		return true
	}
}

// awaitSpawnConfirmation publishes a request to every connected browser and
// holds only this HTTP request. The first decision wins; a missing UI or an
// expired request is intentionally treated as refusal.
func (s *Server) awaitSpawnConfirmation(ctx context.Context, parent *session, body spawnChildRequest) (spawnConfirmationDecision, bool) {
	id := fmt.Sprintf("sc-%d", time.Now().UnixNano())
	result := make(chan spawnConfirmationDecision, 1)
	s.orchestration.mu.Lock()
	s.orchestration.spawnConfirmations[id] = result
	s.orchestration.mu.Unlock()
	defer func() {
		s.orchestration.mu.Lock()
		delete(s.orchestration.spawnConfirmations, id)
		s.orchestration.mu.Unlock()
	}()
	s.broadcast(proto.Message{
		Type: "spawn_confirmation_requested", SpawnConfirmationID: id,
		SessionID: parent.ID, Role: body.Role, Provider: body.Provider, Model: body.Model,
		CWD: body.CWD, InitialPrompt: body.InitialPrompt,
	})
	timer := time.NewTimer(orchestrationSpawnConfirmTimeout)
	defer timer.Stop()
	select {
	case decision := <-result:
		return decision, decision.Approved
	case <-ctx.Done():
		return spawnConfirmationDecision{}, false
	case <-timer.C:
		return spawnConfirmationDecision{}, false
	}
}

func (s *Server) handleSpawnConfirmation(w http.ResponseWriter, r *http.Request, parentID int) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body spawnConfirmationResponse
	if !decodeJSON(w, r, &body) {
		return
	}
	if strings.TrimSpace(body.ConfirmationID) == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "confirmation_id is required")
		return
	}
	s.orchestration.mu.Lock()
	ch := s.orchestration.spawnConfirmations[body.ConfirmationID]
	s.orchestration.mu.Unlock()
	if ch == nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "spawn confirmation is no longer pending")
		return
	}
	select {
	case ch <- spawnConfirmationDecision{Approved: body.Approved, Provider: body.Provider, Model: body.Model}:
		writeJSON(w, map[string]bool{"ok": true})
	default:
		writeJSONError(w, http.StatusConflict, "already_decided", "spawn confirmation has already been decided")
	}
}

func (s *Server) ensureOrchestrationBoardForRefusal(parent *session, body spawnChildRequest) (string, error) {
	orchestrationID := parent.OrchestrationID
	if orchestrationID == "" {
		orchestrationID = fmt.Sprintf("s%d-%d", parent.ID, time.Now().UnixNano())
	}
	boardPath, err := s.ensureOrchestrationBoard(orchestrationID, parent, body)
	if err != nil {
		return "", err
	}
	s.markConductor(parent.ID, orchestrationID, boardPath)
	s.registerBoardSession(orchestrationID, boardPath, parent.ID, "conductor")
	return boardPath, nil
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
	case "meta":
		s.handleSessionMeta(w, r, id)
	case "spawn-child":
		s.handleSpawnChild(w, r, id)
	case "spawn-confirm":
		s.handleSpawnConfirmation(w, r, id)
	case "send-child":
		s.handleSendChild(w, r, id)
	case "inject":
		s.handleSessionInject(w, r, id)
	case "children":
		s.handleSessionChildren(w, r, id)
	default:
		writeJSONError(w, http.StatusNotFound, "not_found", "not found")
	}
}

type childSpawnPreparation struct {
	orchestrationID string
	boardPath       string
	childCWD        string
	branch          string
}

type childSpawnResult struct {
	ID             int
	BoardPath      string
	CWD            string
	WorktreeBranch string
}

type spawnPreparationError struct {
	status int
	code   string
	detail string
}

func (e *spawnPreparationError) Error() string { return e.detail }

// resolveChildRole validates and normalizes the role before it participates
// in board paths, labels, or child prompts.
func resolveChildRole(body *spawnChildRequest) error {
	if strings.TrimSpace(body.Role) == "" {
		return fmt.Errorf("role is required")
	}
	body.Role = sanitizeRole(body.Role)
	if body.Role == "" {
		return fmt.Errorf("role is required")
	}
	return nil
}

func (s *Server) preparePromptAndWorktree(parentID int, parent *session, body spawnChildRequest, cfg config.OrchestrationConfig) (childSpawnPreparation, error) {
	cwd := strings.TrimSpace(body.CWD)
	if cwd == "" {
		cwd = parent.CWD
	}
	if cwd == "" {
		cwd = s.hubCWD
	}
	info, statErr := os.Stat(cwd)
	if statErr != nil || !info.IsDir() {
		return childSpawnPreparation{}, &spawnPreparationError{status: http.StatusBadRequest, code: "bad_request", detail: "cwd does not exist or is not a directory"}
	}
	if spawnCwdTooBroad(cwd) {
		return childSpawnPreparation{}, &spawnPreparationError{status: http.StatusBadRequest, code: "bad_request", detail: "cwd is too broad (system root or home root)"}
	}

	orchestrationID := parent.OrchestrationID
	if orchestrationID == "" {
		orchestrationID = fmt.Sprintf("s%d-%d", parentID, time.Now().UnixNano())
	}
	boardPath, err := s.ensureOrchestrationBoard(orchestrationID, parent, body)
	if err != nil {
		return childSpawnPreparation{}, &spawnPreparationError{status: http.StatusInternalServerError, code: "board_error", detail: errorDetail("board error", err)}
	}
	s.markConductor(parentID, orchestrationID, boardPath)
	childCWD, branch, worktreeNote := s.prepareChildWorktree(cwd, orchestrationID, body.Role, cfg)
	if worktreeNote != "" {
		_ = s.appendBoardSection(boardPath, "hub", fmt.Sprintf("%s\n", worktreeNote))
	}
	return childSpawnPreparation{orchestrationID: orchestrationID, boardPath: boardPath, childCWD: childCWD, branch: branch}, nil
}

func (s *Server) dispatchSpawn(parentID int, parent *session, body spawnChildRequest, prep childSpawnPreparation) (childSpawnResult, error) {
	label := fmt.Sprintf("orch-%s-%s-%d", safeToken(prep.orchestrationID), body.Role, time.Now().UnixNano())
	spawnedAt := time.Now()
	meta := pendingChild{
		ParentSessionID: parentID,
		Role:            body.Role,
		Auto:            true,
		Depth:           parent.Depth + 1,
		OrchestrationID: prep.orchestrationID,
		BoardPath:       prep.boardPath,
		WorktreeBranch:  prep.branch,
		SpawnedAt:       spawnedAt,
	}
	s.orchestration.mu.Lock()
	s.orchestration.pending[label] = meta
	s.orchestration.mu.Unlock()
	childID, err := s.spawnWrappedSession(spawnWrappedSpec{
		Provider:       body.Provider,
		CWD:            prep.childCWD,
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
		return childSpawnResult{}, err
	}
	s.registerBoardSession(prep.orchestrationID, prep.boardPath, parentID, "conductor")
	s.registerBoardChild(prep.orchestrationID, prep.boardPath, childID, parentID, body.Role, spawnedAt)
	s.setChildRestartData(prep.orchestrationID, childID, spawnWrappedSpec{
		Provider: body.Provider, CWD: prep.childCWD, Model: body.Model, ModelSelection: body.ModelSelection,
		RiskConfirmed: true, PermissionMode: body.PermissionMode, Sandbox: body.Sandbox,
		AskForApproval: body.AskForApproval, Route: body.Route,
	}, body.InitialPrompt, prep.branch, 0)
	prompt := buildChildInitialPrompt(body.InitialPrompt, prep.boardPath, body.Role, prep.branch, childID)
	s.safeGo("inject_initial_prompt_child", func() { s.injectInitialPrompt(childID, prompt) })
	return childSpawnResult{ID: childID, BoardPath: prep.boardPath, CWD: prep.childCWD, WorktreeBranch: prep.branch}, nil
}

func (s *Server) handleSpawnChild(w http.ResponseWriter, r *http.Request, parentID int) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body spawnChildRequest
	if !decodeJSON(w, r, &body) {
		return
	}
	if err := resolveChildRole(&body); err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
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
	if s.spawnConfirmationRequired(body.Provider) {
		decision, ok := s.awaitSpawnConfirmation(r.Context(), parent, body)
		if !ok {
			boardPath, boardErr := s.ensureOrchestrationBoardForRefusal(parent, body)
			if boardErr == nil {
				_ = s.appendBoardSection(boardPath, "hub", fmt.Sprintf("refused: role=%s reason=user_refusal\n", body.Role))
			}
			writeJSONStatus(w, http.StatusForbidden, httpErrorResp{OK: false, Error: "spawn_refused", Detail: "child spawn was refused by the user"})
			return
		}
		if strings.TrimSpace(decision.Provider) != "" {
			body.Provider = strings.TrimSpace(decision.Provider)
		}
		if strings.TrimSpace(decision.Model) != "" {
			body.Model = strings.TrimSpace(decision.Model)
		}
		if !validOrchestrationProvider(body.Provider) || !spawnValidModelLabel(body.Model) || strings.HasPrefix(body.Model, "-") {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid provider or model selected for child")
			return
		}
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
	// 同 role の生存子がいる場合の重複 spawn ガード。conductor が修正指示・再確認指示を
	// 毎回 spawn で出すと生存子数が max_children_per_parent に到達して詰まる実測があった
	// （plan_orchestration-conductor-improvements.md C2）。既存子への指示は send を使わせる。
	if !body.Force {
		if live := s.liveChildForRole(parentID, body.Role); live != nil {
			writeJSONStatus(w, http.StatusConflict, httpErrorResp{OK: false, Error: "duplicate_role_child", Detail: fmt.Sprintf("live child #%d already exists for role %q; use `many-ai-cli orchestrate send --role %s \"<text>\"` to instruct it, or pass --force to spawn another", live.ID, body.Role, body.Role)})
			return
		}
	}

	prep, err := s.preparePromptAndWorktree(parentID, parent, body, cfg)
	if err != nil {
		var prepErr *spawnPreparationError
		if errors.As(err, &prepErr) {
			writeJSONError(w, prepErr.status, prepErr.code, prepErr.detail)
			return
		}
		writeJSONError(w, http.StatusInternalServerError, "board_error", errorDetail("child preparation failed", err))
		return
	}
	result, err := s.dispatchSpawn(parentID, parent, body, prep)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "spawn_error", errorDetail("spawn error", err))
		return
	}
	writeJSON(w, map[string]any{"ok": true, "session_id": result.ID, "board_path": result.BoardPath, "cwd": result.CWD, "worktree_branch": result.WorktreeBranch})
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

// isTerminalSessionState は再起動・追加指示の対象にしない終端状態。
func isTerminalSessionState(state string) bool {
	switch state {
	case "completed", "error", "disconnected", "done", "timeout", "dismissed":
		return true
	default:
		return false
	}
}

// liveChildForRole は parentID の子のうち role が一致し、終端状態でなく wrapper が
// 接続中のセッションのコピーを返す。複数いる場合は最新 spawn（ID 最大）を選ぶ。
// いなければ nil。
func (s *Server) liveChildForRole(parentID int, role string) *session {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	var found *session
	for _, ses := range s.sessions {
		if ses.ParentSessionID != parentID || ses.Role != role {
			continue
		}
		if isTerminalSessionState(ses.State) {
			continue
		}
		// wrapper 未接続の子へ inject しても pending に積むだけなので live としない。
		if s.wrappers[ses.ID] == nil {
			continue
		}
		if found == nil || ses.ID > found.ID {
			found = ses
		}
	}
	if found == nil {
		return nil
	}
	cp := *found
	return &cp
}

// handleSendChild は conductor から既存の子セッションへ追加指示を送る。
// board へ宛先付き conductor 記帳を残してから子 PTY へ注入する（spawn 枠を消費しない
// 指示経路。plan_orchestration-conductor-improvements.md C2）。
func (s *Server) handleSendChild(w http.ResponseWriter, r *http.Request, parentID int) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body sendChildRequest
	if !decodeJSON(w, r, &body) {
		return
	}
	body.Role = sanitizeRole(body.Role)
	if body.Role == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "role is required")
		return
	}
	if strings.TrimSpace(body.Text) == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "text is required")
		return
	}
	s.sessionsMu.Lock()
	parent := s.sessions[parentID]
	boardPath := ""
	if parent != nil {
		boardPath = parent.BoardPath
	}
	s.sessionsMu.Unlock()
	if parent == nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "parent session not found")
		return
	}
	child := s.liveChildForRole(parentID, body.Role)
	if child == nil {
		writeJSONError(w, http.StatusNotFound, "no_live_child", fmt.Sprintf("no live child for role %q; use `many-ai-cli orchestrate spawn --role %s \"<prompt>\"`", body.Role, body.Role))
		return
	}
	// body.Text は conductor 経由でユーザー由来のフリーテキストが入るため、
	// BEL/ESC 等の C0 制御文字を除去してから board 記録と PTY 注入の両方に使う。
	safeText := sanitizeInjectText(body.Text)
	// board への記録が先。子が更新通知を受けて board を読むとき、指示本文が既に載っている
	// ようにする（注入が先だと子が board を読んでも指示が見つからない窓ができる）。
	if boardPath != "" {
		if err := s.appendBoardSection(boardPath, "conductor", fmt.Sprintf("@%s session=%d への指示:\n%s\n", child.Role, child.ID, safeText)); err != nil {
			s.logger.Warn("send-child board append failed", "board", boardPath, "err", err)
		}
	}
	s.injectText(child.ID, fmt.Sprintf("\n[orchestration] instruction from conductor (session=%d):\n%s\n", parentID, safeText), true, false)
	writeJSON(w, map[string]any{"ok": true, "session_id": child.ID, "role": child.Role, "board_path": boardPath})
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
		FilePath:       childProgressPath(path, sessionID),
	}
	s.orchestration.mu.Unlock()
}

func (s *Server) setChildRestartData(boardID string, sessionID int, spec spawnWrappedSpec, initialPrompt, branch string, retries int) {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	if board := s.orchestration.boards[boardID]; board != nil {
		if child := board.Children[sessionID]; child != nil {
			child.RestartSpec, child.InitialPrompt, child.WorktreeBranch, child.TimeoutRetries = spec, initialPrompt, branch, retries
		}
	}
}

// childProgressPath は子専用進捗ファイルのパス（board と同じディレクトリ）。
func childProgressPath(boardPath string, sessionID int) string {
	return filepath.Join(filepath.Dir(boardPath), fmt.Sprintf("child-%d.md", sessionID))
}

func newOrchestrationBoard(id, path string) *orchestrationBoard {
	now := time.Now()
	return &orchestrationBoard{
		ID:             id,
		Path:           path,
		Sessions:       map[int]string{},
		Children:       map[int]*orchestrationChild{},
		Done:           map[int]bool{},
		IdleWarned:     map[int]bool{},
		TimedOut:       map[int]bool{},
		PendingNotices: map[int]string{},
		LastWrite:      now,
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
		s.scanOrchestrationChildFiles(b.ID, now)
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
	s.flushQueuedBoardNotices(now)
}

// scanOrchestrationChildFiles は子専用進捗ファイル（child-<ID>.md）を監視する。
// mtime / サイズの変化だけで子の活動（LastBoardWrite）を更新し、変化があったときのみ
// 内容を読んで DONE 検出と関係セッションへの更新通知を行う（C4: 子ごとファイル分離）。
func (s *Server) scanOrchestrationChildFiles(boardID string, now time.Time) {
	type childFile struct {
		id   int
		path string
		size int64
		mod  time.Time
	}
	s.orchestration.mu.Lock()
	b := s.orchestration.boards[boardID]
	if b == nil {
		s.orchestration.mu.Unlock()
		return
	}
	files := make([]childFile, 0, len(b.Children))
	for id, child := range b.Children {
		if child.FilePath == "" {
			continue
		}
		files = append(files, childFile{id: id, path: child.FilePath, size: child.FileSize, mod: child.FileMod})
	}
	s.orchestration.mu.Unlock()

	type update struct {
		id    int
		info  os.FileInfo
		text  string
		dones []boardDoneEvent
	}
	var updates []update
	for _, f := range files {
		info, err := os.Stat(f.path)
		if err != nil {
			continue // 進捗ファイル未作成（spawn 直後 or 旧プロンプトの子）
		}
		if info.Size() == f.size && info.ModTime().Equal(f.mod) {
			continue
		}
		data, err := os.ReadFile(f.path)
		if err != nil {
			continue
		}
		updates = append(updates, update{id: f.id, info: info, text: string(data), dones: detectBoardDoneEvents(string(data))})
	}
	if len(updates) == 0 {
		return
	}

	type notice struct {
		sessionID int
		text      string
	}
	var notices []notice
	var doneIDs []int
	s.orchestration.mu.Lock()
	b = s.orchestration.boards[boardID]
	if b == nil {
		s.orchestration.mu.Unlock()
		return
	}
	for _, u := range updates {
		child := b.Children[u.id]
		if child == nil {
			continue
		}
		child.FileSize = u.info.Size()
		child.FileMod = u.info.ModTime()
		child.LastBoardWrite = now
		for _, ev := range u.dones {
			// 自ファイルなので session id 表記が欠けていても本人の DONE とみなす
			if ev.SessionID == 0 || ev.SessionID == u.id {
				b.Done[u.id] = true
				child.Done = true
				doneIDs = append(doneIDs, u.id)
				break
			}
		}
		for sessionID := range b.Sessions {
			if sessionID == u.id {
				continue
			}
			notices = append(notices, notice{sessionID: sessionID, text: fmt.Sprintf("\n[orchestration] progress updated by %s (session=%d): %s\n", child.Role, u.id, child.FilePath)})
		}
	}
	s.orchestration.mu.Unlock()
	for _, n := range notices {
		s.notifyBoardSession(boardID, n.sessionID, n.text)
	}
	for _, id := range doneIDs {
		s.markChildState(id, "done")
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
	var authorizedDoneIDs []int
	var rejectedDones []boardDoneEvent
	for _, ev := range dones {
		if sessionID, ok := authorizeBoardDoneLocked(stored, ev, writerID); ok {
			stored.Done[sessionID] = true
			child := stored.Children[sessionID]
			child.Done = true
			child.LastBoardWrite = now
			authorizedDoneIDs = append(authorizedDoneIDs, sessionID)
		} else {
			rejectedDones = append(rejectedDones, ev)
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
		s.notifyBoardSession(boardID, sessionID, fmt.Sprintf("\n[orchestration] board updated by %s: %s\n", updatedBy, boardPath))
	}
	for _, ev := range rejectedDones {
		s.logger.Warn("orchestration DONE rejected",
			"board_id", boardID,
			"source_session_id", writerID,
			"claimed_session_id", ev.SessionID,
			"role", ev.Role)
	}
	for _, sessionID := range authorizedDoneIDs {
		s.markChildState(sessionID, "done")
	}
}

// authorizeBoardDoneLocked binds a shared-board DONE marker to the child that
// wrote the source section. A child must not be able to name a sibling's
// session ID in its own progress update and complete that sibling (IDOR).
// The caller holds orchestration.mu.
func authorizeBoardDoneLocked(board *orchestrationBoard, ev boardDoneEvent, writerID int) (int, bool) {
	sessionID := ev.SessionID
	if sessionID == 0 {
		sessionID = uniqueChildSessionForRole(board, ev.Role)
	}
	if sessionID == 0 || writerID == 0 || sessionID != writerID {
		return 0, false
	}
	if _, ok := board.Children[sessionID]; !ok {
		return 0, false
	}
	return sessionID, true
}

// notifyBoardSession applies board_notify_mode to conductor notifications only.
// Child and ordinary session behavior remains unchanged; this is deliberately
// scoped so P-19 does not alter normal terminal input delivery.
func (s *Server) notifyBoardSession(boardID string, sessionID int, text string) {
	mode := config.EffectiveBoardNotifyMode(s.snapshotCfg().Orchestration.BoardNotifyMode)
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	isConductor := ses != nil && ses.OrchestrationID != "" && ses.ParentSessionID == 0
	s.sessionsMu.Unlock()
	if !isConductor {
		s.injectText(sessionID, text, true, false)
		return
	}

	switch mode {
	case config.BoardNotifySoft:
		s.setBoardNotifyPending(sessionID, true)
	case config.BoardNotifyQueueUntilIdle:
		s.orchestration.mu.Lock()
		if board := s.orchestration.boards[boardID]; board != nil {
			if board.PendingNotices == nil {
				board.PendingNotices = map[int]string{}
			}
			board.PendingNotices[sessionID] = text
		}
		s.orchestration.mu.Unlock()
		s.setBoardNotifyPending(sessionID, true)
	case config.BoardNotifyInterrupt:
		s.setBoardNotifyPending(sessionID, false)
		s.injectText(sessionID, text, true, false)
	}
}

// flushQueuedBoardNotices sends one latest board notification after the
// conductor has been output-idle. Workflow-idle will be added by P-47; until
// then this intentionally conservative output-only gate is the sole criterion.
func (s *Server) flushQueuedBoardNotices(now time.Time) {
	type queuedNotice struct {
		boardID   string
		sessionID int
		text      string
	}
	var pending []queuedNotice
	s.orchestration.mu.Lock()
	for boardID, board := range s.orchestration.boards {
		for sessionID, text := range board.PendingNotices {
			pending = append(pending, queuedNotice{boardID: boardID, sessionID: sessionID, text: text})
		}
	}
	s.orchestration.mu.Unlock()

	for _, notice := range pending {
		if !s.conductorOutputIdle(notice.sessionID, now) {
			continue
		}
		s.orchestration.mu.Lock()
		board := s.orchestration.boards[notice.boardID]
		text, stillPending := "", false
		if board != nil {
			text, stillPending = board.PendingNotices[notice.sessionID]
			if stillPending {
				delete(board.PendingNotices, notice.sessionID)
			}
		}
		s.orchestration.mu.Unlock()
		if !stillPending {
			continue
		}
		s.setBoardNotifyPending(notice.sessionID, false)
		s.injectText(notice.sessionID, text, true, false)
	}
}

func (s *Server) conductorOutputIdle(sessionID int, now time.Time) bool {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[sessionID]
	if ses == nil || ses.ParentSessionID != 0 || ses.OrchestrationID == "" {
		return false
	}
	if ses.initialInjectPending || ses.Activity.AwaitingUser {
		return false
	}
	return ses.Activity.IsIdle()
}

func (s *Server) setBoardNotifyPending(sessionID int, pending bool) {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	if ses == nil || ses.BoardNotifyPending == pending {
		s.sessionsMu.Unlock()
		return
	}
	ses.BoardNotifyPending = pending
	msg := sessionUpdateMessage(ses)
	s.sessionsMu.Unlock()
	s.broadcast(msg)
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
	// PTY 出力時刻のスナップショット。board 記帳が止まっていても PTY 出力が動いている子は
	// 作業中（plan 読込・実装・レビュー等）とみなし idle warning を出さない。board 記帳時刻
	// だけの判定は実測で偽陽性を連発した（plan_orchestration-conductor-improvements.md C1）。
	// sessionsMu → orchestration.mu の順に短く取り、入れ子にしない。
	lastOutputs := map[int]time.Time{}
	s.sessionsMu.Lock()
	for id, ses := range s.sessions {
		lastOutputs[id] = ses.lastOutputAt
	}
	s.sessionsMu.Unlock()
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
			if cfg.IdleDoneThresholdSec > 0 {
				threshold := time.Duration(cfg.IdleDoneThresholdSec) * time.Second
				boardIdle := now.Sub(child.LastBoardWrite) > threshold
				ptyIdle := true
				if last, ok := lastOutputs[id]; ok && !last.IsZero() {
					ptyIdle = now.Sub(last) > threshold
				}
				if boardIdle && ptyIdle {
					if !b.IdleWarned[id] {
						b.IdleWarned[id] = true
						notices = append(notices, notice{parentID: child.ParentID, childID: id, role: child.Role, kind: "idle", threshold: cfg.IdleDoneThresholdSec})
					}
				} else if b.IdleWarned[id] {
					// board か PTY の活動が再開したらラッチを解除し、再び沈黙したら改めて 1 回警告する
					delete(b.IdleWarned, id)
				}
			}
		}
	}
	s.orchestration.mu.Unlock()
	for _, n := range notices {
		switch n.kind {
		case "timeout":
			s.notifyOrchestrationError(n.parentID, "timeout", fmt.Sprintf("role=%s id=%d threshold=%ds", n.role, n.childID, n.threshold))
			s.markChildState(n.childID, n.state)
			if cfg.TimeoutRespawn {
				s.respawnTimedOutChild(boardID, n.childID, cfg.MaxTimeoutRespawns)
			}
		case "idle":
			s.injectText(n.parentID, fmt.Sprintf("\n[orchestration] idle warning role=%s id=%d no board update and no PTY output for %ds\n", n.role, n.childID, n.threshold), true, false)
		}
	}
}

// completeOrchestrationChildOnSessionEnd makes EOF a first-class completion
// route. The process result remains completed/error in the session list, while
// the orchestration board records that the conductor no longer needs to wait
// for a DONE marker that was never printed.
func (s *Server) completeOrchestrationChildOnSessionEnd(sessionID int, state string) {
	var boardID, boardPath, role string
	var parentID int
	s.orchestration.mu.Lock()
	for id, board := range s.orchestration.boards {
		child := board.Children[sessionID]
		if child == nil {
			continue
		}
		board.Done[sessionID] = true
		child.Done = true
		boardID, boardPath, role, parentID = id, board.Path, child.Role, child.ParentID
		break
	}
	s.orchestration.mu.Unlock()
	if boardID == "" {
		return
	}
	_ = s.appendBoardSection(boardPath, "hub", fmt.Sprintf("child completed without DONE marker: role=%s session=%d state=%s (session_end)\n", role, sessionID, state))
	s.notifyBoardSession(boardID, parentID, fmt.Sprintf("\n[orchestration] child complete via session_end role=%s id=%d state=%s\n", role, sessionID, state))
}

// respawnTimedOutChild retries an opted-in child once (or the configured small
// limit) using the original isolated working directory and prompt. It is kept
// asynchronous so the board poller cannot stall while a CLI registers.
func (s *Server) respawnTimedOutChild(boardID string, sessionID, maxRetries int) {
	if maxRetries <= 0 {
		return
	}
	s.orchestration.mu.Lock()
	board := s.orchestration.boards[boardID]
	if board == nil || board.Children[sessionID] == nil {
		s.orchestration.mu.Unlock()
		return
	}
	child := *board.Children[sessionID]
	if child.TimeoutRetries >= maxRetries || child.RestartSpec.Provider == "" {
		s.orchestration.mu.Unlock()
		return
	}
	// Latch before starting the goroutine so a subsequent poll cannot enqueue a
	// duplicate retry for the same timed-out child.
	board.Children[sessionID].TimeoutRetries = maxRetries
	s.orchestration.mu.Unlock()

	s.safeGo("orchestration_timeout_respawn", func() {
		label := fmt.Sprintf("orch-%s-%s-retry-%d", safeToken(boardID), child.Role, time.Now().UnixNano())
		spec := child.RestartSpec
		spec.Label = label
		meta := pendingChild{ParentSessionID: child.ParentID, Role: child.Role, Auto: true, Depth: 1, OrchestrationID: boardID, BoardPath: board.Path, WorktreeBranch: child.WorktreeBranch, SpawnedAt: time.Now()}
		s.orchestration.mu.Lock()
		s.orchestration.pending[label] = meta
		s.orchestration.mu.Unlock()
		newID, err := s.spawnWrappedSession(spec, 20*time.Second)
		if err != nil {
			s.orchestration.mu.Lock()
			delete(s.orchestration.pending, label)
			s.orchestration.mu.Unlock()
			s.notifyOrchestrationError(child.ParentID, "timeout_respawn", fmt.Sprintf("role=%s id=%d: %v", child.Role, sessionID, err))
			return
		}
		s.registerBoardChild(boardID, board.Path, newID, child.ParentID, child.Role, time.Now())
		s.setChildRestartData(boardID, newID, child.RestartSpec, child.InitialPrompt, child.WorktreeBranch, child.TimeoutRetries+1)
		_ = s.appendBoardSection(board.Path, "hub", fmt.Sprintf("timeout retry spawned: role=%s old_session=%d session=%d\n", child.Role, sessionID, newID))
		s.injectInitialPrompt(newID, buildChildInitialPrompt(child.InitialPrompt, board.Path, child.Role, child.WorktreeBranch, newID))
	})
}

func detectBoardDoneEvents(text string) []boardDoneEvent {
	var events []boardDoneEvent
	seen := map[boardDoneEvent]bool{}
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if !strings.HasPrefix(line, "## DONE ") && !strings.HasPrefix(line, "## SUCCESS ") {
			continue
		}
		payload := strings.TrimPrefix(line, "## DONE ")
		if strings.HasPrefix(line, "## SUCCESS ") {
			payload = strings.TrimPrefix(line, "## SUCCESS ")
		}
		fields := strings.Fields(strings.TrimSpace(payload))
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
		Type:                "session_update",
		SessionID:           ses.ID,
		Provider:            ses.Provider,
		Display:             ses.Display,
		CWD:                 ses.CWD,
		Branch:              ses.Branch,
		Label:               ses.Label,
		Model:               ses.Model,
		Route:               ses.Route,
		State:               ses.State,
		OutputIdle:          ses.Activity.OutputIdle,
		WorkflowActive:      ses.Activity.WorkflowActive,
		AwaitingUser:        ses.Activity.AwaitingUser,
		AwaitingApproval:    ses.Activity.AwaitingApproval,
		Activity:            activityMessage(ses.Activity),
		ApprovalSourceEpoch: ensureApprovalSourceEpochLocked(ses),
		LastOutputAt:        ses.LastOutputAt,
		StartedAt:           ses.StartedAt,
		ParentSessionID:     ses.ParentSessionID,
		Role:                ses.Role,
		Auto:                ses.Auto,
		Depth:               ses.Depth,
		OrchestrationID:     ses.OrchestrationID,
		BoardPath:           ses.BoardPath,
		WorktreeBranch:      ses.WorktreeBranch,
		BoardNotifyPending:  ses.BoardNotifyPending,
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
	if !pressEnter {
		s.injectRaw(sessionID, text)
		return
	}
	// 本文と確定 \r を同一チャンクで送ると、内側 CLI がペースト取り込み中の \r を確定キーと
	// 扱わず入力欄に張り付いたまま送信されない（Grok 実測 2026-07-11・チャット送信と同根）。
	// AI CLI（全 wrap 対象が ?2004h 宣言済み）はブラケットペーストで包み、確定 \r は
	// trySendInput の splitBracketedPasteSubmit が別書き込み + 遅延で送る。
	// shell は 2004 未宣言（素の PowerShell 等でマーカーがリテラル混入）がありうるため従来どおり。
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	isShell := ses != nil && ses.Provider == "shell"
	s.sessionsMu.Unlock()
	if isShell {
		if !strings.HasSuffix(text, "\r") {
			text += "\r"
		}
		s.injectRaw(sessionID, text)
		return
	}
	// ペースト本体に生の \r が残ると一部 CLI が確定キーと誤解するため末尾の改行類は落とす
	// （確定は末尾に付ける \r だけが担う）。
	text = strings.TrimRight(text, "\r\n")
	s.injectRaw(sessionID, bracketedPasteStart+text+bracketedPasteEnd+"\r")
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
	// チャット送信・injectText と同じくブラケットペーストで包み、確定 \r は
	// splitBracketedPasteSubmit（trySendInput）が別書き込み + 遅延で送る。
	// 「本文+\r」同一チャンクだと本文がエコーされても \r が確定キーとして
	// 処理されないことがある（Grok 実測 2026-07-11）。エコー検証（marker）は
	// 可視テキストで行うためマーカー包みの影響を受けない。
	text := bracketedPasteStart + strings.TrimRight(prompt, "\r\n") + bracketedPasteEnd + "\r"
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
	id := strconv.Itoa(sessionID)
	var b strings.Builder
	b.WriteString("You are an orchestration child session.\n")
	b.WriteString("Role: " + role + "\n")
	b.WriteString("Session ID: " + id + "\n")
	b.WriteString("Shared board (read-only for you): " + boardPath + "\n")
	b.WriteString("Your progress file (write here): " + childProgressPath(boardPath, sessionID) + "\n")
	if branch != "" {
		b.WriteString("Worktree branch: " + branch + "\n")
	}
	// 進捗・DONE は子専用ファイルへ。board.md は conductor の指示・全体状況の読み取り専用に
	// することで、共有 board への同時書き込み競合と記帳名義ゆれを避ける（C4）。
	b.WriteString("Read the board before acting; the conductor posts instructions there. Write your progress ONLY to your progress file (create it on first write), as `## " + role + " session=" + id + " <RFC3339 time>` sections, each including a `status: running|blocked|done|failed` line. When complete, append `## DONE " + role + " session=" + id + "` (or the explicit success form `## SUCCESS " + role + " session=" + id + "`) and a concise summary to your progress file. Do not write to the shared board.\n\n")
	// base はユーザー・conductor 由来のフリーテキスト。BEL/ESC 等の C0 制御文字が
	// 混入していると PTY 経由で子セッションの端末エコー・Hub UI レンダリングに
	// エスケープシーケンス（タイトル詐称・画面クリア等）を注入できてしまうため
	// git_common.go の sanitizeCommitMessage と同型のフィルタで除去する。
	b.WriteString(sanitizeInjectText(base))
	return b.String()
}

// sanitizeInjectText は orchestration 経由で PTY へ inject する任意テキストから
// C0 制御文字と DEL を除去する（\t / \n は保持し、\r は \n に正規化する）。
// git_common.go の sanitizeCommitMessage と同じ規準・目的だが、こちらは長さ
// 上限を持たず・末尾 TrimSpace もしない（プロンプトの改行構造を保つため）。
func sanitizeInjectText(s string) string {
	s = strings.ReplaceAll(s, "\r\n", "\n")
	s = strings.ReplaceAll(s, "\r", "\n")
	return strings.Map(func(r rune) rune {
		if (r < 0x20 && r != '\t' && r != '\n') || r == 0x7f {
			return -1
		}
		return r
	}, s)
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
	// 2026-07-04 の実運用（plan_orchestration-conductor-improvements.md C3）で確立した
	// conductor 運用ルール。spawn 反復による枠涸渇・停止指示の解釈違い・レビューと修正の
	// レースを構造的に防ぐ。
	b.WriteString("Operating rules:\n")
	b.WriteString("- To give follow-up instructions to an existing live child, run `many-ai-cli orchestrate send --role <role> \"<text>\"` instead of spawning again. spawn is rejected (409) while a live child exists for the role; send injects the text into the child and records it on the board automatically.\n")
	b.WriteString("- When the user asks you to stop, confirm in one line whether they mean immediately or after the current work unit completes (default: after completion).\n")
	b.WriteString("- Do not dispatch a reviewer while the implementation child is still working on fixes; wait for its `## DONE` entry on the board first.\n")
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
	// #nosec G702 -- exec.CommandContext は argv 直接渡しで shell を介さないため
	// 「コマンド注入」経路が無い。branch と childDir は safeToken() 済み、cwd は自ホストの
	// session cwd（ユーザー本人が指定した自マシンのパス）。
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
