package hub

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log/slog"
	"net"
	"net/http"
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
	"any-ai-cli/internal/wrapper"
	"any-ai-cli/web"
	"golang.org/x/net/websocket"
)

// idleAfter: PTY 出力が静止してから running → waiting に遷移するまでの時間。
// tickerInterval: 状態評価 ticker の間隔。
// maxPTYBuf: UI 再接続時リプレイ用の PTY バッファ上限（セッションごと）。
// uiPingInterval: UI WebSocket keepalive ping の送信間隔。
const (
	idleAfter              = 500 * time.Millisecond
	tickerInterval         = 200 * time.Millisecond
	maxPTYBuf              = 512 * 1024 // 512 KB
	replayTailForNonActive = 64 * 1024  // 64 KB: 非アクティブセッションの UI 接続時 replay 上限
	uiPingInterval         = 30 * time.Second
	branchLookupTimeout    = 250 * time.Millisecond
	branchRefreshAfter     = 2 * time.Second
	vtResizeDebounce       = 200 * time.Millisecond
	approvalConsumedTTL    = 10 * time.Second

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
	lastOutputAt    time.Time // idleAfter 計算用。LastOutputAt と同期して更新する
	approvalVisible bool
	branchCheckedAt time.Time

	// JSON 外: UI 再接続時リプレイ用リングバッファ（末尾 maxPTYBuf bytes）
	ptyBuf []byte

	// JSON 外: Go 側 native approval 検出用 VT バッファ。
	vt                       *vtBuffer
	vtResizeDebounceUntil    time.Time
	nativeApprovalSig        string
	nativeApprovalConsumed   string
	nativeApprovalConsumedAt time.Time

	// JSON 外: wrapper に最後に送った PTY サイズ（同サイズの resize を skip して不要な SIGWINCH を防ぐ）
	lastCols int
	lastRows int

	// JSON 外: セッション履歴（JSONL）
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
	s.mu.Lock()
	localCfg := append([]config.LocalModel(nil), s.cfg.LocalModels...)
	s.mu.Unlock()
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

type Server struct {
	cfg         *config.Config
	logger      *slog.Logger
	httpSrv     *http.Server
	devMode     bool   // --dev: web/ をファイルシステムから直接サーブ（再コンパイル不要）
	hubCWD      string // serve 起動時の os.Getwd() を保存
	version     string // main.version (ldflags 経由) を保持し /api/info で返す
	parentShell string

	mu       sync.Mutex
	nextID   int
	sessions map[int]*session
	wrappers map[int]*websocket.Conn
	uis      map[*websocket.Conn]*uiConn

	slashCmdMu    sync.Mutex
	slashCmdCache map[string]*slashCmdCacheEntry // key: provider

	usageLinkCache *usageLinkCache

	modelsCache       *modelsCache
	modelsRemoteCache *modelsRemoteCache

	lastUICols int
	lastUIRows int
	idleTimer  *time.Timer
	idleGen    uint64 // incremented on each startIdleTimerLocked / stopIdleTimerLocked to invalidate stale callbacks

	stopMu   sync.Mutex
	stopFunc context.CancelFunc

	autoOpenBrowser bool
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

func NewServer(cfg *config.Config, logger *slog.Logger, devMode bool, version string) (*Server, error) {
	hubCWD, _ := os.Getwd()
	s := &Server{
		cfg:               cfg,
		logger:            logger,
		devMode:           devMode,
		hubCWD:            hubCWD,
		version:           version,
		parentShell:       wrapper.DetectShell(),
		sessions:          map[int]*session{},
		wrappers:          map[int]*websocket.Conn{},
		uis:               map[*websocket.Conn]*uiConn{},
		slashCmdCache:     map[string]*slashCmdCacheEntry{},
		usageLinkCache:    &usageLinkCache{},
		modelsCache:       &modelsCache{},
		modelsRemoteCache: &modelsRemoteCache{},
	}
	if devMode {
		logger.Info("dev mode: serving web assets from ./web/src/")
	}
	var staticHandler http.Handler
	if devMode {
		staticHandler = http.FileServer(http.Dir(filepath.Join("web", "src")))
	} else {
		subFS, err := fs.Sub(web.FS, "src")
		if err != nil {
			return nil, err
		}
		staticHandler = http.FileServer(http.FS(subFS))
	}
	// 承認パターン JSON はユーザー設定ディレクトリ ~/.any-ai-cli/approval-patterns/
	// から配信する。official / custom の 2 プロファイル管理で、旧 <provider>.json は
	// 初回起動時に <provider>.custom.json へマイグレートする。フロント既存ロード経路
	// との互換のためアクティブプロファイル内容を <provider>.json にミラーする。
	if err := SyncApprovalPatterns(cfg.ApprovalProfiles); err != nil {
		logger.Warn("sync approval patterns failed", "err", err)
	}
	approvalPatternsHandler := http.StripPrefix("/approval-patterns/",
		http.FileServer(http.Dir(approvalPatternsDir())))
	mux := http.NewServeMux()
	mux.HandleFunc("/", s.handleIndex)
	mux.Handle("/app.js", staticHandler)
	mux.Handle("/app/", staticHandler)
	mux.Handle("/styles.css", staticHandler)
	mux.Handle("/styles/", staticHandler)
	mux.Handle("/icon.svg", staticHandler)
	mux.Handle("/i18n.js", staticHandler)
	mux.Handle("/i18n/", staticHandler)
	mux.Handle("/vendor/", staticHandler)
	mux.Handle("/approval-patterns/", approvalPatternsHandler)
	mux.Handle("/ws", websocket.Server{
		Handshake: s.wsHandshake,
		Handler:   s.handleWS,
	})
	mux.HandleFunc("/api/info", s.handleInfo)
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
	s.httpSrv = &http.Server{Addr: fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port), Handler: mux}
	return s, nil
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
	basePort := s.cfg.Hub.Port
	for p := basePort; p < basePort+100; p++ {
		addr := fmt.Sprintf("127.0.0.1:%d", p)
		var e error
		ln, e = net.Listen("tcp", addr)
		if e == nil {
			if p != basePort {
				s.httpSrv.Addr = addr
				s.cfg.Hub.Port = p
				s.logger.Info("preferred port in use, using alternative port", "from", basePort, "to", p)
			}
			break
		}
	}
	if ln == nil {
		return fmt.Errorf("no available port found in range %d-%d", basePort, basePort+99)
	}
	_ = os.WriteFile(pidPath, []byte(fmt.Sprintf("%d", os.Getpid())), 0o644)
	setConsoleTitle("any-ai-cli [hub] - DO NOT CLOSE")
	setConsoleIcon()
	s.logger.Info("ANY-AI-CLI started", "url", fmt.Sprintf("http://%s/?token=%s", s.httpSrv.Addr, s.cfg.Token))
	fmt.Print(startupBanner(s.version, s.httpSrv.Addr, s.cfg.Token))
	if s.autoOpenBrowser {
		_ = s.OpenBrowser()
	}
	if s.cfg.Approval.Enabled {
		s.injectApprovalRules()
	}
	go s.stateTicker(runCtx)
	go s.cleanAttachments()
	go s.cleanSpawnLogs()
	go s.cleanSessionLogs()
	go s.recoverTranscripts()
	go s.approvalPatternsRemoteSync(runCtx)
	go func() {
		<-runCtx.Done()
		if s.cfg.Approval.Enabled {
			s.removeApprovalRules()
		}
		// Stop the Hub server without marking wrapper sessions as intentionally
		// disconnected. Closing the HTTP server drops WS connections after the
		// listener is gone, so wrappers treat this as Hub-down and enter their
		// reconnect grace period. Explicit session termination still goes through
		// /api/kill-all, dismiss, or idle-timeout.
		_ = s.httpSrv.Close()
		_ = os.Remove(pidPath)
	}()
	err := s.httpSrv.Serve(ln)
	if errors.Is(err, http.ErrServerClosed) {
		return nil
	}
	return err
}

// recoverTranscripts は logs/sessions/*.jsonl のうち、対応する .txt が
// 無い、もしくは .jsonl より古いものを遡って WriteTranscriptFile で生成する。
// Hub クラッシュ等で wrapperMessageLoop の終了処理を通れず .txt が作成
// されなかった場合の救済（通常運用では Close 直後に .txt が生成される）。
func (s *Server) recoverTranscripts() {
	dir := filepath.Join(s.cfg.Hub.LogDir, "sessions")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".jsonl") {
			continue
		}
		jsonlPath := filepath.Join(dir, e.Name())
		txtPath := sessionlog.TranscriptPath(jsonlPath)
		jsonlInfo, statErr := os.Stat(jsonlPath)
		if statErr != nil {
			continue
		}
		if txtInfo, err := os.Stat(txtPath); err == nil {
			if !txtInfo.ModTime().Before(jsonlInfo.ModTime()) {
				continue
			}
		}
		if err := sessionlog.WriteTranscriptFile(jsonlPath, txtPath); err != nil {
			s.logger.Warn("transcript recovery failed", "path", txtPath, "err", err)
		}
	}
}

