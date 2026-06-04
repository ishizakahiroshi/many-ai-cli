//go:build windows

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
	"strconv"
	"strings"
	"sync"
	"time"
)

//go:embed ui
var uiFS embed.FS

// UIServer is a short-lived HTTP server that serves the profile selection page.
// It starts on a random loopback port and shuts down after a successful
// connection or when ctx is cancelled.
type UIServer struct {
	token  string
	port   int
	server *http.Server

	mu         sync.Mutex
	connectReq *connectRequest      // set when POST /api/connect is received
	connectRes *connectResult       // set when connection resolves
	connCancel context.CancelFunc   // cancels the active connection goroutine
}

type connectRequest struct {
	Name string `json:"name"`
}

type connectResult struct {
	HubURL string // non-empty on success
	Err    string // non-empty on failure
}

// profilesResponse is the JSON envelope for GET /api/profiles.
type profilesResponse struct {
	OK       bool      `json:"ok"`
	Profiles []Profile `json:"profiles"`
	LastUsed string    `json:"last_used,omitempty"`
}

// NewUIServer creates a UIServer. Call Serve to start it.
func NewUIServer() (*UIServer, error) {
	token, err := generateToken()
	if err != nil {
		return nil, fmt.Errorf("generate ui token: %w", err)
	}
	return &UIServer{token: token}, nil
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

	s.server = &http.Server{
		Handler:      mux,
		ReadTimeout:  30 * time.Second,
		WriteTimeout: 60 * time.Second,
	}

	go func() {
		<-ctx.Done()
		shutCtx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		defer cancel()
		_ = s.server.Shutdown(shutCtx)
	}()

	go func() {
		_ = s.server.Serve(ln)
	}()

	pageURL := fmt.Sprintf("http://127.0.0.1:%d/?token=%s", s.port, s.token)
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
		"ok":     false,
		"error":  detail,
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
		writeUIJSON(w, profilesResponse{
			OK:       true,
			Profiles: pf.Profiles,
			LastUsed: pf.LastUsed,
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

	// Cancel any in-progress connection.
	s.mu.Lock()
	if s.connCancel != nil {
		s.connCancel()
	}
	s.connectReq = &req
	s.connectRes = nil
	ctx, cancel := context.WithTimeout(context.Background(), 120*time.Second)
	s.connCancel = cancel
	s.mu.Unlock()

	go s.runConnection(ctx, profile)

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

// runConnection starts the Connector for profile and stores the result.
func (s *UIServer) runConnection(ctx context.Context, profile Profile) {
	var conn Connector
	var err error
	switch profile.Type {
	case ProfileTypeWSL:
		conn = NewWSLConnector()
	case ProfileTypeSSH:
		conn = NewSSHConnector()
	default:
		s.setConnectResult("", fmt.Sprintf("unsupported profile type %q", profile.Type))
		return
	}

	urlCh := make(chan string, 1)
	errCh := make(chan error, 1)

	if err = conn.Start(ctx, profile, urlCh, errCh); err != nil {
		s.setConnectResult("", err.Error())
		return
	}

	select {
	case hubURL, ok := <-urlCh:
		if !ok || hubURL == "" {
			// errCh may have the reason.
			select {
			case connErr := <-errCh:
				if connErr != nil {
					s.setConnectResult("", connErr.Error())
					return
				}
			default:
			}
			s.setConnectResult("", "接続に失敗しました (Hub URL が取得できませんでした)")
			return
		}
		// Update last_used.
		s.updateLastUsed(profile.Name)
		s.setConnectResult(hubURL, "")
	case connErr := <-errCh:
		msg := "接続に失敗しました"
		if connErr != nil {
			msg = connErr.Error()
		}
		s.setConnectResult("", msg)
	case <-ctx.Done():
		s.setConnectResult("", "接続がタイムアウトしました")
	}
}

func (s *UIServer) setConnectResult(hubURL, errMsg string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.connectRes = &connectResult{HubURL: hubURL, Err: errMsg}
}

func (s *UIServer) updateLastUsed(name string) {
	pf, err := LoadProfiles()
	if err != nil {
		return
	}
	pf.LastUsed = name
	_ = SaveProfiles(pf)
}
