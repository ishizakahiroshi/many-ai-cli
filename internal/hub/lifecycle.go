package hub

import (
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	neturl "net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/config"
	hublog "many-ai-cli/internal/log"
	"many-ai-cli/internal/proto"
)

func (s *Server) handleKillAll(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	s.logger.Info("kill all wrappers requested via UI")
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
	// 起動ログ（"MANY-AI-CLI started"）と対になる終了ログ。reason で3経路
	// （cli_stop / ui_shutdown / signal）を判別できるようにする
	//（plan_hub-lifecycle-logging.md C1）。
	s.logger.Info("MANY-AI-CLI stopping", "reason", "ui_shutdown", "pid", os.Getpid(), "instance_id", s.instanceID)
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

// gracefulShutdownWait bounds how long stopWithPIDPath waits for the Hub
// process to actually exit after an /api/shutdown request before giving up
// and falling back to SIGKILL (plan_hub-lifecycle-logging.md C2).
const gracefulShutdownWait = 5 * time.Second

// gracefulShutdownPoll is the polling interval used while waiting for the
// Hub PID to disappear during gracefulShutdownWait.
const gracefulShutdownPoll = 100 * time.Millisecond

// gracefulShutdownReqTimeout bounds the /api/shutdown POST itself, separate
// from gracefulShutdownWait which bounds the wait for the process to exit.
const gracefulShutdownReqTimeout = 2 * time.Second

func Stop(cfg *config.Config) error {
	pidPath := filepath.Join(os.TempDir(), "many-ai-cli.pid")
	// stop コマンドを実行している側（Hub 本体とは別プロセス）のログに
	// 「stop 要求を送った」ことを残す。SIGKILL 自体は Go ランタイムで捕捉不可能
	// なため、この一行（と gracefulStop 内のログ）が cli_stop 経路で残る記録に
	// なる（plan_hub-lifecycle-logging.md C1）。
	logger := hublog.NewFileLogger(cfg.Hub.LogDir, cfg.Log, false, false)
	// 設定済みポートが古い（auto-port-move後）場合に備え、hub-runtime.json 経由
	// の実ポートも確認する。見つからなければ設定値のまま試し、それも失敗すれば
	// gracefulStop が false を返して force kill にフォールバックする。
	port, ok := runningHubPort(cfg)
	if !ok {
		port = cfg.Hub.Port
	}
	return stopWithPIDPath(pidPath, logger, port, cfg.Token)
}

func stopWithPIDPath(pidPath string, logger *slog.Logger, port int, token string) error {
	b, err := os.ReadFile(pidPath)
	if err != nil {
		return fmt.Errorf("hub pid not found")
	}
	pid, err := parseHubPID(b)
	if err != nil {
		_ = os.Remove(pidPath)
		return err
	}

	if gracefulStop(pid, port, token, logger) {
		_ = os.Remove(pidPath)
		return nil
	}

	if logger != nil {
		logger.Info("MANY-AI-CLI stopping", "reason", "cli_stop", "pid", pid)
	}
	// This is still an intentional stop (the user ran `many-ai-cli stop`),
	// even though the graceful request failed and we're falling back to
	// SIGKILL. The Hub process itself can't clean up after a SIGKILL, so
	// this CLI process clears the marker on its behalf — otherwise the next
	// startup would misreport a deliberate forced-stop as a crash
	// (plan_hub-lifecycle-logging.md C4).
	removeHubStateMarker()
	p, err := os.FindProcess(pid)
	if err != nil {
		return err
	}
	err = p.Kill()
	_ = os.Remove(pidPath)
	return err
}

// gracefulStop asks the running Hub to shut itself down via the same
// /api/shutdown endpoint the UI's shutdown button uses, then waits up to
// gracefulShutdownWait for the PID to actually disappear. It returns true
// only when the process is confirmed gone; stopWithPIDPath falls back to
// SIGKILL for every other outcome (unreachable port, non-200 response,
// process still alive after the wait).
//
// This is what lets `many-ai-cli stop` produce a real "MANY-AI-CLI
// stopping" / reason=ui_shutdown-style log entry from the Hub's own
// process (via handleShutdown) before this CLI process ever resorts to a
// SIGKILL the Hub cannot log for itself (plan_hub-lifecycle-logging.md C2).
func gracefulStop(pid, port int, token string, logger *slog.Logger) bool {
	if port <= 0 {
		return false
	}
	if logger != nil {
		logger.Info("graceful shutdown requested", "reason", "cli_stop", "pid", pid, "port", port)
	}
	url := localHubURL(port, "/api/shutdown", token)
	client := &http.Client{Timeout: gracefulShutdownReqTimeout}
	resp, err := client.Post(url, "application/json", nil)
	if err != nil {
		if logger != nil {
			logger.Info("falling back to force kill", "reason", "cli_stop", "pid", pid, "error", err.Error())
		}
		return false
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		if logger != nil {
			logger.Info("falling back to force kill", "reason", "cli_stop", "pid", pid, "status", resp.StatusCode)
		}
		return false
	}
	if waitForPIDExit(pid, gracefulShutdownWait, gracefulShutdownPoll) {
		return true
	}
	if logger != nil {
		logger.Info("falling back to force kill", "reason", "cli_stop", "pid", pid, "timeout", gracefulShutdownWait.String())
	}
	return false
}

// waitForPIDExit polls pidAlive until pid disappears or timeout elapses,
// returning true iff the process was confirmed gone within timeout.
func waitForPIDExit(pid int, timeout, interval time.Duration) bool {
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if !pidAlive(pid) {
			return true
		}
		time.Sleep(interval)
	}
	return !pidAlive(pid)
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

// hubStateMarkerName is the "still running" marker file written on Hub
// startup and removed on every clean shutdown path (cli_stop / ui_shutdown /
// signal). If it is still present at the next startup, the previous run
// never reached a clean shutdown — most likely a crash/panic or a forced
// kill from outside `many-ai-cli stop` (plan_hub-lifecycle-logging.md C4).
const hubStateMarkerName = "hub.state"

// hubStateMarkerPath returns the path of the marker file under ~/.many-ai-cli.
// Returns an empty string if the home directory cannot be resolved; callers
// treat that as "skip the check" rather than an error, since this is a
// best-effort diagnostic, not a correctness-critical mechanism.
func hubStateMarkerPath() string {
	dir, err := config.Dir()
	if err != nil {
		return ""
	}
	return filepath.Join(dir, hubStateMarkerName)
}

// previousShutdownStatus inspects (without removing) the marker file left
// behind by a prior Hub run. It must be called before writeHubStateMarker
// so it observes the *previous* run's marker, not the current one.
func previousShutdownStatus() string {
	path := hubStateMarkerPath()
	if path == "" {
		return ""
	}
	if _, err := os.Stat(path); err == nil {
		return "unclean"
	}
	return "clean"
}

// writeHubStateMarker records that a Hub instance is running. It is written
// once the listener is bound (mirroring pidPath) and removed on every clean
// shutdown path.
func writeHubStateMarker() {
	path := hubStateMarkerPath()
	if path == "" {
		return
	}
	_ = os.WriteFile(path, []byte(strconv.Itoa(os.Getpid())), 0o600)
}

// removeHubStateMarker clears the "still running" marker. Safe to call
// multiple times or when the file doesn't exist.
func removeHubStateMarker() {
	path := hubStateMarkerPath()
	if path == "" {
		return
	}
	_ = os.Remove(path)
}

// hubProbeTimeout bounds each /api/info liveness probe in IsRunning /
// runningHubPort.
const hubProbeTimeout = 500 * time.Millisecond

// IsRunning returns true if a Hub for this config is already running —
// either at the configured port or at the actually-bound port recorded in
// hub-runtime.json (the two differ after a port auto-move, e.g. when an
// SSH tunnel occupies the configured port).
func IsRunning(cfg *config.Config) bool {
	_, ok := runningHubPort(cfg)
	return ok
}

// runningHubPort returns the port of a verifiably running Hub that accepts
// this config's token. It checks the configured port first, then falls back
// to hub-runtime.json with the double guard (PID alive + /api/info probe).
func runningHubPort(cfg *config.Config) (int, bool) {
	return runningHubPortWith(cfg, pidAlive, func(port int) bool {
		return probeHubInfo(port, cfg.Token, hubProbeTimeout)
	})
}

// runningHubPortWith is runningHubPort with injectable alive/probe for tests.
// Tunnel safety: probe requires a 200 from /api/info with cfg.Token, so a
// foreign Hub (e.g. a remote Hub behind an SSH tunnel with a different
// token) occupying a recorded port is never reported as "running".
func runningHubPortWith(cfg *config.Config, alive func(pid int) bool, probe func(port int) bool) (int, bool) {
	if probe(cfg.Hub.Port) {
		return cfg.Hub.Port, true
	}
	rt, err := readHubRuntime()
	if err != nil || rt == nil {
		return 0, false
	}
	if !alive(rt.PID) {
		// PID が死んでいる残骸だけ掃除する。probe 失敗のみ（PID 生存）は
		// 一時的な無応答の可能性があるためファイルを残す。
		removeHubRuntimeIfPID(rt.PID)
		return 0, false
	}
	if rt.Port == cfg.Hub.Port || !probe(rt.Port) {
		return 0, false
	}
	return rt.Port, true
}

// probeHubInfo reports whether a Hub answers 200 to /api/info on port with
// token within timeout.
func probeHubInfo(port int, token string, timeout time.Duration) bool {
	url := localHubURL(port, "/api/info", token)
	client := &http.Client{Timeout: timeout}
	resp, err := client.Get(url)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}

func PrintStatus(cfg *config.Config) error {
	url := localHubURL(cfg.Hub.Port, "/", cfg.Token)
	// Hub がハングしていても status コマンドが固まらないようタイムアウトを設定する。
	// URL は 127.0.0.1 固定の自己生成値（gosec G107 対象外化も兼ねる）。
	client := &http.Client{Timeout: 3 * time.Second}
	resp, err := client.Get(url)
	if err != nil {
		fmt.Println("stopped")
		return nil
	}
	defer resp.Body.Close()
	if resp.StatusCode == 200 {
		fmt.Println("running", url)
		if stale, ok := probeBinaryStale(cfg.Hub.Port, cfg.Token); ok && stale {
			fmt.Println("WARNING: running Hub is a STALE binary (the on-disk exe differs from the one this Hub started with).")
			fmt.Println("         Restart the Hub to apply your rebuild: `many-ai-cli stop` then start it again.")
		}
		return nil
	}
	fmt.Println("stopped")
	return nil
}

// probeBinaryStale は /api/info の binary_stale を取得する。
// 取得・パースに失敗したら ok=false（status は警告を出さない）。
func probeBinaryStale(port int, token string) (stale bool, ok bool) {
	url := localHubURL(port, "/api/info", token)
	client := &http.Client{Timeout: hubProbeTimeout}
	resp, err := client.Get(url)
	if err != nil {
		return false, false
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return false, false
	}
	var info struct {
		BinaryStale bool `json:"binary_stale"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&info); err != nil {
		return false, false
	}
	return info.BinaryStale, true
}

func localHubURL(port int, path string, token string) string {
	if path == "" {
		path = "/"
	}
	return fmt.Sprintf("http://127.0.0.1:%d%s?token=%s", port, path, neturl.QueryEscape(token))
}
