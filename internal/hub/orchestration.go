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
	"sort"
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
//
// この静止は「入力受付の準備完了」の判定には使えない。実測（2026-08-31 session #14 /
// claude v2.1.251 / Windows）では splash 描画から入力欄の描画まで 12 秒あり、その途中に
// 8 秒間まったく出力が無い区間があった。quiet をいくら伸ばしても、その無音を準備完了と
// 誤読する。到達の担保は下の注入後エコー観測が持つ。
const (
	orchestrationInjectQuiet   = 300 * time.Millisecond
	orchestrationInjectMaxWait = 5 * time.Second
)

// injectInitialPrompt のエコー検証パラメータ。静止時間だけの判定は codex 等の
// 起動シーケンス途中の静止窓で誤発火し、readline 未起動の TUI に注入バイトが
// 捨てられる（bugfix_orchestration-codex-child-spawn-failures_2026-07-04.md 根本原因 A）。
// 注入後に PTY 画面へ注入テキストのエコーが現れることを実観測し、現れなければ再注入する。
//
// EchoWait は実測に合わせる。session #14 では注入から入力欄への反映まで 13.4 秒かかって
// おり、3 秒では必ず空振りして再注入していた（同じ本文が 4 通届いた）。再注入は本当に
// 落とされたときの最後の保険なので、回数は 2 に抑える。
const (
	orchestrationInjectEchoWait    = 20 * time.Second
	orchestrationInjectMaxAttempts = 2
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
	roles map[string]map[string]orchestrationRoleAssignment
	// spawnConfirmations holds every spawn confirmation the Hub has not yet
	// seen a browser decide, keyed by its ID. Unlike the pre-C1 design, this
	// is a Hub-side hold with no deadline: a pending confirmation lives here
	// until a browser decides it or its parent goes away, independent of the
	// HTTP request that registered it
	// (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C1).
	spawnConfirmations map[string]*pendingSpawnConfirmation
	// relays holds every relay loop (relay.go) keyed by its own orchestration
	// ID. The map is guarded by mu; each relayRun has its own mutex for its
	// transitions so mu is never held across spawn / inject / git calls.
	relays map[string]*relayRun
	// relayMeta mirrors, under mu, the few run fields that listing and the
	// child budget need (parent, attached, active, spawned). Readers never
	// take a run's mutex for them, which keeps run.mu → mu the only order.
	relayMeta map[string]relayMeta
	relaySeq  uint64
	// spawnConfirmSeq disambiguates spawn confirmation IDs generated in the
	// same instant. time.Now().UnixNano() alone collides on platforms with
	// coarse clock resolution (observed on Windows: two calls a few
	// microseconds apart return the identical value), which would let two
	// different confirmations share one ID and answer each other's POST.
	spawnConfirmSeq uint64
	stopOnce        sync.Once
	stopCh          chan struct{}
}

// orchestrationRoleAssignment は起動フォームの「子役割の詳細設定」アコーディオンで
// 役割ごとに指定された CLI + モデルの組。
type orchestrationRoleAssignment struct {
	Provider string `json:"provider"`
	Model    string `json:"model"`
	// Subscription は、この role に割り当てる子セッションのサブスクリプション
	// profile ID（"auto" も可）。空文字は「role 別の指定なし」を意味し、
	// resolveChildSubscription が親継承・グローバル既定へフォールバックする。
	Subscription string `json:"subscription,omitempty"`
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
	// StartupFailed ラッチは D9 (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md)。
	// 起動ハンドシェイク未達として一度通知・停止した子には、同じ子で
	// checkOrchestrationChildTimers が timeout 判定を重ねて 2 通目を出すことを防ぐ。
	StartupFailed map[int]bool
	LastSize      int64
	LastMod       time.Time
	LastWrite     time.Time
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
	// 起動ハンドシェイク未達の早期検知用フィールド
	// (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md, D2)。
	//
	// PromptDeliveredAt: 初期プロンプトの配送が確認できた時刻（エコー観測 or 遅延エコー
	// 観測）。ゼロ値は「未確認」。
	PromptDeliveredAt time.Time
	// PromptFailed: 配送失敗が既に reportInjectFailure 経由で親へ報告済み。
	PromptFailed bool
	// StandbySince: standby が続き始めた時刻。running へ戻ったらゼロへ戻す
	// (markRunning)。ゼロ値は「現在 standby ではない、または一度も standby になっていない」。
	StandbySince time.Time
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
	// SubscriptionProfileID は子セッションを起動するサブスクリプション profile。
	// 省略時は CLI 自身のログイン環境（従来どおり）。存在しない ID を指定した場合、
	// 子は起動せずエラーになる（別アカウントへ黙って倒れない）。
	SubscriptionProfileID string `json:"subscription_profile_id"`
	// SameTree は tri-state（`/api/spawn` の IsolateWorktree と同じ形）。
	// nil は「orchestration.worktree_auto の設定に従う」（既定）。明示 true は
	// 「worktree を使わず、CWD で直接動かす」（`orchestrate spawn -same-tree` /
	// `orchestrate relay -same-tree` と同じ語）。
	// C3 (plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md)。
	SameTree *bool `json:"same_tree"`
}

// pendingSpawnConfirmation is a spawn confirmation the Hub is holding while
// no browser has decided it yet. It is deliberately decoupled from the HTTP
// request that created it: the calling AI's own tool call can be killed by
// its shell timeout long before a human looks at the browser, and the
// confirmation must still be answerable after that happens
// (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C1).
type pendingSpawnConfirmation struct {
	ID       string
	ParentID int
	Role     string
	// RequestedProvider is the raw provider value the caller originally
	// passed (before role-mapping / memory / parent-provider resolution).
	// performSpawn only updates the remembered role→provider mapping when
	// this is empty, matching the pre-C1 behaviour.
	RequestedProvider string
	// Body is the request as resolved at registration time, before any
	// override the browser's decision may apply.
	Body        spawnChildRequest
	RequestedAt time.Time
	Decided     bool
	// WaiterGone is set by waitSpawnConfirmation when the HTTP request that
	// registered this confirmation gives up (its context is done) while the
	// confirmation is still undecided. The decision side reads it to know
	// whether its HTTP response will ever reach anyone, and therefore
	// whether it must also reach the parent through the board instead.
	WaiterGone bool
	// Outcome carries the decision exactly once, written by the goroutine
	// that processes it (resolveSpawnConfirmationDecision /
	// expireSpawnConfirmationsForParent / registerSpawnConfirmation on
	// supersede). It is buffered so that write never blocks on a waiter that
	// stopped listening.
	Outcome chan spawnConfirmationOutcome
}

