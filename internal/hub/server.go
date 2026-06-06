package hub

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"hash/fnv"
	"io/fs"
	"log/slog"
	"net"
	"net/http"
	neturl "net/url"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	"any-ai-cli/internal/attach"
	"any-ai-cli/internal/config"
	"any-ai-cli/internal/proto"
	"any-ai-cli/internal/sessionlog"
	"any-ai-cli/internal/sessionstore"
	"any-ai-cli/internal/wrapper"
	"any-ai-cli/web"
	"golang.org/x/net/websocket"
)

// idleAfter: PTY 出力が静止してから running → standby/waiting に遷移するまでの時間。
// /workflows 等でバックグラウンドエージェントが動いている間は進捗ツリーの再描画が
// 数秒おきのバースト出力になるため、500ms では running↔standby が点滅する。
// 3s に延長してヒステリシスを持たせる（standby→running は markRunning で即時のまま。
// 承認検出 waiting は approvalVisible フラグで idleAfter を待たず即時遷移するため影響なし）。
// tickerInterval: 状態評価 ticker の間隔。
// maxPTYBuf: UI 再接続時リプレイ用の PTY バッファ上限（セッションごと）。
// uiPingInterval: UI WebSocket keepalive ping の送信間隔。
const (
	idleAfter                    = 3 * time.Second
	tickerInterval               = 200 * time.Millisecond
	maxPTYBuf                    = 512 * 1024 // 512 KB
	replayTailForNonActive       = 64 * 1024  // 64 KB: 非アクティブセッションの UI 接続時 replay 上限
	uiPingInterval               = 30 * time.Second
	branchLookupTimeout          = 250 * time.Millisecond
	branchRefreshAfter           = 2 * time.Second
	branchRefreshWorkers         = 4
	nativeApprovalClearMissLimit = 3
	nativeApprovalBlankLineLimit = 2
	vtResizeDebounce             = 200 * time.Millisecond
	approvalConsumedTTL          = 10 * time.Second
	// approvalVisibleLease: session_hint(approval_visible=true) の有効期限。
	// UI は承認可視中 5s 間隔（approval-ui.js の APPROVAL_HINT_REASSERT_MS）で
	// 再主張するため、リース 15s = 再主張 3 回分。リロード desync・複数クライアント・
	// H9 復元固着など false ヒントが失われるどの経路でも最大 15s で自動回復する。
	// ただし Hub 自身の go_vt detector が native prompt を見ている間
	// （nativeApprovalSig != ""）はリース切れでもクリアしない。
	approvalVisibleLease = 15 * time.Second
	wsMaxPayloadBytes    = 2 << 20 // 2 MiB: UI/wrapper JSON frame receive cap

	// OSC シーケンスをユーザーターン境界マーカーとして ptyBuf に注入する。
	// xterm.js はこのシーケンスを画面に表示しない。
	// 47777 は Hub のデフォルトポートを namespace として流用。
	chatHistoryUserTurnMarker = "\x1b]47777;user-turn\x07"
)

// session の State は次のいずれか:
//
//	"standby"   : wrapper 接続済み・出力静止 + 承認 UI 不可視
//	"running"   : 直近 idleAfter 以内に PTY 出力あり
//	"waiting"   : 出力静止 + 承認 UI 可視（UI からの session_hint で確定）
//	"completed" : プロセス正常終了
//	"error"     : プロセス異常終了
//	"disconnected" : wrapper WebSocket 切断
//
// standby/waiting の振り分けは「PTY バイトのみでは不可能」なため、UI が
// xterm.js のレンダリング済みバッファをスキャンして session_hint で approval_visible
// を伝える。Hub はそれを受けて idleAfter 経過時に倒す state を決める。
type session struct {
	ID           int    `json:"id"`
	Provider     string `json:"provider"`
	Display      string `json:"display_name"`
	CWD          string `json:"cwd"`
	Branch       string `json:"branch,omitempty"`
	Label        string `json:"label,omitempty"` // UI カード 3 行目に【ラベル】として表示
	Model        string `json:"model,omitempty"` // 使用モデル名; UI カード表示用
	Route        string `json:"route,omitempty"` // 接続経路（"ollama" 等）; UI で Ollama バックエンドの識別に使用
	Shell        string `json:"shell,omitempty"`
	State        string `json:"state"`
	LastOutputAt string `json:"last_output_at,omitempty"` // ISO 8601; UI カード「最終応答時刻」用
	StartedAt    string `json:"started_at,omitempty"`     // ISO 8601; UI カード「起動時刻」用
	FirstMessage string `json:"first_message,omitempty"`  // 最初の確定入力; UI カード表示用
	LastMessage  string `json:"last_message,omitempty"`   // 最新の確定入力; UI カード表示用
	EndReason    string `json:"end_reason,omitempty"`     // session_end の reason コード（例: "exec_not_found"）。UI 側で i18n 翻訳して表示

	// JSON 外: 状態評価用
	lastOutputAt      time.Time // idleAfter 計算用。LastOutputAt と同期して更新する
	approvalVisible   bool
	approvalVisibleAt time.Time // approvalVisible=true を最後に受信した時刻（approvalVisibleLease 判定用）
	branchCheckedAt   time.Time

	// JSON 外: UI 再接続時リプレイ用リングバッファ（末尾 maxPTYBuf bytes）
	ptyBuf []byte

	// JSON 外: Go 側 native approval 検出用 VT バッファ。
	vt                        *vtBuffer
	vtResizeDebounceUntil     time.Time
	nativeApprovalSig         string
	nativeApprovalTailSig     string
	nativeApprovalScanQueued  bool
	nativeApprovalClearMisses int
	nativeApprovalConsumed    string
	nativeApprovalConsumedAt  time.Time

	// JSON 外: wrapper に最後に送った PTY サイズ（同サイズの resize を skip して不要な SIGWINCH を防ぐ）
	lastCols int
	lastRows int

	// JSON 外: 起動バナーからの初期モデル検出用。
	// Model が空のセッションのみ対象。検出成功 or 累計バイト超過で打ち切る。
	initialModelScanBytes int
	initialModelScanDone  bool

	// JSON 外: セッション履歴（JSONL）
	StoreID   int64              `json:"-"`
	LogPath   string             `json:"log_path,omitempty"`
	JSONLPath string             `json:"jsonl_path,omitempty"`
	History   *sessionlog.Writer `json:"-"`
}

func (s *session) idleStateName() string {
	if s.approvalVisible {
		return "waiting"
	}
	return "standby"
}

// resolveRoute は provider + model から route を推定する。
// spawn API では body.Route が明示指定されるが、wrapper の register/reattach
// 経路には route 情報が無いため、ここで RouteForModel と同等の推定を行う。
func (s *Server) resolveRoute(provider, model string) string {
	if strings.TrimSpace(model) == "" {
		return ""
	}
	localCfg := s.snapshotLocalModels()
	known := collectOllamaModelIDs(s.modelsCache, localCfg)
	return RouteForModel(provider, model, known)
}

func gitBranch(cwd string) string {
	if strings.TrimSpace(cwd) == "" {
		return ""
	}
	ctx, cancel := context.WithTimeout(context.Background(), branchLookupTimeout)
	defer cancel()
	out, err := exec.CommandContext(ctx, "git", "-C", cwd, "rev-parse", "--abbrev-ref", "HEAD").Output()
	if err != nil {
		return ""
	}
	branch := strings.TrimSpace(string(out))
	if branch != "HEAD" {
		return branch
	}
	out, err = exec.CommandContext(ctx, "git", "-C", cwd, "rev-parse", "--short", "HEAD").Output()
	if err != nil {
		return ""
	}
	hash := strings.TrimSpace(string(out))
	if hash == "" {
		return ""
	}
	return "detached:" + hash
}

// uiConn wraps a single UI WebSocket connection and serialises all outbound
// frames via sendMu so concurrent goroutines (broadcast, pingLoop, sendSnapshot,
// history replay) never interleave partial frames on the same connection.
// closeOnce guarantees conn.Close is called at most once regardless of how
// many goroutines detect a dead connection simultaneously.
type uiConn struct {
	ws        *websocket.Conn
	sendMu    sync.Mutex
	closeOnce sync.Once
}

func newUIConn(ws *websocket.Conn) *uiConn { return &uiConn{ws: ws} }

func (c *uiConn) send(m any) error {
	c.sendMu.Lock()
	defer c.sendMu.Unlock()
	return websocket.JSON.Send(c.ws, m)
}

func (c *uiConn) close() {
	c.closeOnce.Do(func() { _ = c.ws.Close() })
}

// wrapperConn wraps a single wrapper WebSocket connection and serialises all
// outbound Hub-to-wrapper frames. UI input/resize forwarding and shutdown
// notices can be sent from different goroutines, so the raw websocket.Conn must
// not be written concurrently.
type wrapperConn struct {
	ws        *websocket.Conn
	sendMu    sync.Mutex
	closeOnce sync.Once
}

func newWrapperConn(ws *websocket.Conn) *wrapperConn { return &wrapperConn{ws: ws} }

func (c *wrapperConn) send(m any) error {
	c.sendMu.Lock()
	defer c.sendMu.Unlock()
	return websocket.JSON.Send(c.ws, m)
}

func (c *wrapperConn) sendWithDeadline(m any, deadline time.Time) error {
	c.sendMu.Lock()
	defer c.sendMu.Unlock()
	if err := c.ws.SetWriteDeadline(deadline); err != nil {
		return err
	}
	return websocket.JSON.Send(c.ws, m)
}

func (c *wrapperConn) close() {
	c.closeOnce.Do(func() { _ = c.ws.Close() })
}

