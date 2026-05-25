package hub

import (
	"encoding/json"
	"net/http"

	"any-ai-cli/internal/config"
)

// handleModels は spawn フォーム用のモデル一覧を返す。
//   - GET  : キャッシュ尊重（Cloud 24h / Local 60s）
//   - POST : 両キャッシュを invalidate して再取得
func (s *Server) handleModels(w http.ResponseWriter, r *http.Request) {
	if !s.requireToken(w, r) {
		return
	}
	if r.Method != http.MethodGet && r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	force := r.Method == http.MethodPost
	if force {
		s.modelsCache.invalidate()
	}
	s.mu.Lock()
	localCfg := append([]config.LocalModel(nil), s.cfg.LocalModels...)
	s.mu.Unlock()
	resp := buildModelsResponse(s.modelsCache, s.modelsRemoteCache, config.DefaultModelsSource, localCfg, force)
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(resp)
}
