package hub

// servers.go — Hub 内蔵のリモート接続マネージャ + HTTP ハンドラ。
//
// 別 exe `many-ai-cli-launcher` が持つ「リモート Hub へのプロファイル接続」を、
// 稼働中の Hub プロセス自身に内蔵する。SSH/WSL の子プロセス（トンネル /
// リモート serve）は Hub プロセスの子として無窓（CREATE_NO_WINDOW）で起動され、
// Hub が生きている間だけ維持される（= 「裏のターミナル」窓が不要になる）。
//
// 接続ライフサイクルは internal/launcher/ui_server.go の移植。純ロジック
// （Connector / Profile / launcher-active.json）は launcher パッケージを再利用し、
// HTTP 層と状態管理だけ Hub 用に作り直している。接続成功時はブラウザを自動で
// 開かず、status API が hub_url を返してフロントが新規タブで開く。
//
// API（いずれも Hub 既存の token ガード s.guard 配下）:
//   - GET  /api/servers                 プロファイル一覧 + active 接続
//   - POST /api/servers                 プロファイル replace-all 保存
//   - POST /api/servers/connect         {name} で接続開始（非同期）
//   - GET  /api/servers/connect/status  接続状態ポーリング（hub_url を含む）
//   - POST /api/servers/disconnect      {name, mode} で切断

import (
	"context"
	"fmt"
	"log/slog"
	"net/http"
	"net/url"
	"os"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/launcher"
)

// serverConnectWaitTimeout bounds how long a connection may stay in the
// "connecting" state. It must NOT bound the lifetime of an established
// connection — the tunnel lives until the Hub process exits or it is
// explicitly disconnected.
const serverConnectWaitTimeout = 120 * time.Second

const (
	serverDisconnectModeAll        = "all"
	serverDisconnectModeWeb        = "web"
	serverDisconnectModeDisconnect = "disconnect"

	serverDisconnectHubPostTimeout = 3 * time.Second
)

// serverLiveConn tracks one connection goroutine so it can be cancelled
// individually. Established connections move to conns and are not collaterally
// cancelled by a newer connect request (holding several destinations at once
// is the point).
type serverLiveConn struct {
	cancel context.CancelFunc
}

type serverConnectRequest struct {
	Name string `json:"name"`
}

type serverConnectResult struct {
	HubURL string // non-empty on success
	Err    string // non-empty on failure
}

type serverDisconnectRequest struct {
	Name string `json:"name"`
	Mode string `json:"mode"`
}

// serverProfilesResponse is the JSON envelope for GET /api/servers.
type serverProfilesResponse struct {
	OK       bool                   `json:"ok"`
	Profiles []launcher.Profile     `json:"profiles"`
	LastUsed string                 `json:"last_used,omitempty"`
	Active   []serverActiveResponse `json:"active,omitempty"`
}

// serverActiveResponse mirrors launcher.ActiveConnection for the UI and adds
// whether the connection is owned (cancellable) by THIS Hub process.
type serverActiveResponse struct {
	Profile   string    `json:"profile"`
	PID       int       `json:"pid"`
	HubURL    string    `json:"hub_url"`
	StartedAt time.Time `json:"started_at"`
	Owned     bool      `json:"owned"`
}

// serverConnManager owns the Hub-hosted remote connections. It mirrors the
// connection-state machine of launcher.UIServer.
type serverConnManager struct {
	logger *slog.Logger

	mu         sync.Mutex
	connectReq *serverConnectRequest // set when POST /api/servers/connect is received
	connectRes *serverConnectResult  // set when connection resolves
	inflight   *serverLiveConn       // connection being established (not yet resolved)
	conns      map[string]*serverLiveConn
}

func newServerConnManager(logger *slog.Logger) *serverConnManager {
	if logger == nil {
		logger = slog.Default()
	}
	return &serverConnManager{
		logger: logger,
		conns:  map[string]*serverLiveConn{},
	}
}

// closeAll cancels every live connection and clears this process's entries from
// launcher-active.json. Called on Hub shutdown so all hosted tunnels die with
// the Hub.
func (m *serverConnManager) closeAll() {
	m.mu.Lock()
	conns := m.conns
	m.conns = map[string]*serverLiveConn{}
	inflight := m.inflight
	m.inflight = nil
	m.mu.Unlock()

	if inflight != nil {
		inflight.cancel()
	}
	for _, lc := range conns {
		lc.cancel()
	}
	_ = launcher.UnregisterAllForPID()
}

