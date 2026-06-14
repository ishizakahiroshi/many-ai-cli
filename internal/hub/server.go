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

	"many-ai-cli/internal/attach"
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/notify"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/sessionstore"
	"many-ai-cli/internal/wrapper"
	"many-ai-cli/web"
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
	maxPTYBuf                    = 2 * 1024 * 1024 // 2 MB: scrollback 拡大に合わせてアクティブセッションの replay を伸長
	replayTailForNonActive       = 64 * 1024       // 64 KB: 非アクティブセッションの UI 接続時 replay 上限
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

	bracketedPasteEnd         = "\x1b[201~"
	bracketedPasteSubmitDelay = 50 * time.Millisecond

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

	// JSON 外: git 変更統計（直近の refreshBranchForCWD で取得した値）
	gitChecked bool
	gitFiles   int
	gitAdded   int
	gitDeleted int

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

	// JSON 外: 完了サマリー通知の連投抑制用
	lastDoneNotifyAt time.Time

	// JSON 外: 起動バナーからの初期モデル検出用。
	// Model が空のセッションのみ対象。検出成功 or 累計バイト超過で打ち切る。
	initialModelScanBytes int
	initialModelScanDone  bool

	// JSON 外: セッション履歴（JSONL）
	StoreID   int64              `json:"-"`
	LogPath   string             `json:"log_path,omitempty"`
	JSONLPath string             `json:"jsonl_path,omitempty"`
	History   *sessionlog.Writer `json:"-"`

	// JSON 外: per-session 入力直列化ロック（#18）。
	// 複数 UI が同一セッションへ同時入力した場合に、hasPending チェック〜
	// trySendInput（50ms sleep を含む bracketd-paste 二段送信）が
	// sessionsMu 保持外で並行実行されると bracketed-paste 本文と確定 CR
	// が PTY 上でインターリーブする問題を防ぐ。
	// sessionsMu を 50ms sleep 中に保持しないよう、per-session の別ロックで分離する。
	// ロック順序: inputMu は sessionsMu の外側でのみ取得する
	//（sessionsMu 保持中に inputMu を取得しない）。
	inputMu sync.Mutex
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