type Server struct {
	cfg         *config.Config
	logger      *slog.Logger
	httpSrv     *http.Server
	devMode     bool   // --dev: web/ をファイルシステムから直接サーブ（再コンパイル不要）
	hubCWD      string // serve 起動時の os.Getwd() を保存
	version     string // main.version (ldflags 経由) を保持し /api/info で返す
	parentShell string
	instanceID  string // Hub プロセス起動ごとのランダム ID。UI が Hub 再起動（live session ID の振り直し）を検出するために snapshot に同梱する

	// sessionsMu guards session/connection state (nextID, sessions, wrappers,
	// uis, lastUICols/Rows, idleTimer, idleGen). cfgMu guards s.cfg.
	// Lock ordering: the two locks are never held simultaneously — snapshot cfg
	// (snapshotCfg / snapshotLocalModels / idleTimeoutMin) and release cfgMu
	// before taking sessionsMu.
	sessionsMu sync.Mutex
	cfgMu      sync.Mutex
	nextID     int
	sessions   map[int]*session
	wrappers   map[int]*wrapperConn
	uis        map[*websocket.Conn]*uiConn

	slashCmdMu    sync.Mutex
	slashCmdCache map[string]*slashCmdCacheEntry // key: provider

	approvalRulesMu     sync.Mutex
	approvalRuleTargets map[string]approvalRuleTarget // key: normalized path

	// netHint: launcher（SSH tunnel モード）が /api/net-hint で登録する接続元情報。
	// tunnel モードでは既起動の Hub に ANY_AI_CLI_HOST_LABEL を注入できないため、
	// API 経由でサーバ側に保持し、URL クエリヒントを持たないクライアント
	//（PWA・別タブ等）にも /api/info で正しいバッジ情報を返す。
	netHintMu   sync.Mutex
	netHintSSH  bool
	netHintHost string

	usageLinkCache *ttlCache[UsageLinkDefaults]

	modelsCache       *modelsCache
	modelsRemoteCache *ttlCache[modelsDefaults]
	sessionStore      *sessionstore.Store
	push              *pushManager
	logMaintenanceMu  sync.Mutex

	branchRefreshMu       sync.Mutex
	branchRefreshSem      chan struct{}
	branchRefreshInFlight map[string]struct{}

	lastUICols int
	lastUIRows int
	idleTimer  *time.Timer
	idleGen    uint64 // incremented on each startIdleTimerLocked / stopIdleTimerLocked to invalidate stale callbacks

	stopMu   sync.Mutex
	stopFunc context.CancelFunc

	autoOpenBrowser bool
}

type branchRefreshRequest struct {
	id  int
	cwd string
}

func (s *Server) currentHubPort() int {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return s.cfg.Hub.Port
}

// snapshotCfg returns a deep clone of the current config under cfgMu so callers
// can read a consistent snapshot without holding cfgMu during slow work.
func (s *Server) snapshotCfg() *config.Config {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return s.cfg.Clone()
}

// snapshotLocalModels returns a copy of the configured local models under cfgMu.
func (s *Server) snapshotLocalModels() []config.LocalModel {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return append([]config.LocalModel(nil), s.cfg.LocalModels...)
}

// idleTimeoutMin reads the configured idle-timeout minutes under cfgMu. Callers
// snapshot it before taking sessionsMu so the two locks are never nested.
func (s *Server) idleTimeoutMin() int {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return s.cfg.Hub.IdleTimeoutMin
}

const (
	defaultInitCols = 200
	defaultInitRows = 50
)

// reSetModelTo は Claude Code の /model コマンド出力からモデル名を抽出する。
// 例: "└  Set model to Haiku 4.5" → "Haiku 4.5"
var reSetModelTo = regexp.MustCompile(`Set model to ([^\r\n]+)`)

// reCodexModelChanged は Codex CLI の /model コマンド出力からモデル名を抽出する。
// 例: "• Model changed to gpt-5.5 medium" → "gpt-5.5 medium"
var reCodexModelChanged = regexp.MustCompile(`Model changed to ([^\r\n]+)`)

// 起動バナーからの初期モデル検出。--model 指定なしで起動したセッションでも
// カードにモデル名を出すため、VT バッファのレンダリング済み行をスキャンする
// （StripANSI したテキストはカーソル移動由来のスペースが落ちて使えない）。
//
// Claude Code: ロゴ 2 行目 "▝▜█████▛▘  Opus 4.8 (1M context) with medium effort · Claude Max"
//
//	→ ロゴの後ろを取り、" · <プラン>" と " with <x> effort" を落とす → "Opus 4.8 (1M context)"
//
// Codex CLI:  "│ model:       gpt-5.5 xhigh   /model to change │"
//
//	→ "loading"（初期表示）は除外
//
// Copilot CLI: 最下部ステータス行の右端に右寄せでモデル名
//
//	例: " ● Working ...   Claude Haiku 4.5" / "...   GPT-5 mini · low"
//	→ 3 個以上の空白で区切った最後のセグメント。" · <effort>" を落とし、
//	  モデル名らしさ（英字始まり + 数字を含む、または "Auto"）を検査する
//
// Cursor Agent: "<cwd> · <branch>" ステータス行の直上の非空行がモデル名
//
//	例: "  Auto" / 応答中は "  Auto · 7.4%"（context 使用率サフィックスを落とす）
const claudeBannerLogoRow2 = "▝▜█████▛▘"

var (
	reClaudeBannerEffort  = regexp.MustCompile(`\s+with\s+\S+\s+effort$`)
	reCodexBannerModel    = regexp.MustCompile(`model:\s+(.+?)\s+/model to change`)
	reCopilotStatusSplit  = regexp.MustCompile(`\s{3,}`)
	reCopilotEffortSuffix = regexp.MustCompile(`\s+·\s+(?:low|medium|high|xhigh)$`)
	reCopilotModelLike    = regexp.MustCompile(`^[A-Za-z][\w.\- ()]*\d`)
	reCursorPercentSuffix = regexp.MustCompile(`\s+·\s+\d+(?:\.\d+)?%$`)
)

// initialModelScanMaxBytes を超えても検出できなければ諦める（バナーは起動直後に出る）。
const initialModelScanMaxBytes = 256 * 1024

// initialModelScanProviders は起動バナー検出の対象 provider。
var initialModelScanProviders = map[string]bool{
	"claude":       true,
	"codex":        true,
	"copilot":      true,
	"cursor-agent": true,
}

var (
	modelChangeTokens = [][]byte{
		[]byte("Set model to "),
		[]byte("Model changed to "),
	}
	nativeApprovalTriggerTokens = [][]byte{
		[]byte("[ANY-AI-CLI]"),
		[]byte("approval"),
		[]byte("Approval"),
		[]byte("requires approval"),
		[]byte("Requires approval"),
		[]byte("requires permission"),
		[]byte("Requires permission"),
		[]byte("permission"),
		[]byte("Permission"),
		[]byte("confirm"),
		[]byte("Confirm"),
		[]byte("allow"),
		[]byte("Allow"),
		[]byte("deny"),
		[]byte("Deny"),
		[]byte("proceed"),
		[]byte("Proceed"),
		[]byte("cancel"),
		[]byte("Cancel"),
		[]byte("enter to select"),
		[]byte("Enter to select"),
		[]byte("esc to cancel"),
		[]byte("Esc to cancel"),
		[]byte("press enter"),
		[]byte("Press Enter"),
		[]byte("(y)"),
		[]byte("(n)"),
		[]byte("(esc)"),
		[]byte("Yes"),
		[]byte("No"),
	}
)

// newInstanceID は Hub プロセス起動ごとのランダム ID を生成する。
// 乱数取得に失敗した場合は起動時刻ナノ秒で代替する（識別できれば十分なため）。
func newInstanceID() string {
	b := make([]byte, 8)
	if _, err := rand.Read(b); err != nil {
		return strconv.FormatInt(time.Now().UnixNano(), 16)
	}
	return hex.EncodeToString(b)
}

