package wrapper

import (
	"bytes"
	"encoding/base64"
	"flag"
	"fmt"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"sync"
	"sync/atomic"
	"time"

	"any-ai-cli/internal/config"
	"any-ai-cli/internal/proto"
	"any-ai-cli/internal/sessionlog"
	"golang.org/x/net/websocket"
	"golang.org/x/term"
)

const (
	reconnectDialInterval = 2 * time.Second
	replayBufferLimit     = 64 * 1024 // bytes of recent PTY output to replay on reconnect
	hubProbeTimeout       = 1 * time.Second
)

// reconnectGrace returns the wrapper's grace period before giving up after Hub crash.
// 0 disables reconnect entirely (legacy "kill immediately" behavior).
func reconnectGrace(cfg *config.Config) time.Duration {
	sec := cfg.Hub.WrapperReconnectGraceSec
	if sec < 0 {
		sec = 0
	}
	return time.Duration(sec) * time.Second
}

func Run(cfg *config.Config, logger *slog.Logger, provider string, args []string) error {
	fs := flag.NewFlagSet("wrap", flag.ContinueOnError)
	label := fs.String("label", "", "session label shown in UI card")
	model := fs.String("model", "", "model override")
	permissionMode := fs.String("permission-mode", "", "claude permission mode")
	sandbox := fs.String("sandbox", "", "codex sandbox mode")
	askForApproval := fs.String("ask-for-approval", "", "codex ask-for-approval")
	codexOSS := fs.Bool("codex-oss", false, "codex: use --oss to route via local Ollama daemon")
	utf8Session := fs.Bool("utf8", false, "set UTF-8 console encoding for this session (Windows only)")
	_ = fs.Parse(args)
	providerArgs := fs.Args()

	// Reconstruct provider-specific flags from wrapper-parsed flags
	var extra []string
	if *model != "" {
		extra = append(extra, "--model", *model)
	}
	switch provider {
	case "claude":
		if *permissionMode != "" && *permissionMode != "default" {
			extra = append(extra, "--permission-mode", *permissionMode)
		}
	case "codex":
		if *codexOSS {
			extra = append(extra, "--oss")
		}
		if *sandbox != "" {
			extra = append(extra, "--sandbox", *sandbox)
		}
		if *askForApproval != "" {
			extra = append(extra, "--ask-for-approval", *askForApproval)
		}
	}
	providerArgs = append(extra, providerArgs...)

	// Hub にスポーンされた場合、起動中の Hub ポートが ANY_AI_CLI_HUB_PORT で渡される。
	// config.yaml のポートより優先して使い、wrapper が別 Hub を勝手に起動するのを防ぐ。
	if portStr := os.Getenv("ANY_AI_CLI_HUB_PORT"); portStr != "" {
		if port, err2 := strconv.Atoi(portStr); err2 == nil && port > 0 {
			cfg.Hub.Port = port
		}
	}

	if err := ensureHub(cfg); err != nil {
		return err
	}
	cwd, _ := os.Getwd()
	display := map[string]string{"claude": "Claude", "codex": "Codex"}[provider]
	termCols, termRows := 0, 0
	if w, h, err := term.GetSize(int(os.Stdin.Fd())); err == nil && w > 0 && h > 0 {
		termCols, termRows = w, h
	}

	conn, reg, err := dialAndRegister(cfg, provider, display, cwd, *label, *model, termCols, termRows)
	if err != nil {
		return err
	}
	sessionID := reg.SessionID
	setConsoleTitle(fmt.Sprintf("any-ai-cli [#%d:%s]", sessionID, provider))
	setConsoleIcon()
	initCols, initRows := reg.Cols, reg.Rows
	if initCols <= 0 || initRows <= 0 {
		initCols, initRows = 200, 50
	}

	startedAt := time.Now()
	startedAtText := reg.StartedAt
	if startedAtText != "" {
		if parsed, err := time.Parse(time.RFC3339, startedAtText); err == nil {
			startedAt = parsed
		}
	} else {
		startedAtText = startedAt.Format(time.RFC3339)
	}
	rawLogPath, jsonlPath := reg.LogPath, reg.JSONLPath
	if rawLogPath == "" || jsonlPath == "" {
		rawLogPath, jsonlPath = sessionlog.Paths(cfg.Hub.LogDir, sessionlog.Metadata{
			SessionID: sessionID,
			Provider:  provider,
			CWD:       cwd,
			StartedAt: startedAt,
		})
	}
	_ = os.MkdirAll(filepath.Dir(rawLogPath), 0o755)
	lf, err := os.Create(rawLogPath)
	if err != nil {
		logger.Warn("session raw log create failed", "session_id", sessionID, "path", rawLogPath, "err", err)
	}
	if lf != nil {
		defer lf.Close()
	}

	if *utf8Session {
		applyUTF8Session()
	}
	ps, err := startProcess(provider, providerArgs, cwd, initCols, initRows)
	if err != nil {
		// Hub 側の spawn ログ (~/.any-ai-cli/logs/spawn/<provider>-<ts>.log) に
		// 何が起きたかを残し、Hub UI のセッションカード「Disconnected」表示に
		// 「reason: provider not found in PATH」等を 1 行付けるための reason
		// コードを session_end で送る。生のスタックトレースは UI に流さない。
		diagnoseStartFailure(os.Stderr, provider, providerArgs, err)
		reason := classifyStartFailure(err)
		_ = websocket.JSON.Send(conn, proto.Message{
			Type:      "session_end",
			SessionID: sessionID,
			State:     "error",
			ExitCode:  1,
			Reason:    reason,
		})
		_ = conn.Close()
		return err
	}
	defer ps.Close()

	// Shared mutable state (current conn / session id rotate on reconnect)
	var (
		stateMu     sync.Mutex
		sendMu      sync.Mutex // serialises websocket.JSON.Send calls to the current conn
		currentConn = conn
		currentSID  = sessionID
	)
	getConn := func() *websocket.Conn { stateMu.Lock(); defer stateMu.Unlock(); return currentConn }
	getSID := func() int { stateMu.Lock(); defer stateMu.Unlock(); return currentSID }
	// sendMsg sends m to the current conn under sendMu so that concurrent
	// goroutines (PTY output loop, session_end sender) never interleave frames.
	sendMsg := func(m proto.Message) {
		c := getConn()
		if c == nil {
			return
		}
		sendMu.Lock()
		defer sendMu.Unlock()
		// Re-check conn identity under sendMu: a reconnect may have swapped it.
		if cur := getConn(); cur != c {
			return
		}
		_ = websocket.JSON.Send(c, m)
	}
	swapConn := func(c *websocket.Conn, sid int) {
		stateMu.Lock()
		defer stateMu.Unlock()
		if currentConn != nil && currentConn != c {
			_ = currentConn.Close()
		}
		currentConn = c
		if sid > 0 {
			currentSID = sid
		}
	}
	clearConn := func(broken *websocket.Conn) {
		stateMu.Lock()
		defer stateMu.Unlock()
		if currentConn == broken {
			_ = currentConn.Close()
			currentConn = nil
		}
	}

	// Recent PTY output buffer: replayed to UI after a successful reconnect so
	// the new session card has context for what happened during the gap.
	var (
		replayMu  sync.Mutex
		replayBuf bytes.Buffer
	)
	appendReplay := func(chunk []byte) {
		replayMu.Lock()
		defer replayMu.Unlock()
		replayBuf.Write(chunk)
		if replayBuf.Len() > replayBufferLimit {
			replayBuf.Next(replayBuf.Len() - replayBufferLimit)
		}
	}
	snapshotReplay := func() []byte {
		replayMu.Lock()
		defer replayMu.Unlock()
		out := make([]byte, replayBuf.Len())
		copy(out, replayBuf.Bytes())
		return out
	}

	done := make(chan struct{})
	var doneOnce sync.Once
	closeDone := func() { doneOnce.Do(func() { close(done) }) }

	// hub_shutdown を受信したら立つフラグ。意図的シャットダウンと判定し、
	// reconnect supervisor は probeHubAlive と ensureHub を skip して直接
	// grace 期間に入る。PTY は kill しない（= ユーザーが手動で Hub を再起動
	// したら reattach で復活、新コンソール窓ポップアップも発生しない）。
	// 再接続成功時に false に戻す。
	var intentionalShutdown atomic.Bool

	reconnectCh := make(chan struct{}, 1)
	notifyReconnect := func() {
		select {
		case reconnectCh <- struct{}{}:
		default:
		}
	}

	var startReceiveLoop func(c *websocket.Conn)
	startReceiveLoop = func(c *websocket.Conn) {
		go func() {
			defer func() {
				if r := recover(); r != nil {
					logger.Error("hub-to-wrapper goroutine panic", "session_id", getSID(), "recover", r)
					clearConn(c)
					notifyReconnect()
				}
			}()
			for {
				var m proto.Message
				if err := websocket.JSON.Receive(c, &m); err != nil {
					select {
					case <-done:
						return
					default:
					}
					logger.Info("hub WS read failed", "session_id", getSID(), "err", err)
					clearConn(c)
					notifyReconnect()
					return
				}
				switch m.Type {
				case "pty_input":
					if len(m.Data) > 0 {
						data := m.Data
						if provider == "claude" && len(data) > 1 && data[0] == '@' {
							// 旧形式の inject (@path\rtext\r) は互換のため分割する。
							// 新形式 (@path text\r) は画像参照と本文を同じ入力行に残し、最後の Enter だけ分離する。
							if idx := bytes.IndexByte(data, '\r'); idx >= 0 && idx < len(data)-1 {
								_, _ = ps.Write(data[:idx+1])
								time.Sleep(150 * time.Millisecond)
								// ConPTY fix: text\r を1チャンクで書くと \r が Enter でなく改行扱いになる場合がある
								rest := data[idx+1:]
								if len(rest) > 1 && rest[len(rest)-1] == '\r' {
									_, _ = ps.Write(rest[:len(rest)-1])
									time.Sleep(20 * time.Millisecond)
									_, _ = ps.Write(rest[len(rest)-1:])
								} else {
									_, _ = ps.Write(rest)
								}
							} else if len(data) > 1 && data[len(data)-1] == '\r' {
								_, _ = ps.Write(data[:len(data)-1])
								time.Sleep(20 * time.Millisecond)
								_, _ = ps.Write(data[len(data)-1:])
							} else {
								_, _ = ps.Write(data)
							}
						} else if len(data) > 1 && data[len(data)-1] == '\r' {
							// Windows ConPTY では text+\r を1チャンクで書き込むと
							// \r が Enter ではなく改行として処理される場合がある。
							// Codex は入力反映直後の Enter を取りこぼすことがあるため長めに待つ。
							_, _ = ps.Write(data[:len(data)-1])
							if provider == "codex" {
								time.Sleep(180 * time.Millisecond)
							} else {
								time.Sleep(20 * time.Millisecond)
							}
							_, _ = ps.Write(data[len(data)-1:])
						} else {
							_, _ = ps.Write(data)
						}
					}
				case "pty_resize":
					if m.Cols > 0 && m.Rows > 0 {
						_ = ps.Resize(uint16(m.Cols), uint16(m.Rows))
					}
				case "attach_file":
					if err := HandleAttach(m, ps); err != nil {
						logger.Warn("attach inject failed", "err", err)
					}
				case "hub_shutdown":
					logger.Info("hub_shutdown received — skipping reconnect and ensureHub", "session_id", getSID(), "reason", m.Reason)
					intentionalShutdown.Store(true)
					clearConn(c)
					notifyReconnect()
					return
				}
			}
		}()
	}
	startReceiveLoop(conn)

	// Reconnect supervisor: distinguishes intentional session close (Hub HTTP
	// alive) from Hub process exit (Hub HTTP unreachable). The former kills PTY
	// immediately (preserving dismiss / kill-all / idle-timeout UX). For a Hub
	// process exit, auto_shutdown=true treats closing the Hub terminal as the end
	// of all wrapped sessions. When auto_shutdown=false, wrappers wait for a
	// manual Hub restart during the configured grace period, but never spawn a
	// replacement Hub on their own; otherwise closing the Hub terminal revives the
	// Windows console and Web UI unexpectedly.
	// hub_shutdown 受信（= UI からの Hub のみ停止）は intentional フラグ経由
	// の特別経路で、PTY は kill せず grace 期間中の手動 Hub 再起動を待つ。
	go func() {
		for {
			select {
			case <-done:
				return
			case <-reconnectCh:
			}
			intentional := intentionalShutdown.Load()
			if !intentional && probeHubAlive(cfg) {
				logger.Info("hub still alive after WS close — treating as intentional disconnect", "session_id", getSID())
				_ = ps.Close()
				closeDone()
				return
			}
			if !intentional && cfg.Hub.AutoShutdown {
				logger.Info("hub is down and auto_shutdown is enabled; terminating PTY", "session_id", getSID())
				_ = ps.Close()
				closeDone()
				return
			}
			grace := reconnectGrace(cfg)
			if grace <= 0 {
				logger.Info("reconnect grace disabled — terminating PTY", "session_id", getSID())
				_ = ps.Close()
				closeDone()
				return
			}
			if intentional {
				logger.Info("intentional hub shutdown — waiting for manual hub restart within grace period", "session_id", getSID(), "grace_sec", int(grace/time.Second))
			} else {
				logger.Info("hub appears down — entering reconnect grace period", "session_id", getSID(), "grace_sec", int(grace/time.Second))
			}
			deadline := time.Now().Add(grace)
			reconnected := false
			for !reconnected {
				if time.Now().After(deadline) {
					logger.Info("reconnect grace expired — terminating PTY", "session_id", getSID())
					_ = ps.Close()
					closeDone()
					return
				}
				select {
				case <-done:
					return
				case <-time.After(reconnectDialInterval):
				}
				cols, rows := 0, 0
				if w, h, err := term.GetSize(int(os.Stdin.Fd())); err == nil && w > 0 && h > 0 {
					cols, rows = w, h
				}
				newConn, newSID, err := dialAndReattach(cfg, getSID(), provider, display, cwd, *label, *model, startedAtText, rawLogPath, jsonlPath, cols, rows, snapshotReplay())
				if err != nil {
					logger.Debug("reconnect attempt failed", "err", err)
					continue
				}
				if newSID == 0 {
					logger.Warn("hub rejected reattach — terminating PTY", "session_id", getSID())
					_ = ps.Close()
					closeDone()
					return
				}
				logger.Info("wrapper reconnected to hub", "old_sid", getSID(), "new_sid", newSID)
				swapConn(newConn, newSID)
				startReceiveLoop(newConn)
				intentionalShutdown.Store(false)
				reconnected = true
			}
		}
	}()

	// PTY 出力 → Hub WS (pty_data)
	// carry holds an incomplete UTF-8 sequence from the previous read that must
	// be prepended to the next chunk before decoding (pumpChunk handles this).
	rawMaxBytes := int64(cfg.Log.SessionMaxSizeMB) * 1024 * 1024
	var rawWritten int64
	var carry []byte
	buf := make([]byte, 4096)
	for {
		n, readErr := ps.Read(buf)
		if n > 0 {
			raw := make([]byte, n)
			copy(raw, buf[:n])
			var out []byte
			out, carry = pumpChunk(carry, raw)
			if len(out) > 0 {
				out = repairMojibakeUTF8(out)
				if lf != nil && (rawMaxBytes <= 0 || rawWritten < rawMaxBytes) {
					_, _ = lf.Write(out)
					rawWritten += int64(len(out))
				}
				appendReplay(out)
				sendMsg(proto.Message{Type: "pty_data", SessionID: getSID(), Data: out})
			}
		}
		if readErr != nil {
			break
		}
	}
	// Flush any remaining carry bytes (e.g. incomplete sequence at end of stream).
	if len(carry) > 0 {
		flushed := repairMojibakeUTF8(carry)
		if lf != nil && (rawMaxBytes <= 0 || rawWritten < rawMaxBytes) {
			_, _ = lf.Write(flushed)
		}
		appendReplay(flushed)
		sendMsg(proto.Message{Type: "pty_data", SessionID: getSID(), Data: flushed})
	}
	closeDone()

	waitErr := ps.Wait()
	state, code := "completed", 0
	if waitErr != nil {
		state, code = "error", 1
	}
	sendMsg(proto.Message{Type: "session_end", SessionID: getSID(), State: state, ExitCode: code})
	// Close the current conn after the final message is sent.
	if c := getConn(); c != nil {
		_ = c.Close()
	}
	logger.Info("wrapper exit", "session_id", getSID(), "state", state)
	return waitErr
}