// --------------------------------------------------------------------------
// HTTP handlers (methods on *Server, registered in server.go)
// --------------------------------------------------------------------------

// handleServers serves GET (list profiles + active) and POST (replace-all save).
func (s *Server) handleServers(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		pf, err := launcher.LoadProfiles()
		if err != nil {
			writeJSONError(w, http.StatusInternalServerError, "load_failed", err.Error())
			return
		}
		// 稼働中接続の検証（PID + /api/info 疎通の二重ガード）。残骸はこの呼び出しで
		// 掃除される。失敗してもプロファイル一覧自体は返す。
		active, err := launcher.ActiveConnectionsPruned()
		if err != nil {
			active = nil
		}
		writeJSON(w, serverProfilesResponse{
			OK:       true,
			Profiles: pf.Profiles,
			LastUsed: pf.LastUsed,
			Active:   s.serverConns.activeResponses(active),
		})

	case http.MethodPost:
		var req struct {
			Profiles []launcher.Profile `json:"profiles"`
		}
		if !decodeJSON(w, r, &req) {
			return
		}
		pf, err := launcher.LoadProfiles()
		if err != nil {
			writeJSONError(w, http.StatusInternalServerError, "load_failed", err.Error())
			return
		}
		pf.Profiles = req.Profiles
		if err := launcher.Validate(pf); err != nil {
			writeJSONError(w, http.StatusBadRequest, "invalid_profile", err.Error())
			return
		}
		if err := launcher.SaveProfiles(pf); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", err.Error())
			return
		}
		writeJSON(w, map[string]any{"ok": true})
	}
}

// handleServerConnect receives POST {name} and starts the connection in the
// background.
func (s *Server) handleServerConnect(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req serverConnectRequest
	if !decodeJSON(w, r, &req) {
		return
	}
	req.Name = strings.TrimSpace(req.Name)
	if req.Name == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "name is required")
		return
	}

	pf, err := launcher.LoadProfiles()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "load_failed", err.Error())
		return
	}
	var profile launcher.Profile
	found := false
	for _, p := range pf.Profiles {
		if p.Name == req.Name {
			profile = p
			found = true
			break
		}
	}
	if !found {
		writeJSONError(w, http.StatusNotFound, "not_found", fmt.Sprintf("profile %q not found", req.Name))
		return
	}

	m := s.serverConns
	m.mu.Lock()
	// Cancel a connection still being established (the UI drives one connect at
	// a time). Established connections to OTHER profiles are left alone.
	if m.inflight != nil {
		m.inflight.cancel()
		m.inflight = nil
	}
	// Reconnect semantics: tear down an established connection to the SAME
	// profile first (avoids duplicate serve / port conflicts).
	if old, ok := m.conns[req.Name]; ok {
		old.cancel()
		delete(m.conns, req.Name)
		_ = launcher.UnregisterActiveConnection(req.Name)
	}
	reqCopy := req
	m.connectReq = &reqCopy
	m.connectRes = nil
	// No deadline here: the context owns the tunnel for its entire lifetime.
	// The "connecting" phase is bounded separately by serverConnectWaitTimeout.
	ctx, cancel := context.WithCancel(context.Background())
	lc := &serverLiveConn{cancel: cancel}
	m.inflight = lc
	m.mu.Unlock()

	s.safeGo("server_connect", func() { m.runConnection(ctx, lc, profile) })

	writeJSON(w, map[string]any{"ok": true, "status": "connecting"})
}

// handleServerConnectStatus returns the current connection state for polling.
func (s *Server) handleServerConnectStatus(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	m := s.serverConns
	m.mu.Lock()
	req := m.connectReq
	res := m.connectRes
	m.mu.Unlock()

	if req == nil {
		writeJSON(w, map[string]any{"ok": true, "status": "idle"})
		return
	}
	if res == nil {
		writeJSON(w, map[string]any{"ok": true, "status": "connecting", "name": req.Name})
		return
	}
	if res.Err != "" {
		writeJSON(w, map[string]any{"ok": false, "status": "error", "error": res.Err})
		return
	}
	writeJSON(w, map[string]any{"ok": true, "status": "connected", "hub_url": res.HubURL})
}

