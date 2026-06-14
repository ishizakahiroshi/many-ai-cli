// Package launcher provides the local HTTP server that serves the profile
// selection UI. The server binds to 127.0.0.1 on a random free port and
// requires a random token on every API request (same security model as the Hub).
package launcher

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"embed"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io/fs"
	"net"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
)

//go:embed ui
var uiFS embed.FS

// UIServer is the HTTP server that serves the profile selection page.
// It starts on a random loopback port and keeps running (holding any
// tunnels / serve processes started from the page) until ctx is cancelled.
type UIServer struct {
	token  string
	port   int
	server *http.Server

	mu         sync.Mutex
	connectReq *connectRequest      // set when POST /api/connect is received
	connectRes *connectResult       // set when connection resolves
	inflight   *liveConn            // connection being established (not yet resolved)
	conns      map[string]*liveConn // established connections keyed by profile name
}

// liveConn tracks one connection goroutine so it can be cancelled
// individually. 確立済み接続は conns に移り、以降の新規接続要求で
// 巻き添え cancel されない（複数接続先の同時保持が前提のため）。
type liveConn struct {
	cancel context.CancelFunc
}

// connectWaitTimeout bounds how long a connection may stay in the
// "connecting" state. It must NOT bound the lifetime of an established
// connection — the tunnel lives until the launcher process exits.
const connectWaitTimeout = 120 * time.Second

type connectRequest struct {
	Name string `json:"name"`
}

type connectResult struct {
	HubURL string // non-empty on success
	Err    string // non-empty on failure
}

// profilesResponse is the JSON envelope for GET /api/profiles.
// Active lists verified-alive connections (this process and others) so the
// UI can mark already-connected profiles and reuse their Hub URL.
type profilesResponse struct {
	OK       bool             `json:"ok"`
	Profiles []Profile        `json:"profiles"`
	LastUsed string           `json:"last_used,omitempty"`
	Active   []activeResponse `json:"active,omitempty"`
}

// activeResponse mirrors ActiveConnection for the UI and adds whether the
// connection can be cancelled by this launcher process.
type activeResponse struct {
	Profile   string    `json:"profile"`
	PID       int       `json:"pid"`
	HubURL    string    `json:"hub_url"`
	StartedAt time.Time `json:"started_at"`
	Owned     bool      `json:"owned"`
}

type disconnectRequest struct {
	Name string `json:"name"`
	Mode string `json:"mode"`
}

const (
	disconnectModeAll        = "all"
	disconnectModeWeb        = "web"
	disconnectModeDisconnect = "disconnect"

	disconnectHubPostTimeout = 3 * time.Second
)

// NewUIServer creates a UIServer. Call Serve to start it.
func NewUIServer() (*UIServer, error) {
	token, err := generateToken()
	if err != nil {
		return nil, fmt.Errorf("generate ui token: %w", err)
	}
	return &UIServer{token: token, conns: make(map[string]*liveConn)}, nil
}

// generateToken returns a 16-byte random hex string.
func generateToken() (string, error) {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return hex.EncodeToString(b), nil
}

// Serve starts the HTTP server on a random loopback port and returns the URL
// (including ?token=) that the browser should open.  The server stops when
// ctx is cancelled.
func (s *UIServer) Serve(ctx context.Context) (string, error) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return "", fmt.Errorf("listen ui server: %w", err)
	}
	s.port = ln.Addr().(*net.TCPAddr).Port

	mux := http.NewServeMux()
	mux.HandleFunc("/", s.handleIndex)
	mux.HandleFunc("/api/profiles", s.handleProfiles)
	mux.HandleFunc("/api/connect", s.handleConnect)
	mux.HandleFunc("/api/connect/status", s.handleConnectStatus)
	mux.HandleFunc("/api/disconnect", s.handleDisconnect)

	s.server = &http.Server{
		Handler:      mux,
		ReadTimeout:  30 * time.Second,
		WriteTimeout: 60 * time.Second,
	}

	go func() { // #nosec G118 -- shutdown 用 goroutine。親 ctx の終了後に動くため独立 context が必要
		<-ctx.Done()
		shutCtx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		defer cancel()
		_ = s.server.Shutdown(shutCtx)
	}()

	go func() {
		_ = s.server.Serve(ln)
	}()

	pageURL := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", s.port, url.QueryEscape(s.token))
	return pageURL, nil
}

