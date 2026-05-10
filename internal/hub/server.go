package hub

import (
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
	"strconv"
	"strings"
	"sync"
	"time"

	"ai-cli-hub/internal/attach"
	"ai-cli-hub/internal/config"
	"ai-cli-hub/internal/proto"
	"ai-cli-hub/internal/sessionlog"
	"ai-cli-hub/internal/wrapper"
	"ai-cli-hub/web"
	"golang.org/x/net/websocket"
)

// idleAfter: PTY 出力が静止してから running → waiting に遷移するまでの時間。
// tickerInterval: 状態評価 ticker の間隔。
// maxPTYBuf: UI 再接続時リプレイ用の PTY バッファ上限（セッションごと）。
// uiPingInterval: UI WebSocket keepalive ping の送信間隔。
const (
	idleAfter      = 500 * time.Millisecond
	tickerInterval = 200 * time.Millisecond
	maxPTYBuf      = 512 * 1024 // 512 KB
	uiPingInterval = 30 * time.Second
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
	Label        string `json:"label,omitempty"` // UI カード 3 行目に【ラベル】として表示
	Model        string `json:"model,omitempty"` // 使用モデル名; UI カード表示用
	Shell        string `json:"shell,omitempty"`
	State        string `json:"state"`
	LastOutputAt string `json:"last_output_at,omitempty"` // ISO 8601; UI カード「最終応答時刻」用
	StartedAt    string `json:"started_at,omitempty"`     // ISO 8601; UI カード「起動時刻」用
	FirstMessage string `json:"first_message,omitempty"`  // 最初の確定入力; UI カード表示用
	LastMessage  string `json:"last_message,omitempty"`   // 最新の確定入力; UI カード表示用

	// JSON 外: 状態評価用
	lastOutputAt    time.Time // idleAfter 計算用。LastOutputAt と同期して更新する
	approvalVisible bool

	// JSON 外: UI 再接続時リプレイ用リングバッファ（末尾 maxPTYBuf bytes）
	ptyBuf []byte

	// JSON 外: wrapper に最後に送った PTY サイズ（同サイズの resize を skip して不要な SIGWINCH を防ぐ）
	lastCols int
	lastRows int

	// JSON 外: 次の pty_input 送信時に先頭に結合する inject 文字列（画像添付用）
	pendingInject string

	// JSON 外: セッション履歴（JSONL）
	LogPath   string
	JSONLPath string
	History   *sessionlog.Writer
}

func (s *session) idleStateName() string {
	if s.approvalVisible {
		return "waiting"
	}
	return "standby"
}

type Server struct {
	cfg         *config.Config
	logger      *slog.Logger
	httpSrv     *http.Server
	devMode     bool   // --dev: web/ をファイルシステムから直接サーブ（再コンパイル不要）
	hubCWD      string // serve 起動時の os.Getwd() を保存
	parentShell string

	mu       sync.Mutex
	nextID   int
	sessions map[int]*session
	wrappers map[int]*websocket.Conn
	uis      map[*websocket.Conn]struct{}

	slashCmdMu    sync.Mutex
	slashCmdCache map[string]*slashCmdCacheEntry // key: provider

	lastUICols int
	lastUIRows int
	idleTimer  *time.Timer

	stopMu   sync.Mutex
	stopFunc context.CancelFunc
}

type codexRiskSummary struct {
	HighRisk bool
}

type claudeRiskSummary struct {
	HighRisk bool
}

const (
	defaultInitCols = 200
	defaultInitRows = 50
)