// handleServerDisconnect receives POST {name, mode} and stops or detaches an
// active connection. Remote Hub operations are proxied by the Hub so the
// browser never has to call the remote Hub URL directly.
func (s *Server) handleServerDisconnect(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req serverDisconnectRequest
	if !decodeJSON(w, r, &req) {
		return
	}
	req.Name = strings.TrimSpace(req.Name)
	req.Mode = strings.TrimSpace(req.Mode)
	if req.Name == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "name is required")
		return
	}
	if !isServerDisconnectMode(req.Mode) {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid mode")
		return
	}

	active, err := launcher.ActiveConnectionsPruned()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "active_failed", err.Error())
		return
	}
	m := s.serverConns
	owned := m.hasOwnedConnection(req.Name)
	target, found := selectActiveConnection(active, req.Name, owned)
	if !found {
		writeJSONError(w, http.StatusNotFound, "not_found", fmt.Sprintf("profile %q is not active", req.Name))
		return
	}
	if req.Mode == serverDisconnectModeDisconnect && !owned {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "not owned by this Hub")
		return
	}

	var warnings []string
	var remoteErr error
	if req.Mode == serverDisconnectModeAll {
		if err := postHubEndpoint(r.Context(), target.HubURL, "/api/kill-all"); err != nil {
			warnings = append(warnings, err.Error())
		}
	}
	if req.Mode == serverDisconnectModeAll || req.Mode == serverDisconnectModeWeb {
		if err := postHubEndpoint(r.Context(), target.HubURL, "/api/shutdown"); err != nil {
			remoteErr = err
		}
	}

	// Local cleanup must run even when the remote Hub is already unreachable.
	if owned {
		m.cancelOwnedConnection(req.Name)
	}

	if remoteErr != nil {
		msgs := append(warnings, remoteErr.Error())
		writeJSONError(w, http.StatusBadGateway, "remote_error", strings.Join(msgs, "; "))
		return
	}
	resp := map[string]any{"ok": true}
	if len(warnings) > 0 {
		resp["warning"] = strings.Join(warnings, "; ")
	}
	writeJSON(w, resp)
}

// --------------------------------------------------------------------------
// connection lifecycle (ported from launcher/ui_server.go)
// --------------------------------------------------------------------------

// runConnection starts the Connector for profile and stores the result. On
// success the connection is promoted from inflight to conns and keeps running
// until its cancel is called (reconnect, disconnect, or Hub shutdown). On
// failure / timeout the context is cancelled to reap the connector.
func (m *serverConnManager) runConnection(ctx context.Context, lc *serverLiveConn, profile launcher.Profile) {
	conn, err := launcher.ConnectorForQuiet(profile)
	if err != nil {
		m.failConnection(lc, err.Error())
		return
	}

	urlCh := make(chan string, 1)
	errCh := make(chan error, 1)

	if err = conn.Start(ctx, profile, urlCh, errCh); err != nil {
		m.failConnection(lc, err.Error())
		return
	}

	waitTimer := time.NewTimer(serverConnectWaitTimeout)
	defer waitTimer.Stop()

	select {
	case hubURL, ok := <-urlCh:
		if !ok || hubURL == "" {
			select {
			case connErr := <-errCh:
				if connErr != nil {
					m.failConnection(lc, connErr.Error())
					return
				}
			default:
			}
			m.failConnection(lc, "Connection failed (Hub URL was not received)")
			return
		}
		if !m.promoteConnection(lc, profile.Name, hubURL) {
			// Superseded by a newer connect request while waiting; the context
			// is already cancelled, so let the connector die.
			return
		}
		m.updateLastUsed(profile.Name)
		if err := launcher.RegisterActiveConnection(profile.Name, hubURL); err != nil {
			m.logger.Warn("failed to record active connection", "profile", profile.Name, "err", err)
		}
		// 接続終了（errCh close）まで監視する。リモート serve 停止やトンネル切断時に
		// 確立済み接続の登録と launcher-active.json を掃除する。
		m.watchConnection(lc, profile.Name, errCh)
	case connErr := <-errCh:
		msg := "Connection failed"
		if connErr != nil {
			msg = connErr.Error()
		}
		m.failConnection(lc, msg)
	case <-waitTimer.C:
		m.failConnection(lc, "Connection timed out")
	case <-ctx.Done():
		m.failConnection(lc, "Connection cancelled")
	}
}

