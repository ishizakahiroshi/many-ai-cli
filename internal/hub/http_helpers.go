package hub

import (
	"context"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"strconv"
	"strings"
	"time"
)

const (
	// avatarMaxBytes はアバター画像アップロードの最大サイズ（5 MB）。
	avatarMaxBytes = 5 * 1024 * 1024
	// notifySoundMaxBytes は通知音アップロードの最大サイズ（2 MB）。
	notifySoundMaxBytes = 2 * 1024 * 1024
	// jsonBodyMaxBytes keeps local JSON endpoints from accepting unbounded bodies.
	jsonBodyMaxBytes = 1 * 1024 * 1024
)

// newExternalHTTPClient は外部リソース取得用の http.Client を返す。
// - 初回リクエストを含め https 以外は拒否
// - IP リテラルが loopback/private/link-local の場合は拒否
// - リダイレクトは最大 3 回まで
// - https 以外へのリダイレクトは拒否（スキームダウングレード防止）
func newExternalHTTPClient(timeout time.Duration) *http.Client {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	dialer := &net.Dialer{
		Timeout:   timeout,
		KeepAlive: 30 * time.Second,
	}
	transport.DialContext = privateNetworkBlockingDialContext(dialer.DialContext)
	return &http.Client{
		Timeout:   timeout,
		Transport: externalHTTPTransport{base: transport},
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) >= 3 {
				return errors.New("too many redirects")
			}
			if err := validateExternalHTTPSURL(req.URL); err != nil {
				return err
			}
			return nil
		},
	}
}

var makeExternalHTTPClient = newExternalHTTPClient

type dialContextFunc func(ctx context.Context, network, address string) (net.Conn, error)

func privateNetworkBlockingDialContext(next dialContextFunc) dialContextFunc {
	return func(ctx context.Context, network, address string) (net.Conn, error) {
		host, _, err := net.SplitHostPort(address)
		if err != nil {
			return nil, err
		}
		ips, err := net.DefaultResolver.LookupIPAddr(ctx, host)
		if err != nil {
			return nil, err
		}
		for _, ip := range ips {
			if isBlockedNetworkHost(ip.IP.String()) {
				return nil, errors.New("private network host blocked")
			}
		}
		return next(ctx, network, address)
	}
}

type externalHTTPTransport struct {
	base http.RoundTripper
}

func (t externalHTTPTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	if err := validateExternalHTTPSURL(req.URL); err != nil {
		return nil, err
	}
	base := t.base
	if base == nil {
		base = http.DefaultTransport
	}
	return base.RoundTrip(req)
}

func validateExternalHTTPSURL(u *url.URL) error {
	if u == nil {
		return errors.New("missing request URL")
	}
	if strings.ToLower(u.Scheme) != "https" {
		return errors.New("non-https request blocked")
	}
	if isBlockedNetworkHost(u.Hostname()) {
		return errors.New("private network host blocked")
	}
	return nil
}

func isBlockedNetworkHost(host string) bool {
	host = strings.TrimSuffix(strings.ToLower(strings.TrimSpace(host)), ".")
	if i := strings.LastIndex(host, "%"); i >= 0 {
		host = host[:i]
	}
	addr, err := netip.ParseAddr(host)
	if err != nil {
		return false
	}
	addr = addr.Unmap()
	return addr.IsUnspecified() ||
		addr.IsLoopback() ||
		addr.IsPrivate() ||
		addr.IsLinkLocalUnicast() ||
		addr.IsLinkLocalMulticast() ||
		addr.IsMulticast()
}

func validToken(got, want string) bool {
	if got == "" || want == "" {
		return false
	}
	return subtle.ConstantTimeCompare([]byte(got), []byte(want)) == 1
}

func requestToken(r *http.Request) string {
	if got := r.URL.Query().Get("token"); got != "" {
		return got
	}
	auth := strings.TrimSpace(r.Header.Get("Authorization"))
	if token, ok := strings.CutPrefix(auth, "Bearer "); ok {
		return strings.TrimSpace(token)
	}
	return ""
}

func (s *Server) requireToken(w http.ResponseWriter, r *http.Request) bool {
	if !s.validTokenOrTrustedRemote(requestToken(r), r.RemoteAddr) {
		writeJSONError(w, http.StatusUnauthorized, "unauthorized", "unauthorized")
		return false
	}
	return true
}

func (s *Server) validTokenOrTrustedRemote(got, remoteAddr string) bool {
	s.cfgMu.Lock()
	want := s.cfg.Token
	allowBypass := s.cfg.Hub.AllowLoopbackWithoutToken
	trustedNetworks := append([]string(nil), s.cfg.Hub.TrustedNetworks...)
	s.cfgMu.Unlock()
	if validToken(got, want) {
		return true
	}
	if !allowBypass {
		return false
	}
	if isLoopbackRemote(remoteAddr) {
		return true
	}
	return isTrustedRemote(remoteAddr, parseTrustedNetworks(trustedNetworks))
}

func remoteAddrIP(remoteAddr string) net.IP {
	value := strings.TrimSpace(remoteAddr)
	if value == "" {
		return nil
	}
	host, _, err := net.SplitHostPort(value)
	if err == nil {
		value = host
	}
	value = strings.Trim(value, "[]")
	if i := strings.LastIndex(value, "%"); i >= 0 {
		value = value[:i]
	}
	return net.ParseIP(value)
}

func isLoopbackRemote(remoteAddr string) bool {
	ip := remoteAddrIP(remoteAddr)
	return ip != nil && ip.IsLoopback()
}

func parseTrustedNetworks(values []string) []*net.IPNet {
	out := make([]*net.IPNet, 0, len(values))
	for _, raw := range values {
		_, cidr, err := net.ParseCIDR(strings.TrimSpace(raw))
		if err == nil && cidr != nil {
			out = append(out, cidr)
		}
	}
	return out
}