func NewServer(cfg *config.Config, logger *slog.Logger, devMode bool) (*Server, error) {
	hubCWD, _ := os.Getwd()
	s := &Server{
		cfg:           cfg,
		logger:        logger,
		devMode:       devMode,
		hubCWD:        hubCWD,
		parentShell:   wrapper.DetectShell(),
		sessions:      map[int]*session{},
		wrappers:      map[int]*websocket.Conn{},
		uis:           map[*websocket.Conn]struct{}{},
		slashCmdCache: map[string]*slashCmdCacheEntry{},
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
	// 承認パターン JSON はユーザー設定ディレクトリ ~/.ai-cli-hub/approval-patterns/
	// から配信する（ユーザーが編集可能）。初回起動時にデフォルトを書き出す。
	if err := SyncApprovalPatterns(); err != nil {
		logger.Warn("sync approval patterns failed", "err", err)
	}
	approvalPatternsHandler := http.StripPrefix("/approval-patterns/",
		http.FileServer(http.Dir(approvalPatternsDir())))
	mux := http.NewServeMux()
	mux.HandleFunc("/", s.handleIndex)
	mux.Handle("/app.js", staticHandler)
	mux.Handle("/styles.css", staticHandler)
	mux.Handle("/icon.svg", staticHandler)
	mux.Handle("/i18n.js", staticHandler)
	mux.Handle("/i18n/", staticHandler)
	mux.Handle("/vendor/", staticHandler)
	mux.Handle("/approval-patterns/", approvalPatternsHandler)
	mux.Handle("/ws", websocket.Handler(s.handleWS))
	mux.HandleFunc("/api/info", s.handleInfo)
	mux.HandleFunc("/api/spawn", s.handleSpawn)
	mux.HandleFunc("/api/pick-directory", s.handlePickDirectory)
	mux.HandleFunc("/api/kill-all", s.handleKillAll)
	mux.HandleFunc("/api/shutdown", s.handleShutdown)
	mux.HandleFunc("/api/log-config", s.handleLogConfig)
	mux.HandleFunc("/api/open-dir", s.handleOpenDir)
	mux.HandleFunc("/api/idle-timeout", s.handleIdleTimeout)
	mux.HandleFunc("/api/reconnect-grace", s.handleReconnectGrace)
	mux.HandleFunc("/api/approval/status", s.handleApprovalStatus)
	mux.HandleFunc("/api/approval/enable", s.handleApprovalEnable)
	mux.HandleFunc("/api/approval/disable", s.handleApprovalDisable)
	mux.HandleFunc("/api/approval/dismiss", s.handleApprovalDismiss)
	mux.HandleFunc("/api/attach", s.handleAttach)
	mux.HandleFunc("/api/slash-cmd-sources", s.handleSlashCmdSources)
	mux.HandleFunc("/api/slash-commands", s.handleSlashCommands)
	mux.HandleFunc("/api/approval-patterns", s.handleApprovalPatterns)
	mux.HandleFunc("/api/approval-patterns/", s.handleApprovalPatternsItem)
	s.httpSrv = &http.Server{Addr: fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port), Handler: mux}
	return s, nil
}

func (s *Server) Run(ctx context.Context) error {
	runCtx, cancel := context.WithCancel(ctx)
	s.stopMu.Lock()
	s.stopFunc = cancel
	s.stopMu.Unlock()

	pidPath := filepath.Join(os.TempDir(), "ai-cli-hub.pid")
	killStalePid(pidPath)

	ln, err := net.Listen("tcp", s.httpSrv.Addr)
	if err != nil {
		return err
	}
	_ = os.WriteFile(pidPath, []byte(fmt.Sprintf("%d", os.Getpid())), 0o644)
	setConsoleTitle("ai-cli-hub [hub]")
	setConsoleIcon()
	s.logger.Info("AI-CLI-HUB started", "url", fmt.Sprintf("http://%s/?token=%s", s.httpSrv.Addr, s.cfg.Token))
	fmt.Printf("AI-CLI-HUB started: http://%s/?token=%s\n", s.httpSrv.Addr, s.cfg.Token)
	if s.cfg.Approval.Enabled {
		s.injectApprovalRules()
	}
	go s.stateTicker(runCtx)
	go s.cleanAttachments()
	go func() {
		<-runCtx.Done()
		s.killAllWrappers()
		if s.cfg.Approval.Enabled {
			s.removeApprovalRules()
		}
		_ = s.httpSrv.Shutdown(context.Background())
		_ = os.Remove(pidPath)
	}()
	err = s.httpSrv.Serve(ln)
	if errors.Is(err, http.ErrServerClosed) {
		return nil
	}
	return err
}