// spawnConfirmationOutcome is written to pendingSpawnConfirmation.Outcome
// exactly once. Approved is false only for a genuine user refusal (or a
// confirmation that never got a human decision at all — superseded / the
// parent went away). Err is non-nil only when the user approved but
// performSpawn itself failed: the confirmation was decided, the child was
// not created.
type spawnConfirmationOutcome struct {
	Approved bool
	Result   childSpawnResult
	Err      *spawnHTTPError
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
		spawnConfirmations: map[string]*pendingSpawnConfirmation{},
		relays:             map[string]*relayRun{},
		relayMeta:          map[string]relayMeta{},
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
	// An ordinary /api/spawn with both isolate_worktree and orchestration=true
	// has already reserved this label with worktree cleanup metadata. Merge the
	// conductor fields into that entry instead of replacing the cleanup handle.
	meta := s.orchestration.pending[label]
	meta.OrchestrationID = orchestrationID
	meta.SpawnedAt = time.Now()
	s.orchestration.pending[label] = meta
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

// registerSpawnConfirmation publishes a spawn confirmation request to every
// connected browser and holds it in the Hub with no deadline
// (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C1). If
// the same parent already has an undecided confirmation for the same role,
// that older one is superseded (closed without ever being a user_refusal)
// rather than left to pile up — a conductor re-issuing the same spawn while
// the first request is still awaiting a human should not accumulate stale
// confirmations.
func (s *Server) registerSpawnConfirmation(parent *session, requestedProvider string, body spawnChildRequest) (*pendingSpawnConfirmation, *spawnHTTPError) {
	pending := &pendingSpawnConfirmation{
		ParentID:          parent.ID,
		Role:              body.Role,
		RequestedProvider: requestedProvider,
		Body:              body,
		RequestedAt:       time.Now(),
		Outcome:           make(chan spawnConfirmationOutcome, 1),
	}
	cfg := s.snapshotCfg().Orchestration
	var superseded *pendingSpawnConfirmation
	s.orchestration.mu.Lock()
	// The sequence number is required, not cosmetic: time.Now().UnixNano()
	// alone has produced identical values for two calls a few microseconds
	// apart (observed on Windows), which would let two unrelated
	// confirmations collide on the same ID.
	s.orchestration.spawnConfirmSeq++
	pending.ID = fmt.Sprintf("sc-%d-%d", pending.RequestedAt.UnixNano(), s.orchestration.spawnConfirmSeq)
	for id, existing := range s.orchestration.spawnConfirmations {
		if existing.ParentID == parent.ID && existing.Role == body.Role && !existing.Decided {
			superseded = existing
			superseded.Decided = true
			delete(s.orchestration.spawnConfirmations, id)
			break
		}
	}
	if superseded == nil {
		pendingForParent := 0
		pendingTotal := 0
		for _, existing := range s.orchestration.spawnConfirmations {
			if existing.Decided {
				continue
			}
			pendingTotal++
			if existing.ParentID == parent.ID {
				pendingForParent++
			}
		}
		if pendingForParent >= cfg.MaxChildrenPerParent {
			s.orchestration.mu.Unlock()
			return nil, &spawnHTTPError{
				status: http.StatusTooManyRequests,
				code:   "orchestration_limit",
				detail: "pending spawn confirmations per parent",
			}
		}
		if pendingTotal >= cfg.MaxTotalSessions {
			s.orchestration.mu.Unlock()
			return nil, &spawnHTTPError{
				status: http.StatusTooManyRequests,
				code:   "orchestration_limit",
				detail: "pending spawn confirmations total",
			}
		}
	}
	s.orchestration.spawnConfirmations[pending.ID] = pending
	s.orchestration.mu.Unlock()

	if superseded != nil {
		select {
		case superseded.Outcome <- spawnConfirmationOutcome{Approved: false}:
		default:
		}
		s.broadcast(proto.Message{Type: "spawn_confirmation_closed", SpawnConfirmationID: superseded.ID, SessionID: superseded.ParentID, Reason: "superseded"})
	}

	s.broadcast(proto.Message{
		Type: "spawn_confirmation_requested", SpawnConfirmationID: pending.ID,
		SessionID: parent.ID, Role: body.Role, Provider: body.Provider, Model: body.Model,
		CWD: body.CWD, InitialPrompt: body.InitialPrompt, SpawnRequestedAtMs: pending.RequestedAt.UnixMilli(),
	})
	return pending, nil
}

// waitSpawnConfirmation blocks until either a decision has been written to
// p.Outcome, or ctx is done because the caller's own HTTP request died (its
// tool call timed out, or the process holding it was killed). In the latter
// case the pending confirmation itself is left untouched other than noting
// that its waiter is gone; it stays answerable by handleSpawnConfirmation.
func (s *Server) waitSpawnConfirmation(ctx context.Context, p *pendingSpawnConfirmation) (spawnConfirmationOutcome, bool) {
	select {
	case outcome := <-p.Outcome:
		return outcome, true
	case <-ctx.Done():
		s.orchestration.mu.Lock()
		if !p.Decided {
			p.WaiterGone = true
		}
		s.orchestration.mu.Unlock()
		return spawnConfirmationOutcome{}, false
	}
}

// pendingSpawnConfirmationMessages returns spawn_confirmation_requested
// messages for every confirmation the Hub is still holding, oldest request
// first, so a newly (re)connected browser can rebuild the dialogs it would
// otherwise never see again (C2, plan_..._c4_spawn-confirm-ui.md).
func (s *Server) pendingSpawnConfirmationMessages() []proto.Message {
	s.orchestration.mu.Lock()
	pending := make([]*pendingSpawnConfirmation, 0, len(s.orchestration.spawnConfirmations))
	for _, p := range s.orchestration.spawnConfirmations {
		if p.Decided {
			continue
		}
		pending = append(pending, p)
	}
	s.orchestration.mu.Unlock()
	sort.Slice(pending, func(i, j int) bool { return pending[i].RequestedAt.Before(pending[j].RequestedAt) })
	msgs := make([]proto.Message, 0, len(pending))
	for _, p := range pending {
		msgs = append(msgs, proto.Message{
			Type: "spawn_confirmation_requested", SpawnConfirmationID: p.ID,
			SessionID: p.ParentID, Role: p.Body.Role, Provider: p.Body.Provider, Model: p.Body.Model,
			CWD: p.Body.CWD, InitialPrompt: p.Body.InitialPrompt, SpawnRequestedAtMs: p.RequestedAt.UnixMilli(),
		})
	}
	return msgs
}

// hasPendingSpawnConfirmation reports whether parentID is still holding at
// least one undecided spawn confirmation. handleDismiss calls this before
// tearing a session down: dismissing a conductor that is holding a
// confirmation open would silently drop a decision the browser has not made
// yet, so that session must not be removed while this returns true
// (C5, plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md).
func (s *Server) hasPendingSpawnConfirmation(parentID int) bool {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	for _, p := range s.orchestration.spawnConfirmations {
		if p.ParentID == parentID && !p.Decided {
			return true
		}
	}
	return false
}

// expireSpawnConfirmationsForParent fails every undecided spawn confirmation
// belonging to parentID without ever recording a user_refusal — the parent
// disappearing is not a person clicking refuse
// (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C1).
// trigger names the caller for logging only (e.g. "dismiss").
//
// By the time handleDismiss reaches its call to this, hasPendingSpawnConfirmation
// has already refused the dismiss for any parent that had a confirmation
// pending at the start of the request, so in the common case this call finds
// nothing to expire. It stays in place as the safety net for the narrow race
// where a confirmation is registered after that check but before the session
// is actually removed (C5): a session must never be deleted while leaving a
// confirmation permanently unanswerable.
func (s *Server) expireSpawnConfirmationsForParent(parentID int, trigger string) {
	s.orchestration.mu.Lock()
	var expired []*pendingSpawnConfirmation
	for id, p := range s.orchestration.spawnConfirmations {
		if p.ParentID != parentID || p.Decided {
			continue
		}
		p.Decided = true
		expired = append(expired, p)
		delete(s.orchestration.spawnConfirmations, id)
	}
	s.orchestration.mu.Unlock()
	for _, p := range expired {
		select {
		case p.Outcome <- spawnConfirmationOutcome{Approved: false}:
		default:
		}
		s.logger.Info("spawn confirmation expired: parent gone", "confirmation_id", p.ID, "parent_session_id", parentID, "role", p.Role, "trigger", trigger)
		s.broadcast(proto.Message{Type: "spawn_confirmation_closed", SpawnConfirmationID: p.ID, SessionID: parentID, Reason: "parent_gone"})
	}
}

// handleSpawnConfirmation answers the POST from a browser deciding a pending
// spawn confirmation. It responds immediately (the browser does not wait for
// the child to finish launching) and hands the decision to a background
// goroutine, because launching the child can take longer than a human should
// have to wait for this request to return
// (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C1).
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
	pending := s.orchestration.spawnConfirmations[body.ConfirmationID]
	if pending == nil {
		s.orchestration.mu.Unlock()
		writeJSONError(w, http.StatusNotFound, "not_found", "spawn confirmation is no longer pending")
		return
	}
	if pending.Decided {
		s.orchestration.mu.Unlock()
		writeJSONError(w, http.StatusConflict, "already_decided", "spawn confirmation has already been decided")
		return
	}
	pending.Decided = true
	delete(s.orchestration.spawnConfirmations, body.ConfirmationID)
	s.orchestration.mu.Unlock()

	writeJSON(w, map[string]bool{"ok": true})
	decidedProvider := strings.TrimSpace(body.Provider)
	decidedModel := strings.TrimSpace(body.Model)
	approved := body.Approved
	s.safeGo("spawn_confirmation_decided", func() {
		s.resolveSpawnConfirmationDecision(pending, approved, decidedProvider, decidedModel)
	})
}

// resolveSpawnConfirmationDecision runs the effect of one decided spawn
// confirmation: for a refusal it records the single, unambiguous
// user_refusal board entry (C1's only writer of that reason); for an
// approval it is the one place that actually launches the child
// (performSpawn), because launching from both the waiting HTTP handler and
// here would risk a double spawn. It always writes exactly one outcome to
// pending.Outcome — the original HTTP handler may or may not still be
// listening — and always broadcasts spawn_confirmation_closed so every
// connected browser closes its dialog.
func (s *Server) resolveSpawnConfirmationDecision(pending *pendingSpawnConfirmation, approved bool, decidedProvider, decidedModel string) {
	if !approved {
		if parent, _, _ := s.orchestrationParentState(pending.ParentID); parent != nil {
			if boardPath, err := s.ensureOrchestrationBoardForRefusal(parent, pending.Body); err == nil {
				_ = s.appendBoardSection(boardPath, "hub", fmt.Sprintf("refused: role=%s reason=user_refusal\n", pending.Role))
			}
		}
		select {
		case pending.Outcome <- spawnConfirmationOutcome{Approved: false}:
		default:
		}
		s.broadcast(proto.Message{Type: "spawn_confirmation_closed", SpawnConfirmationID: pending.ID, SessionID: pending.ParentID, Reason: "refused"})
		return
	}

	body := pending.Body
	if decidedProvider != "" {
		body.Provider = decidedProvider
	}
	if decidedModel != "" {
		body.Model = decidedModel
	}
	var providerChangeNote string
	if note := providerConfirmationChangeNote(pending.Body.Provider, body.Provider); note != "" {
		s.logger.Warn("orchestration spawn-child confirmation changed provider",
			"parent_session_id", pending.ParentID, "role", pending.Role,
			"requested_provider", pending.Body.Provider, "decided_provider", body.Provider)
		providerChangeNote = note
	}
	result, spawnErr := s.performSpawn(pending.ParentID, pending.RequestedProvider, providerChangeNote, body)

	s.orchestration.mu.Lock()
	waiterGone := pending.WaiterGone
	s.orchestration.mu.Unlock()

	if spawnErr != nil {
		select {
		case pending.Outcome <- spawnConfirmationOutcome{Approved: true, Err: spawnErr}:
		default:
		}
		s.broadcast(proto.Message{Type: "spawn_confirmation_closed", SpawnConfirmationID: pending.ID, SessionID: pending.ParentID, Reason: "spawn_failed", Text: spawnErr.detail})
		if waiterGone {
			s.notifyBoardGoneWaiter(pending, fmt.Sprintf("[MANY-AI-CLI] child spawn failed after your earlier request lost its connection: role=%s detail=%s", pending.Role, spawnErr.detail))
		}
		return
	}

	select {
	case pending.Outcome <- spawnConfirmationOutcome{Approved: true, Result: result}:
	default:
	}
	s.broadcast(proto.Message{Type: "spawn_confirmation_closed", SpawnConfirmationID: pending.ID, SessionID: pending.ParentID, Reason: "approved", SpawnChildSessionID: result.ID})
	if waiterGone {
		_ = s.appendBoardSection(result.BoardPath, "hub", fmt.Sprintf("spawned (confirmed after the requesting session disconnected): role=%s session=%d\n", pending.Role, result.ID))
		s.notifyBoardGoneWaiter(pending, fmt.Sprintf("[MANY-AI-CLI] child spawned after your earlier request lost its connection: role=%s session=#%d", pending.Role, result.ID))
	}
}