func isTrustedRemote(remoteAddr string, networks []*net.IPNet) bool {
	ip := remoteAddrIP(remoteAddr)
	if ip == nil {
		return false
	}
	for _, network := range networks {
		if network != nil && network.Contains(ip) {
			return true
		}
	}
	return false
}

func requireMethod(w http.ResponseWriter, r *http.Request, method string) bool {
	if r.Method != method {
		writeJSONError(w, http.StatusMethodNotAllowed, "method_not_allowed", "method not allowed")
		return false
	}
	return true
}

func requireMethodOneOf(w http.ResponseWriter, r *http.Request, methods ...string) bool {
	for _, method := range methods {
		if r.Method == method {
			return true
		}
	}
	writeJSONError(w, http.StatusMethodNotAllowed, "method_not_allowed", "method not allowed")
	return false
}

func (s *Server) guard(w http.ResponseWriter, r *http.Request, methods ...string) bool {
	if !s.requireToken(w, r) {
		return false
	}
	if len(methods) > 0 && !requireMethodOneOf(w, r, methods...) {
		return false
	}
	if methodRequiresHostCheck(r.Method) && !s.requireAllowedRequestOrigin(w, r) {
		return false
	}
	return true
}

func methodRequiresHostCheck(method string) bool {
	switch method {
	case http.MethodGet, http.MethodHead, http.MethodOptions:
		return false
	default:
		return true
	}
}

func (s *Server) requireAllowedRequestOrigin(w http.ResponseWriter, r *http.Request) bool {
	s.cfgMu.Lock()
	port := s.cfg.Hub.Port
	allowedHosts := append([]string(nil), s.cfg.Hub.AllowedHosts...)
	s.cfgMu.Unlock()
	if !isAllowedHubHost(r.Host, port, allowedHosts...) {
		writeJSONError(w, http.StatusForbidden, "forbidden", "host not allowed")
		return false
	}
	if origin := strings.TrimSpace(r.Header.Get("Origin")); origin != "" {
		if isAllowedHubOrigin(origin, port, allowedHosts...) {
			return true
		}
		writeJSONError(w, http.StatusForbidden, "forbidden", "origin not allowed")
		return false
	}
	site := strings.ToLower(strings.TrimSpace(r.Header.Get("Sec-Fetch-Site")))
	switch site {
	case "", "none", "same-origin":
		return true
	default:
		writeJSONError(w, http.StatusForbidden, "forbidden", "origin not allowed")
		return false
	}
}

func isAllowedHubHost(hostport string, port int, allowedHosts ...string) bool {
	hostport = strings.TrimSpace(hostport)
	if hostport == "" {
		return port <= 0
	}
	host, rawPort, err := net.SplitHostPort(hostport)
	if err != nil {
		host = strings.Trim(hostport, "[]")
		rawPort = ""
	}
	host = strings.TrimSuffix(strings.ToLower(strings.TrimSpace(host)), ".")
	if !isDefaultAllowedHubHost(host) && !isConfiguredAllowedHubHost(host, allowedHosts) {
		return false
	}
	if port <= 0 || rawPort == "" {
		return true
	}
	gotPort, err := strconv.Atoi(rawPort)
	return err == nil && gotPort == port
}

func isDefaultAllowedHubHost(host string) bool {
	return host == "127.0.0.1" || host == "localhost" || host == "::1"
}

func isConfiguredAllowedHubHost(host string, allowedHosts []string) bool {
	for _, raw := range allowedHosts {
		allowed := strings.TrimSuffix(strings.ToLower(strings.Trim(strings.TrimSpace(raw), "[]")), ".")
		if allowed != "" && host == allowed {
			return true
		}
	}
	return false
}

func isAllowedHubOrigin(rawOrigin string, port int, allowedHosts ...string) bool {
	u, err := url.Parse(rawOrigin)
	if err != nil {
		return false
	}
	if strings.ToLower(u.Scheme) != "http" || u.Host == "" {
		return false
	}
	return isAllowedHubHost(u.Host, port, allowedHosts...)
}

func decodeJSON(w http.ResponseWriter, r *http.Request, dst any) bool {
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, jsonBodyMaxBytes))
	if err := dec.Decode(dst); err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid json")
		return false
	}
	return true
}

func writeJSON(w http.ResponseWriter, v any) {
	writeJSONStatus(w, http.StatusOK, v)
}

func writeJSONStatus(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	if status != http.StatusOK {
		w.WriteHeader(status)
	}
	_ = json.NewEncoder(w).Encode(v)
}

type httpErrorResp struct {
	OK     bool   `json:"ok"`
	Error  string `json:"error"`
	Detail string `json:"detail,omitempty"`
}

func writeJSONError(w http.ResponseWriter, status int, code, detail string) {
	if code == "" {
		code = "error"
	}
	if detail == "" {
		detail = http.StatusText(status)
	}
	writeJSONStatus(w, status, httpErrorResp{
		OK:     false,
		Error:  code,
		Detail: detail,
	})
}

func httpErrorCode(status int) string {
	switch status {
	case http.StatusBadRequest:
		return "bad_request"
	case http.StatusUnauthorized:
		return "unauthorized"
	case http.StatusForbidden:
		return "forbidden"
	case http.StatusNotFound:
		return "not_found"
	case http.StatusMethodNotAllowed:
		return "method_not_allowed"
	default:
		if status >= 500 {
			return "internal_error"
		}
		return "error"
	}
}

func errorDetail(prefix string, err error) string {
	if err == nil {
		return prefix
	}
	if prefix == "" {
		return err.Error()
	}
	return fmt.Sprintf("%s: %v", prefix, err)
}