// cleanSpawnLogs removes wrap-process spawn logs (logs/spawn/*.log) older than 7 days.
// These files capture stdout/stderr of each spawned wrap process for trouble-shooting
// (especially GUI-launched Hub where stderr is otherwise lost). One file per spawn is
// kept short-term to debug startup failures; trimming on Hub start prevents accumulation.
func (s *Server) cleanSpawnLogs() {
	dir := filepath.Join(s.cfg.Hub.LogDir, "spawn")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	cutoff := time.Now().Add(-7 * 24 * time.Hour)
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		if info.ModTime().Before(cutoff) {
			_ = os.Remove(filepath.Join(dir, e.Name()))
		}
	}
}

// cleanSessionLogs removes session log triplets (.log / .jsonl / .txt) in
// logs/sessions/ that are older than cfg.Log.SessionRetentionDays days.
// A retention of 0 disables cleanup; negative values are treated as 0.
func (s *Server) cleanSessionLogs() {
	s.mu.Lock()
	days := s.cfg.Log.SessionRetentionDays
	s.mu.Unlock()
	if days <= 0 {
		return
	}
	dir := filepath.Join(s.cfg.Hub.LogDir, "sessions")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	cutoff := time.Now().Add(-time.Duration(days) * 24 * time.Hour)
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		if info.ModTime().Before(cutoff) {
			_ = os.Remove(filepath.Join(dir, e.Name()))
		}
	}
}

// cleanAttachments removes attachment files older than 7 days and then prunes
// any session directories that are now empty.
func (s *Server) cleanAttachments() {
	home, err := os.UserHomeDir()
	if err != nil {
		return
	}
	attachDir := filepath.Join(home, ".any-ai-cli", "attachments")
	if err := attach.CleanOld(attachDir, 7); err != nil {
		s.logger.Warn("attach cleanup failed", "err", err)
	}
	entries, err := os.ReadDir(attachDir)
	if err != nil {
		return
	}
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		sub := filepath.Join(attachDir, e.Name())
		children, _ := os.ReadDir(sub)
		if len(children) == 0 {
			_ = os.Remove(sub)
		}
	}
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
func OpenBrowserForConfig(cfg *config.Config) error {
	url := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", cfg.Hub.Port, cfg.Token)
	return browserCommand(url).Start()
}

// IsRunning returns true if a Hub is already listening at the configured address.
func IsRunning(cfg *config.Config) bool {
	url := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", cfg.Hub.Port, cfg.Token)
	client := &http.Client{Timeout: 500 * time.Millisecond}
	resp, err := client.Get(url)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}

