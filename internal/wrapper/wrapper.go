package wrapper

import (
	"bytes"
	"encoding/base64"
	"flag"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"time"

	"ai-cli-hub/internal/config"
	"ai-cli-hub/internal/proto"
	"ai-cli-hub/internal/sessionlog"
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
		if *sandbox != "" {
			extra = append(extra, "--sandbox", *sandbox)
		}
		if *askForApproval != "" {
			extra = append(extra, "--ask-for-approval", *askForApproval)
		}
	}
	providerArgs = append(extra, providerArgs...)

	if err := ensureHub(cfg); err != nil {
		return err
	}
	cwd, _ := os.Getwd()
	display := map[string]string{"claude": "Claude", "codex": "Codex", "gemini": "Gemini CLI"}[provider]
	termCols, termRows := 0, 0
	if w, h, err := term.GetSize(int(os.Stdin.Fd())); err == nil && w > 0 && h > 0 {
		termCols, termRows = w, h
	}

	conn, reg, err := dialAndRegister(cfg, provider, display, cwd, *label, *model, termCols, termRows)
	if err != nil {
		return err
	}
	sessionID := reg.SessionID
	setConsoleTitle(fmt.Sprintf("ai-cli-hub [#%d:%s]", sessionID, provider))
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

	ps, err := startProcess(provider, providerArgs, cwd, initCols, initRows)
	if err != nil {
		_ = conn.Close()
		return err
	}
	defer ps.Close()

	// Shared mutable state (current conn / session id rotate on reconnect)
	var (
		stateMu     sync.Mutex
		currentConn = conn
		currentSID  = sessionID
	)
	getConn := func() *websocket.Conn { stateMu.Lock(); defer stateMu.Unlock(); return currentConn }
	getSID := func() int { stateMu.Lock(); defer stateMu.Unlock(); return currentSID }
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
							// inject (@path\r) の後に続くテキストがある場合、ピッカー確定後に
							// テキストが届くよう分割送信する。一括送信だとピッカー確定直後の
							// テキスト末尾 \r が「送信」ではなく「改行」として処理されてしまう。
							if idx := bytes.IndexByte(data, '\r'); idx >= 0 && idx < len(data)-1 {
								_, _ = ps.Write(data[:idx+1])
								time.Sleep(150 * time.Millisecond)
								_, _ = ps.Write(data[idx+1:])
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
				}
			}
		}()
	}
	startReceiveLoop(conn)

	// Reconnect supervisor: distinguishes intentional close (Hub HTTP alive)
	// from Hub crash (Hub HTTP unreachable). The former kills PTY immediately
	// (preserving dismiss / kill-all / idle-timeout UX); the latter waits for
	// the configured grace period for Hub to come back, then re-registers.
	go func() {
		for {
			select {
			case <-done:
				return
			case <-reconnectCh:
			}
			if probeHubAlive(cfg) {
				logger.Info("hub still alive after WS close — treating as intentional disconnect", "session_id", getSID())
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
			logger.Info("hub appears down — entering reconnect grace period", "session_id", getSID(), "grace_sec", int(grace/time.Second))
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
				if err := ensureHub(cfg); err != nil {
					logger.Debug("ensure hub during reconnect failed", "err", err)
					continue
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
				reconnected = true
			}
		}
	}()

	// PTY 出力 → Hub WS (pty_data)
	buf := make([]byte, 4096)
	for {
		n, readErr := ps.Read(buf)
		if n > 0 {
			chunk := make([]byte, n)
			copy(chunk, buf[:n])
			if lf != nil {
				_, _ = lf.Write(chunk)
			}
			appendReplay(chunk)
			if c := getConn(); c != nil {
				_ = websocket.JSON.Send(c, proto.Message{Type: "pty_data", SessionID: getSID(), Data: chunk})
			}
		}
		if readErr != nil {
			break
		}
	}
	closeDone()

	waitErr := ps.Wait()
	state, code := "completed", 0
	if waitErr != nil {
		state, code = "error", 1
	}
	if c := getConn(); c != nil {
		_ = websocket.JSON.Send(c, proto.Message{Type: "session_end", SessionID: getSID(), State: state, ExitCode: code})
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
	u := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", cfg.Hub.Port, cfg.Token)
	if resp, err := http.Get(u); err == nil {
		defer resp.Body.Close()
		if resp.StatusCode == http.StatusOK {
			return nil
		}
	}
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	serve := exec.Command(exe, "serve")
	serve.Stdout, serve.Stderr = os.Stdout, os.Stderr
	if err := serve.Start(); err != nil {
		return err
	}
	time.Sleep(800 * time.Millisecond)
	return nil
}

type processSession interface {
	io.ReadWriteCloser
	Wait() error
	Resize(cols, rows uint16) error
}