// --------------------------------------------------------------------------
// token / origin guard helpers
// --------------------------------------------------------------------------

func (s *UIServer) validToken(got string) bool {
	if got == "" || s.token == "" {
		return false
	}
	return subtle.ConstantTimeCompare([]byte(got), []byte(s.token)) == 1
}

func (s *UIServer) requestToken(r *http.Request) string {
	if got := r.URL.Query().Get("token"); got != "" {
		return got
	}
	auth := strings.TrimSpace(r.Header.Get("Authorization"))
	if token, ok := strings.CutPrefix(auth, "Bearer "); ok {
		return strings.TrimSpace(token)
	}
	return ""
}

// requireAuth verifies token and (for mutating methods) Host/Origin.
// Returns false and writes the error response when the check fails.
func (s *UIServer) requireAuth(w http.ResponseWriter, r *http.Request) bool {
	if !s.validToken(s.requestToken(r)) {
		writeUIError(w, http.StatusUnauthorized, "unauthorized")
		return false
	}
	// For state-mutating methods, also verify Host/Origin.
	switch r.Method {
	case http.MethodGet, http.MethodHead, http.MethodOptions:
		return true
	}
	if !isAllowedUIHost(r.Host, s.port) {
		writeUIError(w, http.StatusForbidden, "host not allowed")
		return false
	}
	if origin := strings.TrimSpace(r.Header.Get("Origin")); origin != "" {
		if !isAllowedUIOrigin(origin, s.port) {
			writeUIError(w, http.StatusForbidden, "origin not allowed")
			return false
		}
	}
	return true
}

func isAllowedUIHost(hostport string, port int) bool {
	hostport = strings.TrimSpace(hostport)
	if hostport == "" {
		return false
	}
	host, rawPort, err := net.SplitHostPort(hostport)
	if err != nil {
		host = strings.Trim(hostport, "[]")
		rawPort = ""
	}
	host = strings.TrimSuffix(strings.ToLower(strings.TrimSpace(host)), ".")
	if host != "127.0.0.1" && host != "localhost" && host != "::1" {
		return false
	}
	if port <= 0 || rawPort == "" {
		return true
	}
	gotPort, err := strconv.Atoi(rawPort)
	return err == nil && gotPort == port
}

func isAllowedUIOrigin(rawOrigin string, port int) bool {
	u, err := url.Parse(rawOrigin)
	if err != nil {
		return false
	}
	if strings.ToLower(u.Scheme) != "http" || u.Host == "" {
		return false
	}
	return isAllowedUIHost(u.Host, port)
}

func writeUIError(w http.ResponseWriter, status int, detail string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(map[string]any{
		"ok":    false,
		"error": detail,
	})
}

func writeUIJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(v)
}

// --------------------------------------------------------------------------
// handlers
// --------------------------------------------------------------------------

// handleIndex serves the embedded index.html for any GET / request that has a
// valid token.  Requests without a token receive 401.
func (s *UIServer) handleIndex(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}
	if r.Method != http.MethodGet && r.Method != http.MethodHead {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if !s.validToken(s.requestToken(r)) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}

	sub, err := fs.Sub(uiFS, "ui")
	if err != nil {
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}
	http.ServeFileFS(w, r, sub, "index.html")
}

// handleProfiles serves GET (list) and POST (replace-all) for profiles.
func (s *UIServer) handleProfiles(w http.ResponseWriter, r *http.Request) {
	if !s.requireAuth(w, r) {
		return
	}

	switch r.Method {
	case http.MethodGet:
		pf, err := LoadProfiles()
		if err != nil {
			writeUIError(w, http.StatusInternalServerError, err.Error())
			return
		}
		// 稼働中接続の検証（PID + /api/info 疎通の二重ガード）。残骸は
		// この呼び出しで掃除される。失敗してもプロファイル一覧自体は返す。
		active, err := ActiveConnectionsPruned()
		if err != nil {
			active = nil
		}
		writeUIJSON(w, profilesResponse{
			OK:       true,
			Profiles: pf.Profiles,
			LastUsed: pf.LastUsed,
			Active:   s.activeResponses(active),
		})

	case http.MethodPost:
		var req struct {
			Profiles []Profile `json:"profiles"`
		}
		if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, 512*1024)).Decode(&req); err != nil {
			writeUIError(w, http.StatusBadRequest, "invalid json")
			return
		}
		pf, err := LoadProfiles()
		if err != nil {
			writeUIError(w, http.StatusInternalServerError, err.Error())
			return
		}
		pf.Profiles = req.Profiles
		if err := Validate(pf); err != nil {
			writeUIError(w, http.StatusBadRequest, err.Error())
			return
		}
		if err := SaveProfiles(pf); err != nil {
			writeUIError(w, http.StatusInternalServerError, err.Error())
			return
		}
		writeUIJSON(w, map[string]any{"ok": true})

	default:
		writeUIError(w, http.StatusMethodNotAllowed, "method not allowed")
	}
}