func NewServer(cfg *config.Config, logger *slog.Logger, devMode bool, version string) (*Server, error) {
	hubCWD, _ := os.Getwd()
	s := &Server{
		cfg:                   cfg,
		logger:                logger,
		devMode:               devMode,
		hubCWD:                hubCWD,
		version:               version,
		instanceID:            newInstanceID(),
		parentShell:           wrapper.DetectShell(),
		sessions:              map[int]*session{},
		wrappers:              map[int]*wrapperConn{},
		uis:                   map[*websocket.Conn]*uiConn{},
		slashCmdCache:         map[string]*slashCmdCacheEntry{},
		approvalRuleTargets:   map[string]approvalRuleTarget{},
		usageLinkCache:        newUsageLinkCache(),
		modelsCache:           &modelsCache{},
		modelsRemoteCache:     newModelsRemoteCache(),
		branchRefreshSem:      make(chan struct{}, branchRefreshWorkers),
		branchRefreshInFlight: map[string]struct{}{},
	}
	if store, err := sessionstore.OpenForLogDir(cfg.Hub.LogDir); err != nil {
		logger.Warn("sqlite session store disabled", "err", err)
	} else {
		s.sessionStore = store
		// 前回 run がクラッシュ等で EndSession できずに残した未終了行を閉じる。
		// 放置すると live_session_id ベースの UPDATE（state / first・last message 等）が
		// 同じ live ID を再利用する新セッションの内容で旧行を上書きしてしまう。
		// 再接続猶予中の wrapper が reattach した場合は StartSession の upsert が
		// ended_at を NULL に戻して同じ行を継続利用する。
		if n, err := store.CloseStaleSessions(time.Now(), "hub_restart"); err != nil {
			logger.Warn("close stale session rows failed", "err", err)
		} else if n > 0 {
			logger.Info("closed stale session rows from previous run", "count", n)
		}
	}
	if devMode {
		logger.Info("dev mode: serving web assets from ./web/dist/")
	}
	if push, err := newPushManager(logger); err != nil {
		logger.Warn("web push disabled", "err", err)
	} else {
		s.push = push
	}
	var staticHandler http.Handler
	if devMode {
		staticHandler = http.FileServer(http.Dir(filepath.Join("web", "dist")))
	} else {
		subFS, err := fs.Sub(web.FS, "dist")
		if err != nil {
			return nil, err
		}
		staticHandler = http.FileServer(http.FS(subFS))
	}
	// 承認パターン JSON はユーザー設定ディレクトリ ~/.any-ai-cli/approval-patterns/
	// に保持する。フロント互換の /approval-patterns/*.json はユーザー設定を含むため、
	// 汎用 FileServer ではなく token 必須の専用ハンドラで配信する。
	if err := SyncApprovalPatterns(cfg.ApprovalProfiles); err != nil {
		logger.Warn("sync approval patterns failed", "err", err)
	}
	mux := http.NewServeMux()
	mux.HandleFunc("/", s.handleIndex)
	mux.Handle("/app-entry.js", staticHandler)
	mux.Handle("/app.js", staticHandler)
	mux.Handle("/app/", staticHandler)
	mux.Handle("/styles.css", staticHandler)
	mux.Handle("/styles/", staticHandler)
	mux.Handle("/icon.svg", staticHandler)
	mux.Handle("/icons/", staticHandler)
	mux.Handle("/manifest.webmanifest", staticHandler)
	mux.Handle("/sw.js", staticHandler)
	mux.Handle("/i18n.js", staticHandler)
	mux.Handle("/i18n/", staticHandler)
	mux.Handle("/vendor/", staticHandler)
	mux.HandleFunc("/approval-patterns/", s.handleApprovalPatternAsset)
	mux.Handle("/ws", websocket.Server{
		Handshake: s.wsHandshake,
		Handler:   s.handleWS,
	})
	mux.HandleFunc("/api/info", s.handleInfo)
	mux.HandleFunc("/api/net-hint", s.handleNetHint)
	mux.HandleFunc("/api/avatar", s.handleAvatar)
	mux.HandleFunc("/api/spawn", s.handleSpawn)
	mux.HandleFunc("/api/pick-directory", s.handlePickDirectory)
	mux.HandleFunc("/api/path-exists", s.handlePathExists)
	mux.HandleFunc("/api/pick-file", s.handlePickFile)
	mux.HandleFunc("/api/open-file", s.handleOpenFile)
	mux.HandleFunc("/api/open-default-file", s.handleOpenDefaultFile)
	mux.HandleFunc("/api/open-folder", s.handleOpenFolder)
	mux.HandleFunc("/api/open-terminal", s.handleOpenTerminal)
	mux.HandleFunc("/api/file-open-app", s.handleFileOpenApp)
	mux.HandleFunc("/api/terminal-app", s.handleTerminalApp)
	mux.HandleFunc("/api/kill-all", s.handleKillAll)
	mux.HandleFunc("/api/shutdown", s.handleShutdown)
	mux.HandleFunc("/api/log-config", s.handleLogConfig)
	mux.HandleFunc("/api/session-chat", s.handleSessionChat)
	mux.HandleFunc("/api/session-search", s.handleSessionSearch)
	mux.HandleFunc("/api/session-store/reset", s.handleSessionStoreReset)
	mux.HandleFunc("/api/open-dir", s.handleOpenDir)
	mux.HandleFunc("/api/idle-timeout", s.handleIdleTimeout)
	mux.HandleFunc("/api/reconnect-grace", s.handleReconnectGrace)
	mux.HandleFunc("/api/encoding-check", s.handleEncodingCheck)
	mux.HandleFunc("/api/approval/status", s.handleApprovalStatus)
	mux.HandleFunc("/api/approval/enable", s.handleApprovalEnable)
	mux.HandleFunc("/api/approval/disable", s.handleApprovalDisable)
	mux.HandleFunc("/api/approval/dismiss", s.handleApprovalDismiss)
	mux.HandleFunc("/api/attach", s.handleAttach)
	mux.HandleFunc("/api/slash-cmd-sources", s.handleSlashCmdSources)
	mux.HandleFunc("/api/slash-commands", s.handleSlashCommands)
	mux.HandleFunc("/api/usage-link-defaults", s.handleUsageLinkDefaults)
	mux.HandleFunc("/api/models", s.handleModels)
	mux.HandleFunc("/api/approval-patterns", s.handleApprovalPatterns)
	mux.HandleFunc("/api/approval-patterns/", s.handleApprovalPatternsItem)
	mux.HandleFunc("/api/files-list", s.handleFilesList)
	mux.HandleFunc("/api/files-content", s.handleFilesContent)
	mux.HandleFunc("/api/files-asset", s.handleFilesAsset)
	mux.HandleFunc("/api/files-roots", s.handleFilesRoots)
	mux.HandleFunc("/api/files-move", s.handleFilesMove)
	mux.HandleFunc("/api/files-rename", s.handleFilesRename)
	mux.HandleFunc("/api/files-mkdir", s.handleFilesMkdir)
	mux.HandleFunc("/api/files-save", s.handleFilesSave)
	mux.HandleFunc("/api/files-delete-dir", s.handleFilesDeleteDir)
	mux.HandleFunc("/api/git-log", s.handleGitLog)
	mux.HandleFunc("/api/git-show", s.handleGitShow)
	mux.HandleFunc("/api/git-refs", s.handleGitRefs)
	mux.HandleFunc("/api/git-status", s.handleGitStatus)
	mux.HandleFunc("/api/git-commit-all", s.handleGitCommitAll)
	mux.HandleFunc("/api/git-commit-message", s.handleGitCommitMessage)
	mux.HandleFunc("/api/git-fetch", s.handleGitFetch)
	mux.HandleFunc("/api/git-pull", s.handleGitPull)
	mux.HandleFunc("/api/user-prefs/notify-sound-custom", s.handleUserPrefsNotifySoundCustom)
	mux.HandleFunc("/api/user-prefs/avatar", s.handleUserPrefsAvatarUpload)
	mux.HandleFunc("/api/user-prefs", s.handleUserPrefs)
	mux.HandleFunc("/api/push/status", s.handlePushStatus)
	mux.HandleFunc("/api/push/vapid-public-key", s.handlePushVAPIDPublicKey)
	mux.HandleFunc("/api/push/subscriptions", s.handlePushSubscriptions)
	s.registerWorkbenchRoutes(mux)
	s.httpSrv = &http.Server{Addr: fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port), Handler: withSecurityHeaders(mux)}
	return s, nil
}

const contentSecurityPolicy = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' data: blob:; connect-src 'self' ws://127.0.0.1:* ws://localhost:*; font-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"

func withSecurityHeaders(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		setSecurityHeaders(w.Header())
		next.ServeHTTP(w, r)
	})
}

func setSecurityHeaders(h http.Header) {
	h.Set("Content-Security-Policy", contentSecurityPolicy)
	h.Set("X-Frame-Options", "DENY")
	h.Set("X-Content-Type-Options", "nosniff")
	h.Set("Referrer-Policy", "no-referrer")
	h.Set("Cross-Origin-Opener-Policy", "same-origin")
	h.Set("Permissions-Policy", "camera=(), geolocation=(), payment=(), usb=(), microphone=(self)")
	h.Set("Service-Worker-Allowed", "/")
}

func (s *Server) Run(ctx context.Context) error {
	runCtx, cancel := context.WithCancel(ctx)
	s.stopMu.Lock()
	s.stopFunc = cancel
	s.stopMu.Unlock()

	pidPath := filepath.Join(os.TempDir(), "any-ai-cli.pid")
	killStalePid(pidPath)

	// 設定ポートが使用中の場合（例: WSL 側 Hub が先に起動済み）は空きポートへ自動移行する。
	var ln net.Listener
	basePort := s.currentHubPort()
	boundPort := basePort
	for p := basePort; p < basePort+100; p++ {
		addr := fmt.Sprintf("127.0.0.1:%d", p)
		var e error
		ln, e = net.Listen("tcp", addr)
		if e == nil {
			boundPort = p
			if p != basePort {
				s.httpSrv.Addr = addr
				s.cfgMu.Lock()
				s.cfg.Hub.Port = p
				s.cfgMu.Unlock()
				s.logger.Info("preferred port in use, using alternative port", "from", basePort, "to", p)
			}
			break
		}
	}
	if ln == nil {
		return fmt.Errorf("no available port found in range %d-%d", basePort, basePort+99)
	}
	_ = os.WriteFile(pidPath, []byte(fmt.Sprintf("%d", os.Getpid())), 0o644)
	// 実際にバインドしたポートを hub-runtime.json へ記録する。ポート自動退避後も
	// 引数なし起動の IsRunning / OpenBrowserForConfig が本物の Hub を見つけられる
	//（設定ポートしか見ないと、退避中の Hub を見落として重複起動する）。
	if err := writeHubRuntime(boundPort); err != nil {
		s.logger.Warn("failed to write hub runtime file", "err", err)
	}
	// shutdown_wait ゴルーチン内の Remove は Serve が戻った直後にプロセスが
	// 終了すると実行されないことがある（競合）。PID ファイルが残ると次回 boot の
	// killStalePid が再利用 PID の無関係プロセスを kill しうるため、run() の
	// return で必ず消えるよう同期的にも削除する（二重削除は無害）。
	defer func() {
		_ = os.Remove(pidPath)
		// hub-runtime.json は自 PID 記録時のみ削除（新しい Hub が上書き済みなら
		// 残す）。強制終了の残骸は読み取り側の二重ガードで除外される。
		removeHubRuntimeIfPID(os.Getpid())
	}()
	setConsoleTitle("any-ai-cli [hub] - DO NOT CLOSE")
	setConsoleIcon()
	s.logger.Info("ANY-AI-CLI started", "url", fmt.Sprintf("http://%s/?token=%s", s.httpSrv.Addr, neturl.QueryEscape(s.cfg.Token)))
	fmt.Print(startupBanner(s.version, s.httpSrv.Addr, s.cfg.Token))
	if s.autoOpenBrowser {
		_ = s.OpenBrowser()
	}
	if s.approvalRulesEnabled() {
		s.injectApprovalRules()
	}
	s.safeGo("state_ticker", func() { s.stateTicker(runCtx) })
	s.safeGo("clean_attachments", s.cleanAttachments)
	s.safeGo("clean_spawn_logs", s.cleanSpawnLogs)
	s.safeGo("clean_session_logs", s.cleanSessionLogs)
	s.safeGo("recover_transcripts", s.recoverTranscripts)
	s.safeGo("approval_patterns_remote_sync", func() { s.approvalPatternsRemoteSync(runCtx) })
	s.safeGo("shutdown_wait", func() {
		<-runCtx.Done()
		if s.approvalRulesEnabled() {
			s.removeApprovalRules()
		}
		// Stop the Hub server without marking wrapper sessions as intentionally
		// disconnected. Closing the HTTP server drops WS connections after the
		// listener is gone, so wrappers treat this as Hub-down and enter their
		// reconnect grace period. Explicit session termination still goes through
		// /api/kill-all, dismiss, or idle-timeout.
		_ = s.httpSrv.Close()
		if s.sessionStore != nil {
			_ = s.sessionStore.Close()
		}
		_ = os.Remove(pidPath)
		removeHubRuntimeIfPID(os.Getpid())
	})
	err := s.httpSrv.Serve(ln)
	if errors.Is(err, http.ErrServerClosed) {
		return nil
	}
	return err
}

func (s *Server) OpenBrowser() error {
	return OpenBrowserForConfig(s.cfg)
}

// SetAutoOpenBrowser を true にすると Run() がバインド後にブラウザを自動で開く。
// ポートスキャンで実際のポートが確定してから開くため、引数なし起動や serve --open で使う。
func (s *Server) SetAutoOpenBrowser(v bool) {
	s.autoOpenBrowser = v
}

// OpenBrowserForConfig opens the browser to the Hub URL without needing a running Server.
// ポート自動退避後の Hub（hub-runtime.json に記録）にも正しい URL で繋がるよう、
// 検証済みの実ポートを優先する。確認できない場合は設定ポートにフォールバック。
func OpenBrowserForConfig(cfg *config.Config) error {
	port := cfg.Hub.Port
	if p, ok := runningHubPort(cfg); ok {
		port = p
	}
	url := localHubURL(port, "/", cfg.Token)
	return browserCommand(url).Start()
}