// cleanAttachments removes attachment files older than 7 days and then prunes
// any session directories that are now empty.
func (s *Server) cleanAttachments() {
	home, err := os.UserHomeDir()
	if err != nil {
		return
	}
	attachDir := filepath.Join(home, ".ai-cli-hub", "attachments")
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

// OpenBrowserForConfig opens the browser to the Hub URL without needing a running Server.
func OpenBrowserForConfig(cfg *config.Config) error {
	url := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", cfg.Hub.Port, cfg.Token)
	cmd := exec.Command("rundll32", "url.dll,FileProtocolHandler", url)
	return cmd.Start()
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
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
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
		s.addUI(conn)
		s.sendSnapshot(conn)
		s.sendPTYHistory(conn)
		ctx, cancel := context.WithCancel(context.Background())
		go s.pingLoop(ctx, conn)
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
	history, histErr := sessionlog.NewJSONLWriter(jsonlPath)
	if histErr != nil {
		s.logger.Warn("session history create failed", "path", jsonlPath, "err", histErr)
	}

	s.mu.Lock()
	ses := &session{
		ID:        id,
		Provider:  reg.Provider,
		Display:   reg.Display,
		CWD:       reg.CWD,
		Label:     reg.Label,
		Model:     reg.Model,
		Shell:     reg.Shell,
		State:     "standby",
		StartedAt: startedAt.Format(time.RFC3339),
		LogPath:   rawLogPath,
		JSONLPath: jsonlPath,
		History:   history,
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
	s.mu.Unlock()
	_ = websocket.JSON.Send(conn, proto.Message{Type: "registered", SessionID: id, Cols: initCols, Rows: initRows, StartedAt: ses.StartedAt, LogPath: rawLogPath, JSONLPath: jsonlPath})
	s.broadcast(proto.Message{Type: "session_update", SessionID: id, Provider: reg.Provider, Display: reg.Display, CWD: reg.CWD, Label: reg.Label, Model: reg.Model, Shell: reg.Shell, State: "standby", StartedAt: ses.StartedAt})
	s.writeHistory(id, map[string]any{
		"ts":         startedAt.Format(time.RFC3339),
		"type":       "session_start",
		"session_id": id,
		"provider":   reg.Provider,
		"cwd":        reg.CWD,
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
	history, histErr := sessionlog.NewJSONLWriterAppend(jsonlPath)
	if histErr != nil {
		s.logger.Warn("session history append failed", "path", jsonlPath, "err", histErr)
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
		ID:           acceptedID,
		Provider:     req.Provider,
		Display:      req.Display,
		CWD:          req.CWD,
		Label:        req.Label,
		Model:        req.Model,
		Shell:        req.Shell,
		State:        "running",
		LastOutputAt: lastOutputAt,
		StartedAt:    startedAtText,
		lastOutputAt: lastOutputAtTime,
		ptyBuf:       replay,
		lastCols:     req.Cols,
		lastRows:     req.Rows,
		LogPath:      rawLogPath,
		JSONLPath:    jsonlPath,
		History:      history,
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
	s.broadcast(proto.Message{Type: "session_update", SessionID: acceptedID, Provider: req.Provider, Display: req.Display, CWD: req.CWD, Label: req.Label, Model: req.Model, Shell: req.Shell, State: "running", LastOutputAt: lastOutputAt, StartedAt: startedAtText})
	s.writeHistory(acceptedID, map[string]any{
		"ts":             now.Format(time.RFC3339),
		"type":           "session_reattach",
		"session_id":     acceptedID,
		"old_session_id": req.SessionID,
		"provider":       req.Provider,
		"cwd":            req.CWD,
		"label":          req.Label,
		"model":          req.Model,
		"shell":          req.Shell,
		"pid":            req.PID,
		"renumbered":     acceptedID != req.SessionID,
	})
	s.wrapperMessageLoop(conn, acceptedID)
}

func (s *Server) wrapperMessageLoop(conn *websocket.Conn, id int) {
	for {
		var m proto.Message
		if err := websocket.JSON.Receive(conn, &m); err != nil {
			s.logger.Info("wrapper WS closed", "session_id", id, "err", err)
			break
		}
		m.SessionID = id
		switch m.Type {
		case "pty_data":
			s.writeHistory(id, map[string]any{
				"ts":         time.Now().Format(time.RFC3339),
				"type":       "pty_output",
				"session_id": id,
				"data_b64":   sessionlog.EncodeBase64(m.Data),
				"text":       sessionlog.StripANSI(string(m.Data)),
			})
			s.mu.Lock()
			if ses := s.sessions[id]; ses != nil {
				ses.ptyBuf = append(ses.ptyBuf, m.Data...)
				if len(ses.ptyBuf) > maxPTYBuf {
					ses.ptyBuf = ses.ptyBuf[len(ses.ptyBuf)-maxPTYBuf:]
				}
			}
			s.mu.Unlock()
			s.broadcast(m)
			s.markRunning(id)
		case "session_end":
			s.writeHistory(id, map[string]any{
				"ts":         time.Now().Format(time.RFC3339),
				"type":       "session_end",
				"session_id": id,
				"state":      m.State,
				"exit_code":  m.ExitCode,
			})
			if m.State == "completed" || m.State == "error" {
				s.mu.Lock()
				if cur := s.sessions[id]; cur != nil {
					cur.State = m.State
				}
				s.mu.Unlock()
			}
		}
	}

	// wrapper 切断
	s.mu.Lock()
	delete(s.wrappers, id)
	var historyToClose *sessionlog.Writer
	if cur := s.sessions[id]; cur != nil && cur.State != "completed" && cur.State != "error" {
		cur.State = "disconnected"
	}
	endState := "disconnected"
	if cur := s.sessions[id]; cur != nil {
		endState = cur.State
		historyToClose = cur.History
		cur.History = nil
	}
	s.mu.Unlock()
	if endState == "disconnected" {
		if historyToClose != nil {
			_ = historyToClose.Event(map[string]any{
				"ts":         time.Now().Format(time.RFC3339),
				"type":       "session_end",
				"session_id": id,
				"state":      endState,
				"exit_code":  0,
			})
		}
	}
	if historyToClose != nil {
		_ = historyToClose.Close()
	}
	s.broadcast(proto.Message{Type: "session_end", SessionID: id, State: endState})
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
			// pendingInject は Codex 等の provider が使用（Claude は attach_request 時に直送）
			inject := ""
			if ses != nil && ses.pendingInject != "" {
				inject = ses.pendingInject
				ses.pendingInject = ""
			}
			combined := inject + m.Text
			var firstMsgBroadcast *proto.Message
			if ses != nil && strings.HasSuffix(m.Text, "\r") {
				text := strings.TrimRight(m.Text, "\r\n")
				if text == "/clear" {
					// /clear でセッション概要をリセット（次の入力が新しい概要になる）
					ses.FirstMessage = ""
					ses.LastMessage = ""
					msg := proto.Message{Type: "session_update", SessionID: m.SessionID, Provider: ses.Provider, Display: ses.Display, CWD: ses.CWD, Label: ses.Label, Model: ses.Model, State: ses.State, LastOutputAt: ses.LastOutputAt}
					firstMsgBroadcast = &msg
				} else if text != "" {
					if ses.FirstMessage == "" {
						ses.FirstMessage = text
					}
					// 数字のみ（選択肢番号）は LastMessage を更新しない
					if !isDigitsOnly(text) {
						ses.LastMessage = text
					}
					msg := proto.Message{Type: "session_update", SessionID: m.SessionID, Provider: ses.Provider, Display: ses.Display, CWD: ses.CWD, Label: ses.Label, Model: ses.Model, State: ses.State, LastOutputAt: ses.LastOutputAt, FirstMessage: ses.FirstMessage, LastMessage: ses.LastMessage}
					firstMsgBroadcast = &msg
				}
			}
			s.mu.Unlock()
			if wc != nil {
				_ = websocket.JSON.Send(wc, proto.Message{Type: "pty_input", SessionID: m.SessionID, Data: []byte(combined)})
			}
			s.writeHistory(m.SessionID, map[string]any{
				"ts":                  time.Now().Format(time.RFC3339),
				"type":                "user_input",
				"session_id":          m.SessionID,
				"text":                m.Text,
				"combined_has_inject": inject != "",
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
		case "session_dismiss":
			s.mu.Lock()
			wc := s.wrappers[m.SessionID]
			_, exists := s.sessions[m.SessionID]
			var historyToClose *sessionlog.Writer
			if exists {
				historyToClose = s.sessions[m.SessionID].History
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

			// attachments ディレクトリは ~/.ai-cli-hub/attachments
			homeDir, err := os.UserHomeDir()
			if err != nil {
				s.logger.Warn("attach_request: os.UserHomeDir failed", "err", err)
				continue
			}
			attachDir := filepath.Join(homeDir, ".ai-cli-hub", "attachments")

			savedPath, inject, err := attach.Save(attachDir, m.SessionID, provider, imgData, m.Filename)
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

			// claude は attach_file メッセージで wrapper へ直送し、後続の pty_input と
			// 1 チャンク化されないようにする（picker 確定の \r が「送信」扱いされて
			// 画像とテキストが別投稿に分裂するのを防ぐ）。codex 等は従来どおり pendingInject。
			s.mu.Lock()
			wc := s.wrappers[m.SessionID]
			useDirect := provider == "claude" && wc != nil
			if !useDirect {
				if ses := s.sessions[m.SessionID]; ses != nil {
					ses.pendingInject += inject
				}
			}
			s.mu.Unlock()
			if useDirect {
				_ = websocket.JSON.Send(wc, proto.Message{Type: "attach_file", SessionID: m.SessionID, Inject: inject})
			}
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
	provider, display, cwd, label, model, lastOutputAt := ses.Provider, ses.Display, ses.CWD, ses.Label, ses.Model, ses.LastOutputAt
	s.mu.Unlock()
	if changed {
		s.broadcast(proto.Message{Type: "session_update", SessionID: id, Provider: provider, Display: display, CWD: cwd, Label: label, Model: model, State: "running", LastOutputAt: lastOutputAt})
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
		label        string
		model        string
		state        string
		lastOutputAt string
	}
	var changes []change
	for id, ses := range s.sessions {
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
			changes = append(changes, change{id: id, provider: ses.Provider, display: ses.Display, cwd: ses.CWD, label: ses.Label, model: ses.Model, state: newState, lastOutputAt: ses.LastOutputAt})
		}
	}
	s.mu.Unlock()
	for _, c := range changes {
		s.broadcast(proto.Message{Type: "session_update", SessionID: c.id, Provider: c.provider, Display: c.display, CWD: c.cwd, Label: c.label, Model: c.model, State: c.state, LastOutputAt: c.lastOutputAt})
	}
}

func (s *Server) addUI(c *websocket.Conn) {
	s.mu.Lock()
	s.uis[c] = struct{}{}
	s.stopIdleTimerLocked()
	s.mu.Unlock()
}

func (s *Server) removeUI(c *websocket.Conn) {
	s.mu.Lock()
	delete(s.uis, c)
	if len(s.uis) == 0 {
		s.startIdleTimerLocked()
	}
	s.mu.Unlock()
}

func (s *Server) startIdleTimerLocked() {
	min := s.cfg.Hub.IdleTimeoutMin
	if min <= 0 || s.idleTimer != nil {
		return
	}
	d := time.Duration(min) * time.Minute
	s.idleTimer = time.AfterFunc(d, func() {
		s.logger.Info("idle timeout reached, killing all wrappers", "minutes", min)
		s.killAllWrappers()
		s.mu.Lock()
		s.idleTimer = nil
		s.mu.Unlock()
	})
}

func (s *Server) stopIdleTimerLocked() {
	if s.idleTimer == nil {
		return
	}
	s.idleTimer.Stop()
	s.idleTimer = nil
}

// pingLoop は uiPingInterval ごとに UI WebSocket へ JSON ping を送り続ける。
// keepalive として機能し、dead connection を検出したら s.uis から除去して終了する。
func (s *Server) pingLoop(ctx context.Context, conn *websocket.Conn) {
	t := time.NewTicker(uiPingInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			if err := websocket.JSON.Send(conn, map[string]string{"type": "ping"}); err != nil {
				s.logger.Warn("ping failed, removing dead UI connection", "err", err)
				s.removeUI(conn)
				return
			}
		}
	}
}

func (s *Server) sendSnapshot(c *websocket.Conn) {
	s.mu.Lock()
	list := make([]*session, 0, len(s.sessions))
	for _, ses := range s.sessions {
		list = append(list, ses)
	}
	s.mu.Unlock()
	b, _ := json.Marshal(list)
	_ = websocket.JSON.Send(c, map[string]any{"type": "snapshot", "sessions": json.RawMessage(b)})
}

// sendPTYHistory は UI 再接続時に各セッションの PTY バッファをリプレイする。
// 履歴を送るセッションは lastCols/lastRows をリセットし、UI が続けて送る
// pty_resize がスキップされないようにする（TUI 全画面再描画を促すため）。
func (s *Server) sendPTYHistory(c *websocket.Conn) {
	s.mu.Lock()
	type item struct {
		id  int
		buf []byte
	}
	var items []item
	for id, ses := range s.sessions {
		if len(ses.ptyBuf) > 0 {
			buf := make([]byte, len(ses.ptyBuf))
			copy(buf, ses.ptyBuf)
			items = append(items, item{id: id, buf: buf})
			ses.lastCols = 0
			ses.lastRows = 0
		}
	}
	s.mu.Unlock()
	for _, it := range items {
		_ = websocket.JSON.Send(c, proto.Message{Type: "pty_data", SessionID: it.id, Data: it.buf})
	}
}

func (s *Server) broadcast(m any) {
	s.mu.Lock()
	conns := make([]*websocket.Conn, 0, len(s.uis))
	for c := range s.uis {
		conns = append(conns, c)
	}
	s.mu.Unlock()
	for _, c := range conns {
		if err := websocket.JSON.Send(c, m); err != nil {
			s.logger.Warn("broadcast: UI send failed, removing dead connection", "err", err)
			s.removeUI(c)
		}
	}
}

func (s *Server) handleInfo(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]string{"cwd": s.hubCWD})
}

func (s *Server) handleKillAll(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
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
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	s.logger.Info("shutdown requested via UI")
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
	go s.requestStop()
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

func (s *Server) handleSpawn(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	var body struct {
		Provider       string `json:"provider"`
		CWD            string `json:"cwd"`
		Model          string `json:"model"`
		ModelSelection string `json:"model_selection_mode"`
		RiskConfirmed  bool   `json:"risk_confirmed"`
		Label          string `json:"label"`
		PermissionMode string `json:"permission_mode"`
		Sandbox        string `json:"sandbox"`
		AskForApproval string `json:"ask_for_approval"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		http.Error(w, "bad request", http.StatusBadRequest)
		return
	}
	if body.Provider != "claude" && body.Provider != "codex" {
		http.Error(w, "invalid provider", http.StatusBadRequest)
		return
	}
	validPermModes := map[string]bool{
		"": true, "default": true, "plan": true,
		"acceptEdits": true, "auto": true, "bypassPermissions": true,
	}
	validSandboxes := map[string]bool{
		"": true, "read-only": true, "workspace-write": true, "danger-full-access": true,
	}
	validApprovals := map[string]bool{
		"": true, "untrusted": true, "on-request": true, "never": true,
	}
	validModelSelection := map[string]bool{
		"": true, "auto": true, "explicit": true, "required": true,
	}
	if !validPermModes[body.PermissionMode] || !validSandboxes[body.Sandbox] || !validApprovals[body.AskForApproval] || !validModelSelection[body.ModelSelection] {
		http.Error(w, "bad request", http.StatusBadRequest)
		return
	}
	cwd := body.CWD
	if cwd == "" {
		cwd = s.hubCWD
	}
	exe, err := os.Executable()
	if err != nil {
		http.Error(w, "executable error: "+err.Error(), http.StatusInternalServerError)
		return
	}
	wrapArgs := []string{"wrap", body.Provider}
	resolvedModel := strings.TrimSpace(body.Model)

	if body.Label != "" {
		wrapArgs = append(wrapArgs, "--label", body.Label)
	}

	switch body.Provider {
	case "claude":
		mode := body.ModelSelection
		if mode == "" {
			mode = "auto"
		}
		currentModel := s.getLastModel("claude")
		if resolvedModel == "" {
			resolvedModel = currentModel
		}
		risk := evaluateClaudeRisk(currentModel, resolvedModel, body.PermissionMode)
		if risk.HighRisk && mode != "required" {
			mode = "required"
		}
		if mode == "required" && !body.RiskConfirmed {
			http.Error(w, "risk confirmation required", http.StatusBadRequest)
			return
		}
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
		if body.PermissionMode != "" && body.PermissionMode != "default" {
			wrapArgs = append(wrapArgs, "--permission-mode", body.PermissionMode)
		}
	case "codex":
		mode := body.ModelSelection
		if mode == "" {
			mode = "auto"
		}
		currentModel := s.getLastModel("codex")
		if resolvedModel == "" {
			resolvedModel = currentModel
		}
		risk := evaluateCodexRisk(currentModel, resolvedModel, body.Sandbox, body.AskForApproval)
		if risk.HighRisk && mode != "required" {
			mode = "required"
		}
		if mode == "required" && !body.RiskConfirmed {
			http.Error(w, "risk confirmation required", http.StatusBadRequest)
			return
		}
		if resolvedModel != "" {
			wrapArgs = append(wrapArgs, "--model", resolvedModel)
		}
		if body.Sandbox != "" {
			wrapArgs = append(wrapArgs, "--sandbox", body.Sandbox)
		}
		if body.AskForApproval != "" {
			wrapArgs = append(wrapArgs, "--ask-for-approval", body.AskForApproval)
		}
	}
	cmd := exec.Command(exe, wrapArgs...)
	cmd.Dir = cwd
	cmd.Env = append(os.Environ(), "AI_CLI_HUB=1")
	if s.parentShell != "" {
		cmd.Env = append(cmd.Env, "AI_CLI_HUB_PARENT_SHELL="+s.parentShell)
	}
	setCmdSysProcAttr(cmd)
	if err := cmd.Start(); err != nil {
		http.Error(w, "spawn error: "+err.Error(), http.StatusInternalServerError)
		return
	}
	if resolvedModel != "" {
		if err := s.setLastModel(body.Provider, resolvedModel); err != nil {
			s.logger.Warn("failed to save last model", "provider", body.Provider, "error", err)
		}
	}
	go func() { _ = cmd.Wait() }()
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
}

func (s *Server) getLastModel(provider string) string {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.cfg.Spawn.LastModel == nil {
		s.cfg.Spawn.LastModel = map[string]string{}
	}
	return strings.TrimSpace(s.cfg.Spawn.LastModel[provider])
}

func (s *Server) setLastModel(provider, model string) error {
	s.mu.Lock()
	if s.cfg.Spawn.LastModel == nil {
		s.cfg.Spawn.LastModel = map[string]string{}
	}
	s.cfg.Spawn.LastModel[provider] = model
	s.mu.Unlock()
	return config.Save(s.cfg)
}

func evaluateCodexRisk(currentModel, nextModel, sandbox, approval string) codexRiskSummary {
	modelChanged := strings.TrimSpace(nextModel) != "" && strings.TrimSpace(nextModel) != strings.TrimSpace(currentModel)
	highPermission := sandbox == "danger-full-access" || approval == "never"
	return codexRiskSummary{
		HighRisk: modelChanged || highPermission,
	}
}

func evaluateClaudeRisk(currentModel, nextModel, permissionMode string) claudeRiskSummary {
	modelChanged := strings.TrimSpace(nextModel) != "" && strings.TrimSpace(nextModel) != strings.TrimSpace(currentModel)
	highPermission := permissionMode == "bypassPermissions"
	return claudeRiskSummary{
		HighRisk: modelChanged || highPermission,
	}
}

func (s *Server) handleAttach(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
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
	attachDir := filepath.Join(homeDir, ".ai-cli-hub", "attachments")
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
	// claude は attach_file メッセージで wrapper へ直送（pendingInject 経由だと
	// 後続の pty_input と 1 チャンク化されて picker 確定の \r が送信扱いになる）。
	s.mu.Lock()
	wc := s.wrappers[sessionID]
	useDirect := provider == "claude" && wc != nil
	if !useDirect {
		if ses := s.sessions[sessionID]; ses != nil {
			ses.pendingInject += inject
		}
	}
	s.mu.Unlock()
	if useDirect {
		_ = websocket.JSON.Send(wc, proto.Message{Type: "attach_file", SessionID: sessionID, Inject: inject})
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
}

func (s *Server) handlePickDirectory(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
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

// handleOpenDir opens a directory in the OS file manager (explorer / Finder / xdg-open).
//
// Security:
//   - token required
//   - request must come from a loopback address (defense-in-depth on top of the
//     127.0.0.1 bind that NewServer already enforces)
//   - only the configured log_dir or attach_dir is permitted; arbitrary paths are
//     rejected so an XSS in the UI cannot turn this into "open any folder"
func (s *Server) handleOpenDir(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
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
		Kind string `json:"kind"` // "log" or "attach"
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		http.Error(w, "bad request", http.StatusBadRequest)
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
		target = filepath.Join(home, ".ai-cli-hub", "attachments")
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

func (s *Server) handleLogConfig(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	switch r.Method {
	case http.MethodGet:
		s.mu.Lock()
		logCfg := s.cfg.Log
		logDir := s.cfg.Hub.LogDir
		s.mu.Unlock()
		home, _ := os.UserHomeDir()
		attachDir := filepath.Join(home, ".ai-cli-hub", "attachments")
		type logConfigResp struct {
			config.LogConfig
			LogDir    string `json:"log_dir"`
			AttachDir string `json:"attach_dir"`
		}
		_ = json.NewEncoder(w).Encode(logConfigResp{logCfg, logDir, attachDir})
	case http.MethodPost:
		var body config.LogConfig
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(w, "bad request", http.StatusBadRequest)
			return
		}
		if body.MaxSizeMB < 1 {
			body.MaxSizeMB = 1
		} else if body.MaxSizeMB > 1000 {
			body.MaxSizeMB = 1000
		}
		if body.MaxBackups < 0 {
			body.MaxBackups = 0
		} else if body.MaxBackups > 100 {
			body.MaxBackups = 100
		}
		s.mu.Lock()
		s.cfg.Log = body
		s.mu.Unlock()
		if err := config.Save(s.cfg); err != nil {
			http.Error(w, "save failed: "+err.Error(), http.StatusInternalServerError)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

func (s *Server) handleIdleTimeout(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	switch r.Method {
	case http.MethodGet:
		s.mu.Lock()
		min := s.cfg.Hub.IdleTimeoutMin
		s.mu.Unlock()
		_ = json.NewEncoder(w).Encode(map[string]int{"idle_timeout_min": min})
	case http.MethodPost:
		var body struct {
			IdleTimeoutMin int `json:"idle_timeout_min"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(w, "bad request", http.StatusBadRequest)
			return
		}
		if body.IdleTimeoutMin < 0 {
			body.IdleTimeoutMin = 0
		} else if body.IdleTimeoutMin > 1440 {
			body.IdleTimeoutMin = 1440
		}
		s.mu.Lock()
		s.cfg.Hub.IdleTimeoutMin = body.IdleTimeoutMin
		if len(s.uis) > 0 {
			s.stopIdleTimerLocked()
		} else {
			s.stopIdleTimerLocked()
			s.startIdleTimerLocked()
		}
		s.mu.Unlock()
		if err := config.Save(s.cfg); err != nil {
			http.Error(w, "save failed: "+err.Error(), http.StatusInternalServerError)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

func (s *Server) handleReconnectGrace(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	switch r.Method {
	case http.MethodGet:
		s.mu.Lock()
		sec := s.cfg.Hub.WrapperReconnectGraceSec
		s.mu.Unlock()
		_ = json.NewEncoder(w).Encode(map[string]int{"wrapper_reconnect_grace_sec": sec})
	case http.MethodPost:
		var body struct {
			WrapperReconnectGraceSec int `json:"wrapper_reconnect_grace_sec"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(w, "bad request", http.StatusBadRequest)
			return
		}
		if body.WrapperReconnectGraceSec < 0 {
			body.WrapperReconnectGraceSec = 0
		} else if body.WrapperReconnectGraceSec > 86400 {
			body.WrapperReconnectGraceSec = 86400
		}
		s.mu.Lock()
		s.cfg.Hub.WrapperReconnectGraceSec = body.WrapperReconnectGraceSec
		s.mu.Unlock()
		if err := config.Save(s.cfg); err != nil {
			http.Error(w, "save failed: "+err.Error(), http.StatusInternalServerError)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

func (s *Server) handleApprovalPatterns(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	patterns, err := ReadApprovalPatterns()
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(patterns)
}

func (s *Server) handleApprovalPatternsItem(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	rest := strings.TrimPrefix(r.URL.Path, "/api/approval-patterns/")
	parts := strings.SplitN(rest, "/", 2)
	provider := parts[0]
	if provider == "" || !IsKnownApprovalProvider(provider) {
		http.Error(w, "unknown provider", http.StatusNotFound)
		return
	}
	if len(parts) == 2 && parts[1] == "reset" {
		if r.Method != http.MethodPost {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		if err := ResetApprovalPatterns(provider); err != nil {
			http.Error(w, err.Error(), http.StatusInternalServerError)
			return
		}
		w.WriteHeader(http.StatusNoContent)
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
	if err := WriteApprovalPatterns(provider, cleaned); err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

func Stop(cfg *config.Config) error {
	pidPath := filepath.Join(os.TempDir(), "ai-cli-hub.pid")
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