func (s *Server) handleIndex(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	var b []byte
	var err error
	if s.devMode {
		b, err = os.ReadFile(filepath.Join("web", "src", "index.html"))
	} else {
		b, err = web.FS.ReadFile("src/index.html")
	}
	if err != nil {
		http.Error(w, "asset missing", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store, must-revalidate")
	_, _ = w.Write(b)
}

// wsHandshake は WebSocket ハンドシェイク時に Origin を検証する。
// 許可: http://127.0.0.1:<port> / http://localhost:<port> / Origin ヘッダ無し（ラッパー等 CLI 由来）。
// 不一致は handshake エラーで拒否する。
func (s *Server) wsHandshake(cfg *websocket.Config, req *http.Request) error {
	origin := req.Header.Get("Origin")
	if origin == "" {
		// CLI / ラッパー由来の接続は Origin を持たないため許可する。
		return nil
	}
	s.mu.Lock()
	port := s.cfg.Hub.Port
	s.mu.Unlock()
	portStr := strconv.Itoa(port)
	allowed := []string{
		"http://127.0.0.1:" + portStr,
		"http://localhost:" + portStr,
	}
	for _, a := range allowed {
		if origin == a {
			return nil
		}
	}
	return fmt.Errorf("origin not allowed: %s", origin)
}

func (s *Server) handleWS(conn *websocket.Conn) {
	defer conn.Close()
	var m proto.Message
	if err := websocket.JSON.Receive(conn, &m); err != nil {
		return
	}
	if m.Token != s.cfg.Token {
		return
	}
	if m.Role == "ui" {
		if m.Cols > 0 && m.Rows > 0 {
			s.mu.Lock()
			s.lastUICols, s.lastUIRows = m.Cols, m.Rows
			s.mu.Unlock()
		}
		uc, historyItems := s.addUIWithHistory(conn, m.UIActiveSessionID)
		s.sendSnapshot(uc)
		for _, item := range historyItems {
			_ = uc.send(item)
		}
		ctx, cancel := context.WithCancel(context.Background())
		go s.pingLoop(ctx, uc)
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

func (s *Server) wrapperLoop(conn *websocket.Conn, reg proto.Message) {
	startedAt := time.Now()
	branch := gitBranch(reg.CWD)
	s.mu.Lock()
	s.nextID++
	id := s.nextID
	initCols, initRows := s.lastUICols, s.lastUIRows
	s.mu.Unlock()

	rawLogPath, jsonlPath := sessionlog.Paths(s.cfg.Hub.LogDir, sessionlog.Metadata{
		SessionID: id,
		Provider:  reg.Provider,
		CWD:       reg.CWD,
		StartedAt: startedAt,
	})
	s.mu.Lock()
	jsonlMaxBytes := int64(s.cfg.Log.SessionMaxSizeMB) * 1024 * 1024
	s.mu.Unlock()
	history, histErr := sessionlog.NewJSONLWriter(jsonlPath, jsonlMaxBytes)
	if histErr != nil {
		s.logger.Warn("session history create failed", "path", jsonlPath, "err", histErr)
	}

	regRoute := strings.TrimSpace(reg.Route)
	if regRoute == "" {
		regRoute = s.resolveRoute(reg.Provider, reg.Model)
	}
	s.mu.Lock()
	ses := &session{
		ID:              id,
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
	s.wrappers[id] = conn
	s.mu.Unlock()
	if initCols == 0 || initRows == 0 {
		// UIが未接続の場合はラッパーが報告した呼び出し元端末サイズを優先する
		if reg.Cols > 0 && reg.Rows > 0 {
			initCols, initRows = reg.Cols, reg.Rows
		} else {
			initCols, initRows = defaultInitCols, defaultInitRows
		}
	}
	s.mu.Lock()
	ses.lastCols, ses.lastRows = initCols, initRows
	ses.vt = newVTBuffer(initCols, initRows)
	s.mu.Unlock()
	_ = websocket.JSON.Send(conn, proto.Message{Type: "registered", SessionID: id, Cols: initCols, Rows: initRows, StartedAt: ses.StartedAt, LogPath: rawLogPath, JSONLPath: jsonlPath})
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
	s.wrapperMessageLoop(conn, id)
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
	s.mu.Lock()
	jsonlMaxBytesReattach := int64(s.cfg.Log.SessionMaxSizeMB) * 1024 * 1024
	s.mu.Unlock()
	history, histErr := sessionlog.NewJSONLWriterAppend(jsonlPath, jsonlMaxBytesReattach)
	if histErr != nil {
		s.logger.Warn("session history append failed", "path", jsonlPath, "err", histErr)
	}

	reqRoute := strings.TrimSpace(req.Route)
	if reqRoute == "" {
		reqRoute = s.resolveRoute(req.Provider, req.Model)
	}
	s.mu.Lock()
	acceptedID := req.SessionID
	if s.wrappers[acceptedID] != nil {
		s.nextID++
		acceptedID = s.nextID
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
	s.wrappers[acceptedID] = conn
	if s.nextID < acceptedID {
		s.nextID = acceptedID
	}
	s.mu.Unlock()
	if oldHistory != nil {
		_ = oldHistory.Close()
	}
	_ = websocket.JSON.Send(conn, proto.Message{Type: "reattach_ack", SessionID: acceptedID})
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
	s.wrapperMessageLoop(conn, acceptedID)
}

// finalizeTranscript は JSONL パスからトランスクリプトファイルを生成する。
// wrapperMessageLoop と session_dismiss の 2 箇所で同一の transcript 生成コードが
// 重複していたため、ここに集約する。
func (s *Server) finalizeTranscript(id int, jsonlPath string) {
	if jsonlPath == "" {
		return
	}
	transcriptPath := sessionlog.TranscriptPath(jsonlPath)
	if err := sessionlog.WriteTranscriptFile(jsonlPath, transcriptPath); err != nil {
		s.logger.Warn("transcript generation failed", "session_id", id, "path", transcriptPath, "err", err)
	}
}

func (s *Server) wrapperMessageLoop(conn *websocket.Conn, id int) {
	for {
		var m proto.Message
		if err := websocket.JSON.Receive(conn, &m); err != nil {
			s.logger.Debug("wrapper WS closed", "session_id", id, "err", err)
			break
		}
		m.SessionID = id
		switch m.Type {
		case "pty_data":
			s.writeHistory(id, map[string]any{
				"ts":         time.Now().Format(time.RFC3339),
				"type":       "pty_output",
				"session_id": id,
				"data_b64":   sessionlog.EncodeBase64([]byte(sessionlog.MaskSecrets(string(m.Data)))),
				"text":       sessionlog.MaskSecrets(sessionlog.StripANSI(string(m.Data))),
			})
			var provider string
			var vtLines []string
			var debounceUntil time.Time
			s.mu.Lock()
			if ses := s.sessions[id]; ses != nil {
				ses.ptyBuf = append(ses.ptyBuf, m.Data...)
				if len(ses.ptyBuf) > maxPTYBuf {
					ses.ptyBuf = ses.ptyBuf[len(ses.ptyBuf)-maxPTYBuf:]
				}
				if ses.vt == nil {
					ses.vt = newVTBuffer(ses.lastCols, ses.lastRows)
				}
				ses.vt.Write(m.Data)
				vtLines = ses.vt.TailLines(vtTailLinesForApproval)
				provider = ses.Provider
				debounceUntil = ses.vtResizeDebounceUntil
			}
			s.mu.Unlock()
			s.broadcast(m)
			if provider != "" && time.Now().After(debounceUntil) {
				s.handleNativeApprovalDetection(id, detectNativeApproval(provider, vtLines))
			}
			s.markRunning(id)
			s.detectModelChange(id, m.Data)
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
				s.mu.Lock()
				if cur := s.sessions[id]; cur != nil {
					cur.State = m.State
					if m.Reason != "" {
						cur.EndReason = m.Reason
					}
				}
				s.mu.Unlock()
			}
		}
	}

	// wrapper 切断
	s.mu.Lock()
	delete(s.wrappers, id)
	var historyToClose *sessionlog.Writer
	var jsonlPathForTranscript string
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
	}
	s.mu.Unlock()
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
	s.finalizeTranscript(id, jsonlPathForTranscript)
	s.broadcast(proto.Message{Type: "session_end", SessionID: id, State: endState, Reason: endReason})
}

func (s *Server) handleNativeApprovalDetection(id int, approval *nativeApproval) {
	now := time.Now()
	var msg *proto.Message
	s.mu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.mu.Unlock()
		return
	}
	if now.Before(ses.vtResizeDebounceUntil) {
		s.mu.Unlock()
		return
	}
	if approval == nil {
		if ses.nativeApprovalSig != "" {
			sig := ses.nativeApprovalSig
			ses.nativeApprovalSig = ""
			msg = &proto.Message{
				Type:           "approval_cleared",
				SessionID:      id,
				Provider:       ses.Provider,
				ApprovalSig:    sig,
				ApprovalSource: approvalSourceGoVT,
			}
		}
		s.mu.Unlock()
		if msg != nil {
			s.broadcast(*msg)
		}
		return
	}
	if ses.nativeApprovalConsumed == approval.Sig && now.Sub(ses.nativeApprovalConsumedAt) < approvalConsumedTTL {
		s.mu.Unlock()
		return
	}
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
	s.mu.Unlock()
	if msg != nil {
		s.broadcast(*msg)
	}
}

func (s *Server) markNativeApprovalConsumed(m proto.Message) {
	if m.SessionID <= 0 || m.ApprovalSig == "" {
		return
	}
	now := time.Now()
	var clearMsg *proto.Message
	s.mu.Lock()
	ses := s.sessions[m.SessionID]
	if ses == nil {
		s.mu.Unlock()
		return
	}
	ses.nativeApprovalConsumed = m.ApprovalSig
	ses.nativeApprovalConsumedAt = now
	if ses.nativeApprovalSig == m.ApprovalSig {
		ses.nativeApprovalSig = ""
		clearMsg = &proto.Message{
			Type:           "approval_cleared",
			SessionID:      m.SessionID,
			Provider:       ses.Provider,
			ApprovalSig:    m.ApprovalSig,
			ApprovalSource: approvalSourceGoVT,
		}
	}
	s.mu.Unlock()
	if clearMsg != nil {
		s.broadcast(*clearMsg)
	}
}

// detectModelChange は PTY 出力からモデル変更を検出し、
// セッションの Model フィールドを更新して UI に session_update を送る。
// Claude Code の "Set model to <name>" / Codex CLI の "Model changed to <name>" を対象とする。
func (s *Server) detectModelChange(id int, data []byte) {
	text := sessionlog.StripANSI(string(data))
	s.mu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.mu.Unlock()
		return
	}
	provider := ses.Provider
	s.mu.Unlock()

	var match []string
	switch provider {
	case "claude":
		match = reSetModelTo.FindStringSubmatch(text)
	case "codex":
		match = reCodexModelChanged.FindStringSubmatch(text)
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
	newRoute := s.resolveRoute(provider, newModel)
	s.mu.Lock()
	ses = s.sessions[id]
	if ses == nil || ses.Model == newModel {
		s.mu.Unlock()
		return
	}
	ses.Model = newModel
	ses.Route = newRoute
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
	s.mu.Unlock()
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
			if m.Cols > 0 && m.Rows > 0 {
				s.mu.Lock()
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
				s.mu.Unlock()
				if wc != nil && !skip {
					_ = websocket.JSON.Send(wc, m)
				}
			}
		case "pty_input":
			// UI から Text フィールドで受け取り、wrapper には Data ([]byte) に変換して転送
			s.mu.Lock()
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
					ses.ptyBuf = append(ses.ptyBuf, marker...)
					if len(ses.ptyBuf) > maxPTYBuf {
						ses.ptyBuf = ses.ptyBuf[len(ses.ptyBuf)-maxPTYBuf:]
					}
					injectMarker = true
				}
			}
			s.mu.Unlock()
			if injectMarker {
				s.broadcast(proto.Message{Type: "pty_data", SessionID: m.SessionID, Data: []byte(chatHistoryUserTurnMarker)})
			}
			if wc != nil {
				_ = websocket.JSON.Send(wc, proto.Message{Type: "pty_input", SessionID: m.SessionID, Data: []byte(combined)})
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
		case "session_hint":
			s.mu.Lock()
			ses := s.sessions[m.SessionID]
			if ses != nil {
				ses.approvalVisible = m.ApprovalVisible
			}
			s.mu.Unlock()
		case "approval_consumed":
			s.markNativeApprovalConsumed(m)
		case "session_history_reset":
			s.mu.Lock()
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
			s.mu.Unlock()
			for _, id := range ids {
				s.writeHistory(id, map[string]any{
					"ts":         time.Now().Format(time.RFC3339),
					"type":       "session_history_reset",
					"session_id": id,
				})
			}
			if m.SessionID > 0 && len(ids) == 0 {
				continue
			}
			s.broadcast(proto.Message{Type: "session_history_reset", SessionID: m.SessionID})
			for _, update := range updates {
				s.broadcast(update)
			}
		case "session_dismiss":
			s.mu.Lock()
			wc := s.wrappers[m.SessionID]
			_, exists := s.sessions[m.SessionID]
			var historyToClose *sessionlog.Writer
			var jsonlPathForTranscript string
			if exists {
				historyToClose = s.sessions[m.SessionID].History
				jsonlPathForTranscript = s.sessions[m.SessionID].JSONLPath
				s.sessions[m.SessionID].History = nil
				delete(s.sessions, m.SessionID)
				delete(s.wrappers, m.SessionID)
			}
			s.mu.Unlock()
			if !exists {
				continue
			}
			if historyToClose != nil {
				_ = historyToClose.Event(map[string]any{
					"ts":         time.Now().Format(time.RFC3339),
					"type":       "session_dismiss",
					"session_id": m.SessionID,
				})
			}
			if wc != nil {
				wc.Close()
			}
			if historyToClose != nil {
				_ = historyToClose.Close()
			}
			s.finalizeTranscript(m.SessionID, jsonlPathForTranscript)
			s.broadcast(proto.Message{Type: "session_removed", SessionID: m.SessionID})
		case "attach_request":
			if m.ImageData == "" {
				s.logger.Warn("attach_request: missing image_data", "session_id", m.SessionID)
				continue
			}
			imgData, err := base64.StdEncoding.DecodeString(m.ImageData)
			if err != nil {
				s.logger.Warn("attach_request: failed to decode base64", "session_id", m.SessionID, "err", err)
				continue
			}

			// セッション情報（provider）を mutex 保護で取得
			s.mu.Lock()
			var provider string
			if ses := s.sessions[m.SessionID]; ses != nil {
				provider = ses.Provider
			}
			s.mu.Unlock()

			// attachments ディレクトリは ~/.any-ai-cli/attachments
			homeDir, err := os.UserHomeDir()
			if err != nil {
				s.logger.Warn("attach_request: os.UserHomeDir failed", "err", err)
				continue
			}
			attachDir := filepath.Join(homeDir, ".any-ai-cli", "attachments")

			savedPath, _, err := attach.Save(attachDir, m.SessionID, provider, imgData, m.Filename)
			if err != nil {
				s.logger.Warn("attach_request: Save failed", "session_id", m.SessionID, "err", err)
				continue
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
		}
	}
}

// markRunning は PTY 出力受信時に呼ばれ、状態を running に更新して
// lastOutputAt を現在時刻に進める。状態遷移があった場合のみ broadcast する。
// approvalVisible=true の間は running への強制遷移を行わない（カーソルブリンク等の
// 継続的な PTY データで "待機中" 判定が阻害されるのを防ぐ）。
func (s *Server) markRunning(id int) {
	s.mu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.mu.Unlock()
		return
	}
	now := time.Now()
	ses.lastOutputAt = now
	ses.LastOutputAt = now.Format(time.RFC3339)
	if ses.approvalVisible {
		s.mu.Unlock()
		return
	}
	changed := ses.State != "running"
	if changed {
		ses.State = "running"
	}
	provider, display, cwd, branch, label, model, route, lastOutputAt := ses.Provider, ses.Display, ses.CWD, ses.Branch, ses.Label, ses.Model, ses.Route, ses.LastOutputAt
	s.mu.Unlock()
	if changed {
		s.broadcast(proto.Message{Type: "session_update", SessionID: id, Provider: provider, Display: display, CWD: cwd, Branch: branch, Label: label, Model: model, Route: route, State: "running", LastOutputAt: lastOutputAt})
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
	s.mu.Lock()
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
	}
	type branchCheck struct {
		id  int
		cwd string
	}
	var changes []change
	var branchChecks []branchCheck
	for id, ses := range s.sessions {
		if now.Sub(ses.branchCheckedAt) >= branchRefreshAfter {
			ses.branchCheckedAt = now
			branchChecks = append(branchChecks, branchCheck{id: id, cwd: ses.CWD})
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
			changes = append(changes, change{id: id, provider: ses.Provider, display: ses.Display, cwd: ses.CWD, branch: ses.Branch, label: ses.Label, model: ses.Model, route: ses.Route, state: newState, lastOutputAt: ses.LastOutputAt})
		}
	}
	s.mu.Unlock()
	for _, c := range changes {
		s.broadcast(proto.Message{Type: "session_update", SessionID: c.id, Provider: c.provider, Display: c.display, CWD: c.cwd, Branch: c.branch, Label: c.label, Model: c.model, Route: c.route, State: c.state, LastOutputAt: c.lastOutputAt})
	}
	for _, bc := range branchChecks {
		s.refreshBranch(bc.id, bc.cwd)
	}
}

