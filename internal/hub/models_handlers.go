package hub

import (
	"net/http"

	"many-ai-cli/internal/config"
)

// handleModels は spawn フォーム用のモデル一覧を返す。
//   - GET  : キャッシュ尊重（Cloud 24h / Local 60s）
//   - POST : 両キャッシュを invalidate して再取得
func (s *Server) handleModels(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	force := r.Method == http.MethodPost
	if force {
		s.modelsCache.invalidate()
	}
	s.cfgMu.Lock()
	localCfg := append([]config.LocalModel(nil), s.cfg.LocalModels...)
	source := s.cfg.ModelsSource
	s.cfgMu.Unlock()
	if source == "" {
		source = config.DefaultModelsSource
	}
	resp := buildModelsResponse(s.modelsCache, s.modelsRemoteCache, source, localCfg, force)
	writeJSON(w, resp)
}