// gitChangeStats は cwd の Git 変更統計を返す。
// files = git status --porcelain の非空行数（変更ファイル数）。
// added / deleted = git diff --numstat HEAD の集計値。
// いずれかのコマンドが失敗した場合は 0,0,0 を返す（git 未インストール / 非 git ディレクトリを含む）。
func gitChangeStats(cwd string) (files, added, deleted int) {
	if strings.TrimSpace(cwd) == "" {
		return 0, 0, 0
	}
	ctx, cancel := context.WithTimeout(context.Background(), branchLookupTimeout)
	defer cancel()

	// 変更ファイル数: git status --porcelain の非空行数
	statusOut, err := exec.CommandContext(ctx, "git", "-C", cwd, "status", "--porcelain").Output()
	if err != nil {
		return 0, 0, 0
	}
	for _, line := range strings.Split(string(statusOut), "\n") {
		if strings.TrimSpace(line) != "" {
			files++
		}
	}

	// 追加/削除行数: git diff --numstat HEAD
	numstatOut, err := exec.CommandContext(ctx, "git", "-C", cwd, "diff", "--numstat", "HEAD").Output()
	if err != nil {
		// HEAD が無い（初期コミット前）等のエラーは 0 として扱う
		return files, 0, 0
	}
	for _, line := range strings.Split(string(numstatOut), "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		parts := strings.SplitN(line, "\t", 3)
		if len(parts) < 2 {
			continue
		}
		// バイナリファイルは "-" になるので 0 扱い
		a, errA := strconv.Atoi(parts[0])
		d, errD := strconv.Atoi(parts[1])
		if errA == nil {
			added += a
		}
		if errD == nil {
			deleted += d
		}
	}
	return files, added, deleted
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

// broadcastWriteTimeout は UI WebSocket への JSON フレーム書き込みデッドライン。
// 受信側が詰まっている場合にサーバー全体がブロックされないための上限（finding #4）。
const broadcastWriteTimeout = 5 * time.Second

func newUIConn(ws *websocket.Conn) *uiConn { return &uiConn{ws: ws} }

func (c *uiConn) send(m any) error {
	c.sendMu.Lock()
	defer c.sendMu.Unlock()
	return websocket.JSON.Send(c.ws, m)
}

// sendWithDeadline は deadline までに JSON フレームを送信する（finding #4: 書き込みブロック防止）。
func (c *uiConn) sendWithDeadline(m any, deadline time.Time) error {
	c.sendMu.Lock()
	defer c.sendMu.Unlock()
	if err := c.ws.SetWriteDeadline(deadline); err != nil {
		return err
	}
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
	// pendingInput は wrapper 未接続・送信失敗で届けられなかったユーザー入力を
	// セッションごとに順序保持でバッファする。wrapper の (再)接続時に
	// flushPendingInput が順番に再送するため、入力が黙って失われない。
	// sessionsMu で保護。
	pendingInput map[int][]string

	slashCmdMu    sync.Mutex
	slashCmdCache map[string]*slashCmdCacheEntry // key: provider

	approvalRulesMu     sync.Mutex
	approvalRuleTargets map[string]approvalRuleTarget // key: normalized path

	// netHint: launcher（SSH tunnel モード）が /api/net-hint で登録する接続元情報。
	// tunnel モードでは既起動の Hub に MANY_AI_CLI_HOST_LABEL を注入できないため、
	// API 経由でサーバ側に保持し、URL クエリヒントを持たないクライアント
	//（PWA・別タブ等）にも /api/info で正しいバッジ情報を返す。
	netHintMu      sync.Mutex
	netHintSSH     bool
	netHintHost    string
	netHintEnvKind string

	usageLinkCache *ttlCache[UsageLinkDefaults]

	modelsCache       *modelsCache
	modelsRemoteCache *ttlCache[modelsDefaults]
	sessionStore      *sessionstore.Store
	push              *pushManager
	notifyMgr         *notify.Manager
	logMaintenanceMu  sync.Mutex
	whisperMu         sync.Mutex
	whisperInstall    whisperInstallState
	whisperCmd        *exec.Cmd
	whisperJob        whisperProcessJob
	whisperServerURL  string

	branchRefreshMu       sync.Mutex
	branchRefreshSem      chan struct{}
	branchRefreshInFlight map[string]struct{}

	lastUICols int
	lastUIRows int
	idleTimer  *time.Timer
	idleGen    uint64 // incremented on each startIdleTimerLocked / stopIdleTimerLocked to invalidate stale callbacks

	stopMu   sync.Mutex
	stopFunc context.CancelFunc

	// serverConns: 内蔵リモート接続マネージャ（SSH/WSL トンネルを Hub 子プロセス
	// として無窓で抱える）。servers.go 参照。
	serverConns *serverConnManager

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
	defaultInitCols   = 200
	defaultInitRows   = 50
	minUsableInitCols = 80
	minUsableInitRows = 20
)

func usableInitPTYSize(cols, rows int) (int, int, bool) {
	if cols < minUsableInitCols || rows < minUsableInitRows {
		return 0, 0, false
	}
	return cols, rows, true
}

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

const doneNotifyMinInterval = 60 * time.Second

var doneSummaryMarkerOpen = []byte("[MANY-AI-CLI-DONE]")
var doneSummaryMarkerClose = []byte("[/MANY-AI-CLI-DONE]")

var (
	modelChangeTokens = [][]byte{
		[]byte("Set model to "),
		[]byte("Model changed to "),
	}
	nativeApprovalTriggerTokens = [][]byte{
		[]byte("[MANY-AI-CLI]"),
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

func init() {
	// nativeApprovalJaTokens（approval_detector.go で定義）を
	// nativeApprovalTriggerTokens に追記する。
	// single source: 日本語ヒント語を approval_detector.go の 1 箇所で管理し、
	// PTY チャンクトリガー（ここ）と VT テール最終ゲート（nativeApprovalLooksValid）の
	// 両方に自動反映させる。
	for _, tok := range nativeApprovalJaTokens {
		nativeApprovalTriggerTokens = append(nativeApprovalTriggerTokens, []byte(tok))
	}
}

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
		pendingInput:          map[int][]string{},
		slashCmdCache:         map[string]*slashCmdCacheEntry{},
		approvalRuleTargets:   map[string]approvalRuleTarget{},
		usageLinkCache:        newUsageLinkCache(),
		modelsCache:           &modelsCache{},
		modelsRemoteCache:     newModelsRemoteCache(),
		branchRefreshSem:      make(chan struct{}, branchRefreshWorkers),
		branchRefreshInFlight: map[string]struct{}{},
		serverConns:           newServerConnManager(logger),
	}
	if store, err := sessionstore.OpenForLogDir(cfg.Hub.LogDir); err != nil {
		logger.Warn("sqlite session store disabled", "err", err)
	} else {
		s.sessionStore = store
		store.SetOnWriteError(func(liveSessionID int, err error) {
			logger.Warn("sqlite session event write failed", "session_id", liveSessionID, "err", err)
		})
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
	s.notifyMgr = notify.New(configToNotify(cfg.Notify), logger)
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
	// 承認パターン JSON はユーザー設定ディレクトリ ~/.many-ai-cli/approval-patterns/
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
	mux.Handle("/whisper-recorder-worklet.js", staticHandler)
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
	mux.HandleFunc("/api/spawn-grid", s.handleSpawnGrid)
	mux.HandleFunc("/api/pick-directory", s.handlePickDirectory)
	mux.HandleFunc("/api/path-exists", s.handlePathExists)
	mux.HandleFunc("/api/pick-file", s.handlePickFile)
	mux.HandleFunc("/api/open-default-file", s.handleOpenDefaultFile)
	mux.HandleFunc("/api/open-folder", s.handleOpenFolder)
	mux.HandleFunc("/api/open-terminal", s.handleOpenTerminal)
	mux.HandleFunc("/api/terminal-app", s.handleTerminalApp)
	mux.HandleFunc("/api/kill-all", s.handleKillAll)
	mux.HandleFunc("/api/shutdown", s.handleShutdown)
	mux.HandleFunc("/api/log-config", s.handleLogConfig)
	mux.HandleFunc("/api/session-chat", s.handleSessionChat)
	mux.HandleFunc("/api/session-log", s.handleSessionLog)
	mux.HandleFunc("/api/session-search", s.handleSessionSearch)
	mux.HandleFunc("/api/session-store/reset", s.handleSessionStoreReset)
	mux.HandleFunc("/api/logs/purge", s.handleLogsPurge)
	mux.HandleFunc("/api/logs/legacy-notice", s.handleLegacyLogsNotice)
	mux.HandleFunc("/api/attachments/purge", s.handleAttachmentsPurge)
	mux.HandleFunc("/api/open-dir", s.handleOpenDir)
	mux.HandleFunc("/api/idle-timeout", s.handleIdleTimeout)
	mux.HandleFunc("/api/reconnect-grace", s.handleReconnectGrace)
	mux.HandleFunc("/api/notify-config", s.handleNotifyConfig)
	mux.HandleFunc("/api/notify-test", s.handleNotifyTest)
	mux.HandleFunc("/api/notify-generate-topic", s.handleNotifyGenerateTopic)
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
	mux.HandleFunc("/api/files-download", s.handleFilesDownload)
	mux.HandleFunc("/api/files-roots", s.handleFilesRoots)
	mux.HandleFunc("/api/files-move", s.handleFilesMove)
	mux.HandleFunc("/api/files-rename", s.handleFilesRename)
	mux.HandleFunc("/api/files-mkdir", s.handleFilesMkdir)
	mux.HandleFunc("/api/files-create", s.handleFilesCreate)
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
	mux.HandleFunc("/api/git-push", s.handleGitPush)
	mux.HandleFunc("/api/user-prefs/notify-sound-custom", s.handleUserPrefsNotifySoundCustom)
	mux.HandleFunc("/api/user-prefs/avatar", s.handleUserPrefsAvatarUpload)
	mux.HandleFunc("/api/user-prefs", s.handleUserPrefs)
	mux.HandleFunc("/api/push/status", s.handlePushStatus)
	mux.HandleFunc("/api/push/vapid-public-key", s.handlePushVAPIDPublicKey)
	mux.HandleFunc("/api/push/subscriptions", s.handlePushSubscriptions)
	mux.HandleFunc("/api/voice/transcribe", s.handleVoiceTranscribe)
	mux.HandleFunc("/api/whisper/status", s.handleWhisperStatus)
	mux.HandleFunc("/api/whisper/install", s.handleWhisperInstall)
	mux.HandleFunc("/api/whisper/uninstall", s.handleWhisperUninstall)
	mux.HandleFunc("/api/whisper/start", s.handleWhisperStart)
	mux.HandleFunc("/api/whisper/stop", s.handleWhisperStop)
	mux.HandleFunc("/api/session-usage", s.handleSessionUsage)
	// 内蔵リモート接続（🖥 Server ボタン）。servers.go 参照。
	mux.HandleFunc("/api/servers", s.handleServers)
	mux.HandleFunc("/api/servers/connect", s.handleServerConnect)
	mux.HandleFunc("/api/servers/connect/status", s.handleServerConnectStatus)
	mux.HandleFunc("/api/servers/disconnect", s.handleServerDisconnect)
	s.registerWorkbenchRoutes(mux)
	s.httpSrv = &http.Server{
		Addr:    fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port),
		Handler: withSecurityHeaders(mux),
		// Slowloris 対策（gosec G112）。WS を長く張るため ReadTimeout は設定せず、
		// ヘッダ読み取りのみタイムアウトさせる。
		ReadHeaderTimeout: 10 * time.Second,
	}
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

	pidPath := filepath.Join(os.TempDir(), "many-ai-cli.pid")
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
	_ = os.WriteFile(pidPath, []byte(fmt.Sprintf("%d", os.Getpid())), 0o600)
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
	setConsoleTitle("many-ai-cli [hub] - DO NOT CLOSE")
	setConsoleIcon()
	s.logger.Info("MANY-AI-CLI started", "url", fmt.Sprintf("http://%s/?token=%s", s.httpSrv.Addr, neturl.QueryEscape(s.cfg.Token)))
	cfgSnapshot := s.snapshotCfg()
	fmt.Print(startupBanner(s.version, s.httpSrv.Addr, cfgSnapshot.Token, startupBannerAccess{
		AllowLoopbackWithoutToken: cfgSnapshot.Hub.AllowLoopbackWithoutToken,
		TrustedNetworks:           cfgSnapshot.Hub.TrustedNetworks,
		AllowedHosts:              cfgSnapshot.Hub.AllowedHosts,
	}))
	if s.autoOpenBrowser {
		_ = s.OpenBrowser()
	}
	if s.approvalRulesEnabled() {
		s.injectApprovalRules()
	}
	if s.tokenStatusbarEnabled() {
		s.injectUsageHooks()
	}
	s.safeGo("state_ticker", func() { s.stateTicker(runCtx) })
	s.safeGo("clean_attachments", s.cleanAttachments)
	s.safeGo("clean_spawn_logs", s.cleanSpawnLogs)
	s.safeGo("clean_session_logs", s.cleanSessionLogs)
	s.safeGo("maintenance_loop", func() { s.maintenanceLoop(runCtx) })
	s.safeGo("recover_transcripts", s.recoverTranscripts)
	s.safeGo("approval_patterns_remote_sync", func() { s.approvalPatternsRemoteSync(runCtx) })
	s.safeGo("shutdown_wait", func() {
		<-runCtx.Done()
		if s.approvalRulesEnabled() {
			s.removeApprovalRules()
		}
		s.removeAllUsageHooks()
		s.stopManagedWhisper()
		// Stop the Hub server without marking wrapper sessions as intentionally
		// disconnected. Closing the HTTP server drops WS connections after the
		// listener is gone, so wrappers treat this as Hub-down and enter their
		// reconnect grace period. Explicit session termination still goes through
		// /api/kill-all, dismiss, or idle-timeout.
		_ = s.httpSrv.Close()
		// 内蔵リモート接続の SSH/WSL 子プロセスを全て落とし、launcher-active.json の
		// 自 PID 分を掃除する（Hub 終了でトンネルも落ちるのが期待動作）。
		// httpSrv.Close() の後に呼ぶこと: 先に HTTP を閉じれば、shutdown 中に新規
		// /api/servers/connect が UnregisterAllForPID の後で接続を登録し、旧
		// watchConnection の UnregisterActiveConnection に巻き添えで消される競合を防げる。
		s.serverConns.closeAll()
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
	// UI が URL から token を除去した後のリロード（token なし GET /）でも
	// 認証が通るよう、HttpOnly cookie に token を保持させる。
	// SameSite=Strict によりクロスサイト送信されないため CSRF 経路にはならない。
	//
	// ただし Set-Cookie は「有効な token が実際に提示された」要求にのみ行う。
	// requireToken は allow_loopback_without_token 有効時に token 未提示の loopback
	// 要求も通すが、その経路（バイパスのみ通過した無 token 要求）に対して全権 token を
	// Set-Cookie で配ってしまうと、同一マシンの任意プロセスが生 HTTP 応答から実 token を
	// 採取できる。requestToken の実 token 一致を再評価し、真のときだけ Cookie を発行する。
	s.cfgMu.Lock()
	tok := s.cfg.Token
	s.cfgMu.Unlock()
	if tok != "" && validToken(requestToken(r), tok) {
		http.SetCookie(w, &http.Cookie{
			Name:     tokenCookieName,
			Value:    tok,
			Path:     "/",
			HttpOnly: true,
			SameSite: http.SameSiteStrictMode,
			// 永続セッション Cookie 化を避け、失効口を与える。起動毎 token と
			// 不一致になれば（Hub 再起動で token がローテートされた等）期限切れ後に
			// 再認証へ倒れる。MaxAge>0 なら Expires も併せて付与される。
			MaxAge: int(tokenCookieMaxAge / time.Second),
		})
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
	allowedHosts := append([]string(nil), s.cfg.Hub.AllowedHosts...)
	s.cfgMu.Unlock()
	if !isAllowedHubHost(req.Host, port, allowedHosts...) {
		return fmt.Errorf("host not allowed: %s", req.Host)
	}
	origin := req.Header.Get("Origin")
	if origin == "" {
		// CLI / ラッパー由来の接続は Origin を持たないため許可する。
		return nil
	}
	if isAllowedHubOrigin(origin, port, allowedHosts...) {
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
	req := conn.Request()
	remoteAddr := ""
	if req != nil {
		remoteAddr = req.RemoteAddr
	}
	if !s.validTokenOrTrustedRemote(m.Token, remoteAddr) {
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
	initCols, initRows, _ = usableInitPTYSize(initCols, initRows)
	s.sessionsMu.Unlock()

	rawLogPath, jsonlPath := sessionlog.Paths(s.cfg.Hub.LogDir, sessionlog.Metadata{
		SessionID: id,
		Provider:  reg.Provider,
		CWD:       reg.CWD,
		StartedAt: startedAt,
	})
	s.cfgMu.Lock()
	jsonlMaxBytes := int64(s.cfg.Log.SessionMaxSizeMB) * 1024 * 1024
	sessionLogEnabled := s.cfg.Log.SessionEnabled
	s.cfgMu.Unlock()
	// セッションログが無効（既定）なら .jsonl を作らない（空ファイルも残さない）。
	var history *sessionlog.Writer
	if sessionLogEnabled {
		var histErr error
		history, histErr = sessionlog.NewJSONLWriter(jsonlPath, jsonlMaxBytes)
		if histErr != nil {
			s.logger.Warn("session history create failed", "path", jsonlPath, "err", histErr)
		}
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
		if cols, rows, ok := usableInitPTYSize(reg.Cols, reg.Rows); ok {
			initCols, initRows = cols, rows
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
	if s.tokenStatusbarEnabled() {
		s.injectUsageHooks()
	}
	_ = wc.send(proto.Message{Type: "registered", SessionID: id, Cols: initCols, Rows: initRows, StartedAt: ses.StartedAt, LogPath: rawLogPath, JSONLPath: jsonlPath, TokenStatusbar: s.tokenStatusbarEnabled()})
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
	sessionLogEnabled := s.cfg.Log.SessionEnabled
	s.cfgMu.Unlock()
	// セッションログが無効（既定）なら .jsonl を作らない。
	var history *sessionlog.Writer
	if sessionLogEnabled {
		var histErr error
		history, histErr = sessionlog.NewJSONLWriterAppend(jsonlPath, jsonlMaxBytesReattach)
		if histErr != nil {
			s.logger.Warn("session history append failed", "path", jsonlPath, "err", histErr)
		}
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
	// wrapper が一時切断中に届かなかった保留入力を、再接続したこの wrapper へ順番に再送する。
	// 他のバックグラウンド goroutine と同様 safeGo で起動し、panic で Hub 全体を巻き込まないようにする。
	s.safeGo("flush_pending_input", func() { s.flushPendingInput(acceptedID) })
	if oldHistory != nil {
		_ = oldHistory.Close()
	}
	if s.approvalRulesEnabled() {
		s.injectApprovalRules()
	}
	if s.tokenStatusbarEnabled() {
		s.injectUsageHooks()
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
				shouldCheckApproval := isAIProvider(provider) &&
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
			if bytes.Contains(m.Data, doneSummaryMarkerOpen) {
				s.handleDoneSummaryMarker(id, m.Data)
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
	s.removeInactiveUsageHooks(endedProvider, endedCWD)
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
			s.notifyApprovalOutbound(id, approval.Sig, msg.Provider, approval.Question, approval.Context)
		}
	}
}

// handleDoneSummaryMarker は PTY データから [MANY-AI-CLI-DONE] マーカーを検出し、
// 完了サマリー通知を発火する。設定が OFF のセッション・連投抑制中はスキップ。
func (s *Server) handleDoneSummaryMarker(id int, data []byte) {
	open := bytes.Index(data, doneSummaryMarkerOpen)
	if open < 0 {
		return
	}
	start := open + len(doneSummaryMarkerOpen)
	closeIdx := bytes.Index(data[start:], doneSummaryMarkerClose)
	if closeIdx < 0 {
		return
	}
	summary := strings.TrimSpace(string(data[start : start+closeIdx]))
	if summary == "" {
		return
	}

	now := time.Now()
	s.cfgMu.Lock()
	doneSummaryEnabled := s.cfg.UserPrefs.DoneSummaryNotify.Enabled
	s.cfgMu.Unlock()
	if !doneSummaryEnabled {
		return
	}

	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	// Shell session は done summary push notification の対象外
	if !isAIProvider(ses.Provider) {
		s.sessionsMu.Unlock()
		return
	}
	if !ses.lastDoneNotifyAt.IsZero() && now.Sub(ses.lastDoneNotifyAt) < doneNotifyMinInterval {
		s.sessionsMu.Unlock()
		return
	}
	ses.lastDoneNotifyAt = now
	titleName := strings.TrimSpace(ses.Display)
	if titleName == "" {
		titleName = strings.TrimSpace(ses.Provider)
	}
	if titleName == "" {
		titleName = "many-ai-cli"
	}
	if ses.Label != "" {
		titleName = fmt.Sprintf("%s #%d [%s]", titleName, id, ses.Label)
	} else {
		titleName = fmt.Sprintf("%s #%d", titleName, id)
	}
	provider := ses.Provider
	s.sessionsMu.Unlock()

	s.notifyDoneOutbound(id, provider, titleName, summary)
	s.notifyDonePush(id, provider, titleName, summary)
}

// notifyDoneOutbound は ntfy/webhook バックエンドへのタスク完了通知を行う。
func (s *Server) notifyDoneOutbound(id int, provider, titleName, summary string) {
	if s.notifyMgr == nil {
		return
	}
	s.notifyMgr.SendDone(notify.DonePayload{
		SessionID: id,
		Provider:  provider,
		Title:     titleName,
		Summary:   summary,
	})
}

// notifyDonePush は Web Push でタスク完了通知を送信する。
// Web Push 経路は notifyApprovalPush を流用する（同じ Web Push チャンネルを使う）。
func (s *Server) notifyDonePush(id int, provider, titleName, summary string) {
	s.notifyApprovalPush(id, fmt.Sprintf("done-%d-%d", id, time.Now().UnixNano()), provider, summary, "")
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
			if s.handleHistoryReset(m) {
				continue
			}
		case "session_dismiss":
			if s.handleDismiss(m) {
				continue
			}
		case "attach_request":
			if s.handleAttachRequest(m) {
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
	if !skip {
		s.broadcast(proto.Message{Type: "pty_resize", SessionID: m.SessionID, Cols: m.Cols, Rows: m.Rows})
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
			maskedText := sessionlog.MaskSecrets(text)
			if ses.FirstMessage == "" {
				ses.FirstMessage = maskedText
			}
			// 数字のみ（選択肢番号）は LastMessage を更新しない
			if !isDigitsOnly(text) {
				ses.LastMessage = maskedText
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
	s.submitInput(wc, m.SessionID, combined)
	s.writeHistory(m.SessionID, map[string]any{
		"ts":         time.Now().Format(time.RFC3339),
		"type":       "user_input",
		"session_id": m.SessionID,
		"text":       sessionlog.MaskSecrets(m.Text),
	})
	if firstMsgBroadcast != nil {
		s.broadcast(*firstMsgBroadcast)
	}
}

func splitBracketedPasteSubmit(text string) (first string, delayed string) {
	if !strings.HasSuffix(text, bracketedPasteEnd+"\r") {
		return text, ""
	}
	return strings.TrimSuffix(text, "\r"), "\r"
}

// maxPendingInputPerSession は 1 セッションあたりの保留入力の上限。
// wrapper が長時間戻らないケースで無制限に溜まるのを防ぐ。超過時は古い方から捨てる。
const maxPendingInputPerSession = 100

// submitInput はユーザー入力を wrapper へ届ける。wrapper 未接続・送信失敗時は
// 入力を順序保持でバッファし、wrapper の (再)接続時に flushPendingInput が自動再送する
// （= 黙って捨てない）。既に保留中の入力があるセッションでは、新規入力を直送せず
// 末尾へ積んで順序を保つ。
//
// per-session inputMu (#18) により、複数 UI が同一セッションへ同時に入力しても
// hasPending チェック〜trySendInput（50ms sleep 含む bracketd-paste 二段送信）が
// 直列化され、bracketed-paste 本文と確定 CR のインターリーブが起きない。
// sessionsMu は inputMu の外側でのみ取得し、50ms sleep 中に保持しない。
func (s *Server) submitInput(wc *wrapperConn, sessionID int, combined string) {
	// session ポインタを短期間だけ sessionsMu で取得する。
	// session が既に削除済みの場合は nil になるので早期リターンする。
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	s.sessionsMu.Unlock()

	if ses == nil {
		// セッションが既に終了している場合は入力を捨てる（黙って失わない挙動は
		// 存在するセッションへの入力に限る）。
		return
	}

	// per-session 入力直列化ロック: hasPending チェック〜trySendInput 完了まで保持。
	// 複数 UI が同時にこの関数を呼んでも、同一 sessionID に対しては 1 件ずつ処理される。
	ses.inputMu.Lock()
	defer ses.inputMu.Unlock()

	s.sessionsMu.Lock()
	hasPending := len(s.pendingInput[sessionID]) > 0
	if hasPending {
		s.pendingInput[sessionID] = appendPendingInput(s.pendingInput[sessionID], combined)
	}
	s.sessionsMu.Unlock()
	if hasPending {
		s.notifyInputDeferred(sessionID)
		return
	}
	if rem := s.trySendInput(wc, sessionID, combined); rem != "" {
		s.sessionsMu.Lock()
		s.pendingInput[sessionID] = appendPendingInput(s.pendingInput[sessionID], rem)
		s.sessionsMu.Unlock()
		s.notifyInputDeferred(sessionID)
	}
}

// trySendInput は combined を wrapper へ送る。届けられなかった残り（未送信部分）を返す
// （"" = 全て送信済み）。bracketed-paste の確定 \r は別書き込み + 50ms 遅延で送る従来挙動を保つ。
// first まで送れて delayed(\r) だけ失敗した場合は \r のみを残りとして返し、本文の二重送信を避ける。
func (s *Server) trySendInput(wc *wrapperConn, sessionID int, combined string) (remaining string) {
	if wc == nil {
		s.logger.Warn("pty_input deferred: no wrapper connected", "session_id", sessionID)
		return combined
	}
	first, delayed := splitBracketedPasteSubmit(combined)
	if err := wc.send(proto.Message{Type: "pty_input", SessionID: sessionID, Data: []byte(first)}); err != nil {
		s.logger.Warn("pty_input deferred: send failed", "session_id", sessionID, "stage", "first", "err", err)
		return combined
	}
	if delayed != "" {
		time.Sleep(bracketedPasteSubmitDelay)
		if err := wc.send(proto.Message{Type: "pty_input", SessionID: sessionID, Data: []byte(delayed)}); err != nil {
			s.logger.Warn("pty_input deferred: send failed", "session_id", sessionID, "stage", "delayed", "err", err)
			return delayed
		}
	}
	return ""
}

// flushPendingInput は wrapper の (再)接続後に保留入力を順番に再送する。
// trySendInput が遅延 sleep しうるため goroutine で呼ぶ前提。再送に失敗した場合は
// 残りを先頭へ戻し、次の接続でリトライする。
// per-session inputMu (#18) を保持して実行するため、フラッシュ中に submitInput が
// 割り込んで入力順序が乱れることはない。
func (s *Server) flushPendingInput(sessionID int) {
	// session ポインタを短期間だけ sessionsMu で取得する。
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	s.sessionsMu.Unlock()
	if ses == nil {
		return
	}

	// per-session 入力直列化ロック: pending ドレイン中に submitInput が割り込まないよう保持。
	ses.inputMu.Lock()
	defer ses.inputMu.Unlock()

	s.sessionsMu.Lock()
	pending := s.pendingInput[sessionID]
	delete(s.pendingInput, sessionID)
	wc := s.wrappers[sessionID]
	s.sessionsMu.Unlock()
	if len(pending) == 0 {
		return
	}
	if wc == nil {
		s.requeuePendingInput(sessionID, pending)
		return
	}
	var remainder []string
	for i, combined := range pending {
		if rem := s.trySendInput(wc, sessionID, combined); rem != "" {
			remainder = append(remainder, rem)
			remainder = append(remainder, pending[i+1:]...)
			break
		}
	}
	if len(remainder) > 0 {
		s.requeuePendingInput(sessionID, remainder)
		return
	}
	s.logger.Info("flushed deferred pty_input", "session_id", sessionID, "count", len(pending))
}

// requeuePendingInput は再送できなかった残りを保留キューの先頭へ戻す
// （フラッシュ中に新規到着した入力は後ろに残す）。
func (s *Server) requeuePendingInput(sessionID int, queue []string) {
	s.sessionsMu.Lock()
	if existing := s.pendingInput[sessionID]; len(existing) > 0 {
		queue = append(queue, existing...)
	}
	if len(queue) > maxPendingInputPerSession {
		queue = queue[len(queue)-maxPendingInputPerSession:]
	}
	s.pendingInput[sessionID] = queue
	s.sessionsMu.Unlock()
}

// appendPendingInput は保留キューへ 1 件積み、上限超過分を古い方から捨てる。
func appendPendingInput(q []string, item string) []string {
	q = append(q, item)
	if len(q) > maxPendingInputPerSession {
		q = q[len(q)-maxPendingInputPerSession:]
	}
	return q
}

// notifyInputDeferred は UI へ「入力を保留した（wrapper 未接続/送信失敗）」を通知する。
func (s *Server) notifyInputDeferred(sessionID int) {
	s.broadcast(proto.Message{Type: "input_deferred", SessionID: sessionID})
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
		delete(s.pendingInput, m.SessionID)
	}
	s.sessionsMu.Unlock()
	if !exists {
		return true
	}
	// セッション破棄時に usageStat も解放する（メモリ無制限増加を防ぐ）。
	// usageStatsMu のロック順序のため sessionsMu 解放後に呼ぶ。
	DeleteSessionUsageStat(m.SessionID)
	if historyToClose != nil {
		_ = historyToClose.Event(map[string]any{
			"ts":         time.Now().Format(time.RFC3339),
			"type":       "session_dismiss",
			"session_id": m.SessionID,
		})
	}
	if s.sessionStore != nil {
		_ = s.sessionStore.StoreEventAsync(m.SessionID, map[string]any{
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

	// attachments ディレクトリは ~/.many-ai-cli/attachments
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
			approvalID := fmt.Sprintf("ui-%d-%s", c.id, c.lastOutputAt)
			s.notifyApprovalPush(c.id, approvalID, c.provider, "", "")
			s.notifyApprovalOutbound(c.id, approvalID, c.provider, "", "")
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
	gitFiles, gitAdded, gitDeleted := gitChangeStats(cwd)
	msgs := make([]proto.Message, 0, len(ids))
	s.sessionsMu.Lock()
	for _, id := range ids {
		ses := s.sessions[id]
		if ses == nil || ses.CWD != cwd {
			continue
		}
		branchChanged := ses.Branch != branch
		gitChanged := !ses.gitChecked || ses.gitFiles != gitFiles || ses.gitAdded != gitAdded || ses.gitDeleted != gitDeleted
		if !branchChanged && !gitChanged {
			continue
		}
		ses.Branch = branch
		ses.gitChecked = true
		ses.gitFiles = gitFiles
		ses.gitAdded = gitAdded
		ses.gitDeleted = gitDeleted
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
			GitChecked:   true,
			GitFiles:     gitFiles,
			GitAdded:     gitAdded,
			GitDeleted:   gitDeleted,
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
		// lastCols/lastRows はリセットしない。ここで 0 にすると attach 直後の
		// fit → pty_resize が handleResize の skip 判定を必ず通過し、サイズ未変更でも
		// PTY へ resize（SIGWINCH 相当）が届いて TUI が全画面再描画する。replay 済みの
		// 旧フレーム（フッター等）はスクロールバックに残るため二重描画になる。
		// lastCols は「PTY に最後に送った実サイズ」なので保持したままで正しく、
		// 新 UI のサイズが本当に異なる場合のみ resize が通る。
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
	sessionIDs := make([]int, 0, len(s.sessions))
	providerByID := make(map[int]string, len(s.sessions))
	for _, ses := range s.sessions {
		list = append(list, ses)
		sessionIDs = append(sessionIDs, ses.ID)
		providerByID[ses.ID] = ses.Provider
	}
	// json.Marshal は sessionsMu 保持下で行う。list は *session ポインタを保持し、
	// markRunning / evaluateIdle / applyDetectedModel 等が sessionsMu 下で同じフィールド
	// （State / Model / Branch 等）を書き換えるため、ロック外で Marshal すると read/write
	// data race になる（-race ビルドで検出可能）。
	b, _ := json.Marshal(list)
	s.sessionsMu.Unlock()
	// hub_instance: Hub 再起動を UI が検出するための起動毎 ID。
	// UI 側は前回値と異なる場合に live session ID キーのローカル状態
	// （チャット・ターミナルバッファ等）を破棄してから snapshot を適用する。
	_ = uc.send(map[string]any{"type": "snapshot", "sessions": json.RawMessage(b), "hub_instance": s.instanceID})

	// C3: UI 接続時に既存セッションの usageStat をまとめて送る。
	// これにより再接続時・リロード時にステータスバーが即座に復元される。
	// ロック順序（usage_stat.go の不変条件）: usageStatsMu 保持中に sessionsMu を取得しない。
	// provider は上の sessionsMu 区間で確定済みの providerByID から引く（ネスト取得を避ける）。
	usageStatsMu.Lock()
	for _, id := range sessionIDs {
		if stat, ok := usageStats[id]; ok {
			_ = uc.send(proto.Message{
				Type:           "usage_stat",
				SessionID:      id,
				Provider:       providerByID[id],
				CostUSD:        stat.CostUSD,
				CostKnown:      stat.CostKnown,
				TokensIn:       stat.TokensIn,
				TokensOut:      stat.TokensOut,
				TokensCache:    stat.TokensCache,
				TokensTotal:    stat.TokensTotal,
				CtxWindow:      stat.CtxWindow,
				UsageModel:     stat.UsageModel,
				UsageStartedAt: stat.StartedAt,
			})
		}
	}
	usageStatsMu.Unlock()
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
	// セッションログが無効（既定）なら .jsonl・SQLite いずれにも本文を残さない。
	// .log の抑止は wrapper 側、.jsonl writer の不生成は wrapperLoop/reattachLoop 側で
	// 行うが、SQLite の StoreEvent もここを通るため一括でゲートする。
	s.cfgMu.Lock()
	sessionLogEnabled := s.cfg.Log.SessionEnabled
	s.cfgMu.Unlock()
	if !sessionLogEnabled {
		return
	}
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
		// SQLite への書き込みは非同期キュー経由。pty_data のホットパスから
		// 呼ばれるため、DB の遅延・障害で UI 配信（broadcast）を止めない。
		if dropped := s.sessionStore.StoreEventAsync(sessionID, event); dropped > 0 && dropped%1000 == 1 {
			s.logger.Warn("sqlite session event queue full; dropping events", "session_id", sessionID, "dropped_total", dropped)
		}
	}
}
