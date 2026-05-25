package hub

import (
	"crypto/subtle"
	"encoding/json"
	"errors"
	"net/http"
	"time"
)

const (
	// avatarMaxBytes はアバター画像アップロードの最大サイズ（5 MB）。
	avatarMaxBytes = 5 * 1024 * 1024
	// notifySoundMaxBytes は通知音アップロードの最大サイズ（2 MB）。
	notifySoundMaxBytes = 2 * 1024 * 1024
)

// newExternalHTTPClient は外部リソース取得用の http.Client を返す。
// - リダイレクトは最大 3 回まで
// - https 以外へのリダイレクトは拒否（スキームダウングレード防止）
func newExternalHTTPClient(timeout time.Duration) *http.Client {
	return &http.Client{
		Timeout: timeout,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) >= 3 {
				return errors.New("too many redirects")
			}
			if req.URL.Scheme != "https" {
				return errors.New("non-https redirect blocked")
			}
			return nil
		},
	}
}

func (s *Server) requireToken(w http.ResponseWriter, r *http.Request) bool {
	// crypto/subtle.ConstantTimeCompare でタイミング攻撃を防ぐ。
	got := r.URL.Query().Get("token")
	if subtle.ConstantTimeCompare([]byte(got), []byte(s.cfg.Token)) != 1 {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return false
	}
	return true
}

func requireMethod(w http.ResponseWriter, r *http.Request, method string) bool {
	if r.Method != method {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return false
	}
	return true
}

func decodeJSON(w http.ResponseWriter, r *http.Request, dst any) bool {
	if err := json.NewDecoder(r.Body).Decode(dst); err != nil {
		http.Error(w, "bad request", http.StatusBadRequest)
		return false
	}
	return true
}

func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(v)
}