func (s *Server) handleIndex(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	var b []byte
	var err error
	if s.devMode {
		b, err = os.ReadFile(filepath.Join("web", "dist", "index.html"))
	} else {
		b, err = web.FS.ReadFile("dist/index.html")
	}
	if err != nil {
		http.Error(w, "asset missing", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store, must-revalidate")
	setSecurityHeaders(w.Header())
	_, _ = w.Write(b)
}

// wsHandshake は WebSocket ハンドシェイク時に Origin を検証する。
// 許可: http://127.0.0.1:<port> / http://localhost:<port> / Origin ヘッダ無し（ラッパー等 CLI 由来）。
// 不一致は handshake エラーで拒否する。
func (s *Server) wsHandshake(cfg *websocket.Config, req *http.Request) error {
	s.cfgMu.Lock()
	port := s.cfg.Hub.Port
	s.cfgMu.Unlock()
	if !isAllowedHubHost(req.Host, port) {
		return fmt.Errorf("host not allowed: %s", req.Host)
	}
	origin := req.Header.Get("Origin")
	if origin == "" {
		// CLI / ラッパー由来の接続は Origin を持たないため許可する。
		return nil
	}
	if isAllowedHubOrigin(origin, port) {
		return nil
	}
	return fmt.Errorf("origin not allowed: %s", origin)
}

func (s *Server) handleWS(conn *websocket.Conn) {
	defer conn.Close()
	limitWSReceive(conn)
	var m proto.Message
	if err := websocket.JSON.Receive(conn, &m); err != nil {
		return
	}
	if !validToken(m.Token, s.cfg.Token) {
		return
	}
	if m.Role == "ui" {
		if m.Cols > 0 && m.Rows > 0 {
			s.sessionsMu.Lock()
			s.lastUICols, s.lastUIRows = m.Cols, m.Rows
			s.sessionsMu.Unlock()
		}
		uc, historyItems := s.addUIWithHistory(conn, m.UIActiveSessionID)
		s.sendSnapshot(uc)
		for _, item := range historyItems {
			_ = uc.send(item)
		}
		ctx, cancel := context.WithCancel(context.Background())
		s.safeGo("ui_ping_loop", func() { s.pingLoop(ctx, uc) })
		s.uiLoop(conn)
		cancel()
		return
	}
	switch m.Type {
	case "register":
		s.wrapperLoop(conn, m)
	case "reattach":
		s.reattachLoop(conn, m)
	default:
		return
	}
}

func limitWSReceive(conn *websocket.Conn) {
	conn.MaxPayloadBytes = wsMaxPayloadBytes
}

func (s *Server) wrapperLoop(conn *websocket.Conn, reg proto.Message) {
	startedAt := time.Now()
	branch := gitBranch(reg.CWD)
	s.sessionsMu.Lock()
	s.nextID++
	id := s.nextID
	initCols, initRows := s.lastUICols, s.lastUIRows
	s.sessionsMu.Unlock()

	rawLogPath, jsonlPath := sessionlog.Paths(s.cfg.Hub.LogDir, sessionlog.Metadata{
		SessionID: id,
		Provider:  reg.Provider,
		CWD:       reg.CWD,
		StartedAt: startedAt,
	})
	s.cfgMu.Lock()
	jsonlMaxBytes := int64(s.cfg.Log.SessionMaxSizeMB) * 1024 * 1024
	s.cfgMu.Unlock()
	history, histErr := sessionlog.NewJSONLWriter(jsonlPath, jsonlMaxBytes)
	if histErr != nil {
		s.logger.Warn("session history create failed", "path", jsonlPath, "err", histErr)
	}

	regRoute := strings.TrimSpace(reg.Route)
	if regRoute == "" {
		regRoute = s.resolveRoute(reg.Provider, reg.Model)
	}
	var storeID int64
	if s.sessionStore != nil {
		var storeErr error
		storeID, storeErr = s.sessionStore.StartSession(sessionstore.SessionStart{
			LiveSessionID: id,
			Provider:      reg.Provider,
			Display:       reg.Display,
			CWD:           reg.CWD,
			Branch:        branch,
			Label:         reg.Label,
			Model:         reg.Model,
			Route:         regRoute,
			Shell:         reg.Shell,
			State:         "standby",
			StartedAt:     startedAt.Format(time.RFC3339),
			LogPath:       rawLogPath,
			JSONLPath:     jsonlPath,
		})
		if storeErr != nil {
			s.logger.Warn("sqlite session start failed", "session_id", id, "err", storeErr)
		}
	}
	s.sessionsMu.Lock()
	ses := &session{
		ID:              id,
		StoreID:         storeID,
		Provider:        reg.Provider,
		Display:         reg.Display,
		CWD:             reg.CWD,
		Branch:          branch,
		Label:           reg.Label,
		Model:           reg.Model,
		Route:           regRoute,
		Shell:           reg.Shell,
		State:           "standby",
		StartedAt:       startedAt.Format(time.RFC3339),
		branchCheckedAt: startedAt,
		LogPath:         rawLogPath,
		JSONLPath:       jsonlPath,
		History:         history,
	}
	s.sessions[id] = ses
	wc := newWrapperConn(conn)
	s.wrappers[id] = wc
	s.sessionsMu.Unlock()
	if initCols == 0 || initRows == 0 {
		// UIが未接続の場合はラッパーが報告した呼び出し元端末サイズを優先する
		if reg.Cols > 0 && reg.Rows > 0 {
			initCols, initRows = reg.Cols, reg.Rows
		} else {
			initCols, initRows = defaultInitCols, defaultInitRows
		}
	}
	s.sessionsMu.Lock()
	ses.lastCols, ses.lastRows = initCols, initRows
	ses.vt = newVTBuffer(initCols, initRows)
	s.sessionsMu.Unlock()
	if s.approvalRulesEnabled() {
		s.injectApprovalRules()
	}
	_ = wc.send(proto.Message{Type: "registered", SessionID: id, Cols: initCols, Rows: initRows, StartedAt: ses.StartedAt, LogPath: rawLogPath, JSONLPath: jsonlPath})
	s.logger.Info("session registered", "id", id, "provider", reg.Provider, "cwd", reg.CWD, "pid", reg.PID)
	s.broadcast(proto.Message{Type: "session_update", SessionID: id, Provider: reg.Provider, Display: reg.Display, CWD: reg.CWD, Branch: branch, Label: reg.Label, Model: reg.Model, Route: regRoute, Shell: reg.Shell, State: "standby", StartedAt: ses.StartedAt, LogPath: rawLogPath, JSONLPath: jsonlPath})
	s.writeHistory(id, map[string]any{
		"ts":         startedAt.Format(time.RFC3339),
		"type":       "session_start",
		"session_id": id,
		"provider":   reg.Provider,
		"cwd":        reg.CWD,
		"branch":     branch,
		"label":      reg.Label,
		"model":      reg.Model,
		"shell":      reg.Shell,
		"pid":        reg.PID,
	})
	s.wrapperMessageLoop(wc, id)
}

func (s *Server) reattachLoop(conn *websocket.Conn, req proto.Message) {
	if req.SessionID <= 0 {
		_ = websocket.JSON.Send(conn, proto.Message{Type: "reattach_reject", Reason: "invalid session_id"})
		return
	}
	startedAt := time.Now()
	startedAtText := req.StartedAt
	if startedAtText != "" {
		if parsed, err := time.Parse(time.RFC3339, startedAtText); err == nil {
			startedAt = parsed
		} else {
			startedAtText = startedAt.Format(time.RFC3339)
		}
	} else {
		startedAtText = startedAt.Format(time.RFC3339)
	}
	branch := gitBranch(req.CWD)

	replay, err := base64.StdEncoding.DecodeString(req.ReplayB64)
	if err != nil {
		_ = websocket.JSON.Send(conn, proto.Message{Type: "reattach_reject", SessionID: req.SessionID, Reason: "invalid replay_b64"})
		return
	}
	if len(replay) > maxPTYBuf {
		replay = replay[len(replay)-maxPTYBuf:]
	}

	rawLogPath, jsonlPath := req.LogPath, req.JSONLPath
	if rawLogPath == "" || jsonlPath == "" {
		rawLogPath, jsonlPath = sessionlog.Paths(s.cfg.Hub.LogDir, sessionlog.Metadata{
			SessionID: req.SessionID,
			Provider:  req.Provider,
			CWD:       req.CWD,
			StartedAt: startedAt,
		})
	}
	s.cfgMu.Lock()
	jsonlMaxBytesReattach := int64(s.cfg.Log.SessionMaxSizeMB) * 1024 * 1024
	s.cfgMu.Unlock()
	history, histErr := sessionlog.NewJSONLWriterAppend(jsonlPath, jsonlMaxBytesReattach)
	if histErr != nil {
		s.logger.Warn("session history append failed", "path", jsonlPath, "err", histErr)
	}

	reqRoute := strings.TrimSpace(req.Route)
	if reqRoute == "" {
		reqRoute = s.resolveRoute(req.Provider, req.Model)
	}
	var storeID int64
	if s.sessionStore != nil {
		var storeErr error
		storeID, storeErr = s.sessionStore.StartSession(sessionstore.SessionStart{
			LiveSessionID: req.SessionID,
			Provider:      req.Provider,
			Display:       req.Display,
			CWD:           req.CWD,
			Branch:        branch,
			Label:         req.Label,
			Model:         req.Model,
			Route:         reqRoute,
			Shell:         req.Shell,
			State:         "running",
			StartedAt:     startedAtText,
			LogPath:       rawLogPath,
			JSONLPath:     jsonlPath,
		})
		if storeErr != nil {
			s.logger.Warn("sqlite session reattach failed", "session_id", req.SessionID, "err", storeErr)
		}
	}
	s.sessionsMu.Lock()
	acceptedID := req.SessionID
	if s.wrappers[acceptedID] != nil {
		s.nextID++
		acceptedID = s.nextID
	}
	if acceptedID != req.SessionID && s.sessionStore != nil {
		var storeErr error
		storeID, storeErr = s.sessionStore.StartSession(sessionstore.SessionStart{
			LiveSessionID: acceptedID,
			Provider:      req.Provider,
			Display:       req.Display,
			CWD:           req.CWD,
			Branch:        branch,
			Label:         req.Label,
			Model:         req.Model,
			Route:         reqRoute,
			Shell:         req.Shell,
			State:         "running",
			StartedAt:     startedAtText,
			LogPath:       rawLogPath,
			JSONLPath:     jsonlPath,
		})
		if storeErr != nil {
			s.logger.Warn("sqlite session reattach renumber failed", "session_id", acceptedID, "err", storeErr)
		}
	}
	var oldHistory *sessionlog.Writer
	if cur := s.sessions[acceptedID]; cur != nil {
		oldHistory = cur.History
	}
	now := time.Now()
	lastOutputAt := ""
	var lastOutputAtTime time.Time
	if len(replay) > 0 {
		lastOutputAtTime = now
		lastOutputAt = now.Format(time.RFC3339)
	}
	s.sessions[acceptedID] = &session{
		ID:              acceptedID,
		StoreID:         storeID,
		Provider:        req.Provider,
		Display:         req.Display,
		CWD:             req.CWD,
		Branch:          branch,
		Label:           req.Label,
		Model:           req.Model,
		Route:           reqRoute,
		Shell:           req.Shell,
		State:           "running",
		LastOutputAt:    lastOutputAt,
		StartedAt:       startedAtText,
		lastOutputAt:    lastOutputAtTime,
		branchCheckedAt: now,
		ptyBuf:          replay,
		vt:              newVTBuffer(req.Cols, req.Rows),
		lastCols:        req.Cols,
		lastRows:        req.Rows,
		LogPath:         rawLogPath,
		JSONLPath:       jsonlPath,
		History:         history,
	}
	if s.sessions[acceptedID].vt != nil && len(replay) > 0 {
		s.sessions[acceptedID].vt.Write(replay)
	}
	wc := newWrapperConn(conn)
	s.wrappers[acceptedID] = wc
	if s.nextID < acceptedID {
		s.nextID = acceptedID
	}
	s.sessionsMu.Unlock()
	if oldHistory != nil {
		_ = oldHistory.Close()
	}
	if s.approvalRulesEnabled() {
		s.injectApprovalRules()
	}
	_ = wc.send(proto.Message{Type: "reattach_ack", SessionID: acceptedID})
	s.broadcast(proto.Message{Type: "session_update", SessionID: acceptedID, Provider: req.Provider, Display: req.Display, CWD: req.CWD, Branch: branch, Label: req.Label, Model: req.Model, Route: reqRoute, Shell: req.Shell, State: "running", LastOutputAt: lastOutputAt, StartedAt: startedAtText, LogPath: rawLogPath, JSONLPath: jsonlPath})
	s.writeHistory(acceptedID, map[string]any{
		"ts":             now.Format(time.RFC3339),
		"type":           "session_reattach",
		"session_id":     acceptedID,
		"old_session_id": req.SessionID,
		"provider":       req.Provider,
		"cwd":            req.CWD,
		"branch":         branch,
		"label":          req.Label,
		"model":          req.Model,
		"shell":          req.Shell,
		"pid":            req.PID,
		"renumbered":     acceptedID != req.SessionID,
	})
	s.wrapperMessageLoop(wc, acceptedID)
}

func (s *Server) wrapperMessageLoop(wc *wrapperConn, id int) {
	for {
		var m proto.Message
		if err := websocket.JSON.Receive(wc.ws, &m); err != nil {
			s.logger.Debug("wrapper WS closed", "session_id", id, "err", err)
			break
		}
		m.SessionID = id
		switch m.Type {
		case "pty_data":
			now := time.Now()
			maskedRaw := sessionlog.MaskSecrets(string(m.Data))
			cleanText := sessionlog.StripANSI(maskedRaw)
			s.writeHistory(id, map[string]any{
				"ts":         now.Format(time.RFC3339),
				"type":       "pty_output",
				"session_id": id,
				"data_b64":   sessionlog.EncodeBase64([]byte(maskedRaw)),
				"text":       cleanText,
			})
			var provider string
			var vtLines []string
			var initialModelLines []string
			var initialModelCWD string
			scanNativeApproval := false
			hadNativeApprovalSig := false
			chunkHasApprovalTrigger := ptyChunkContainsAny(m.Data, nativeApprovalTriggerTokens)
			s.sessionsMu.Lock()
			if ses := s.sessions[id]; ses != nil {
				ses.ptyBuf = appendPTYReplay(ses.ptyBuf, m.Data)
				if ses.vt == nil {
					ses.vt = newVTBuffer(ses.lastCols, ses.lastRows)
				}
				ses.vt.Write(m.Data)
				provider = ses.Provider
				if chunkHasApprovalTrigger && now.Before(ses.vtResizeDebounceUntil) {
					ses.nativeApprovalScanQueued = true
				}
				shouldCheckApproval := provider != "" &&
					now.After(ses.vtResizeDebounceUntil) &&
					(chunkHasApprovalTrigger || ses.nativeApprovalSig != "" || ses.nativeApprovalScanQueued)
				if shouldCheckApproval {
					hadNativeApprovalSig = ses.nativeApprovalSig != ""
					vtLines = ses.vt.TailLines(vtTailLinesForApproval)
					tailSig := nativeApprovalTailSignature(vtLines)
					if tailSig != ses.nativeApprovalTailSig {
						ses.nativeApprovalTailSig = tailSig
						scanNativeApproval = true
					}
					ses.nativeApprovalScanQueued = false
				}
				// 起動バナーからの初期モデル検出（--model 指定なしのセッション向け）。
				// Model が埋まる・上限バイト超過のどちらかで打ち切る。
				if !ses.initialModelScanDone && ses.Model == "" && initialModelScanProviders[provider] {
					ses.initialModelScanBytes += len(m.Data)
					if ses.initialModelScanBytes > initialModelScanMaxBytes {
						ses.initialModelScanDone = true
					} else {
						initialModelCWD = ses.CWD
						initialModelLines = ses.vt.Lines()
					}
				}
			}
			s.sessionsMu.Unlock()
			s.broadcast(m)
			if scanNativeApproval {
				approval := detectNativeApproval(provider, vtLines)
				if approval == nil && hadNativeApprovalSig && shouldSuppressNativeApprovalClearMiss(provider, vtLines) {
					s.resetNativeApprovalClearMisses(id)
				} else {
					s.handleNativeApprovalDetection(id, approval)
				}
			}
			s.markRunning(id)
			s.detectModelChange(id, m.Data, cleanText)
			if initialModelLines != nil {
				s.detectInitialModel(id, provider, initialModelCWD, initialModelLines)
			}
		case "session_end":
			histEvent := map[string]any{
				"ts":         time.Now().Format(time.RFC3339),
				"type":       "session_end",
				"session_id": id,
				"state":      m.State,
				"exit_code":  m.ExitCode,
			}
			if m.Reason != "" {
				histEvent["reason"] = m.Reason
			}
			s.writeHistory(id, histEvent)
			if m.State == "completed" || m.State == "error" {
				s.sessionsMu.Lock()
				if cur := s.sessions[id]; cur != nil {
					cur.State = m.State
					if m.Reason != "" {
						cur.EndReason = m.Reason
					}
				}
				s.sessionsMu.Unlock()
			}
		}
	}

	// wrapper 切断
	s.sessionsMu.Lock()
	if s.wrappers[id] == wc {
		delete(s.wrappers, id)
	}
	var historyToClose *sessionlog.Writer
	var jsonlPathForTranscript string
	var endedProvider, endedCWD string
	if cur := s.sessions[id]; cur != nil && cur.State != "completed" && cur.State != "error" {
		cur.State = "disconnected"
	}
	endState := "disconnected"
	endReason := ""
	if cur := s.sessions[id]; cur != nil {
		endState = cur.State
		historyToClose = cur.History
		cur.History = nil
		jsonlPathForTranscript = cur.JSONLPath
		endReason = cur.EndReason
		endedProvider = cur.Provider
		endedCWD = cur.CWD
	}
	s.sessionsMu.Unlock()
	if endState == "disconnected" {
		if historyToClose != nil {
			ev := map[string]any{
				"ts":         time.Now().Format(time.RFC3339),
				"type":       "session_end",
				"session_id": id,
				"state":      endState,
				"exit_code":  0,
			}
			if endReason != "" {
				ev["reason"] = endReason
			}
			_ = historyToClose.Event(ev)
		}
	}
	if historyToClose != nil {
		_ = historyToClose.Close()
	}
	if s.sessionStore != nil {
		s.sessionStore.EndSession(id, endState, endReason, time.Now())
	}
	s.removeInactiveApprovalRules(providerApprovalRuleTargets(endedProvider, endedCWD))
	s.finalizeTranscript(id, jsonlPathForTranscript)
	s.broadcast(proto.Message{Type: "session_end", SessionID: id, State: endState, Reason: endReason})
}

func (s *Server) resetNativeApprovalClearMisses(id int) {
	s.sessionsMu.Lock()
	if ses := s.sessions[id]; ses != nil {
		ses.nativeApprovalClearMisses = 0
	}
	s.sessionsMu.Unlock()
}

func (s *Server) handleNativeApprovalDetection(id int, approval *nativeApproval) {
	now := time.Now()
	var msg *proto.Message
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	if now.Before(ses.vtResizeDebounceUntil) {
		s.sessionsMu.Unlock()
		return
	}
	if approval == nil {
		if ses.nativeApprovalSig != "" {
			ses.nativeApprovalClearMisses++
			if ses.nativeApprovalClearMisses >= nativeApprovalClearMissLimit {
				sig := ses.nativeApprovalSig
				ses.nativeApprovalSig = ""
				ses.nativeApprovalClearMisses = 0
				msg = &proto.Message{
					Type:           "approval_cleared",
					SessionID:      id,
					Provider:       ses.Provider,
					ApprovalSig:    sig,
					ApprovalSource: approvalSourceGoVT,
				}
			}
		}
		s.sessionsMu.Unlock()
		if msg != nil {
			s.broadcast(*msg)
		}
		return
	}
	if ses.nativeApprovalConsumed == approval.Sig && now.Sub(ses.nativeApprovalConsumedAt) < approvalConsumedTTL {
		s.sessionsMu.Unlock()
		return
	}
	ses.nativeApprovalClearMisses = 0
	if ses.nativeApprovalSig != approval.Sig {
		ses.nativeApprovalSig = approval.Sig
		msg = &proto.Message{
			Type:             "approval_detected",
			SessionID:        id,
			Provider:         ses.Provider,
			ApprovalSig:      approval.Sig,
			ApprovalKind:     approval.Kind,
			ApprovalSource:   approvalSourceGoVT,
			ApprovalQuestion: approval.Question,
			ApprovalContext:  approval.Context,
			ApprovalOptions:  approval.Options,
			DetectedAt:       now.Format(time.RFC3339),
		}
	}
	s.sessionsMu.Unlock()
	if msg != nil {
		if s.sessionStore != nil && msg.Type == "approval_detected" {
			s.sessionStore.StoreApprovalDetected(id, approval.Sig, approvalSourceGoVT, approval.Kind, approval.Question, approval.Context, approval.Options, now)
		}
		s.broadcast(*msg)
		if msg.Type == "approval_detected" {
			s.notifyApprovalPush(id, approval.Sig, msg.Provider, approval.Question, approval.Context)
		}
	}
}

func (s *Server) markNativeApprovalConsumed(m proto.Message) {
	if m.SessionID <= 0 || m.ApprovalSig == "" {
		return
	}
	now := time.Now()
	var clearMsg *proto.Message
	s.sessionsMu.Lock()
	ses := s.sessions[m.SessionID]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	ses.nativeApprovalConsumed = m.ApprovalSig
	ses.nativeApprovalConsumedAt = now
	if ses.nativeApprovalSig == m.ApprovalSig {
		ses.nativeApprovalSig = ""
		ses.nativeApprovalClearMisses = 0
		clearMsg = &proto.Message{
			Type:           "approval_cleared",
			SessionID:      m.SessionID,
			Provider:       ses.Provider,
			ApprovalSig:    m.ApprovalSig,
			ApprovalSource: approvalSourceGoVT,
		}
	}
	s.sessionsMu.Unlock()
	if s.sessionStore != nil {
		s.sessionStore.StoreApprovalConsumed(m.SessionID, m.ApprovalSig, m.SentText, now)
	}
	if clearMsg != nil {
		s.broadcast(*clearMsg)
	}
}

func appendPTYReplay(buf, data []byte) []byte {
	if len(data) == 0 {
		return buf
	}
	if cap(buf) > maxPTYBuf {
		if len(buf) > maxPTYBuf {
			buf = buf[len(buf)-maxPTYBuf:]
		}
		compact := make([]byte, len(buf), maxPTYBuf)
		copy(compact, buf)
		buf = compact
	}
	if len(data) >= maxPTYBuf {
		if cap(buf) < maxPTYBuf {
			buf = make([]byte, maxPTYBuf)
		} else {
			buf = buf[:maxPTYBuf]
		}
		copy(buf, data[len(data)-maxPTYBuf:])
		return buf
	}
	if len(buf)+len(data) <= maxPTYBuf {
		return append(buf, data...)
	}
	keep := maxPTYBuf - len(data)
	if keep > 0 {
		copy(buf, buf[len(buf)-keep:])
		buf = buf[:keep]
	} else {
		buf = buf[:0]
	}
	return append(buf, data...)
}

func ptyChunkContainsAny(data []byte, tokens [][]byte) bool {
	for _, token := range tokens {
		if bytes.Contains(data, token) {
			return true
		}
	}
	return false
}

func nativeApprovalTailSignature(lines []string) string {
	h := fnv.New64a()
	for _, line := range lines {
		_, _ = h.Write([]byte(line))
		_, _ = h.Write([]byte{0})
	}
	return strconv.FormatUint(h.Sum64(), 16)
}

func shouldSuppressNativeApprovalClearMiss(provider string, lines []string) bool {
	if !providerSupportsShortcutApproval(provider) {
		return false
	}
	nonEmpty := 0
	for _, line := range lines {
		if strings.TrimSpace(line) == "" {
			continue
		}
		nonEmpty++
		if nonEmpty > nativeApprovalBlankLineLimit {
			return false
		}
	}
	return true
}

// detectModelChange は PTY 出力からモデル変更を検出し、
// セッションの Model フィールドを更新して UI に session_update を送る。
// Claude Code の "Set model to <name>" / Codex CLI の "Model changed to <name>" を対象とする。
func (s *Server) detectModelChange(id int, data []byte, cleanText string) {
	if !ptyChunkContainsAny(data, modelChangeTokens) {
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	provider := ses.Provider
	s.sessionsMu.Unlock()

	var match []string
	switch provider {
	case "claude":
		match = reSetModelTo.FindStringSubmatch(cleanText)
	case "codex":
		match = reCodexModelChanged.FindStringSubmatch(cleanText)
	default:
		return
	}
	if match == nil {
		return
	}
	newModel := strings.TrimSpace(match[1])
	if newModel == "" {
		return
	}
	s.applyDetectedModel(id, provider, newModel, false)
}

// detectInitialModel は VT バッファのレンダリング済み行から起動バナーの
// モデル名を抽出し、Model が空のセッションに反映する。
// /model 変更（detectModelChange）と違い既存値は上書きしない。
func (s *Server) detectInitialModel(id int, provider, cwd string, vtLines []string) {
	model := extractBannerModel(provider, cwd, vtLines)
	if model == "" {
		return
	}
	s.applyDetectedModel(id, provider, model, true)
}

// extractBannerModel は起動バナー / ステータス行からモデル名を抽出する
// （見つからなければ ""）。cwd は cursor-agent のステータス行アンカーに使う。
func extractBannerModel(provider, cwd string, lines []string) string {
	switch provider {
	case "claude":
		for _, line := range lines {
			idx := strings.Index(line, claudeBannerLogoRow2)
			if idx < 0 {
				continue
			}
			rest := strings.TrimSpace(line[idx+len(claudeBannerLogoRow2):])
			// " · Claude Max" 等のプラン表記を落とす
			if before, _, found := strings.Cut(rest, "·"); found {
				rest = strings.TrimSpace(before)
			}
			rest = reClaudeBannerEffort.ReplaceAllString(rest, "")
			if rest != "" {
				return rest
			}
		}
	case "codex":
		for _, line := range lines {
			m := reCodexBannerModel.FindStringSubmatch(line)
			if m == nil {
				continue
			}
			model := strings.TrimSpace(m[1])
			if model != "" && !strings.EqualFold(model, "loading") {
				return model
			}
		}
	case "copilot":
		// 最下部の非空行（ステータス行）の右端セグメントを候補にする。
		// ラベルが無いため、モデル名らしさの検査で誤検出を防ぐ。
		for i := len(lines) - 1; i >= 0; i-- {
			line := strings.TrimSpace(lines[i])
			if line == "" {
				continue
			}
			segs := reCopilotStatusSplit.Split(line, -1)
			seg := strings.TrimSpace(segs[len(segs)-1])
			seg = reCopilotEffortSuffix.ReplaceAllString(seg, "")
			if seg == "Auto" || (len(seg) <= 40 && reCopilotModelLike.MatchString(seg)) {
				return seg
			}
			return "" // 最下部の非空行のみ見る（それより上はステータス行ではない）
		}
	case "cursor-agent":
		if cwd == "" {
			return ""
		}
		// "<cwd> · <branch>" 行を探し、その直上の非空行をモデル名とみなす。
		for i, line := range lines {
			t := strings.TrimSpace(line)
			if !strings.HasPrefix(t, cwd+" · ") && t != cwd {
				continue
			}
			for j := i - 1; j >= 0; j-- {
				above := strings.TrimSpace(lines[j])
				if above == "" {
					continue
				}
				above = reCursorPercentSuffix.ReplaceAllString(above, "")
				// プロンプト残骸（"→ ..." 等）は除外
				if above != "" && !strings.ContainsAny(above, "→❯") && len(above) <= 60 {
					return above
				}
				break
			}
		}
	}
	return ""
}

// applyDetectedModel はセッションの Model / Route を更新して session_update を
// broadcast する。onlyIfEmpty=true のときは Model 未設定のセッションのみ更新する
// （起動バナー検出が /model 変更や --model 指定を上書きしないため）。
func (s *Server) applyDetectedModel(id int, provider, newModel string, onlyIfEmpty bool) {
	newRoute := s.resolveRoute(provider, newModel)
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses.Model == newModel || (onlyIfEmpty && ses.Model != "") {
		s.sessionsMu.Unlock()
		return
	}
	ses.Model = newModel
	ses.Route = newRoute
	ses.initialModelScanDone = true
	update := proto.Message{
		Type:         "session_update",
		SessionID:    id,
		Provider:     ses.Provider,
		Display:      ses.Display,
		CWD:          ses.CWD,
		Branch:       ses.Branch,
		Label:        ses.Label,
		Model:        ses.Model,
		Route:        ses.Route,
		State:        ses.State,
		LastOutputAt: ses.LastOutputAt,
		FirstMessage: ses.FirstMessage,
		LastMessage:  ses.LastMessage,
	}
	s.sessionsMu.Unlock()
	s.broadcast(update)
}

func (s *Server) uiLoop(conn *websocket.Conn) {
	for {
		var m proto.Message
		if err := websocket.JSON.Receive(conn, &m); err != nil {
			s.logger.Info("ui WS closed", "err", err)
			s.removeUI(conn)
			return
		}
		switch m.Type {
		case "pty_resize":
			s.handleResize(m)
		case "pty_input":
			s.handleInput(m)
		case "session_hint":
			s.handleHint(m)
		case "approval_consumed":
			s.handleConsumed(m)
		case "session_history_reset":
			if skip := s.handleHistoryReset(m); skip {
				continue
			}
		case "session_dismiss":
			if skip := s.handleDismiss(m); skip {
				continue
			}
		case "attach_request":
			if skip := s.handleAttachRequest(m); skip {
				continue
			}
		}
	}
}

// handleResize は pty_resize メッセージを処理する。
// UI 側の端末サイズ変更を受け、セッションの VT バッファをリサイズして wrapper へ転送する。
func (s *Server) handleResize(m proto.Message) {
	if m.Cols <= 0 || m.Rows <= 0 {
		return
	}
	s.sessionsMu.Lock()
	s.lastUICols, s.lastUIRows = m.Cols, m.Rows
	ses := s.sessions[m.SessionID]
	skip := ses != nil && ses.lastCols == m.Cols && ses.lastRows == m.Rows
	if ses != nil && !skip {
		ses.lastCols, ses.lastRows = m.Cols, m.Rows
		if ses.vt == nil {
			ses.vt = newVTBuffer(m.Cols, m.Rows)
		} else {
			ses.vt.Resize(m.Cols, m.Rows)
		}
		ses.vtResizeDebounceUntil = time.Now().Add(vtResizeDebounce)
	}
	wc := s.wrappers[m.SessionID]
	s.sessionsMu.Unlock()
	if wc != nil && !skip {
		_ = wc.send(m)
	}
}

// handleInput は pty_input メッセージを処理する。
// UI から Text フィールドで受け取り、wrapper には Data ([]byte) に変換して転送する。
// Enter 確定時はセッション概要（FirstMessage/LastMessage）を更新し、
// ユーザーターン境界マーカーを ptyBuf に注入する。
func (s *Server) handleInput(m proto.Message) {
	s.sessionsMu.Lock()
	wc := s.wrappers[m.SessionID]
	ses := s.sessions[m.SessionID]
	combined := m.Text
	var firstMsgBroadcast *proto.Message
	var injectMarker bool
	if ses != nil && strings.HasSuffix(m.Text, "\r") {
		text := strings.TrimRight(m.Text, "\r\n")
		if text == "/clear" {
			// /clear でセッション概要をリセット（次の入力が新しい概要になる）
			ses.FirstMessage = ""
			ses.LastMessage = ""
			msg := proto.Message{Type: "session_update", SessionID: m.SessionID, Provider: ses.Provider, Display: ses.Display, CWD: ses.CWD, Branch: ses.Branch, Label: ses.Label, Model: ses.Model, Route: ses.Route, State: ses.State, LastOutputAt: ses.LastOutputAt}
			firstMsgBroadcast = &msg
		} else if text != "" {
			if ses.FirstMessage == "" {
				ses.FirstMessage = text
			}
			// 数字のみ（選択肢番号）は LastMessage を更新しない
			if !isDigitsOnly(text) {
				ses.LastMessage = text
			}
			msg := proto.Message{Type: "session_update", SessionID: m.SessionID, Provider: ses.Provider, Display: ses.Display, CWD: ses.CWD, Branch: ses.Branch, Label: ses.Label, Model: ses.Model, Route: ses.Route, State: ses.State, LastOutputAt: ses.LastOutputAt, FirstMessage: ses.FirstMessage, LastMessage: ses.LastMessage}
			firstMsgBroadcast = &msg
			// ユーザーターン境界マーカーを ptyBuf に注入する
			marker := []byte(chatHistoryUserTurnMarker)
			ses.ptyBuf = appendPTYReplay(ses.ptyBuf, marker)
			injectMarker = true
		}
	}
	s.sessionsMu.Unlock()
	if injectMarker {
		s.broadcast(proto.Message{Type: "pty_data", SessionID: m.SessionID, Data: []byte(chatHistoryUserTurnMarker)})
	}
	if wc != nil {
		_ = wc.send(proto.Message{Type: "pty_input", SessionID: m.SessionID, Data: []byte(combined)})
	}
	s.writeHistory(m.SessionID, map[string]any{
		"ts":         time.Now().Format(time.RFC3339),
		"type":       "user_input",
		"session_id": m.SessionID,
		"text":       m.Text,
	})
	if firstMsgBroadcast != nil {
		s.broadcast(*firstMsgBroadcast)
	}
}

// handleHint は session_hint メッセージを処理する。
// UI が xterm.js バッファをスキャンして判定した approval_visible を受け取り、
// セッションの approvalVisible フィールドを更新する。
func (s *Server) handleHint(m proto.Message) {
	s.sessionsMu.Lock()
	ses := s.sessions[m.SessionID]
	if ses != nil {
		ses.approvalVisible = m.ApprovalVisible
		if m.ApprovalVisible {
			ses.approvalVisibleAt = time.Now()
		} else {
			ses.approvalVisibleAt = time.Time{}
		}
	}
	s.sessionsMu.Unlock()
}

// handleConsumed は approval_consumed メッセージを処理する。
func (s *Server) handleConsumed(m proto.Message) {
	s.markNativeApprovalConsumed(m)
}

// handleHistoryReset は session_history_reset メッセージを処理する。
// 戻り値が true の場合、呼び出し元の uiLoop は当該ターンを continue する。
func (s *Server) handleHistoryReset(m proto.Message) (skip bool) {
	s.sessionsMu.Lock()
	ids := make([]int, 0, 1)
	updates := make([]proto.Message, 0, 1)
	resetOne := func(id int, ses *session) {
		if ses == nil {
			return
		}
		ses.ptyBuf = nil
		ses.FirstMessage = ""
		ses.LastMessage = ""
		if ses.vt != nil {
			ses.vt.Reset()
		}
		ses.nativeApprovalSig = ""
		ses.nativeApprovalTailSig = ""
		ses.nativeApprovalScanQueued = false
		ses.nativeApprovalClearMisses = 0
		ses.nativeApprovalConsumed = ""
		ses.nativeApprovalConsumedAt = time.Time{}
		ids = append(ids, id)
		updates = append(updates, proto.Message{Type: "session_update", SessionID: id, Provider: ses.Provider, Display: ses.Display, CWD: ses.CWD, Branch: ses.Branch, Label: ses.Label, Model: ses.Model, Route: ses.Route, State: ses.State, LastOutputAt: ses.LastOutputAt, StartedAt: ses.StartedAt})
	}
	if m.SessionID > 0 {
		resetOne(m.SessionID, s.sessions[m.SessionID])
	} else {
		for id, ses := range s.sessions {
			resetOne(id, ses)
		}
	}
	s.sessionsMu.Unlock()
	for _, id := range ids {
		if s.sessionStore != nil {
			s.sessionStore.ClearSessionHistory(id)
		}
		s.writeHistory(id, map[string]any{
			"ts":         time.Now().Format(time.RFC3339),
			"type":       "session_history_reset",
			"session_id": id,
		})
	}
	if m.SessionID > 0 && len(ids) == 0 {
		return true
	}
	s.broadcast(proto.Message{Type: "session_history_reset", SessionID: m.SessionID})
	for _, update := range updates {
		s.broadcast(update)
	}
	return false
}

// handleDismiss は session_dismiss メッセージを処理する。
// セッションを削除し、JSONL を閉じてトランスクリプトを生成する。
// 戻り値が true の場合、呼び出し元の uiLoop は当該ターンを continue する。
func (s *Server) handleDismiss(m proto.Message) (skip bool) {
	s.sessionsMu.Lock()
	wc := s.wrappers[m.SessionID]
	_, exists := s.sessions[m.SessionID]
	var historyToClose *sessionlog.Writer
	var jsonlPathForTranscript string
	var endedProvider, endedCWD string
	if exists {
		ses := s.sessions[m.SessionID]
		historyToClose = ses.History
		jsonlPathForTranscript = ses.JSONLPath
		endedProvider = ses.Provider
		endedCWD = ses.CWD
		ses.History = nil
		delete(s.sessions, m.SessionID)
		delete(s.wrappers, m.SessionID)
	}
	s.sessionsMu.Unlock()
	if !exists {
		return true
	}
	if historyToClose != nil {
		_ = historyToClose.Event(map[string]any{
			"ts":         time.Now().Format(time.RFC3339),
			"type":       "session_dismiss",
			"session_id": m.SessionID,
		})
	}
	if s.sessionStore != nil {
		_ = s.sessionStore.StoreEvent(m.SessionID, map[string]any{
			"ts":         time.Now().Format(time.RFC3339),
			"type":       "session_dismiss",
			"session_id": m.SessionID,
		})
		s.sessionStore.EndSession(m.SessionID, "dismissed", "", time.Now())
	}
	if wc != nil {
		wc.close()
	}
	if historyToClose != nil {
		_ = historyToClose.Close()
	}
	s.removeInactiveApprovalRules(providerApprovalRuleTargets(endedProvider, endedCWD))
	s.finalizeTranscript(m.SessionID, jsonlPathForTranscript)
	s.broadcast(proto.Message{Type: "session_removed", SessionID: m.SessionID})
	return false
}

// handleAttachRequest は attach_request メッセージを処理する。
// base64 デコード、ファイル保存、履歴記録を行う。
// 戻り値が true の場合、呼び出し元の uiLoop は当該ターンを continue する。
func (s *Server) handleAttachRequest(m proto.Message) (skip bool) {
	if m.ImageData == "" {
		s.logger.Warn("attach_request: missing image_data", "session_id", m.SessionID)
		return true
	}
	imgData, err := base64.StdEncoding.DecodeString(m.ImageData)
	if err != nil {
		s.logger.Warn("attach_request: failed to decode base64", "session_id", m.SessionID, "err", err)
		return true
	}

	// セッション情報（provider）を mutex 保護で取得
	s.sessionsMu.Lock()
	var provider string
	if ses := s.sessions[m.SessionID]; ses != nil {
		provider = ses.Provider
	}
	s.sessionsMu.Unlock()

	// attachments ディレクトリは ~/.any-ai-cli/attachments
	attachDir, err := attachmentsDir()
	if err != nil {
		s.logger.Warn("attach_request: os.UserHomeDir failed", "err", err)
		return true
	}

	savedPath, _, err := attach.Save(attachDir, m.SessionID, provider, imgData, m.Filename)
	if err != nil {
		s.logger.Warn("attach_request: Save failed", "session_id", m.SessionID, "err", err)
		return true
	}
	s.logger.Info("attach saved", "session_id", m.SessionID, "path", savedPath)
	s.writeHistory(m.SessionID, map[string]any{
		"ts":         time.Now().Format(time.RFC3339),
		"type":       "attach",
		"session_id": m.SessionID,
		"path":       savedPath,
		"filename":   m.Filename,
		"provider":   provider,
	})
	return false
}

// markRunning は PTY 出力受信時に呼ばれ、状態を running に更新して
// lastOutputAt を現在時刻に進める。状態遷移があった場合のみ broadcast する。
// approvalVisible=true の間は running への強制遷移を行わない（カーソルブリンク等の
// 継続的な PTY データで "待機中" 判定が阻害されるのを防ぐ）。
func (s *Server) markRunning(id int) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	now := time.Now()
	ses.lastOutputAt = now
	ses.LastOutputAt = now.Format(time.RFC3339)
	if ses.approvalVisible {
		s.sessionsMu.Unlock()
		return
	}
	changed := ses.State != "running"
	if changed {
		ses.State = "running"
	}
	provider, display, cwd, branch, label, model, route, lastOutputAt := ses.Provider, ses.Display, ses.CWD, ses.Branch, ses.Label, ses.Model, ses.Route, ses.LastOutputAt
	s.sessionsMu.Unlock()
	if changed {
		s.broadcast(proto.Message{Type: "session_update", SessionID: id, Provider: provider, Display: display, CWD: cwd, Branch: branch, Label: label, Model: model, Route: route, State: "running", LastOutputAt: lastOutputAt})
	}
	if s.sessionStore != nil {
		s.sessionStore.UpdateSessionState(id, "running", lastOutputAt)
	}
}

// stateTicker は idleAfter 経過後の running → waiting 遷移を担う。
func (s *Server) stateTicker(ctx context.Context) {
	defer func() {
		if r := recover(); r != nil {
			s.logger.Error("stateTicker panic recovered", "recover", fmt.Sprintf("%v", r))
		}
	}()
	t := time.NewTicker(tickerInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			s.evaluateIdle()
		}
	}
}

func (s *Server) evaluateIdle() {
	now := time.Now()
	s.sessionsMu.Lock()
	type change struct {
		id           int
		provider     string
		display      string
		cwd          string
		branch       string
		label        string
		model        string
		route        string
		state        string
		lastOutputAt string
		approvalWait bool
	}
	var changes []change
	var branchChecks []branchRefreshRequest
	for id, ses := range s.sessions {
		if now.Sub(ses.branchCheckedAt) >= branchRefreshAfter {
			ses.branchCheckedAt = now
			branchChecks = append(branchChecks, branchRefreshRequest{id: id, cwd: ses.CWD})
		}
		// approvalVisible リース切れの自動クリア:
		// UI からの false ヒントが失われても（リロード desync・複数クライアント等）
		// 再主張が止まれば waiting 固着から自動回復する。
		// go_vt detector がまだ native prompt を見ている間は UI 不在でも維持する。
		if ses.approvalVisible && ses.nativeApprovalSig == "" && now.Sub(ses.approvalVisibleAt) >= approvalVisibleLease {
			ses.approvalVisible = false
			ses.approvalVisibleAt = time.Time{}
		}
		var newState string
		switch ses.State {
		case "running":
			if ses.approvalVisible {
				// 承認UI表示中はアイドルタイマーを待たず即 waiting に遷移
				newState = "waiting"
			} else if !ses.lastOutputAt.IsZero() && now.Sub(ses.lastOutputAt) >= idleAfter {
				newState = ses.idleStateName()
			}
		case "waiting", "standby":
			// approvalVisible のフリップに追従（UI hint 反映）
			newState = ses.idleStateName()
		}
		if newState != "" && newState != ses.State {
			ses.State = newState
			changes = append(changes, change{id: id, provider: ses.Provider, display: ses.Display, cwd: ses.CWD, branch: ses.Branch, label: ses.Label, model: ses.Model, route: ses.Route, state: newState, lastOutputAt: ses.LastOutputAt, approvalWait: newState == "waiting" && ses.approvalVisible})
		}
	}
	s.sessionsMu.Unlock()
	for _, c := range changes {
		s.broadcast(proto.Message{Type: "session_update", SessionID: c.id, Provider: c.provider, Display: c.display, CWD: c.cwd, Branch: c.branch, Label: c.label, Model: c.model, Route: c.route, State: c.state, LastOutputAt: c.lastOutputAt})
		if s.sessionStore != nil {
			s.sessionStore.UpdateSessionState(c.id, c.state, c.lastOutputAt)
		}
		if c.approvalWait {
			s.notifyApprovalPush(c.id, fmt.Sprintf("ui-%d-%s", c.id, c.lastOutputAt), c.provider, "", "")
		}
	}
	s.queueBranchRefreshes(branchChecks)
}

func (s *Server) queueBranchRefreshes(checks []branchRefreshRequest) {
	if len(checks) == 0 {
		return
	}
	byCWD := make(map[string][]int, len(checks))
	for _, check := range checks {
		cwd := strings.TrimSpace(check.cwd)
		if cwd == "" {
			continue
		}
		byCWD[cwd] = append(byCWD[cwd], check.id)
	}
	if len(byCWD) == 0 {
		return
	}
	s.branchRefreshMu.Lock()
	if s.branchRefreshSem == nil {
		s.branchRefreshSem = make(chan struct{}, branchRefreshWorkers)
	}
	if s.branchRefreshInFlight == nil {
		s.branchRefreshInFlight = make(map[string]struct{})
	}
	for cwd, ids := range byCWD {
		if _, ok := s.branchRefreshInFlight[cwd]; ok {
			continue
		}
		s.branchRefreshInFlight[cwd] = struct{}{}
		cwd := cwd
		ids := append([]int(nil), ids...)
		sem := s.branchRefreshSem
		s.safeGo("branch refresh", func() {
			sem <- struct{}{}
			defer func() {
				<-sem
				s.branchRefreshMu.Lock()
				delete(s.branchRefreshInFlight, cwd)
				s.branchRefreshMu.Unlock()
			}()
			s.refreshBranchForCWD(cwd, ids)
		})
	}
	s.branchRefreshMu.Unlock()
}

func (s *Server) refreshBranchForCWD(cwd string, ids []int) {
	branch := gitBranch(cwd)
	msgs := make([]proto.Message, 0, len(ids))
	s.sessionsMu.Lock()
	for _, id := range ids {
		ses := s.sessions[id]
		if ses == nil || ses.CWD != cwd || ses.Branch == branch {
			continue
		}
		ses.Branch = branch
		msgs = append(msgs, proto.Message{
			Type:         "session_update",
			SessionID:    id,
			Provider:     ses.Provider,
			Display:      ses.Display,
			CWD:          ses.CWD,
			Branch:       ses.Branch,
			Label:        ses.Label,
			Model:        ses.Model,
			Route:        ses.Route,
			State:        ses.State,
			LastOutputAt: ses.LastOutputAt,
			StartedAt:    ses.StartedAt,
			FirstMessage: ses.FirstMessage,
			LastMessage:  ses.LastMessage,
		})
	}
	s.sessionsMu.Unlock()
	for _, msg := range msgs {
		s.broadcast(msg)
	}
}

// addUIWithHistory atomically registers c in the broadcast set and captures a
// snapshot of every session's ptyBuf at the same instant, then returns those
// snapshots as ready-to-send messages. Callers must send the returned messages
// to c after this call; any PTY data arriving after the lock is released is
// delivered via broadcast, so the snapshot and live stream do not overlap.
// activeSessionID は UI が現在表示中のセッション ID。このセッションは全量 replay し、
// 他は replayTailForNonActive バイトの tail のみ送信する（UI 接続時のメモリ・帯域削減）。
func (s *Server) addUIWithHistory(c *websocket.Conn, activeSessionID int) (*uiConn, []proto.Message) {
	var items []proto.Message
	s.sessionsMu.Lock()
	uc := newUIConn(c)
	s.uis[c] = uc
	s.stopIdleTimerLocked()
	for id, ses := range s.sessions {
		if len(ses.ptyBuf) == 0 {
			continue
		}
		raw := ses.ptyBuf
		if id != activeSessionID && len(raw) > replayTailForNonActive {
			tail := raw[len(raw)-replayTailForNonActive:]
			marker := []byte(chatHistoryUserTurnMarker)
			if !bytes.Contains(tail, marker) {
				// 64KB 末尾にマーカーがない場合、最後のマーカー位置まで遡って含める
				if lastIdx := bytes.LastIndex(raw, marker); lastIdx >= 0 {
					raw = raw[lastIdx:]
				} else {
					raw = tail
				}
			} else {
				raw = tail
			}
		}
		buf := make([]byte, len(raw))
		copy(buf, raw)
		items = append(items, proto.Message{Type: "pty_data", SessionID: id, Data: buf})
		ses.lastCols = 0
		ses.lastRows = 0
	}
	count := len(s.uis)
	s.sessionsMu.Unlock()
	s.logger.Info("UI connected", "ui_count", count, "active_session", activeSessionID)
	return uc, items
}

func (s *Server) removeUI(c *websocket.Conn) {
	idleMin := s.idleTimeoutMin()
	s.sessionsMu.Lock()
	uc, ok := s.uis[c]
	if !ok {
		s.sessionsMu.Unlock()
		return
	}
	delete(s.uis, c)
	count := len(s.uis)
	if count == 0 {
		s.startIdleTimerLocked(idleMin)
	}
	s.sessionsMu.Unlock()
	// Ensure the underlying TCP connection is closed so that any goroutine
	// blocked on Receive (e.g. uiLoop) unblocks and exits.
	uc.close()
	s.logger.Info("UI disconnected", "ui_count", count)
}

// startIdleTimerLocked starts the idle-timeout timer. Caller must hold
// sessionsMu. idleMin is the configured timeout, snapshotted via idleTimeoutMin
// before taking sessionsMu so cfgMu is never held under sessionsMu.
func (s *Server) startIdleTimerLocked(idleMin int) {
	if idleMin <= 0 || s.idleTimer != nil {
		return
	}
	s.idleGen++
	gen := s.idleGen
	d := time.Duration(idleMin) * time.Minute
	s.idleTimer = time.AfterFunc(d, func() {
		s.sessionsMu.Lock()
		if s.idleGen != gen {
			// A newer timer was started (UI reconnected) or the timer was
			// stopped; skip the kill to avoid evicting a just-reconnected UI.
			s.sessionsMu.Unlock()
			return
		}
		s.idleTimer = nil
		s.sessionsMu.Unlock()
		s.logger.Info("idle timeout reached, killing all wrappers", "minutes", idleMin)
		s.killAllWrappers()
	})
}

func (s *Server) stopIdleTimerLocked() {
	if s.idleTimer == nil {
		return
	}
	s.idleGen++ // invalidate any in-flight AfterFunc callback
	s.idleTimer.Stop()
	s.idleTimer = nil
}

// pingLoop は uiPingInterval ごとに UI WebSocket へ JSON ping を送り続ける。
// keepalive として機能し、dead connection を検出したら s.uis から除去して終了する。
func (s *Server) pingLoop(ctx context.Context, uc *uiConn) {
	t := time.NewTicker(uiPingInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			if err := uc.send(map[string]string{"type": "ping"}); err != nil {
				s.logger.Warn("ping failed, removing dead UI connection", "err", err)
				s.removeUI(uc.ws)
				return
			}
		}
	}
}