// watchConnection blocks until errCh is closed (= the connection terminated;
// see the Connector contract), then removes the established connection from
// conns and launcher-active.json.
func (m *serverConnManager) watchConnection(lc *serverLiveConn, name string, errCh <-chan error) {
	for connErr := range errCh {
		if connErr != nil {
			m.logger.Warn("server connection error", "profile", name, "err", connErr)
		}
	}
	lc.cancel()
	m.mu.Lock()
	current := m.conns[name] == lc
	if current {
		delete(m.conns, name)
	}
	m.mu.Unlock()
	// 既に新しい接続へ置き換わっている（再接続中）場合は、新しい接続の登録を
	// 消さないよう何もしない。
	if current {
		_ = launcher.UnregisterActiveConnection(name)
		m.logger.Info("server connection closed", "profile", name)
	}
}

// promoteConnection moves lc from inflight to the established map and publishes
// the success result. Returns false when lc has been superseded by a newer
// connect request (its context is already cancelled).
func (m *serverConnManager) promoteConnection(lc *serverLiveConn, name, hubURL string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.inflight != lc {
		return false
	}
	m.inflight = nil
	m.conns[name] = lc
	m.connectRes = &serverConnectResult{HubURL: hubURL}
	return true
}

// failConnection cancels lc's context (reaping the connector goroutine) and
// publishes the error result, unless this attempt has already been superseded.
func (m *serverConnManager) failConnection(lc *serverLiveConn, errMsg string) {
	lc.cancel()
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.inflight == lc {
		m.inflight = nil
		m.connectRes = &serverConnectResult{Err: errMsg}
	}
}

func (m *serverConnManager) updateLastUsed(name string) {
	pf, err := launcher.LoadProfiles()
	if err != nil {
		return
	}
	pf.LastUsed = name
	_ = launcher.SaveProfiles(pf)
}

func (m *serverConnManager) activeResponses(active []launcher.ActiveConnection) []serverActiveResponse {
	if len(active) == 0 {
		return nil
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	out := make([]serverActiveResponse, 0, len(active))
	for _, c := range active {
		_, owned := m.conns[c.Profile]
		out = append(out, serverActiveResponse{
			Profile:   c.Profile,
			PID:       c.PID,
			HubURL:    c.HubURL,
			StartedAt: c.StartedAt,
			Owned:     owned,
		})
	}
	return out
}

func (m *serverConnManager) hasOwnedConnection(name string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	_, ok := m.conns[name]
	return ok
}

func (m *serverConnManager) cancelOwnedConnection(name string) {
	m.mu.Lock()
	lc, ok := m.conns[name]
	if ok {
		delete(m.conns, name)
	}
	m.mu.Unlock()
	if !ok {
		return
	}
	lc.cancel()
	_ = launcher.UnregisterActiveConnection(name)
}

// --------------------------------------------------------------------------
// helpers
// --------------------------------------------------------------------------

func isServerDisconnectMode(mode string) bool {
	switch mode {
	case serverDisconnectModeAll, serverDisconnectModeWeb, serverDisconnectModeDisconnect:
		return true
	default:
		return false
	}
}

// selectActiveConnection picks the active connection for name, preferring the
// entry owned by this process when owned is true.
func selectActiveConnection(active []launcher.ActiveConnection, name string, owned bool) (launcher.ActiveConnection, bool) {
	var selected launcher.ActiveConnection
	found := false
	for _, c := range active {
		if c.Profile != name {
			continue
		}
		if !found {
			selected = c
			found = true
		}
		if owned && c.PID == os.Getpid() {
			return c, true
		}
	}
	return selected, found
}

// postHubEndpoint POSTs to a path on the remote Hub reached through the local
// forwarded port. The target is always loopback (the tunnel endpoint), so a
// plain client is used — NOT newExternalHTTPClient, which blocks loopback.
func postHubEndpoint(parent context.Context, hubURL, path string) error {
	u, err := url.Parse(hubURL)
	if err != nil {
		return fmt.Errorf("%s: parse hub url: %w", path, err)
	}
	u.Path = path
	u.RawPath = ""
	u.Fragment = ""

	ctx, cancel := context.WithTimeout(parent, serverDisconnectHubPostTimeout)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, u.String(), nil)
	if err != nil {
		return fmt.Errorf("%s: create request: %w", path, err)
	}
	client := &http.Client{Timeout: serverDisconnectHubPostTimeout}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("%s: %w", path, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("%s: %s", path, resp.Status)
	}
	return nil
}