// handleConnect receives POST {name} and starts the connection in the background.
func (s *UIServer) handleConnect(w http.ResponseWriter, r *http.Request) {
	if !s.requireAuth(w, r) {
		return
	}
	if r.Method != http.MethodPost {
		writeUIError(w, http.StatusMethodNotAllowed, "method not allowed")
		return
	}

	var req connectRequest
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, 4*1024)).Decode(&req); err != nil {
		writeUIError(w, http.StatusBadRequest, "invalid json")
		return
	}
	if req.Name == "" {
		writeUIError(w, http.StatusBadRequest, "name is required")
		return
	}

	pf, err := LoadProfiles()
	if err != nil {
		writeUIError(w, http.StatusInternalServerError, err.Error())
		return
	}
	var profile Profile
	found := false
	for _, p := range pf.Profiles {
		if p.Name == req.Name {
			profile = p
			found = true
			break
		}
	}
	if !found {
		writeUIError(w, http.StatusNotFound, fmt.Sprintf("profile %q not found", req.Name))
		return
	}

	s.mu.Lock()
	// Cancel a connection still being established (the UI drives one
	// connect at a time). Established connections to OTHER profiles are
	// left alone — holding several destinations at once is the point.
	if s.inflight != nil {
		s.inflight.cancel()
		s.inflight = nil
	}
	// Reconnect semantics: an established connection to the SAME profile
	// is torn down first (avoids duplicate serve / port conflicts).
	if old, ok := s.conns[req.Name]; ok {
		old.cancel()
		delete(s.conns, req.Name)
		_ = UnregisterActiveConnection(req.Name)
	}
	s.connectReq = &req
	s.connectRes = nil
	// No deadline here: the context owns the tunnel for its entire
	// lifetime. The "connecting" phase is bounded separately by
	// connectWaitTimeout in runConnection.
	ctx, cancel := context.WithCancel(context.Background())
	lc := &liveConn{cancel: cancel}
	s.inflight = lc
	s.mu.Unlock()

	go s.runConnection(ctx, lc, profile)

	writeUIJSON(w, map[string]any{"ok": true, "status": "connecting"})
}

// handleConnectStatus returns the current connection state for polling.
func (s *UIServer) handleConnectStatus(w http.ResponseWriter, r *http.Request) {
	if !s.requireAuth(w, r) {
		return
	}
	if r.Method != http.MethodGet {
		writeUIError(w, http.StatusMethodNotAllowed, "method not allowed")
		return
	}

	s.mu.Lock()
	req := s.connectReq
	res := s.connectRes
	s.mu.Unlock()

	if req == nil {
		writeUIJSON(w, map[string]any{"ok": true, "status": "idle"})
		return
	}
	if res == nil {
		writeUIJSON(w, map[string]any{"ok": true, "status": "connecting", "name": req.Name})
		return
	}
	if res.Err != "" {
		writeUIJSON(w, map[string]any{"ok": false, "status": "error", "error": res.Err})
		return
	}
	writeUIJSON(w, map[string]any{"ok": true, "status": "connected", "hub_url": res.HubURL})
}