// dialAndRegister opens a WS to the Hub and performs the register handshake.
func dialAndRegister(cfg *config.Config, provider, display, cwd, label, model string, termCols, termRows int) (*websocket.Conn, proto.Message, error) {
	wsURL := url.URL{Scheme: "ws", Host: fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port), Path: "/ws"}
	conn, err := websocket.Dial(wsURL.String(), "", "http://127.0.0.1/")
	if err != nil {
		return nil, proto.Message{}, err
	}
	if err := websocket.JSON.Send(conn, proto.Message{
		Type:     "register",
		Role:     "wrapper",
		Provider: provider,
		Display:  display,
		CWD:      cwd,
		Label:    label,
		Model:    model,
		PID:      os.Getpid(),
		Shell:    DetectShell(),
		Token:    cfg.Token,
		Cols:     termCols,
		Rows:     termRows,
	}); err != nil {
		_ = conn.Close()
		return nil, proto.Message{}, err
	}
	var reg proto.Message
	if err := websocket.JSON.Receive(conn, &reg); err != nil {
		_ = conn.Close()
		return nil, proto.Message{}, err
	}
	return conn, reg, nil
}

func dialAndReattach(cfg *config.Config, sessionID int, provider, display, cwd, label, model, startedAt, rawLogPath, jsonlPath string, termCols, termRows int, replay []byte) (*websocket.Conn, int, error) {
	wsURL := url.URL{Scheme: "ws", Host: fmt.Sprintf("127.0.0.1:%d", cfg.Hub.Port), Path: "/ws"}
	conn, err := websocket.Dial(wsURL.String(), "", "http://127.0.0.1/")
	if err != nil {
		return nil, 0, err
	}
	if err := websocket.JSON.Send(conn, proto.Message{
		Type:      "reattach",
		Role:      "wrapper",
		SessionID: sessionID,
		Provider:  provider,
		Display:   display,
		CWD:       cwd,
		Label:     label,
		Model:     model,
		PID:       os.Getpid(),
		Shell:     DetectShell(),
		Token:     cfg.Token,
		Cols:      termCols,
		Rows:      termRows,
		StartedAt: startedAt,
		LogPath:   rawLogPath,
		JSONLPath: jsonlPath,
		ReplayB64: base64.StdEncoding.EncodeToString(replay),
	}); err != nil {
		_ = conn.Close()
		return nil, 0, err
	}
	var resp proto.Message
	if err := websocket.JSON.Receive(conn, &resp); err != nil {
		_ = conn.Close()
		return nil, 0, err
	}
	if resp.Type == "reattach_reject" {
		_ = conn.Close()
		return nil, 0, nil
	}
	if resp.Type != "reattach_ack" || resp.SessionID <= 0 {
		_ = conn.Close()
		return nil, 0, fmt.Errorf("unexpected reattach response: %s", resp.Type)
	}
	return conn, resp.SessionID, nil
}