func (s *Server) sendSnapshot(uc *uiConn) {
	s.sessionsMu.Lock()
	list := make([]*session, 0, len(s.sessions))
	for _, ses := range s.sessions {
		list = append(list, ses)
	}
	s.sessionsMu.Unlock()
	b, _ := json.Marshal(list)
	// hub_instance: Hub 再起動を UI が検出するための起動毎 ID。
	// UI 側は前回値と異なる場合に live session ID キーのローカル状態
	// （チャット履歴・ターミナルバッファ等）を破棄してから snapshot を適用する。
	_ = uc.send(map[string]any{"type": "snapshot", "sessions": json.RawMessage(b), "hub_instance": s.instanceID})
}

func (s *Server) broadcast(m any) {
	s.sessionsMu.Lock()
	ucs := make([]*uiConn, 0, len(s.uis))
	for _, uc := range s.uis {
		ucs = append(ucs, uc)
	}
	s.sessionsMu.Unlock()
	for _, uc := range ucs {
		if err := uc.send(m); err != nil {
			s.logger.Warn("broadcast: UI send failed, removing dead connection", "err", err)
			s.removeUI(uc.ws)
		}
	}
}

// persistConfig takes a snapshot of s.cfg under cfgMu and saves it to disk
// outside the lock to avoid holding cfgMu during file I/O and to prevent
// concurrent map iteration/write panics in yaml.Marshal.
func (s *Server) persistConfig() error {
	return config.Save(s.snapshotCfg())
}

func isDigitsOnly(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}

func (s *Server) writeHistory(sessionID int, event map[string]any) {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	var w *sessionlog.Writer
	if ses != nil {
		w = ses.History
	}
	s.sessionsMu.Unlock()
	if w != nil {
		if err := w.Event(event); err != nil {
			s.logger.Warn("session history write failed", "session_id", sessionID, "err", err)
		}
	}
	if s.sessionStore != nil {
		if err := s.sessionStore.StoreEvent(sessionID, event); err != nil {
			s.logger.Warn("sqlite session event write failed", "session_id", sessionID, "err", err)
		}
	}
}
