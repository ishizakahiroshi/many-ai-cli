package hub

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"any-ai-cli/internal/config"
	"any-ai-cli/internal/proto"
)

func (s *Server) handleKillAll(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	s.killAllWrappers()
	writeJSON(w, map[string]bool{"ok": true})
}

func (s *Server) killAllWrappers() {
	s.sessionsMu.Lock()
	conns := make([]*wrapperConn, 0, len(s.wrappers))
	for _, wc := range s.wrappers {
		conns = append(conns, wc)
	}
	s.sessionsMu.Unlock()
	for _, wc := range conns {
		wc.close()
	}
}

func (s *Server) handleShutdown(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	s.logger.Info("shutdown requested via UI")
	s.broadcastHubShutdown("ui_shutdown")
	writeJSON(w, map[string]bool{"ok": true})
	s.safeGo("request_stop", s.requestStop)
}

// broadcastHubShutdown は接続中の全 wrapper に hub_shutdown を通知し、
// 「これは Hub クラッシュではなく意図的シャットダウンだ」ことを伝える。
// 通知を受けた wrapper は reconnect grace に入らず ensureHub を呼ばないので、
// CREATE_NEW_CONSOLE による Hub 復活ターミナル窓のポップアップが発生しない。
func (s *Server) broadcastHubShutdown(reason string) {
	s.sessionsMu.Lock()
	conns := make([]*wrapperConn, 0, len(s.wrappers))
	for _, wc := range s.wrappers {
		conns = append(conns, wc)
	}
	s.sessionsMu.Unlock()
	msg := proto.Message{Type: "hub_shutdown", Reason: reason}
	for _, wc := range conns {
		_ = wc.sendWithDeadline(msg, time.Now().Add(500*time.Millisecond))
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

func Stop(cfg *config.Config) error {
	pidPath := filepath.Join(os.TempDir(), "any-ai-cli.pid")
	return stopWithPIDPath(pidPath)
}

func stopWithPIDPath(pidPath string) error {
	b, err := os.ReadFile(pidPath)
	if err != nil {
		return fmt.Errorf("hub pid not found")
	}
	pid, err := parseHubPID(b)
	if err != nil {
		_ = os.Remove(pidPath)
		return err
	}
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
//
// The file is removed *before* killing, and the current process is never a
// target: PID 名前空間が独立したコンテナでは Hub が毎回同じ PID（例: 常に 11）で
// 起動するため、前回 boot の残骸ファイルを無条件に kill すると新しい Hub が
// 起動直後に自分自身を SIGKILL してしまう。しかも自殺すると Remove に到達せず
// ファイルが残り続け、以降の再起動が全て同じ死に方をするループになる。
func killStalePid(pidPath string) {
	b, err := os.ReadFile(pidPath)
	if err != nil {
		return
	}
	_ = os.Remove(pidPath)
	pid, err := parseHubPID(b)
	if err != nil {
		return
	}
	if pid == os.Getpid() {
		return
	}
	if p, err := os.FindProcess(pid); err == nil {
		_ = p.Kill()
	}
}

func parseHubPID(b []byte) (int, error) {
	pid, err := strconv.Atoi(strings.TrimSpace(string(b)))
	if err != nil {
		return 0, fmt.Errorf("invalid hub pid file: %w", err)
	}
	if pid <= 0 {
		return 0, fmt.Errorf("invalid hub pid file")
	}
	return pid, nil
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