// notifyBoardGoneWaiter tells the parent conductor about a spawn
// confirmation outcome that no live HTTP response can deliver anymore. Only
// resolveSpawnConfirmationDecision's approved path calls this — a refusal is
// already recorded on the board unconditionally and does not need to
// interrupt the conductor.
func (s *Server) notifyBoardGoneWaiter(pending *pendingSpawnConfirmation, text string) {
	parent, _, _ := s.orchestrationParentState(pending.ParentID)
	if parent == nil || parent.OrchestrationID == "" {
		return
	}
	s.notifyBoardSession(parent.OrchestrationID, pending.ParentID, text)
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
	case "relay":
		if r.Method == http.MethodGet {
			s.handleRelayGet(w, r, id)
		} else {
			s.handleRelayStart(w, r, id)
		}
	case "relay-stop":
		s.handleRelayStop(w, r, id)
	case "relay-resume":
		s.handleRelayResume(w, r, id)
	case "relay-cleanup":
		s.handleRelayCleanup(w, r, id)
	default:
		writeJSONError(w, http.StatusNotFound, "not_found", "not found")
	}
}

type childSpawnPreparation struct {
	orchestrationID string
	boardPath       string
	childCWD        string
	branch          string
	// absoluteCWD is the requested CWD after absolutization but before any
	// worktree substitution. dispatchSpawn logs it next to childCWD so a
	// -cwd request that got redirected into a worktree is traceable from the
	// log alone (C1, plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md).
	// relaySpawn does not set it (relay resolves its own worktree elsewhere),
	// so it is blank for relay-originated spawns.
	absoluteCWD string
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
	} else if !filepath.IsAbs(cwd) {
		// A relative --cwd must resolve against the conductor (parent) session's
		// directory, not the Hub process cwd. Otherwise os.Stat and
		// spawnCwdTooBroad evaluate the wrong path: a relative value slips past
		// the "too broad" guard and, if the Hub was started in $HOME, a
		// bypassPermissions child can be planted at $HOME.
		base := parent.CWD
		if base == "" {
			base = s.hubCWD
		}
		cwd = filepath.Join(base, cwd)
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

	// C3 (plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md): SameTree は
	// tri-state。nil のときだけ config の worktree_auto を見る（既定どおり worktree が
	// 勝つ）。明示 true は worktree を使わず、要求 cwd をそのまま子 cwd にする。
	var childCWD, branch, worktreeNote string
	if body.SameTree != nil && *body.SameTree {
		childCWD = cwd
		worktreeNote = "worktree skip: requested by --same-tree"
	} else {
		childCWD, branch, worktreeNote = s.prepareChildWorktree(cwd, orchestrationID, body.Role, cfg)
	}
	if worktreeNote != "" {
		_ = s.appendBoardSection(boardPath, "hub", fmt.Sprintf("%s\n", worktreeNote))
	}
	return childSpawnPreparation{orchestrationID: orchestrationID, boardPath: boardPath, childCWD: childCWD, branch: branch, absoluteCWD: cwd}, nil
}

func (s *Server) dispatchSpawn(parentID int, parent *session, body spawnChildRequest, prep childSpawnPreparation) (childSpawnResult, error) {
	// C1 (plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md): 子へ実際に渡す
	// 最終値を1行に残す。cwd は「要求値・絶対化後・worktree 適用後」の3つを並べ、-cwd が
	// どこで worktree の cwd へ置き換わったかを1行で追えるようにする（症状Bの判断材料）。
	// board への worktree created/reuse/skip 記録は preparePromptAndWorktree 側で今まで
	// どおり残る。ここは関数の先頭でのみ追記し、末尾（inject 系）には触れない。
	s.logger.Info("orchestration spawn-child dispatch",
		"parent_session_id", parentID, "role", body.Role,
		"provider", body.Provider, "model", body.Model,
		"requested_cwd", body.CWD, "absolute_cwd", prep.absoluteCWD, "resolved_cwd", prep.childCWD,
		"worktree_branch", prep.branch,
		"has_subscription_profile_id", strings.TrimSpace(body.SubscriptionProfileID) != "",
	)
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
	// AI 経由の spawn（`orchestrate spawn`）はサブスクリプションを指定する手段が無く、
	// 以前は常に空＝CLI 既定のログインで子が起きていた。複数契約を使い分けている利用者には
	// 事故になるので、ここで解決する（resolveChildSubscription が正本）。
	//
	// **body へ代入せずローカル変数で持つ。** TestLiveSessionAuthIsNeverSwapped が
	// 「SubscriptionProfileID への代入は session 生成時（wrapper_loop.go）だけ」を
	// 機械で守っている。ここは生成に渡す値を決めているのであって、実行中セッションの
	// 認証を差し替えているのではない。検査を緩めずに意図を通すため、代入を作らない。
	subscriptionID := strings.TrimSpace(body.SubscriptionProfileID)
	if subscriptionID == "" {
		subscriptionID = s.resolveChildSubscription(parent, body.Provider, body.Role)
	}
	childID, err := s.spawnWrappedSession(spawnWrappedSpec{
		Provider:              body.Provider,
		CWD:                   prep.childCWD,
		Model:                 body.Model,
		ModelSelection:        body.ModelSelection,
		RiskConfirmed:         body.RiskConfirmed,
		Label:                 label,
		PermissionMode:        body.PermissionMode,
		Sandbox:               body.Sandbox,
		AskForApproval:        body.AskForApproval,
		Route:                 body.Route,
		SubscriptionProfileID: subscriptionID,
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
		RiskConfirmed: body.RiskConfirmed, PermissionMode: body.PermissionMode, Sandbox: body.Sandbox,
		AskForApproval: body.AskForApproval, Route: body.Route,
		SubscriptionProfileID: subscriptionID,
	}, body.InitialPrompt, prep.branch, 0)
	prompt := buildChildInitialPrompt(body.InitialPrompt, prep.boardPath, body.Role, prep.branch, childID)
	notice := injectNotice{ParentID: parentID, BoardPath: prep.boardPath, Role: body.Role}
	s.safeGo("inject_initial_prompt_child", func() { s.injectInitialPromptNotify(childID, prompt, notice) })
	return childSpawnResult{ID: childID, BoardPath: prep.boardPath, CWD: prep.childCWD, WorktreeBranch: prep.branch}, nil
}

// spawnHTTPError is the structured failure shape performSpawn returns. The
// HTTP handler (no confirmation required) turns it straight into
// writeJSONError; the confirmation-decision goroutine (which has no
// http.ResponseWriter) turns it into a board note instead
// (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C1).
type spawnHTTPError struct {
	status int
	code   string
	detail string
}

func (e *spawnHTTPError) Error() string { return e.detail }