// probeHubAlive returns true if the Hub HTTP server responds at 127.0.0.1:port.
// Used after a WS read failure to decide whether the disconnect was intentional
// (Hub still up — dismiss / kill-all / idle-timeout) or a Hub crash.
func probeHubAlive(cfg *config.Config) bool {
	u := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", cfg.Hub.Port, cfg.Token)
	client := &http.Client{Timeout: hubProbeTimeout}
	resp, err := client.Get(u)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}

func ensureHub(cfg *config.Config) error {
	// Hub にスポーンされた場合（ANY_AI_CLI=1）、Hub は既に動いている。
	// 新 Hub を起動すると PID ファイル経由で実際の Hub が kill される危険があるため
	// プローブと起動を一切スキップする。
	if os.Getenv("ANY_AI_CLI") == "1" {
		return nil
	}
	u := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", cfg.Hub.Port, cfg.Token)
	if resp, err := http.Get(u); err == nil {
		defer resp.Body.Close()
		if resp.StatusCode == http.StatusOK {
			return nil
		}
	}
	// 設定ポートが WSL 側 Hub など別プロセスに使用されている場合に備え、
	// 実際にバインドできるポートを確認してから Hub を起動する。
	// cfg.Hub.Port を更新することで後続の dialAndRegister も正しいポートを使う。
	cfg.Hub.Port = findFreePort(cfg.Hub.Port)
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	serve := exec.Command(exe, "serve", "--port", strconv.Itoa(cfg.Hub.Port))
	prepareHubSpawn(serve)
	if err := serve.Start(); err != nil {
		return err
	}
	time.Sleep(800 * time.Millisecond)
	return nil
}

// findFreePort は preferred ポートから順に 127.0.0.1 でバインドできる最初のポートを返す。
// preferred が空いていればそのまま返す。100 ポート試してすべて塞がれていたら preferred を返す。
func findFreePort(preferred int) int {
	for p := preferred; p < preferred+100; p++ {
		ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", p))
		if err == nil {
			_ = ln.Close()
			return p
		}
	}
	return preferred
}

type processSession interface {
	io.ReadWriteCloser
	Wait() error
	Resize(cols, rows uint16) error
}
