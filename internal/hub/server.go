package hub

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"io/fs"
	"log/slog"
	"net"
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

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/attach"
	"many-ai-cli/internal/autoapproval"
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/notify"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/sessionstore"
	"many-ai-cli/internal/wrapper"
	"many-ai-cli/web"
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

	bracketedPasteStart       = "\x1b[200~"
	bracketedPasteEnd         = "\x1b[201~"
	bracketedPasteSubmitDelay = 50 * time.Millisecond

	// OSC シーケンスをユーザーターン境界マーカーとして ptyBuf に注入する。
	// xterm.js はこのシーケンスを画面に表示しない。
	// 47777 は Hub のデフォルトポートを namespace として流用。
	chatHistoryUserTurnMarker = "\x1b]47777;user-turn\x07"
)

// SessionActivity が状態の正本で、State は互換表示用の派生値である。
// approval_visible は UI が xterm.js バッファをスキャンして session_hint で伝える。
type session struct {
	ID                 int    `json:"id"`
	Provider           string `json:"provider"`
	Display            string `json:"display_name"`
	CWD                string `json:"cwd"`
	Branch             string `json:"branch,omitempty"`
	Label              string `json:"label,omitempty"` // UI カード 3 行目に【ラベル】として表示
	Pinned             bool   `json:"pinned,omitempty"`
	Color              string `json:"color,omitempty"`
	Note               string `json:"note,omitempty"`
	AutoTitle          string `json:"auto_title,omitempty"`
	Model              string `json:"model,omitempty"` // 使用モデル名; UI カード表示用
	Route              string `json:"route,omitempty"` // 接続経路（"ollama" 等）; UI で Ollama バックエンドの識別に使用
	Shell              string `json:"shell,omitempty"`
	ParentSessionID    int    `json:"parent_session_id,omitempty"`
	Role               string `json:"role,omitempty"`
	Auto               bool   `json:"auto,omitempty"`
	Depth              int    `json:"depth,omitempty"`
	OrchestrationID    string `json:"orchestration_id,omitempty"`
	BoardPath          string `json:"board_path,omitempty"`
	WorktreeBranch     string `json:"worktree_branch,omitempty"`
	NormalWorktree     normalWorktree
	WorktreeCleanup    string
	BoardNotifyPending bool `json:"board_notify_pending,omitempty"`
	// Activity is the authoritative three-axis activity model. State is kept
	// below only as a compatibility display label for older clients.
	Activity     SessionActivity `json:"activity"`
	State        string          `json:"state"`
	LastOutputAt string          `json:"last_output_at,omitempty"` // ISO 8601; UI カード「最終応答時刻」用
	StartedAt    string          `json:"started_at,omitempty"`     // ISO 8601; UI カード「起動時刻」用
	FirstMessage string          `json:"first_message,omitempty"`  // 最初の確定入力; UI カード表示用
	LastMessage  string          `json:"last_message,omitempty"`   // 最新の確定入力; UI カード表示用
	EndReason    string          `json:"end_reason,omitempty"`     // session_end の reason コード（例: "exec_not_found"）。UI 側で i18n 翻訳して表示
	HomeDir      string          `json:"-"`
	CodexHome    string          `json:"-"`
	ClaudeDir    string          `json:"-"`

	// JSON 外: 状態評価用
	lastOutputAt      time.Time // idleAfter 計算用。LastOutputAt と同期して更新する
	approvalVisible   bool
	approvalVisibleAt time.Time // approvalVisible=true を最後に受信した時刻（approvalVisibleLease 判定用）
	branchCheckedAt   time.Time

	// JSON 外: 初期プロンプト注入ゲート。orchestration セッション（conductor / 子）の
	// spawn 直後〜injectInitialPrompt 完了までユーザー入力を pendingInput へ保留する。
	// CLI 起動途中の TUI は入力バイトを捨てるため、注入前に届いた入力は黙って失われる
	// （docs/local/bugfix_orchestration-codex-child-spawn-failures_2026-07-04.md）。
	// initialInjectGateAt は注入経路の事故でゲートが張り付いたまま残った場合の
	// 期限判定（initialInjectGateMaxAge）に使う。
	initialInjectPending bool
	initialInjectGateAt  time.Time

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
	approvalMarkerSig         string

	// JSON 外: wrapper に最後に送った PTY サイズ（同サイズの resize を skip して不要な SIGWINCH を防ぐ）
	lastCols int
	lastRows int

	// JSON 外: 完了サマリー通知の連投抑制用
	lastDoneNotifyAt time.Time

	// JSON 外: Git タブ「Ask AI」コミットメッセージ生成の待ち受け状態。
	// 接続中の AI セッションへ生成プロンプトを注入し、PTY 出力から
	// [MANY-AI-CLI-COMMIT] マーカーを拾ってフォームへ反映する。
	commitMsgAwait      bool            // マーカー待ち受け中
	commitMsgDeadline   time.Time       // 待ち受けの打ち切り時刻
	commitMsgLang       string          // 生成言語（ja/en）。タイムアウト文言に使用
	commitMsgBuf        strings.Builder // ANSI 除去済み出力の蓄積（マーカー抽出用・上限つき）
	commitMsgProgressed bool            // AI からの最初の非空出力で 1 回だけ commit_msg_progress を送る
	// doneMsgBuf は [MANY-AI-CLI-DONE]…[/MANY-AI-CLI-DONE] のマーカーが 4096 バイトの
	// PTY 読み取りチャンク境界をまたいでも検出できるよう、ANSI 除去済み出力を
	// セッション単位で累積するスキャンバッファ。commitMsgBuf と同型で上限つき。
	// マーカー検出（あるいは十分な余白蓄積）でリセットする。
	doneMsgBuf strings.Builder

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
	// ポインタで保持する（AUDIT-11）: session を JSON スナップショット用に値コピーする
	// 箇所（orchestration.go の cp := *ses）で sync.Mutex を値コピーすると go vet の
	// copylocks に触れるため。session 生成時に必ず new(sync.Mutex) を設定すること
	//（未設定＝nil のまま Lock すると nil pointer panic になる）。
	inputMu *sync.Mutex
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
	knownLmStudio := collectLMStudioModelIDs(s.modelsCache)
	return RouteForModel(provider, model, known, knownLmStudio)
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

func (c *wrapperConn) send(m any) (err error) {
	if c == nil || c.ws == nil {
		return fmt.Errorf("wrapper not connected")
	}
	c.sendMu.Lock()
	defer c.sendMu.Unlock()
	// ゼロ値 Conn や切断済みは x/net/websocket が panic することがあるため回収する。
	defer func() {
		if r := recover(); r != nil {
			err = fmt.Errorf("wrapper send failed: %v", r)
		}
	}()
	return websocket.JSON.Send(c.ws, m)
}

func (c *wrapperConn) sendWithDeadline(m any, deadline time.Time) (err error) {
	if c == nil || c.ws == nil {
		return fmt.Errorf("wrapper not connected")
	}
	c.sendMu.Lock()
	defer c.sendMu.Unlock()
	defer func() {
		if r := recover(); r != nil {
			err = fmt.Errorf("wrapper send failed: %v", r)
		}
	}()
	if err := c.ws.SetWriteDeadline(deadline); err != nil {
		return err
	}
	return websocket.JSON.Send(c.ws, m)
}

func (c *wrapperConn) close() {
	c.closeOnce.Do(func() {
		if c.ws != nil {
			_ = c.ws.Close()
		}
	})
}

type Server struct {
	cfg       *config.Config
	logger    *slog.Logger
	httpSrv   *http.Server
	devMode   bool   // --dev: web/ をファイルシステムから直接サーブ（再コンパイル不要）
	hubCWD    string // serve 起動時の os.Getwd() を保存
	version   string // main.version (ldflags 経由) を保持し /api/info で返す
	gitCommit string // main.gitCommit (ldflags 経由・任意)。/api/info で返す
	buildTime string // main.buildTime (ldflags 経由・任意)。/api/info で返す
	// binGuard は「稼働中 Hub が起動時のバイナリのままか」を判定する。
	// /api/info が binary_sha256 と binary_stale を申告するのに使い、
	// wrapper・launcher・status・UI はこのフラグを読むだけで stale を扱える。
	binGuard     *binaryGuard
	webSrcHash   string // hash of web/src/ baked into dist/.src-hash at build time
	webDistFresh bool   // true if current web/src/ matches webSrcHash (always true on VPS/Docker)
	parentShell  string
	instanceID   string // Hub プロセス起動ごとのランダム ID。UI が Hub 再起動（live session ID の振り直し）を検出するために snapshot に同梱する

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

	// autoApprovalMu guards the local, explicitly enabled whitelist policy and
	// the bounded runtime history used by Settings simulation.
	autoApprovalMu      sync.Mutex
	autoApprovalPolicy  *autoapproval.Policy
	autoApprovalHistory []autoApprovalCandidate

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
	oneTapApprovals   *oneTapApprovalManager
	orchestration     *orchestrationManager

	// 任意リモート PIN（pin_auth.go）。lazy 生成のため pinLim() 経由でアクセスする。
	pinLimiterMu sync.Mutex
	pinLimiter   *pinLimiter
	// SEC-C: 既知デバイス（IP+UA ハッシュ → 最終接続時刻）。未知デバイスの初回 remote 接続で通知。
	devicesMu    sync.Mutex
	knownDevices map[string]time.Time

	logMaintenanceMu sync.Mutex
	whisperMu        sync.Mutex
	whisperInstall   whisperInstallState
	whisperCmd       *exec.Cmd
	whisperJob       whisperProcessJob
	whisperServerURL string
	// whisperStarting は startManagedWhisper 中の TOCTOU (HUB-3) を防ぐ排他フラグ。
	// 未起動判定と cmd.Start() の間で別リクエストが並走して二重起動しないよう、
	// 予約 → 起動 → 登録 の一連を単一クリティカルセクション相当にする。
	whisperStarting bool

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

	// profileSaveMu は launcher-profiles.yaml への Load→Modify→Save を直列化する
	// （HUB-8）。/api/servers POST が並行呼びされたときのロストアップデートを防ぐ。
	profileSaveMu sync.Mutex

	autoOpenBrowser bool

	// tsRunner: tailscale CLI 実行の抽象（テストでモック注入）。nil なら exec 実装。
	tsRunner tailscaleRunner

	// sshdProber: OpenSSH Server 状態検知の抽象（テストでモック注入）。nil なら OS 実装。
	sshdProber sshdProber

	// bugReportGistRunner / bugReportSaveMarkdown はバグ報告の外部送信・
	// ローカル保存境界。テストでは必ず差し替え、gh / 実 home を触らない。
	bugReportGistRunner   bugReportGistRunner
	bugReportSaveMarkdown func(string) (string, error)
	bugReportLogPreviewMu sync.Mutex
	bugReportLogPreviews  map[string]bugReportLogPreview
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

// doneMsgScanBufMax は doneMsgBuf の上限。commitMsgBuf と同じ発想で、
// マーカーがチャンク境界をまたいでも検出できる程度に大きく、かつ長時間の
// アイドル出力でメモリを圧迫しない値。数 KB あれば通常の 1〜2 文の DONE
// サマリー全体を余裕でカバーできる。
const doneMsgScanBufMax = 16 * 1024

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

// computeWebSrcHash は srcDir 配下の .ts/.js ファイル（vendor/ 除外・.d.ts 除外）を
// ソート順に SHA256 ハッシュし、先頭 12 文字の hex 文字列を返す。
// build.mjs の generateSrcHash() と同じアルゴリズムで、両者の出力を直接比較できる。
func computeWebSrcHash(srcDir string) string {
	var files []string
	_ = filepath.WalkDir(srcDir, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if d.IsDir() {
			if d.Name() == "vendor" {
				return filepath.SkipDir
			}
			return nil
		}
		name := d.Name()
		if strings.HasSuffix(name, ".d.ts") {
			return nil
		}
		if strings.HasSuffix(name, ".ts") || strings.HasSuffix(name, ".js") {
			files = append(files, p)
		}
		return nil
	})
	if len(files) == 0 {
		return ""
	}
	sort.Strings(files)
	h := sha256.New()
	for _, f := range files {
		rel, _ := filepath.Rel(srcDir, f)
		rel = filepath.ToSlash(rel)
		content, err := os.ReadFile(f)
		if err != nil {
			return ""
		}
		_, _ = h.Write([]byte(rel + "\n"))
		_, _ = h.Write(content)
		_, _ = h.Write([]byte("\n"))
	}
	result := fmt.Sprintf("%x", h.Sum(nil))
	if len(result) > 12 {
		return result[:12]
	}
	return result
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

func NewServer(cfg *config.Config, logger *slog.Logger, devMode bool, version string, build BuildInfo) (*Server, error) {
	hubCWD, _ := os.Getwd()
	binGuard := newBinaryGuard()
	if binGuard.StartSHA() == "" {
		// 致命的にはしない。binary_sha256 が空でも Hub は起動でき、
		// stale 検知が無効化されるだけ。
		logger.Warn("self binary hash unavailable; stale-binary detection disabled")
	}
	s := &Server{
		cfg:                   cfg,
		logger:                logger,
		devMode:               devMode,
		hubCWD:                hubCWD,
		version:               version,
		gitCommit:             build.GitCommit,
		buildTime:             build.BuildTime,
		binGuard:              binGuard,
		instanceID:            newInstanceID(),
		parentShell:           wrapper.DetectShell(),
		sessions:              map[int]*session{},
		wrappers:              map[int]*wrapperConn{},
		uis:                   map[*websocket.Conn]*uiConn{},
		pendingInput:          map[int][]string{},
		slashCmdCache:         map[string]*slashCmdCacheEntry{},
		approvalRuleTargets:   map[string]approvalRuleTarget{},
		autoApprovalHistory:   make([]autoApprovalCandidate, 0, 100),
		usageLinkCache:        newUsageLinkCache(),
		modelsCache:           &modelsCache{},
		modelsRemoteCache:     newModelsRemoteCache(),
		orchestration:         newOrchestrationManager(),
		branchRefreshSem:      make(chan struct{}, branchRefreshWorkers),
		branchRefreshInFlight: map[string]struct{}{},
		serverConns:           newServerConnManager(logger),
	}
	if actions, err := newOneTapApprovalManager(); err != nil {
		return nil, err
	} else {
		s.oneTapApprovals = actions
	}
	if policy, err := autoapproval.Load(); err != nil {
		logger.Warn("auto approval policy unavailable", "err", err)
		s.autoApprovalPolicy = &autoapproval.Policy{}
	} else {
		s.autoApprovalPolicy = policy
		for _, warning := range policy.Warnings {
			logger.Warn("auto approval rule disabled", "warning", warning)
		}
	}
	// web/dist 鮮度チェック: dist/.src-hash (ビルド時焼き付け) と現在の web/src/ を比較。
	// web/src/ が存在しない環境（VPS/Docker）ではチェックをスキップし fresh 扱いとする。
	{
		bakedHash := ""
		if hashBytes, err := web.FS.ReadFile("dist/.src-hash"); err == nil {
			bakedHash = strings.TrimSpace(string(hashBytes))
		}
		s.webSrcHash = bakedHash
		webSrcDir := filepath.Join(hubCWD, "web", "src")
		if bakedHash == "" {
			s.webDistFresh = true // hash 未生成（初回ビルド前等）は誤警告を避けて fresh 扱い
		} else if _, err := os.Stat(webSrcDir); err != nil {
			s.webDistFresh = true // web/src/ が無い環境（VPS/Docker）はチェック不可
		} else {
			currentHash := computeWebSrcHash(webSrcDir)
			s.webDistFresh = currentHash == bakedHash
			if !s.webDistFresh {
				logger.Warn("web/dist is stale: run make build",
					"dist_hash", bakedHash,
					"src_hash", currentHash,
				)
			}
		}
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
	mux.HandleFunc("/api/bug-report/preview", s.handleBugReportPreview)
	mux.HandleFunc("/api/bug-report/finalize", s.handleBugReportFinalize)
	mux.HandleFunc("/api/doctor", s.handleDoctor)
	mux.HandleFunc("/api/mobile-connect", s.handleMobileConnect)
	mux.HandleFunc("/api/mobile-connect/tailscale", s.handleTailscaleStatus)
	mux.HandleFunc("/api/mobile-connect/tailscale/serve", s.handleTailscaleServe)
	mux.HandleFunc("/api/auth/revoke-all", s.handleAuthRevokeAll)
	mux.HandleFunc("/api/auth/status", s.handleAuthStatus)
	mux.HandleFunc("/api/auth/login", s.handleAuthLogin)
	mux.HandleFunc("/api/auth/logout", s.handleAuthLogout)
	mux.HandleFunc("/api/auth/set-pin", s.handleAuthSetPIN)
	mux.HandleFunc("/api/net-hint", s.handleNetHint)
	mux.HandleFunc("/api/avatar", s.handleAvatar)
	mux.HandleFunc("/api/spawn", s.handleSpawn)
	mux.HandleFunc("/api/spawn-grid", s.handleSpawnGrid)
	// /api/session/:id/meta is the public singular route described in the UX
	// contract. Keep the older plural /api/sessions/ namespace for orchestration.
	mux.HandleFunc("/api/session/", s.handleSessionMetaAPI)
	mux.HandleFunc("/api/sessions/", s.handleSessionAPI)
	mux.HandleFunc("/api/pick-directory", s.handlePickDirectory)
	mux.HandleFunc("/api/path-exists", s.handlePathExists)
	mux.HandleFunc("/api/list-subdirs", s.handleListSubdirs)
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
	mux.HandleFunc("/api/grok-history", s.handleGrokHistory)
	mux.HandleFunc("/api/session-search", s.handleSessionSearch)
	mux.HandleFunc("/api/session-history", s.handleSessionHistory)
	mux.HandleFunc("/api/session-store/reset", s.handleSessionStoreReset)
	mux.HandleFunc("/api/logs/purge", s.handleLogsPurge)
	mux.HandleFunc("/api/logs/legacy-notice", s.handleLegacyLogsNotice)
	mux.HandleFunc("/api/attachments/purge", s.handleAttachmentsPurge)
	mux.HandleFunc("/api/open-dir", s.handleOpenDir)
	mux.HandleFunc("/api/idle-timeout", s.handleIdleTimeout)
	mux.HandleFunc("/api/reconnect-grace", s.handleReconnectGrace)
	mux.HandleFunc("/api/input-config", s.handleInputConfig)
	mux.HandleFunc("/api/orchestration-config", s.handleOrchestrationConfig)
	mux.HandleFunc("/api/notify-config", s.handleNotifyConfig)
	mux.HandleFunc("/api/notify-test", s.handleNotifyTest)
	mux.HandleFunc("/api/notify-generate-topic", s.handleNotifyGenerateTopic)
	mux.HandleFunc("/api/encoding-check", s.handleEncodingCheck)
	mux.HandleFunc("/api/approval/status", s.handleApprovalStatus)
	mux.HandleFunc("/api/approval/enable", s.handleApprovalEnable)
	mux.HandleFunc("/api/approval/disable", s.handleApprovalDisable)
	mux.HandleFunc("/api/approval/dismiss", s.handleApprovalDismiss)
	mux.HandleFunc("/api/approval/batch", s.handleApprovalBatch)
	mux.HandleFunc("/api/approval-action/", s.handleOneTapApproval)
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
	mux.HandleFunc("/api/git-diff", s.handleGitDiff)
	mux.HandleFunc("/api/git-commit-all", s.handleGitCommitAll)
	mux.HandleFunc("/api/git-commit-message", s.handleGitCommitMessage)
	mux.HandleFunc("/api/git-fetch", s.handleGitFetch)
	mux.HandleFunc("/api/git-pull", s.handleGitPull)
	mux.HandleFunc("/api/git-push", s.handleGitPush)
	mux.HandleFunc("/api/user-prefs/notify-sound-custom", s.handleUserPrefsNotifySoundCustom)
	mux.HandleFunc("/api/user-prefs/avatar", s.handleUserPrefsAvatarUpload)
	mux.HandleFunc("/api/user-prefs", s.handleUserPrefs)
	mux.HandleFunc("/api/auto-approval/status", s.handleAutoApprovalStatus)
	mux.HandleFunc("/api/auto-approval/simulate", s.handleAutoApprovalSimulation)
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
	mux.HandleFunc("/api/profiles/fetch", s.handleProfilesFetch)
	s.httpSrv = &http.Server{
		Addr:    fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port),
		Handler: withSecurityHeaders(mux),
		// Slowloris 対策（gosec G112）。WS を長く張るため ReadTimeout は設定せず、
		// ヘッダ読み取りのみタイムアウトさせる。
		ReadHeaderTimeout: 10 * time.Second,
	}
	return s, nil
}

// cspConnectSrcBase は loopback（SSHトンネル経由のスマホ含む）向けの WebSocket 許可元。
const cspConnectSrcBase = "'self' ws://127.0.0.1:* ws://localhost:*"

// cspWithConnectSrc は connect-src 部分だけ差し替えた CSP を組み立てる。
func cspWithConnectSrc(connectSrc string) string {
	return "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' data: blob:; connect-src " + connectSrc + "; font-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"
}

// contentSecurityPolicy は API レスポンス等に付与する静的 CSP（loopback のみ）。
var contentSecurityPolicy = cspWithConnectSrc(cspConnectSrcBase)

// documentCSP は HTML ドキュメント（handleIndex）に付与する CSP。
// VPN 直アクセス（例 http://100.x:port / https://名前.ts.net）でも WebSocket が
// 張れるよう、hub.allowed_hosts に登録済みの host を ws://host:* / wss://host:* として
// connect-src に展開する（C5 / G2）。allowed_hosts は config.Validate 済みで
// ポート・ワイルドカード・スキームを含まない host 文字列のみ（安全に補間できる）。
func (s *Server) documentCSP() string {
	s.cfgMu.Lock()
	hosts := append([]string(nil), s.cfg.Hub.AllowedHosts...)
	s.cfgMu.Unlock()
	parts := []string{cspConnectSrcBase}
	for _, h := range hosts {
		h = strings.TrimSuffix(strings.TrimSpace(h), ".")
		if h == "" {
			continue
		}
		if strings.Contains(h, ":") { // IPv6 リテラルは [] で括る
			h = "[" + h + "]"
		}
		parts = append(parts, "ws://"+h+":*", "wss://"+h+":*")
	}
	return cspWithConnectSrc(strings.Join(parts, " "))
}

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
	// 永続ログ（hub.log）にはトークンを平文で残さない。ライブの全権トークンが
	// ローテーション済みログに残ると、トラブルシュートでログを共有した際に漏洩する。
	// 実トークン入りの URL は stdout の起動バナー（下記 startupBanner）だけに出す。
	s.logger.Info("MANY-AI-CLI started", "url", fmt.Sprintf("http://%s/?token=***", s.httpSrv.Addr))
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
	s.safeGo("orchestration_board_loop", func() { s.orchestrationBoardLoop(runCtx) })
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
		if s.orchestration != nil {
			s.orchestration.stop()
		}
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
	// guardBase で token に加え Host 許可リスト検証（DNS リバインディング防御）も通す。
	// GET なので CSRF（Origin）追加チェックは課されず、PIN ゲート（guard）は通さない
	// ので PIN 未入力でも index は返り、フロントが PIN モーダルを出せる。
	if !s.guardBase(w, r, http.MethodGet) {
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
	// SEC-C: リモートからの（token 認証済み）ページ取得を記録し、未知デバイスなら通知する。
	// PIN 未入力でもここは通る（モーダルを出すため）。盗まれた token の使用を即検知する。
	s.noteRemoteDevice(r, "page")
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
	// ドキュメントの CSP は allowed_hosts を connect-src に展開した動的版で上書きする
	// （VPN 直アクセス時に WebSocket が CSP で弾かれないようにする / C5）。
	w.Header().Set("Content-Security-Policy", s.documentCSP())
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
	// register の m.Token（URLクエリ→localStorage 由来）が無効でも、ハンドシェイク
	// 要求の Cookie/Authorization/クエリ（requestToken）を、ページ GET / と同じ
	// フォールバックで受け付ける。スマホ/PWA で localStorage の token が揮発しても
	// Cookie（MANY_AI_CLI_token）が生きていれば WS を確立でき、セッション一覧が空に
	// ならない（bugfix_mobile-ws-token-cookie-fallback）。
	if !s.validTokenOrTrustedRemote(m.Token, req) &&
		!(req != nil && s.validTokenOrTrustedRemote(requestToken(req), req)) {
		return
	}
	// リモート PIN ゲート（pin_auth.go）。loopback / PIN 無効時は素通し。
	// wrapper は同一ホスト（loopback・Host=127.0.0.1）から接続するため影響しない。
	// tailscale serve 経由（loopback 元・Host=tailnet 名）は remotePINRequired が
	// true を返すため PIN cookie を要求する。
	if s.remotePINRequired(req) && !s.hasValidPINCookie(req) {
		return
	}
	if m.Role == "ui" {
		// SEC-C: リモートからの UI 接続を記録し、未知デバイスなら本人へ通知する。
		s.noteRemoteDevice(req, "ws")
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

// wrapper 接続ライフサイクル関連の 3 関数 (wrapperLoop / reattachLoop /
// wrapperMessageLoop) は C4 追加分割で internal/hub/wrapper_loop.go へ移動した。

// 承認検出まわりの 9 関数 (resetNativeApprovalClearMisses / handleNativeApprovalDetection /
// handleDoneSummaryMarker / notifyDoneOutbound / notifyDonePush /
// markNativeApprovalConsumed / appendPTYReplay / ptyChunkContainsAny /
// nativeApprovalTailSignature / shouldSuppressNativeApprovalClearMiss) は
// C4 リファクタで internal/hub/approval_native.go へ移動した。

// detectModelChange は PTY 出力からモデル変更を検出し、
// セッションの Model フィールドを更新して UI に session_update を送る。
// Claude Code の "Set model to <name>" / Codex CLI の "Model changed to <name>" を対象とする。
// モデル検出まわりの 4 関数 (detectModelChange / detectInitialModel /
// extractBannerModel / applyDetectedModel) は C4 追加分割で
// internal/hub/model_detect.go へ移動した。

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
		// PTY サイズ変更の履歴を .jsonl に残す。表示崩れ（xterm と PTY の
		// サイズ不一致）の再発時に、いつ・どのサイズへ変わったかを生ログと
		// 突き合わせるための観測用（bugfix_codex-terminal-gap-resize-mismatch_2026-07-05.md）。
		s.writeHistory(m.SessionID, map[string]any{
			"ts":         time.Now().Format(time.RFC3339),
			"type":       "pty_resize",
			"session_id": m.SessionID,
			"cols":       m.Cols,
			"rows":       m.Rows,
		})
	}
}

// 入力ゲートまわりの 10 関数 (handleInput / splitBracketedPasteSubmit /
// sessionInjectGated / submitInput / submitInputWithGate / trySendInput /
// flushPendingInput / requeuePendingInput / appendPendingInput /
// notifyInputDeferred) と定数 (maxPendingInputPerSession / initialInjectGateMaxAge)
// は C4 追加分割で internal/hub/input_gate.go へ移動した。

// handleHint は session_hint メッセージを処理し、待機軸と派生 State を即時更新する。
func (s *Server) handleHint(m proto.Message) {
	s.sessionsMu.Lock()
	ses := s.sessions[m.SessionID]
	var update proto.Message
	if ses != nil {
		ses.approvalVisible = m.ApprovalVisible
		if m.ApprovalVisible {
			ses.approvalVisibleAt = time.Now()
		} else {
			ses.approvalVisibleAt = time.Time{}
		}
		if !isTerminalSessionState(ses.State) {
			ses.Activity.AwaitingApproval = ses.approvalVisible
			ses.Activity.AwaitingUser = ses.approvalVisible
			ses.Activity.Normalize()
			ses.State = ses.Activity.DisplayState()
			update = sessionUpdateMessage(ses)
		}
	}
	s.sessionsMu.Unlock()
	if update.SessionID != 0 {
		s.broadcast(update)
	}
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
		ses.approvalMarkerSig = ""
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
			if err := s.sessionStore.ClearSessionHistory(id); err != nil {
				// 失敗しても broadcast は継続する（現行 UI 挙動維持）が、
				// 記録は必ず残す。以前は void 呼び出しで握り潰されており、
				// SQLite クリア失敗時にも UI へ「削除成功」を通知していた。
				s.logger.Warn("clear session history failed", "session_id", id, "err", err)
			}
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
//
// Hub map に既に無い ID でも session_removed を broadcast する（冪等 dismiss）。
// 理由: inject 中 dismiss → 後続 session_update で UI だけ幽霊化したケースや、
// session_removed 取りこぼし後の再 × で、UI が永遠に消えないのを防ぐ。
func (s *Server) handleDismiss(m proto.Message) (skip bool) {
	s.sessionsMu.Lock()
	wc := s.wrappers[m.SessionID]
	_, exists := s.sessions[m.SessionID]
	var historyToClose *sessionlog.Writer
	var jsonlPathForTranscript string
	var endedProvider, endedCWD string
	var endedWorktree normalWorktree
	var endedWorktreeCleanup string
	if exists {
		ses := s.sessions[m.SessionID]
		historyToClose = ses.History
		jsonlPathForTranscript = ses.JSONLPath
		endedProvider = ses.Provider
		endedCWD = ses.CWD
		endedWorktree = ses.NormalWorktree
		endedWorktreeCleanup = ses.WorktreeCleanup
		ses.History = nil
		delete(s.sessions, m.SessionID)
		delete(s.wrappers, m.SessionID)
		delete(s.pendingInput, m.SessionID)
	}
	s.sessionsMu.Unlock()
	if !exists {
		// map には無いが UI 側に残っている幽霊カードを落とす。
		s.broadcast(proto.Message{Type: "session_removed", SessionID: m.SessionID})
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
	s.removeInactiveUsageHooks(endedProvider, endedCWD)
	if err := cleanupNormalWorktree(endedWorktree, endedWorktreeCleanup); err != nil {
		s.logger.Warn("worktree retained after session dismissal", "path", endedWorktree.Path, "err", err)
	}
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
// アイドル状態機械の 5 関数 (markRunning / stateTicker / evaluateIdle /
// startIdleTimerLocked / stopIdleTimerLocked) は C4 追加分割で
// internal/hub/idle_state.go へ移動した。
// branch 再取得の 2 関数 (queueBranchRefreshes / refreshBranchForCWD) は
// internal/hub/branch_refresh.go へ移動した。
// UI broadcast の 5 関数 (addUIWithHistory / removeUI / pingLoop / sendSnapshot /
// broadcast) は internal/hub/ui_broadcast.go へ移動した。

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