// performSpawn runs every step of child creation that used to sit inline in
// handleSpawnChild after a spawn confirmation (if one was required) has
// already been decided: re-reading the parent and orchestration limits from
// a fresh snapshot, the duplicate-role guard, worktree/prompt preparation,
// and dispatch. It never touches an http.ResponseWriter, so it is callable
// from either handleSpawnChild directly (no confirmation required) or
// resolveSpawnConfirmationDecision (confirmation required) — exactly one of
// those two call sites runs for a given spawn request, so the child is never
// launched twice for it.
//
// requestedProvider is the caller's original, pre-resolution provider value;
// it is only used to decide whether to update the remembered role→provider
// mapping (an explicit --provider should not change future defaults).
// providerChangeNote, when non-empty, is appended to the child's board once
// the board exists (used when a confirmation decision changed the provider
// from what was requested).
func (s *Server) performSpawn(parentID int, requestedProvider, providerChangeNote string, body spawnChildRequest) (childSpawnResult, *spawnHTTPError) {
	if !validOrchestrationProvider(body.Provider) {
		return childSpawnResult{}, &spawnHTTPError{status: http.StatusBadRequest, code: "bad_request", detail: invalidProviderDetail(body.Provider)}
	}
	if !spawnValidModelLabel(body.Model) || strings.HasPrefix(body.Model, "-") {
		return childSpawnResult{}, &spawnHTTPError{status: http.StatusBadRequest, code: "bad_request", detail: "invalid model selected for child"}
	}
	parent, childCount, totalSessions := s.orchestrationParentState(parentID)
	if parent == nil {
		return childSpawnResult{}, &spawnHTTPError{status: http.StatusNotFound, code: "not_found", detail: "parent session not found"}
	}
	cfg := s.snapshotCfg().Orchestration
	applyChildApprovalDefaults(&body, cfg.ChildFullBypassEnabled())
	if parent.Depth >= cfg.MaxDepth {
		s.notifyOrchestrationError(parentID, "depth", "max orchestration depth reached")
		return childSpawnResult{}, &spawnHTTPError{status: http.StatusTooManyRequests, code: "orchestration_limit", detail: "max orchestration depth reached"}
	}
	if childCount >= cfg.MaxChildrenPerParent {
		s.notifyOrchestrationError(parentID, "children_per_parent", "max children per parent reached")
		return childSpawnResult{}, &spawnHTTPError{status: http.StatusTooManyRequests, code: "orchestration_limit", detail: "max children per parent reached"}
	}
	if totalSessions >= cfg.MaxTotalSessions {
		s.notifyOrchestrationError(parentID, "total_sessions", "max total sessions reached")
		return childSpawnResult{}, &spawnHTTPError{status: http.StatusTooManyRequests, code: "orchestration_limit", detail: "max total sessions reached"}
	}
	// 同 role の生存子がいる場合の重複 spawn ガード。conductor が修正指示・再確認指示を
	// 毎回 spawn で出すと生存子数が max_children_per_parent に到達して詰まる実測があった
	// （plan_orchestration-conductor-improvements.md C2）。既存子への指示は send を使わせる。
	if !body.Force {
		if live := s.liveChildForRole(parentID, body.Role); live != nil {
			return childSpawnResult{}, &spawnHTTPError{status: http.StatusConflict, code: "duplicate_role_child", detail: fmt.Sprintf("live child #%d already exists for role %q; use `many-ai-cli orchestrate send --role %s \"<text>\"` to instruct it, or pass --force to spawn another", live.ID, body.Role, body.Role)}
		}
	}

	prep, err := s.preparePromptAndWorktree(parentID, parent, body, cfg)
	if err != nil {
		var prepErr *spawnPreparationError
		if errors.As(err, &prepErr) {
			return childSpawnResult{}, &spawnHTTPError{status: prepErr.status, code: prepErr.code, detail: prepErr.detail}
		}
		return childSpawnResult{}, &spawnHTTPError{status: http.StatusInternalServerError, code: "board_error", detail: errorDetail("child preparation failed", err)}
	}
	if providerChangeNote != "" {
		_ = s.appendBoardSection(prep.boardPath, "hub", providerChangeNote+"\n")
	}
	result, err := s.dispatchSpawn(parentID, parent, body, prep)
	if err != nil {
		return childSpawnResult{}, &spawnHTTPError{status: http.StatusInternalServerError, code: "spawn_error", detail: errorDetail("spawn error", err)}
	}
	// 起動できた組み合わせだけを覚える。失敗した provider を記憶すると、次回も同じ失敗を繰り返す。
	// 明示 --provider を渡した spawn は記憶を更新しない。「今回はこれ」という指定であって、
	// 次回以降の既定を変える意思とは限らないため（C2, plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md）。
	if strings.TrimSpace(requestedProvider) == "" {
		s.rememberRoleProvider(body.Role, body.Provider)
	}
	return result, nil
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

	// C1 (plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md): AI/CLI が渡した
	// 要求値をここで1行残す。以後 body.Provider 等は解決処理で書き換わるため、この時点の
	// 値を先に確保しておく。initial_prompt の本文は出さない（利用者の作業内容そのもの）。
	requestedProvider := body.Provider
	requestedModel := body.Model
	requestedCWD := body.CWD
	s.logger.Info("orchestration spawn-child requested",
		"parent_session_id", parentID, "role", body.Role,
		"requested_provider", requestedProvider, "requested_model", requestedModel,
		"requested_cwd", requestedCWD, "force", body.Force)

	parent, _, _ := s.orchestrationParentState(parentID)
	if parent == nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "parent session not found")
		return
	}

	// C2 (plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md): provider/model の
	// 解決は resolveSpawnChildProvider（resolveChildProvider が正本の純関数）へ集約した。
	// 明示 --provider は役割対応表・記憶・親 provider のいずれにも負けない。
	providerSource := s.resolveSpawnChildProvider(parent, &body)
	if providerSource == providerSourceDefault {
		parentProvider := ""
		if parent != nil {
			parentProvider = parent.Provider
		}
		s.logger.Info("orchestration child provider fallback used hardcoded default",
			"role", body.Role, "parent_provider", parentProvider, "fallback_provider", "codex")
	}
	s.logger.Info("orchestration spawn-child provider resolved",
		"parent_session_id", parentID, "role", body.Role,
		"provider", body.Provider, "provider_source", providerSource)

	if !validOrchestrationProvider(body.Provider) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", invalidProviderDetail(body.Provider))
		return
	}
	if !spawnValidModelLabel(body.Model) || strings.HasPrefix(body.Model, "-") {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid model value")
		return
	}
	if s.spawnConfirmationRequired(body.Provider) {
		pending, registerErr := s.registerSpawnConfirmation(parent, requestedProvider, body)
		if registerErr != nil {
			writeJSONError(w, registerErr.status, registerErr.code, registerErr.detail)
			return
		}
		outcome, ok := s.waitSpawnConfirmation(r.Context(), pending)
		if !ok {
			// The caller's own HTTP request died (its tool call timed out, or
			// the process holding it was killed). The confirmation is
			// deliberately left exactly as it was: no board write, no
			// response, because nothing has actually been decided yet. It
			// stays pending in the Hub and a browser can still decide it
			// later — handleSpawnConfirmation's goroutine performs the spawn
			// itself in that case
			// (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md C1).
			return
		}
		if !outcome.Approved {
			writeJSONStatus(w, http.StatusForbidden, httpErrorResp{OK: false, Error: "spawn_refused", Detail: "child spawn was refused by the user"})
			return
		}
		if outcome.Err != nil {
			writeJSONError(w, outcome.Err.status, outcome.Err.code, outcome.Err.detail)
			return
		}
		result := outcome.Result
		writeJSON(w, map[string]any{"ok": true, "session_id": result.ID, "board_path": result.BoardPath, "cwd": result.CWD, "worktree_branch": result.WorktreeBranch})
		return
	}

	// No confirmation required for this provider: this handler is the only
	// place deciding whether the child gets created, so it calls performSpawn
	// directly instead of going through the confirmation/decision split.
	result, spawnErr := s.performSpawn(parentID, requestedProvider, "", body)
	if spawnErr != nil {
		writeJSONError(w, spawnErr.status, spawnErr.code, spawnErr.detail)
		return
	}
	writeJSON(w, map[string]any{"ok": true, "session_id": result.ID, "board_path": result.BoardPath, "cwd": result.CWD, "worktree_branch": result.WorktreeBranch})
}