// handleDisconnect receives POST {name, mode} and stops or detaches an active
// connection. Remote Hub operations are proxied by this UI server so the
// browser never has to call the Hub URL directly.
func (s *UIServer) handleDisconnect(w http.ResponseWriter, r *http.Request) {
	if !s.requireAuth(w, r) {
		return
	}
	if r.Method != http.MethodPost {
		writeUIError(w, http.StatusMethodNotAllowed, "method not allowed")
		return
	}

	var req disconnectRequest
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, 4*1024)).Decode(&req); err != nil {
		writeUIError(w, http.StatusBadRequest, "invalid json")
		return
	}
	req.Name = strings.TrimSpace(req.Name)
	req.Mode = strings.TrimSpace(req.Mode)
	if req.Name == "" {
		writeUIError(w, http.StatusBadRequest, "name is required")
		return
	}
	if !isDisconnectMode(req.Mode) {
		writeUIError(w, http.StatusBadRequest, "invalid mode")
		return
	}

	active, err := ActiveConnectionsPruned()
	if err != nil {
		writeUIError(w, http.StatusInternalServerError, err.Error())
		return
	}
	owned := s.hasOwnedConnection(req.Name)
	target, found := selectActiveConnection(active, req.Name, owned)
	if !found {
		writeUIError(w, http.StatusNotFound, fmt.Sprintf("profile %q is not active", req.Name))
		return
	}
	if req.Mode == disconnectModeDisconnect && !owned {
		writeUIError(w, http.StatusBadRequest, "not owned by this launcher")
		return
	}

	var warnings []string
	var remoteErr error
	if req.Mode == disconnectModeAll {
		if err := postHubEndpoint(r.Context(), target.HubURL, "/api/kill-all"); err != nil {
			warnings = append(warnings, err.Error())
		}
	}
	if req.Mode == disconnectModeAll || req.Mode == disconnectModeWeb {
		if err := postHubEndpoint(r.Context(), target.HubURL, "/api/shutdown"); err != nil {
			remoteErr = err
		}
	}

	// Local cleanup must run even when the remote Hub is already unreachable.
	if owned {
		s.cancelOwnedConnection(req.Name)
	}

	if remoteErr != nil {
		msgs := append(warnings, remoteErr.Error())
		writeUIError(w, http.StatusBadGateway, strings.Join(msgs, "; "))
		return
	}
	resp := map[string]any{"ok": true}
	if len(warnings) > 0 {
		resp["warning"] = strings.Join(warnings, "; ")
	}
	writeUIJSON(w, resp)
}

func isDisconnectMode(mode string) bool {
	switch mode {
	case disconnectModeAll, disconnectModeWeb, disconnectModeDisconnect:
		return true
	default:
		return false
	}
}