func (s *Server) refreshBranch(id int, cwd string) {
	branch := gitBranch(cwd)
	s.mu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses.CWD != cwd || ses.Branch == branch {
		s.mu.Unlock()
		return
	}
	ses.Branch = branch
	msg := proto.Message{
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
	}
	s.mu.Unlock()
	s.broadcast(msg)
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
	s.mu.Lock()
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
	s.mu.Unlock()
	s.logger.Info("UI connected", "ui_count", count, "active_session", activeSessionID)
	return uc, items
}

func (s *Server) removeUI(c *websocket.Conn) {
	s.mu.Lock()
	uc, ok := s.uis[c]
	if !ok {
		s.mu.Unlock()
		return
	}
	delete(s.uis, c)
	count := len(s.uis)
	if count == 0 {
		s.startIdleTimerLocked()
	}
	s.mu.Unlock()
	// Ensure the underlying TCP connection is closed so that any goroutine
	// blocked on Receive (e.g. uiLoop) unblocks and exits.
	uc.close()
	s.logger.Info("UI disconnected", "ui_count", count)
}

func (s *Server) startIdleTimerLocked() {
	min := s.cfg.Hub.IdleTimeoutMin
	if min <= 0 || s.idleTimer != nil {
		return
	}
	s.idleGen++
	gen := s.idleGen
	d := time.Duration(min) * time.Minute
	s.idleTimer = time.AfterFunc(d, func() {
		s.mu.Lock()
		if s.idleGen != gen {
			// A newer timer was started (UI reconnected) or the timer was
			// stopped; skip the kill to avoid evicting a just-reconnected UI.
			s.mu.Unlock()
			return
		}
		s.idleTimer = nil
		s.mu.Unlock()
		s.logger.Info("idle timeout reached, killing all wrappers", "minutes", min)
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
	s.mu.Lock()
	list := make([]*session, 0, len(s.sessions))
	for _, ses := range s.sessions {
		list = append(list, ses)
	}
	s.mu.Unlock()
	b, _ := json.Marshal(list)
	_ = uc.send(map[string]any{"type": "snapshot", "sessions": json.RawMessage(b)})
}

func (s *Server) broadcast(m any) {
	s.mu.Lock()
	ucs := make([]*uiConn, 0, len(s.uis))
	for _, uc := range s.uis {
		ucs = append(ucs, uc)
	}
	s.mu.Unlock()
	for _, uc := range ucs {
		if err := uc.send(m); err != nil {
			s.logger.Warn("broadcast: UI send failed, removing dead connection", "err", err)
			s.removeUI(uc.ws)
		}
	}
}

func (s *Server) handleInfo(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	w.Header().Set("Content-Type", "application/json")
	mode := runtimeMode()

	userAvatar := s.cfg.UserPrefs.Avatar
	if userAvatar != "" && !strings.HasPrefix(userAvatar, "http://") && !strings.HasPrefix(userAvatar, "https://") {
		userAvatar = fmt.Sprintf("/api/avatar?token=%s", s.cfg.Token)
	}
	userDisplayName := s.cfg.UserPrefs.DisplayName
	if userDisplayName == "" {
		if v := os.Getenv("USERNAME"); v != "" {
			userDisplayName = v
		} else {
			userDisplayName = os.Getenv("USER")
		}
	}

	_ = json.NewEncoder(w).Encode(map[string]any{
		"cwd":             s.hubCWD,
		"version":         s.version,
		"runtime_mode":    mode,
		"runtime_label":   runtimeLabel(mode),
		"userAvatar":      userAvatar,
		"userDisplayName": userDisplayName,
	})
}

func (s *Server) handleAvatar(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	path := s.cfg.UserPrefs.Avatar
	if path == "" || strings.HasPrefix(path, "http://") || strings.HasPrefix(path, "https://") {
		http.NotFound(w, r)
		return
	}
	// パストラバーサル防止: avatar は固定ディレクトリ（~/.any-ai-cli/）配下のみ許可する。
	allowedDir, err := config.Dir()
	if err != nil {
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}
	cleanPath := filepath.Clean(path)
	if ok, _ := isPathUnderAllowedRoots(cleanPath, allowedDir); !ok {
		http.Error(w, "forbidden", http.StatusForbidden)
		return
	}
	data, err := os.ReadFile(cleanPath)
	if err != nil {
		http.NotFound(w, r)
		return
	}
	ct := http.DetectContentType(data)
	w.Header().Set("Content-Type", ct)
	w.Header().Set("Cache-Control", "max-age=3600")
	_, _ = w.Write(data)
}

func (s *Server) handleKillAll(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if !s.requireToken(w, r) {
		return
	}
	s.killAllWrappers()
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
}

func (s *Server) killAllWrappers() {
	s.mu.Lock()
	conns := make([]*websocket.Conn, 0, len(s.wrappers))
	for _, conn := range s.wrappers {
		conns = append(conns, conn)
	}
	s.mu.Unlock()
	for _, conn := range conns {
		_ = conn.Close()
	}
}

func (s *Server) handleShutdown(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if !s.requireToken(w, r) {
		return
	}
	s.logger.Info("shutdown requested via UI")
	s.broadcastHubShutdown("ui_shutdown")
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
	go s.requestStop()
}

// broadcastHubShutdown は接続中の全 wrapper に hub_shutdown を通知し、
// 「これは Hub クラッシュではなく意図的シャットダウンだ」ことを伝える。
// 通知を受けた wrapper は reconnect grace に入らず ensureHub を呼ばないので、
// CREATE_NEW_CONSOLE による Hub 復活ターミナル窓のポップアップが発生しない。
func (s *Server) broadcastHubShutdown(reason string) {
	s.mu.Lock()
	conns := make([]*websocket.Conn, 0, len(s.wrappers))
	for _, conn := range s.wrappers {
		conns = append(conns, conn)
	}
	s.mu.Unlock()
	msg := proto.Message{Type: "hub_shutdown", Reason: reason}
	for _, conn := range conns {
		_ = conn.SetWriteDeadline(time.Now().Add(500 * time.Millisecond))
		_ = websocket.JSON.Send(conn, msg)
	}
}

func (s *Server) requestStop() {
	time.Sleep(100 * time.Millisecond)
	s.stopMu.Lock()
	stop := s.stopFunc
	s.stopMu.Unlock()
	if stop != nil {
		stop()
	}
}

// persistConfig takes a snapshot of s.cfg under s.mu and saves it to disk
// outside the lock to avoid holding s.mu during file I/O and to prevent
// concurrent map iteration/write panics in yaml.Marshal.
func (s *Server) persistConfig() error {
	s.mu.Lock()
	snap := s.cfg.Clone()
	s.mu.Unlock()
	return config.Save(snap)
}

func (s *Server) handleAttach(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if !s.requireToken(w, r) {
		return
	}
	if err := r.ParseMultipartForm(10 << 20); err != nil {
		http.Error(w, "bad request: "+err.Error(), http.StatusBadRequest)
		return
	}
	sessionID, err := strconv.Atoi(r.FormValue("session_id"))
	if err != nil {
		http.Error(w, "invalid session_id", http.StatusBadRequest)
		return
	}
	file, header, err := r.FormFile("file")
	if err != nil {
		http.Error(w, "missing file", http.StatusBadRequest)
		return
	}
	defer file.Close()
	data, err := io.ReadAll(file)
	if err != nil {
		http.Error(w, "read error: "+err.Error(), http.StatusInternalServerError)
		return
	}
	s.mu.Lock()
	var provider string
	if ses := s.sessions[sessionID]; ses != nil {
		provider = ses.Provider
	}
	s.mu.Unlock()
	if provider == "" {
		http.Error(w, "session not found", http.StatusNotFound)
		return
	}
	homeDir, err := os.UserHomeDir()
	if err != nil {
		http.Error(w, "home dir error", http.StatusInternalServerError)
		return
	}
	attachDir := filepath.Join(homeDir, ".any-ai-cli", "attachments")
	savedPath, inject, err := attach.Save(attachDir, sessionID, provider, data, header.Filename)
	if err != nil {
		http.Error(w, "save error: "+err.Error(), http.StatusInternalServerError)
		return
	}
	s.logger.Info("attach saved via HTTP", "session_id", sessionID, "path", savedPath)
	s.writeHistory(sessionID, map[string]any{
		"ts":         time.Now().Format(time.RFC3339),
		"type":       "attach",
		"session_id": sessionID,
		"path":       savedPath,
		"filename":   header.Filename,
		"provider":   provider,
	})
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{
		"ok":         true,
		"inject":     inject,
		"saved_path": savedPath,
		"filename":   header.Filename,
	})
}

func (s *Server) handlePickDirectory(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	path, err := pickDirectoryNative()
	if err != nil {
		http.Error(w, "pick error: "+err.Error(), http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	if path == "" {
		_ = json.NewEncoder(w).Encode(map[string]any{"ok": false})
		return
	}
	_ = json.NewEncoder(w).Encode(map[string]any{"ok": true, "path": path})
}

// handlePathExists は UI の cwd 入力欄/履歴ドロップダウン向けに、複数パスが
// 「実在するディレクトリ」かをまとめて判定して返す。
// POST {"paths": ["C:\\dev\\foo", ...]} → {"results": {"C:\\dev\\foo": true, ...}}
//
// Spawn 時の Cmd.Dir に渡すと Windows では存在しないディレクトリで
// CreateProcess が ERROR_DIRECTORY を返して分かりにくいので、事前に弾く用途。
func (s *Server) handlePathExists(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var body struct {
		Paths []string `json:"paths"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		http.Error(w, "bad request", http.StatusBadRequest)
		return
	}
	results := make(map[string]bool, len(body.Paths))
	for _, p := range body.Paths {
		if p == "" {
			results[p] = false
			continue
		}
		info, err := os.Stat(p)
		results[p] = err == nil && info.IsDir()
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{"results": results})
}

func PrintStatus(cfg *config.Config) error {
	url := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", cfg.Hub.Port, cfg.Token)
	resp, err := http.Get(url)
	if err != nil {
		fmt.Println("stopped")
		return nil
	}
	defer resp.Body.Close()
	if resp.StatusCode == 200 {
		fmt.Println("running", url)
		return nil
	}
	fmt.Println("stopped")
	return nil
}

// handleOpenDir opens a directory or reveals a file in the OS file manager.
//
// Security:
//   - token required
//   - request must come from a loopback address (defense-in-depth on top of the
//     127.0.0.1 bind that NewServer already enforces)
//   - kind "log"/"attach": only the configured log_dir or attach_dir is permitted;
//     arbitrary paths are rejected so an XSS in the UI cannot turn this into "open any folder"
//   - kind "path": arbitrary absolute paths are permitted; risk is accepted because
//     token auth + loopback-only binding limits exposure, and the operation is "reveal
//     in folder" (not "execute"), which has limited blast radius
func (s *Server) handleOpenDir(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		http.Error(w, "forbidden", http.StatusForbidden)
		return
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		http.Error(w, "forbidden: loopback only", http.StatusForbidden)
		return
	}
	var body struct {
		Kind string `json:"kind"` // "log", "attach", or "path"
		Path string `json:"path"` // kind=="path" のみ使用
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		http.Error(w, "bad request", http.StatusBadRequest)
		return
	}
	if body.Kind == "path" {
		if !filepath.IsAbs(body.Path) {
			http.Error(w, "path must be absolute", http.StatusBadRequest)
			return
		}
		if err := openRevealNative(body.Path); err != nil {
			http.Error(w, "open failed: "+err.Error(), http.StatusInternalServerError)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{"ok": true, "path": body.Path})
		return
	}
	var target string
	switch body.Kind {
	case "log":
		s.mu.Lock()
		target = s.cfg.Hub.LogDir
		s.mu.Unlock()
	case "attach":
		home, err := os.UserHomeDir()
		if err != nil {
			http.Error(w, "home dir unavailable", http.StatusInternalServerError)
			return
		}
		target = filepath.Join(home, ".any-ai-cli", "attachments")
	default:
		http.Error(w, "unknown kind", http.StatusBadRequest)
		return
	}
	if target == "" {
		http.Error(w, "target dir not configured", http.StatusInternalServerError)
		return
	}
	if err := os.MkdirAll(target, 0o755); err != nil {
		http.Error(w, "mkdir failed: "+err.Error(), http.StatusInternalServerError)
		return
	}
	if err := openDirNative(target); err != nil {
		http.Error(w, "open failed: "+err.Error(), http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{"ok": true, "path": target})
}

func (s *Server) handleApprovalPatterns(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	s.mu.Lock()
	profiles := s.cfg.ApprovalProfiles
	s.mu.Unlock()
	patterns, err := ReadActiveApprovalPatterns(profiles)
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(patterns)
}

func (s *Server) handleApprovalPatternsItem(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	rest := strings.TrimPrefix(r.URL.Path, "/api/approval-patterns/")
	switch rest {
	case "profile":
		s.handleApprovalProfile(w, r)
		return
	case "copy-official":
		s.handleApprovalCopyOfficial(w, r)
		return
	}
	parts := strings.SplitN(rest, "/", 2)
	provider := parts[0]
	if provider == "" || !IsKnownApprovalProvider(provider) {
		http.Error(w, "unknown provider", http.StatusNotFound)
		return
	}
	if r.Method != http.MethodPut {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var list []string
	if err := json.NewDecoder(r.Body).Decode(&list); err != nil {
		http.Error(w, "bad request: "+err.Error(), http.StatusBadRequest)
		return
	}
	cleaned := make([]string, 0, len(list))
	for _, item := range list {
		v := strings.TrimSpace(item)
		if v != "" {
			cleaned = append(cleaned, v)
		}
	}
	if err := WriteCustomApprovalPatterns(provider, cleaned); err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	s.refreshActiveMirror()
	w.WriteHeader(http.StatusNoContent)
}

// handleApprovalProfile は GET でアクティブプロファイル一覧、POST で切替を行う。
// POST body: {"provider":"claude","profile":"official"|"custom"}
func (s *Server) handleApprovalProfile(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		s.mu.Lock()
		profiles := config.EffectiveApprovalProfiles(s.cfg.ApprovalProfiles)
		s.mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(profiles)
	case http.MethodPost:
		var body struct {
			Provider string `json:"provider"`
			Profile  string `json:"profile"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(w, "bad request: "+err.Error(), http.StatusBadRequest)
			return
		}
		if !IsKnownApprovalProvider(body.Provider) {
			http.Error(w, "unknown provider", http.StatusBadRequest)
			return
		}
		profile := config.ApprovalProfileName(body.Profile)
		if !IsValidApprovalProfile(profile) {
			http.Error(w, "invalid profile", http.StatusBadRequest)
			return
		}
		s.mu.Lock()
		s.cfg.ApprovalProfiles = config.EffectiveApprovalProfiles(s.cfg.ApprovalProfiles).WithProvider(body.Provider, profile)
		s.mu.Unlock()
		if err := s.persistConfig(); err != nil {
			s.logger.Warn("save config failed", "err", err)
		}
		s.refreshActiveMirror()
		w.WriteHeader(http.StatusNoContent)
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

// handleApprovalCopyOfficial は official → custom コピーを行う。
// POST body: {"provider":"claude"}
func (s *Server) handleApprovalCopyOfficial(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	var body struct {
		Provider string `json:"provider"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		http.Error(w, "bad request: "+err.Error(), http.StatusBadRequest)
		return
	}
	if !IsKnownApprovalProvider(body.Provider) {
		http.Error(w, "unknown provider", http.StatusBadRequest)
		return
	}
	if err := CopyOfficialToCustom(body.Provider); err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	s.refreshActiveMirror()
	w.WriteHeader(http.StatusNoContent)
}

// refreshActiveMirror はアクティブプロファイル内容を <provider>.json に再書き出しする。
// プロファイル切替・custom 上書き・official 更新後に呼ぶ。失敗時は warn ログのみ。
func (s *Server) refreshActiveMirror() {
	s.mu.Lock()
	profiles := s.cfg.ApprovalProfiles
	s.mu.Unlock()
	if err := RefreshActiveMirrors(profiles); err != nil {
		s.logger.Warn("refresh approval pattern mirrors failed", "err", err)
	}
}

func Stop(cfg *config.Config) error {
	pidPath := filepath.Join(os.TempDir(), "any-ai-cli.pid")
	b, err := os.ReadFile(pidPath)
	if err != nil {
		return fmt.Errorf("hub pid not found")
	}
	var pid int
	_, _ = fmt.Sscanf(string(b), "%d", &pid)
	p, err := os.FindProcess(pid)
	if err != nil {
		return err
	}
	err = p.Kill()
	_ = os.Remove(pidPath)
	return err
}

// killStalePid reads the PID file and kills the process if it is still running.
// Errors are silently ignored — the goal is best-effort cleanup before a new serve.
func killStalePid(pidPath string) {
	b, err := os.ReadFile(pidPath)
	if err != nil {
		return
	}
	var pid int
	if _, err := fmt.Sscanf(string(b), "%d", &pid); err != nil {
		_ = os.Remove(pidPath)
		return
	}
	if p, err := os.FindProcess(pid); err == nil {
		_ = p.Kill()
	}
	_ = os.Remove(pidPath)
}

func (s *Server) handlePickFile(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	filterExe := r.URL.Query().Get("filter") == "exe"
	path, err := pickFileNative(filterExe)
	if err != nil {
		http.Error(w, "pick error: "+err.Error(), http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	if path == "" {
		_ = json.NewEncoder(w).Encode(map[string]any{"ok": false})
		return
	}
	_ = json.NewEncoder(w).Encode(map[string]any{"ok": true, "path": path})
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
	s.mu.Lock()
	ses := s.sessions[sessionID]
	var w *sessionlog.Writer
	if ses != nil {
		w = ses.History
	}
	s.mu.Unlock()
	if w == nil {
		return
	}
	if err := w.Event(event); err != nil {
		s.logger.Warn("session history write failed", "session_id", sessionID, "err", err)
	}
}