// applyChildApprovalDefaults は orchestration 子セッションの承認モード既定値を埋める。
// fullBypass が true（既定）のとき、呼び出し側が指定しない場合はプロバイダごとの
// 全許可（承認バイパス相当）と RiskConfirmed=true を埋める。子は自走が前提で、
// 承認プロンプトを人間が張り付いて処理する運用は自走目的と矛盾するため。危険操作の
// 抑制は承認ではなく worktree 隔離と board の禁止事項で行う
// （docs/local/bugfix_orchestration-codex-child-spawn-failures_2026-07-04.md）。
// fullBypass が false（orchestration.child_full_bypass: false）のときは既定を埋めず、
// 呼び出し側の明示値のみを使う（高リスク権限の自動確認を避ける安全側パス）。
func applyChildApprovalDefaults(body *spawnChildRequest, fullBypass bool) {
	if !fullBypass {
		return
	}
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

// markChildPromptDelivered records that a child's initial prompt was confirmed
// delivered (echo observed, or a late echo from a previous attempt).
// injectInitialPromptNotify does not carry a board ID (see its doc comment),
// so — like completeOrchestrationChildOnSessionEnd — this walks every board
// and finds the child by session ID
// (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md C1).
func (s *Server) markChildPromptDelivered(sessionID int, at time.Time) {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	for _, board := range s.orchestration.boards {
		if child := board.Children[sessionID]; child != nil {
			child.PromptDeliveredAt = at
			return
		}
	}
}

// markChildPromptFailed records that a child's initial prompt delivery
// failed. reportInjectFailure has already told the parent/board by the time
// this is called (composerBlocked or attempts exhausted); this only marks
// the child so childStartupFailed does not treat an undelivered prompt as a
// startup failure on top of the delivery failure already reported.
func (s *Server) markChildPromptFailed(sessionID int) {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	for _, board := range s.orchestration.boards {
		if child := board.Children[sessionID]; child != nil {
			child.PromptFailed = true
			return
		}
	}
}

// markChildStandbySince starts the standby clock for a child the first time
// it enters standby (evaluateIdle's fallbackDone transition). It is a no-op
// once the clock is already running, and a no-op for a non-child session
// (boardID resolves to no board, or the session id is not in Children) since
// ses.OrchestrationID is set on conductors too.
func (s *Server) markChildStandbySince(boardID string, sessionID int, since time.Time) {
	if boardID == "" {
		return
	}
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	if board := s.orchestration.boards[boardID]; board != nil {
		if child := board.Children[sessionID]; child != nil && child.StandbySince.IsZero() {
			child.StandbySince = since
		}
	}
}

// resetChildStandbySince clears the standby clock when a child goes back to
// running (markRunning). Called on every PTY output chunk, so it takes the
// boardID directly (from the already-locked session) instead of walking
// every board like markChildPromptDelivered/Failed do.
func (s *Server) resetChildStandbySince(boardID string, sessionID int) {
	if boardID == "" {
		return
	}
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	if board := s.orchestration.boards[boardID]; board != nil {
		if child := board.Children[sessionID]; child != nil {
			child.StandbySince = time.Time{}
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
		StartupFailed:  map[int]bool{},
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
	s.checkRelayReconnect(now)
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
	// A relay board (relay.go) keeps the file bookkeeping below but gets none
	// of the generic DONE handling and progress notices (D-6): the relay counts
	// the DONE lines itself and decides what to inject next.
	owns := s.relayOwns(boardID)

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
		if owns {
			continue
		}
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
	if owns {
		for _, u := range updates {
			s.relayOnChildFileChange(boardID, u.id, u.text)
		}
		return
	}
	for _, n := range notices {
		s.notifyBoardSession(boardID, n.sessionID, n.text)
	}
	for _, id := range doneIDs {
		s.markChildState(id, "done")
	}
}

func (s *Server) handleBoardChange(boardID, boardPath string, info os.FileInfo, text string, now time.Time) {
	// A relay board only records the write; the relay's own instructions and
	// records go there, and neither DONE authorization nor "board updated"
	// notices apply (D-6). relayOwns takes orchestration.mu, so ask before
	// taking the lock here.
	owns := s.relayOwns(boardID)
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
	if owns {
		s.orchestration.mu.Unlock()
		return
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
	injectGated := ses != nil && sessionInjectGated(ses, time.Now())
	s.sessionsMu.Unlock()
	if !isConductor {
		if injectGated {
			// 初期プロンプトが入る前の子へ board 通知を送らない。子は初期プロンプトで
			// 「board を読んでから動け」と指示されるので通知は要らず、ここで送るとゲートが
			// 保留キューへ積み、注入より先に着いて 1 通目が通知になる。
			return
		}
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

// childStartupFailedLocked implements D2's startup-handshake-failure
// judgment (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md)
// from already-read values, taking no locks itself. Callers that already hold
// orchestration.mu (checkOrchestrationChildTimers) can call this directly
// with a pre-fetched approvalVisible; childStartupFailed below is the
// standalone, locking entry point for everyone else (tests included).
//
// True only when BOTH hold: the initial prompt's delivery was confirmed and
// the child has never written its progress file (child.FileMod.IsZero()),
// AND the session has been standby (no PTY output) for at least
// cfg.ChildStartupGraceSeconds while not waiting on an approval prompt.
func childStartupFailedLocked(child *orchestrationChild, approvalVisible bool, now time.Time, cfg config.OrchestrationConfig) bool {
	if !cfg.ChildStartupFailEnabled() {
		return false
	}
	if child.PromptDeliveredAt.IsZero() || child.PromptFailed {
		// 配送が確認できていない（未注入・保留キュー行き・配送失敗のいずれか）。
		// 誤検知で殺すより従来の timeout に任せるほうが安全。
		return false
	}
	if !child.FileMod.IsZero() {
		// 一度でも進捗ファイルを書いた子は働いている。timeout の担当。
		return false
	}
	if approvalVisible {
		// 承認待ちの間は grace のカウントを進めない。
		return false
	}
	if child.StandbySince.IsZero() {
		// standby ではない（まだ running、または一度も standby になっていない）。
		return false
	}
	grace := cfg.ChildStartupGraceSeconds
	if grace <= 0 {
		grace = 60
	}
	return now.Sub(child.StandbySince) >= time.Duration(grace)*time.Second
}

// childStartupFailed is the locking, session-ID-addressed entry point for
// childStartupFailedLocked. It also enforces the D9 latch: once a board has
// recorded StartupFailed[sessionID] (set by handleChildStartupFailed after it
// has acted on a true verdict), this returns false so the same child cannot
// fire the judgment twice.
func (s *Server) childStartupFailed(sessionID int, now time.Time, cfg config.OrchestrationConfig) bool {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	approvalVisible := ses != nil && ses.approvalVisible
	s.sessionsMu.Unlock()

	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	for _, board := range s.orchestration.boards {
		child := board.Children[sessionID]
		if child == nil {
			continue
		}
		if board.StartupFailed[sessionID] {
			return false
		}
		return childStartupFailedLocked(child, approvalVisible, now, cfg)
	}
	return false
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
	// A relay board routes timeout / idle to the relay (relay.go) instead of
	// marking the child or warning the conductor (D-6 / D-15). The latches
	// (TimedOut / IdleWarned) below are shared; the relay releases them when it
	// hands the child its next instruction.
	owns := s.relayOwns(boardID)
	if owns && s.relayTerminal(boardID) {
		return
	}
	boardPath := ""
	// PTY 出力時刻のスナップショット。board 記帳が止まっていても PTY 出力が動いている子は
	// 作業中（plan 読込・実装・レビュー等）とみなし idle warning を出さない。board 記帳時刻
	// だけの判定は実測で偽陽性を連発した（plan_orchestration-conductor-improvements.md C1）。
	// approvalVisible も同じ理由で事前取得する（承認待ちを起動失敗の grace に含めないため。
	// plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md D2）。
	// sessionsMu → orchestration.mu の順に短く取り、入れ子にしない。
	lastOutputs := map[int]time.Time{}
	approvalVisible := map[int]bool{}
	s.sessionsMu.Lock()
	for id, ses := range s.sessions {
		lastOutputs[id] = ses.lastOutputAt
		approvalVisible[id] = ses.approvalVisible
	}
	s.sessionsMu.Unlock()
	s.orchestration.mu.Lock()
	b := s.orchestration.boards[boardID]
	if b != nil {
		boardPath = b.Path
		for id, child := range b.Children {
			if child.Done || b.Done[id] || b.TimedOut[id] || b.StartupFailed[id] {
				continue
			}
			if childStartupFailedLocked(child, approvalVisible[id], now, cfg) {
				notices = append(notices, notice{parentID: child.ParentID, childID: id, role: child.Role, kind: "startup_failed"})
				continue
			}
			if cfg.ChildTimeoutSeconds > 0 {
				// Measure the timeout from the child's last activity, not its
				// spawn time. A child still actively writing to the board or
				// emitting PTY output past ChildTimeoutSeconds is healthy and must
				// not be force-marked "timeout" — that state makes it unsendable
				// and, with TimeoutRespawn, starts a 2nd body in the same worktree.
				lastActivity := child.SpawnedAt
				if child.LastBoardWrite.After(lastActivity) {
					lastActivity = child.LastBoardWrite
				}
				if last, ok := lastOutputs[id]; ok && last.After(lastActivity) {
					lastActivity = last
				}
				if now.Sub(lastActivity) > time.Duration(cfg.ChildTimeoutSeconds)*time.Second {
					b.TimedOut[id] = true
					notices = append(notices, notice{parentID: child.ParentID, childID: id, role: child.Role, kind: "timeout", state: "timeout", threshold: cfg.ChildTimeoutSeconds})
					continue
				}
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
		if n.kind == "startup_failed" {
			// Evidence capture / board record / notify-or-relay / stop all happen
			// regardless of relay ownership (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md
			// C2/C3); handleChildStartupFailed branches on owns internally.
			s.handleChildStartupFailed(boardID, boardPath, n.childID, n.role, n.parentID, owns, cfg)
			continue
		}
		if owns {
			switch n.kind {
			case "timeout":
				s.relayOnChildTimeout(boardID, n.childID)
			case "idle":
				_ = s.appendBoardSection(boardPath, "hub", fmt.Sprintf("idle role=%s id=%d threshold=%ds\n", n.role, n.childID, n.threshold))
				s.relayOnChildIdle(boardID, n.childID)
			}
			continue
		}
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

// childStartupFailureScreenTail collects the last few non-empty screen lines
// as evidence for a startup-failure notification (D4,
// plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md). The
// input is the child's raw provider screen, not an AI-authored DONE line, so
// unlike lastUsefulDoneLine (done_summary.go) this keeps up to maxLines lines
// instead of just one; it reuses that function's box-removal
// (inputBoxTopIndex) and per-line cleanup (cleanTUILine) but writes its own
// multi-line collection since the two inputs are shaped differently.
func (s *Server) childStartupFailureScreenTail(sessionID int, maxLines int) []string {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	var screen []string
	if ses != nil && ses.vt != nil {
		screen = ses.vt.Lines()
	}
	s.sessionsMu.Unlock()
	if len(screen) == 0 {
		return nil
	}
	end := len(screen)
	if top := inputBoxTopIndex(screen); top >= 0 {
		end = top
	}
	var lines []string
	for i := end - 1; i >= 0 && len(lines) < maxLines; i-- {
		raw := strings.TrimSpace(screen[i])
		if strings.HasPrefix(raw, "│") || strings.HasPrefix(raw, "┃") {
			// 枠線で囲まれたパネルの中身。上辺を見つけられなかったときの受け皿
			// （lastUsefulDoneLine と同じ理由）。
			continue
		}
		line := strings.TrimSpace(cleanTUILine(screen[i]))
		if line == "" {
			continue
		}
		lines = append(lines, line)
	}
	// 収集は画面末尾から遡ったので、表示順（上から下）へ戻す。
	for i, j := 0, len(lines)-1; i < j; i, j = i+1, j-1 {
		lines[i], lines[j] = lines[j], lines[i]
	}
	return lines
}

// childStartupFailureScreenTailMaxLines bounds how many screen lines
// handleChildStartupFailed captures as evidence (D4).
const childStartupFailureScreenTailMaxLines = 5

// handleChildStartupFailed is the C2/C3 action taken once childStartupFailed
// (via checkOrchestrationChildTimers) has judged a child as a startup
// failure. It captures the screen tail as evidence, records it on the board,
// latches StartupFailed so this fires once (D9), then either tells the
// parent directly or hands the child to its relay (D10), and finally stops
// the child's wrapper if child_startup_kill is enabled (D6). It never calls
// respawnTimedOutChild (D5): a startup failure is not a timeout, and
// re-spawning with the same spec would repeat the same failure.
//
// owns and cfg are passed in from the caller's already-computed values
// (relayOwns / snapshotCfg) so this does not pay for a second lookup.
func (s *Server) handleChildStartupFailed(boardID, boardPath string, childID int, role string, parentID int, owns bool, cfg config.OrchestrationConfig) {
	tail := sanitizeInjectText(strings.Join(s.childStartupFailureScreenTail(childID, childStartupFailureScreenTailMaxLines), "\n"))
	_ = s.appendBoardSection(boardPath, "hub", fmt.Sprintf("startup failed: role=%s id=%d never produced any progress after its initial prompt was delivered. screen tail:\n%s\n", role, childID, tail))

	s.orchestration.mu.Lock()
	if board := s.orchestration.boards[boardID]; board != nil {
		if board.StartupFailed == nil {
			board.StartupFailed = map[int]bool{}
		}
		board.StartupFailed[childID] = true
	}
	s.orchestration.mu.Unlock()

	if owns {
		s.relayOnChildStartupFailed(boardID, childID)
	} else {
		detail := fmt.Sprintf("role=%s id=%d: the child never produced any progress after its initial prompt was delivered; the child CLI itself needs attention (check its model/auth/provider settings) before retrying the same role/provider. screen tail: %s", role, childID, tail)
		s.notifyOrchestrationError(parentID, "startup_failed", detail)
		s.markChildState(childID, "error")
	}

	if cfg.ChildStartupKillEnabled() {
		s.killWrapper(childID, "startup_failed")
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
	if s.relayOwns(boardID) {
		// The relay stops itself (stopped(child_exited)) and notifies the parent
		// through its own finish path (D-6).
		s.relayOnChildExit(boardID, sessionID, state)
		return
	}
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
		s.injectInitialPromptNotify(newID,
			buildChildInitialPrompt(child.InitialPrompt, board.Path, child.Role, child.WorktreeBranch, newID),
			injectNotice{ParentID: child.ParentID, BoardPath: board.Path, Role: child.Role})
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
		if !strings.HasPrefix(line, "## ") || strings.HasPrefix(line, "## DONE ") || strings.HasPrefix(line, "## SUCCESS ") {
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
		Type:                 "session_update",
		SessionID:            ses.ID,
		Provider:             ses.Provider,
		Display:              ses.Display,
		CWD:                  ses.CWD,
		Branch:               ses.Branch,
		ProjectID:            ses.ProjectID,
		Label:                ses.Label,
		Model:                ses.Model,
		Route:                ses.Route,
		State:                ses.State,
		OutputIdle:           ses.Activity.OutputIdle,
		WorkflowActive:       ses.Activity.WorkflowActive,
		AwaitingUser:         ses.Activity.AwaitingUser,
		AwaitingApproval:     ses.Activity.AwaitingApproval,
		Activity:             activityMessage(ses.Activity),
		ApprovalSourceEpoch:  ensureApprovalSourceEpochLocked(ses),
		LastOutputAt:         ses.LastOutputAt,
		StartedAt:            ses.StartedAt,
		ParentSessionID:      ses.ParentSessionID,
		Role:                 ses.Role,
		Auto:                 ses.Auto,
		Depth:                ses.Depth,
		OrchestrationID:      ses.OrchestrationID,
		BoardPath:            ses.BoardPath,
		WorktreeBranch:       ses.WorktreeBranch,
		BoardNotifyPending:   ses.BoardNotifyPending,
		Relays:               copyRelayStatuses(ses.Relays),
		CrossSessionMessages: copyCrossSessionMessages(ses.CrossSessionMessages),
		SubscriptionID:       ses.SubscriptionProfileID,
		SubscriptionName:     ses.SubscriptionProfileName,
	}
}

// copyRelayStatuses flattens the parent's relay snapshots for the wire. The
// caller holds sessionsMu; the pointers are never mutated after publication.
func copyRelayStatuses(relays []*proto.RelayStatus) []proto.RelayStatus {
	if len(relays) == 0 {
		return nil
	}
	out := make([]proto.RelayStatus, 0, len(relays))
	for _, r := range relays {
		if r != nil {
			out = append(out, *r)
		}
	}
	return out
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

// providerComposerSignals は「入力欄が描画され、キー入力を受け付けられる」ことの陽性シグナル。
// 空白除去済み（collapseWhitespace 後）の画面テキストに対して部分一致で見る。
//
// 静止時間による判定（waitForInputReady）は、起動途中に静止窓を持つ provider では
// 原理的に誤発火する。2026-07-04 の bugfix（根本原因 A）で指摘済みだが、当時の対策は
// 「注入後にエコーが出なければ再注入」というリトライで、**早すぎる 1 回目の書き込み自体は
// 残っていた**。2026-09-01、その 1 回目が codex をモーダルへ固着させ、リトライでも
// Web の承認パネルでも抜けられないことが実測された
// （bugfix_codex-update-screen-swallows-initial-prompt_2026-09-01.md 観測 9）。
//
// **実測で確認した provider だけを載せる。** 載っていない provider は従来どおり静止判定へ
// フォールバックする（未確認の合図で待ち続けて注入が永久に止まる方が害が大きい）。
var providerComposerSignals = map[string][]string{
	// 実測: logs/sessions/codex_2026-09-01_*_s{11,12,14}.log（通常セッション・到達時）
	"codex": {"AskCodextodoanything"},
}

// providerBlockingSignals は「モーダルが出ていて入力を受け付けない」ことのシグナル。
// これが画面にある間は注入しない。**注入すると抜けられなくなる**ため、待つか諦めるかしかない。
var providerBlockingSignals = map[string][]string{
	"codex": {
		"Doyoutrustthecontentsofthisdirectory", // ディレクトリ信頼の確認
		"Updateavailable!",                     // 自己更新メニュー
		"Pressentertocontinue",                 // 上記 2 つの共通フッター
	},
}

// composerWaitResult は waitForComposerReady の判定結果。
type composerWaitResult int

const (
	composerReady    composerWaitResult = iota // 入力欄が出ており、モーダルも無い
	composerBlocked                            // モーダルが出たまま抜けない
	composerUnknown                            // 陽性シグナルを一度も観測できなかった
	composerNoSignal                           // この provider の合図が未定義（従来判定へ）
)

const (
	// 入力欄の描画を待つ上限。claude は splash から入力欄まで 12 秒かかった実測がある
	// （orchestration.go 冒頭のコメント参照）ので、その 3 倍強を取る。
	orchestrationComposerMaxWait = 45 * time.Second
	// 「入力欄あり・モーダル無し」がこの時間続いて初めて ready と判定する。
	// codex は入力欄を描いた直後にモーダルを重ねてくるため、瞬間値では取り違える
	// （実測: s13 では composer 描画とモーダル出現が同じ秒に入っていた）。
	// blocked 判定（モーダルが安定して出続けている）にも同じ定数を使う。ready/blocked
	// どちらも「確定にはこれだけの継続時間を要求する」という同じ意味論であり、
	// 別定数を増やす理由が無い。
	orchestrationComposerStableFor = 1500 * time.Millisecond
)

// waitForComposerReady は入力欄の陽性シグナルを待ち、モーダルが出ていないことも確認する。
// 第 2 戻り値は composerBlocked のときに検出したモーダルのシグナル文字列。
func (s *Server) waitForComposerReady(sessionID int, maxWait time.Duration) (composerWaitResult, string) {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	provider := ""
	if ses != nil {
		provider = ses.Provider
	}
	s.sessionsMu.Unlock()
	if ses == nil {
		return composerUnknown, ""
	}
	readySignals := providerComposerSignals[provider]
	if len(readySignals) == 0 {
		return composerNoSignal, ""
	}
	blockers := providerBlockingSignals[provider]

	deadline := time.Now().Add(maxWait)
	var readySince time.Time
	var blockedSince time.Time
	lastBlocker := ""
	for {
		s.sessionsMu.Lock()
		cur := s.sessions[sessionID]
		screen := ""
		if cur != nil && cur.vt != nil {
			screen = collapseWhitespace(strings.Join(cur.vt.Lines(), ""))
		}
		s.sessionsMu.Unlock()
		if cur == nil {
			return composerUnknown, ""
		}

		blocked := ""
		for _, b := range blockers {
			if strings.Contains(screen, b) {
				blocked = b
				break
			}
		}
		switch {
		case blocked != "":
			lastBlocker = blocked
			readySince = time.Time{}
			// ready 側と対称: モーダルが安定してこの時間出続けたら、deadline を待たず
			// その場で確定させる。ここで即座に返さないと、モーダルが起動直後から
			// 一度も揺らがず出ていても maxWait（45秒）満了まで無意味なポーリングを
			// 続けてから composerBlocked を返すことになり、reportInjectFailure による
			// 通知が最大 45 秒遅れる（2026-09-02 発見）。
			if blockedSince.IsZero() {
				blockedSince = time.Now()
			}
			if time.Since(blockedSince) >= orchestrationComposerStableFor {
				return composerBlocked, lastBlocker
			}
		case containsAnySignal(screen, readySignals):
			blockedSince = time.Time{}
			if readySince.IsZero() {
				readySince = time.Now()
			}
			if time.Since(readySince) >= orchestrationComposerStableFor {
				return composerReady, ""
			}
		default:
			readySince = time.Time{}
			blockedSince = time.Time{}
		}

		if time.Now().After(deadline) {
			if lastBlocker != "" {
				return composerBlocked, lastBlocker
			}
			return composerUnknown, ""
		}
		time.Sleep(orchestrationInjectEchoPoll)
	}
}

func containsAnySignal(screen string, signals []string) bool {
	for _, sig := range signals {
		if strings.Contains(screen, sig) {
			return true
		}
	}
	return false
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
	s.submitInput(sessionID, text)
}

// injectRawBypassGate は初期プロンプト注入（injectInitialPrompt）専用の送信経路。
// initialInjectPending ゲート中でも、ゲートが積んだ保留キューも追い越して wrapper へ
// 直接届ける（通常経路だと注入自体が pendingInput へ回ってデッドロックするため）。
// 戻り値は wrapper へ書けたかどうか。false なら保留キューに入っているので送り直さない。
func (s *Server) injectRawBypassGate(sessionID int, text string) bool {
	return s.submitInputWithGate(sessionID, text, true)
}

// injectInitialPrompt は orchestration セッション（conductor / 子）への初期プロンプトを、
// CLI の入力受付開始を実観測しながら注入する。手順:
//  1. waitForInputReady で出力静止を待つ（従来判定・早期注入の目安）
//  2. 注入し、PTY 画面（VT バッファ）に注入テキストのエコーが現れるかを確認
//  3. 現れなければ CLI 起動途中で入力が捨てられたとみなし再注入（最大 MaxAttempts 回）
//
// 完了・断念にかかわらず initialInjectPending ゲートを解除し、保留中のユーザー入力を
// 順番どおり flush する。goroutine で呼ぶこと（エコー検証で数十秒ブロックしうる）。
// injectNotice は初期プロンプトを届けられなかったときの報告先。
// 子セッションでは board と親（conductor）へ出す。conductor 自身への注入では空でよい。
type injectNotice struct {
	ParentID  int
	BoardPath string
	Role      string
}

// reportInjectFailure は「初期プロンプトが届いていない」ことを board と親へ出す。
//
// これが無いと、Hub は失敗を検知していながら誰にも伝えず、カードは実行中のまま残る。
// 指揮者は届いたつもりで待ち続け、利用者が画面を見るまで気づけない
// （2026-09-01 実測: 指揮者が 3 通の指示を書き終えるまで気づかなかった）。
func (s *Server) reportInjectFailure(sessionID int, notice injectNotice, detail string) {
	if notice.BoardPath != "" {
		_ = s.appendBoardSection(notice.BoardPath, "hub",
			fmt.Sprintf("initial prompt NOT delivered: role=%s session=%d %s\n", notice.Role, sessionID, detail))
	}
	if notice.ParentID > 0 {
		s.notifyOrchestrationError(notice.ParentID, "child_input_blocked",
			fmt.Sprintf("role=%s id=%d: %s", notice.Role, sessionID, detail))
	}
}

func (s *Server) injectInitialPrompt(sessionID int, prompt string) {
	s.injectInitialPromptNotify(sessionID, prompt, injectNotice{})
}

func (s *Server) injectInitialPromptNotify(sessionID int, prompt string, notice injectNotice) {
	defer s.clearInitialInjectGate(sessionID)
	marker := injectEchoMarker(prompt)
	// チャット送信・injectText と同じくブラケットペーストで包み、確定 \r は
	// splitBracketedPasteSubmit（trySendInput）が別書き込み + 遅延で送る。
	// 「本文+\r」同一チャンクだと本文がエコーされても \r が確定キーとして
	// 処理されないことがある（Grok 実測 2026-07-11）。エコー検証（marker）は
	// 可視テキストで行うためマーカー包みの影響を受けない。
	text := bracketedPasteStart + strings.TrimRight(prompt, "\r\n") + bracketedPasteEnd + "\r"
	for attempt := 1; attempt <= orchestrationInjectMaxAttempts; attempt++ {
		// 入力欄が出たことを陽性シグナルで確かめてから書き込む。
		// モーダルが出ている間に書き込むと、そのモーダルから抜けられなくなる。
		switch res, blocker := s.waitForComposerReady(sessionID, orchestrationComposerMaxWait); res {
		case composerReady:
			// 入力欄が安定して出ている。このまま注入する。
		case composerBlocked:
			s.logger.Warn("initial prompt not injected; child TUI is on a modal",
				"session_id", sessionID, "attempt", attempt, "blocker", blocker)
			s.reportInjectFailure(sessionID, notice,
				fmt.Sprintf("child TUI is waiting on a modal (%s) and never reached its composer; the initial prompt was NOT delivered", blocker))
			// C1 (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md): 配送失敗を
			// 記録する。reportInjectFailure が既に親へ child_input_blocked を出したので、
			// 起動失敗判定（childStartupFailed）が同じ子へ重ねて startup_failed を出さない。
			s.markChildPromptFailed(sessionID)
			return
		case composerNoSignal:
			// この provider の合図は未確認。従来の静止判定へフォールバックする。
			s.waitForInputReady(sessionID, orchestrationInjectQuiet, orchestrationInjectMaxWait)
		default: // composerUnknown
			s.logger.Warn("composer signal not observed; falling back to quiet wait",
				"session_id", sessionID, "attempt", attempt)
			s.waitForInputReady(sessionID, orchestrationInjectQuiet, orchestrationInjectMaxWait)
		}
		if attempt > 1 && s.waitForInjectEcho(sessionID, marker, 0) {
			// 前回注入分のエコーが遅れて描画された場合は再注入しない（二重送信防止）
			s.logger.Info("initial prompt echo observed late", "session_id", sessionID, "attempt", attempt)
			s.markChildPromptDelivered(sessionID, time.Now())
			return
		}
		if !s.injectRawBypassGate(sessionID, text) {
			// wrapper へ書けず保留キューへ入った。ゲート解除時の flush がこの 1 通を
			// 届けるので、ここで送り直すと同じ本文が二重に届く。
			s.logger.Warn("initial prompt deferred to pending queue; not retrying",
				"session_id", sessionID, "attempt", attempt)
			return
		}
		if s.waitForInjectEcho(sessionID, marker, orchestrationInjectEchoWait) {
			if attempt > 1 {
				s.logger.Info("initial prompt injected after retry", "session_id", sessionID, "attempt", attempt)
			}
			s.markChildPromptDelivered(sessionID, time.Now())
			return
		}
		s.logger.Warn("initial prompt echo not observed; retrying", "session_id", sessionID, "attempt", attempt)
	}
	s.logger.Warn("initial prompt injection gave up", "session_id", sessionID, "attempts", orchestrationInjectMaxAttempts)
	s.reportInjectFailure(sessionID, notice,
		fmt.Sprintf("the child never echoed the initial prompt after %d attempts; it was NOT delivered", orchestrationInjectMaxAttempts))
	// C1 (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md): 上と同じ理由で
	// 配送失敗を記録する。
	s.markChildPromptFailed(sessionID)
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

// pastePlaceholderRe は「長い貼り付けをプレースホルダへ畳んだ」痕跡（空白除去後）。
// Claude Code は複数行の貼り付けを `[Pasted text #1 +33 lines]` に畳むため、本文の文字列は
// 1 文字も画面に出ない。実測: logs/sessions/claude_2026-08-31_122034_implementation_s14.jsonl
// の 2026-08-31 12:20:47 の行。
var pastePlaceholderRe = regexp.MustCompile(`\[Pastedtext#\d+`)

// waitForInjectEcho は注入テキストが CLI に取り込まれた痕跡が PTY 画面に現れるまで待つ。
// 痕跡は 2 通りある: marker（本文末尾）がそのまま出るか、貼り付けプレースホルダへ畳まれるか。
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
		if strings.Contains(screen, marker) || pastePlaceholderRe.MatchString(screen) {
			return true
		}
		if time.Now().After(deadline) {
			return false
		}
		time.Sleep(orchestrationInjectEchoPoll)
	}
}

// injectEchoMarker は注入テキストの末尾から、エコー検出用の空白除去済み部分文字列を作る。
//
// 先頭ではなく末尾を見るのは、TUI の入力欄が長文の「末尾」しか映さないため。実測
// （2026-08-31 session #14）では、先頭行 `You are an orchestration child session.` は画面へ
// 1 度も出ず、末尾の `…報告してください。` だけが出ていた。先頭を marker にすると claude
// 相手では原理的に一致せず、再注入が必ず撃ち切る。
func injectEchoMarker(prompt string) string {
	line := collapseWhitespace(prompt)
	const maxMarkerRunes = 16
	runes := []rune(line)
	if len(runes) > maxMarkerRunes {
		runes = runes[len(runes)-maxMarkerRunes:]
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
		b.WriteString("  <MANY_AI_CLI_BIN> orchestrate spawn --role <role> \"<prompt>\"\n")
		b.WriteString("(provider/model are resolved automatically from the mapping above; pass --provider/--model to override a specific spawn.)\n")
	} else {
		b.WriteString("No child role mapping was configured. Decide provider/model yourself whenever a child is needed and run:\n")
		b.WriteString("  <MANY_AI_CLI_BIN> orchestrate spawn --role <role> --provider <provider> --model <model> \"<prompt>\"\n")
	}
	b.WriteString("Do not call the Hub HTTP API or handle any auth token directly; this subcommand does it for you.\n")
	b.WriteString("The Hub exposes its exact executable path in MANY_AI_CLI_BIN. Invoke that path, not a many-ai-cli resolved from PATH, for every orchestrate command.\n")
	b.WriteString("To run a plan file through the implementation→review→fix relay without conducting it yourself, run: <MANY_AI_CLI_BIN> orchestrate relay --plan <path-to-plan.md> (roles come from the mapping above; pass --impl/--review provider[/model] if no mapping is configured; add --strong provider[/model] to hand a C to a stronger implementer when it keeps failing review; the relay works in its own git worktree unless you pass --same-tree). Relay children are driven by the Hub: do not spawn or send to them yourself, and you may close this session while a relay is running — it continues without you and you will be notified when it finishes.\n")
	b.WriteString("Use `<MANY_AI_CLI_BIN> orchestrate relay status [--id <orchestration-id>]` to inspect relay state and `<MANY_AI_CLI_BIN> orchestrate relay stop [--id <orchestration-id>]` to stop it.\n")
	// 2026-07-04 の実運用（plan_orchestration-conductor-improvements.md C3）で確立した
	// conductor 運用ルール。spawn 反復による枠涸渇・停止指示の解釈違い・レビューと修正の
	// レースを構造的に防ぐ。
	b.WriteString("Operating rules:\n")
	b.WriteString("- To give follow-up instructions to an existing live child, run `<MANY_AI_CLI_BIN> orchestrate send --role <role> \"<text>\"` instead of spawning again. spawn is rejected (409) while a live child exists for the role; send injects the text into the child and records it on the board automatically.\n")
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

// provider 解決の確定理由（C1/C2, plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md）。
// ログと board 記帳で共通して使う語彙。
const (
	providerSourceExplicit   = "explicit"
	providerSourceRoleMap    = "role_map"
	providerSourceRemembered = "remembered"
	providerSourceParent     = "parent"
	providerSourceDefault    = "default"
)

// resolveChildProvider は spawn child の provider をどこから決めるかを、Server に依存せず
// 値だけで決める純関数。source は判定理由（providerSource* 定数のいずれか）。
//
// 解決順:
//
//  1. explicit  — AI が明示した --provider（ユーザーがそう言った。空でなければ常に勝つ）
//  2. role_map  — オーケストレーション開始時に決めた役割 → provider の対応表
//  3. remembered — その役割で前回実際に起動した provider（user_prefs.spawn.role_provider）
//  4. parent    — 親と同じ provider（親が shell 等で子に使えないときは通らない）
//  5. default   — ハードコード `codex`（3・4 とも使えないときの最後の受け皿）
//
// 3・4 を足した理由（2026-08-29）: 昇格したセッション（普通に始めたセッションが最初の
// spawn で指揮者になる経路）には対応表が無いので、以前はここが `codex` 決め打ちだった。
// Claude で作業している人が「レビューさせて」と言っただけで、黙って別契約の codex が
// 動き出す。親と同じなら少なくとも意外性が無い。役割ごとの記憶があれば、1 度だけ
// 承認ダイアログで直せば以後その役割はその provider になる（毎回聞かなくてよい）。
//
// custom provider の親（`plan_custom-provider-extension-triage.md` C1）は
// `validOrchestrationProvider` が常に偽になるため、上記の「親と同じ」を素通りして
// ステップ5（ハードコード `codex`）へ落ちる。既定の `spawn_confirm_mode` では起動前に
// 確認ダイアログを挟むため完全に無言ではないが、ダイアログ自体は選定理由までは示さず、
// `spawn_confirm_mode: off` の環境では本当に無言になる。せめてログには残す
// （呼び出し側が source == providerSourceDefault のときに Info を出す）。
func resolveChildProvider(requested, roleAssigned, remembered, parentProvider string) (provider, source string) {
	if strings.TrimSpace(requested) != "" {
		return requested, providerSourceExplicit
	}
	if strings.TrimSpace(roleAssigned) != "" {
		return roleAssigned, providerSourceRoleMap
	}
	if validOrchestrationProvider(remembered) {
		return remembered, providerSourceRemembered
	}
	if validOrchestrationProvider(parentProvider) {
		return parentProvider, providerSourceParent
	}
	return "codex", providerSourceDefault
}

// resolveSpawnChildProvider は handleSpawnChild の provider/model 解決ステップを1呼び出しへ
// まとめる（C2）。役割対応表の参照は provider と model で共有し、二重ルックアップを避ける。
// body を直接書き換え、確定理由（providerSource* 定数）を返す。ログ出力は呼び出し側で行う
// （この関数は Server の cfgMu 越しの読み取り以外に副作用を持たない）。
func (s *Server) resolveSpawnChildProvider(parent *session, body *spawnChildRequest) string {
	var roleAssignedProvider string
	if parent != nil {
		if roles := s.orchestrationRolesFor(parent.OrchestrationID); roles != nil {
			if ra, ok := roles[body.Role]; ok {
				roleAssignedProvider = ra.Provider
				if body.Model == "" {
					body.Model = ra.Model
				}
			}
		}
	}
	s.cfgMu.Lock()
	remembered := s.cfg.UserPrefs.Spawn.RoleProvider[body.Role]
	s.cfgMu.Unlock()
	parentProvider := ""
	if parent != nil {
		parentProvider = parent.Provider
	}
	provider, source := resolveChildProvider(body.Provider, roleAssignedProvider, remembered, parentProvider)
	body.Provider = provider
	return source
}

// resolveChildProviderFallback は resolveChildProvider の後方互換 wrapper。--provider の
// 明示指定も役割対応表も無い（handleSpawnChild 側で既に弾かれている）前提で、
// 記憶 → 親 → ハードコード `codex` の3段だけを返す。
func (s *Server) resolveChildProviderFallback(parent *session, role string) string {
	s.cfgMu.Lock()
	remembered := s.cfg.UserPrefs.Spawn.RoleProvider[role]
	s.cfgMu.Unlock()
	parentProvider := ""
	if parent != nil {
		parentProvider = parent.Provider
	}
	provider, source := resolveChildProvider("", "", remembered, parentProvider)
	if source == providerSourceDefault {
		s.logger.Info("orchestration child provider fallback used hardcoded default",
			"role", role, "parent_provider", parentProvider, "fallback_provider", "codex")
	}
	return provider
}

// providerConfirmationChangeNote は、承認ダイアログの決定が確認前の provider と違うときだけ
// board へ残す1行を組み立てる（C2 item 2）。一致するときは空文字（＝書かない）。
func providerConfirmationChangeNote(preConfirmProvider, decidedProvider string) string {
	if preConfirmProvider == decidedProvider {
		return ""
	}
	return fmt.Sprintf("provider changed at confirmation: requested=%s decided=%s", preConfirmProvider, decidedProvider)
}

// rememberRoleProvider は実際に起動した provider を役割ごとに覚える。
// 承認ダイアログで書き換えられた後の値がここへ来るので、「1 度直せば次から効く」が成立する。
// 保存に失敗しても spawn は成功扱いのままにする（記憶は利便性であって正しさではない）。
func (s *Server) rememberRoleProvider(role, provider string) {
	role = strings.TrimSpace(role)
	if role == "" || !validOrchestrationProvider(provider) {
		return
	}
	s.cfgMu.Lock()
	if s.cfg.UserPrefs.Spawn.RoleProvider == nil {
		s.cfg.UserPrefs.Spawn.RoleProvider = map[string]string{}
	}
	if s.cfg.UserPrefs.Spawn.RoleProvider[role] == provider {
		s.cfgMu.Unlock()
		return
	}
	s.cfg.UserPrefs.Spawn.RoleProvider[role] = provider
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		s.logger.Warn("remember role provider failed", "role", role, "provider", provider, "err", err)
	}
}

// resolveChildSubscription は子セッションのサブスクリプション profile を決める。
//
// 以前は AI 経由の spawn で常に空になり、複数契約を使い分けている利用者でも子は必ず
// 「CLI 既定のログイン」で動いていた（`orchestrate spawn` に指定手段が無いため）。
// role 別の profile 指定があればそれを優先し、無ければ親と同じ provider の profile を
// そのまま引き継ぐ。親の profile も無ければ、新規セッションパネルがその provider 用に
// 覚えている値（`subscription_<provider>`）を使う。どれも無ければ従来どおり空＝CLI
// 既定のログイン。
//
// 存在しない ID を渡すと子は起動せずエラーになる（spawnChildRequest の doc 参照）ので、
// 黙って別アカウントへ倒れることはない。
func (s *Server) resolveChildSubscription(parent *session, provider, role string) string {
	if parent != nil {
		if roles := s.orchestrationRolesFor(parent.OrchestrationID); roles != nil {
			if ra, ok := roles[role]; ok && strings.TrimSpace(ra.Subscription) != "" {
				return ra.Subscription
			}
		}
	}
	if parent != nil && parent.Provider == provider && strings.TrimSpace(parent.SubscriptionProfileID) != "" {
		return parent.SubscriptionProfileID
	}
	s.cfgMu.Lock()
	saved := s.cfg.UserPrefs.Spawn.Defaults["subscription_"+provider]
	s.cfgMu.Unlock()
	return strings.TrimSpace(saved)
}

// orchestrationProviders は子セッションに使える provider の全部。**綴りは厳密**で、
// `Codex` も `ChatGPT` も通らない。一覧を定数として持つのは、弾いたときのエラーへ
// そのまま載せるため。AI は綴りを揺らす（Codex / codex / ChatGPT / GPT-5）ので、
// 「invalid provider」とだけ返すと正解に辿り着けず当てずっぽうを繰り返す。
var orchestrationProviders = []string{"claude", "codex", "copilot", "cursor-agent", "opencode", "grok", "command-code"}

func validOrchestrationProvider(provider string) bool {
	for _, p := range orchestrationProviders {
		if provider == p {
			return true
		}
	}
	return false
}

// invalidProviderDetail は弾いた値と有効な一覧を 1 文にする。
func invalidProviderDetail(provider string) string {
	return fmt.Sprintf("invalid provider %q; valid providers are: %s",
		provider, strings.Join(orchestrationProviders, ", "))
}