func (s *UIServer) activeResponses(active []ActiveConnection) []activeResponse {
	if len(active) == 0 {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	out := make([]activeResponse, 0, len(active))
	for _, c := range active {
		_, owned := s.conns[c.Profile]
		out = append(out, activeResponse{
			Profile:   c.Profile,
			PID:       c.PID,
			HubURL:    c.HubURL,
			StartedAt: c.StartedAt,
			Owned:     owned,
		})
	}
	return out
}

func (s *UIServer) hasOwnedConnection(name string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	_, ok := s.conns[name]
	return ok
}

func (s *UIServer) cancelOwnedConnection(name string) {
	s.mu.Lock()
	lc, ok := s.conns[name]
	if ok {
		delete(s.conns, name)
	}
	s.mu.Unlock()
	if !ok {
		return
	}
	lc.cancel()
	_ = UnregisterActiveConnection(name)
}

func selectActiveConnection(active []ActiveConnection, name string, owned bool) (ActiveConnection, bool) {
	var selected ActiveConnection
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

func postHubEndpoint(parent context.Context, hubURL, path string) error {
	u, err := url.Parse(hubURL)
	if err != nil {
		return fmt.Errorf("%s: parse hub url: %w", path, err)
	}
	u.Path = path
	u.RawPath = ""
	u.Fragment = ""

	ctx, cancel := context.WithTimeout(parent, disconnectHubPostTimeout)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, u.String(), nil)
	if err != nil {
		return fmt.Errorf("%s: create request: %w", path, err)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return fmt.Errorf("%s: %w", path, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("%s: %s", path, resp.Status)
	}
	return nil
}

// runConnection starts the Connector for profile and stores the result.
// On success the connection is promoted from inflight to s.conns and keeps
// running until its cancel is called (reconnect or server shutdown).
// On failure / timeout the context is cancelled to reap the connector.
func (s *UIServer) runConnection(ctx context.Context, lc *liveConn, profile Profile) {
	var conn Connector
	var err error
	switch profile.Type {
	case ProfileTypeWSL:
		conn, err = connectorForWSL()
		if err != nil {
			s.failConnection(lc, err.Error())
			return
		}
	case ProfileTypeSSH:
		conn = NewSSHConnector()
	default:
		s.failConnection(lc, fmt.Sprintf("unsupported profile type %q", profile.Type))
		return
	}

	// 選択 UI 経由でもコンソール側に「閉じたらどうなるか」を出しておく
	// （このウィンドウがトンネル / Hub の本体であることはブラウザからは見えないため）。
	fmt.Fprint(os.Stdout, CloseBehaviorNotice(profile))

	urlCh := make(chan string, 1)
	errCh := make(chan error, 1)

	if err = conn.Start(ctx, profile, urlCh, errCh); err != nil {
		s.failConnection(lc, err.Error())
		return
	}

	waitTimer := time.NewTimer(connectWaitTimeout)
	defer waitTimer.Stop()

	select {
	case hubURL, ok := <-urlCh:
		if !ok || hubURL == "" {
			// errCh may have the reason.
			select {
			case connErr := <-errCh:
				if connErr != nil {
					s.failConnection(lc, connErr.Error())
					return
				}
			default:
			}
			s.failConnection(lc, "Connection failed (Hub URL was not received)")
			return
		}
		if !s.promoteConnection(lc, profile.Name, hubURL) {
			// Superseded by a newer connect request while waiting; the
			// context is already cancelled, so just let the connector die.
			return
		}
		// connectWaitTimeout only bounds the "connecting" phase. The
		// established connection lives for the launcher process lifetime
		// (watchConnection below blocks until errCh closes), during which
		// the defer above would not run. Stop the timer now so it does not
		// linger for the whole connection lifetime. defer Stop remains for
		// the timeout/failure paths; a second Stop is harmless.
		waitTimer.Stop()
		// Update last_used and record the connection for other launcher
		// processes (running badge in their selection UI).
		s.updateLastUsed(profile.Name)
		if err := RegisterActiveConnection(profile.Name, hubURL); err != nil {
			fmt.Fprintf(os.Stderr, "many-ai-cli-launcher: failed to record active connection: %v\n", err)
		}
		// 接続終了（errCh close）まで監視する。リモート serve 停止（Web UI の
		// 「Web のみ停止」含む）やトンネル切断時に、確立済み接続の登録と
		// launcher-active.json を掃除し、選択 UI のバッジを実態に合わせる。
		s.watchConnection(lc, profile.Name, errCh)
	case connErr := <-errCh:
		msg := "Connection failed"
		if connErr != nil {
			msg = connErr.Error()
		}
		s.failConnection(lc, msg)
	case <-waitTimer.C:
		s.failConnection(lc, "Connection timed out")
	case <-ctx.Done():
		s.failConnection(lc, "Connection cancelled")
	}
}

// watchConnection blocks until errCh is closed (= the connection terminated;
// see the Connector contract), then removes the established connection from
// s.conns and launcher-active.json. Errors received before the close are
// logged to stderr. runConnection 自体が専用 goroutine なのでブロックしてよい。
func (s *UIServer) watchConnection(lc *liveConn, name string, errCh <-chan error) {
	for connErr := range errCh {
		if connErr != nil {
			fmt.Fprintf(os.Stderr, "many-ai-cli-launcher: connection %q error: %v\n", name, connErr)
		}
	}
	lc.cancel()
	s.mu.Lock()
	current := s.conns[name] == lc
	if current {
		delete(s.conns, name)
	}
	s.mu.Unlock()
	// 既に新しい接続へ置き換わっている（再接続中）場合は、新しい接続の
	// 登録（同 profile・同 PID で上書き済み）を消さないよう何もしない。
	if current {
		_ = UnregisterActiveConnection(name)
		fmt.Fprintf(os.Stdout, "Connection %q closed.\n", name)
	}
}

// promoteConnection moves lc from inflight to the established map and
// publishes the success result. Returns false when lc has been superseded
// by a newer connect request (its context is already cancelled).
func (s *UIServer) promoteConnection(lc *liveConn, name, hubURL string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.inflight != lc {
		return false
	}
	s.inflight = nil
	s.conns[name] = lc
	s.connectRes = &connectResult{HubURL: hubURL}
	return true
}

// failConnection cancels lc's context (reaping the connector goroutine)
// and publishes the error result, unless this attempt has already been
// superseded by a newer connect request.
func (s *UIServer) failConnection(lc *liveConn, errMsg string) {
	lc.cancel()
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.inflight == lc {
		s.inflight = nil
		s.connectRes = &connectResult{Err: errMsg}
	}
}

func (s *UIServer) updateLastUsed(name string) {
	pf, err := LoadProfiles()
	if err != nil {
		return
	}
	pf.LastUsed = name
	_ = SaveProfiles(pf)
}
