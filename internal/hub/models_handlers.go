package hub

import (
	"net/http"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/nvidianim"
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
	ollamaBaseURL := s.cfg.Ollama.BaseURL
	ollamaAllowPrivate := s.cfg.Ollama.AllowPrivateHosts
	lmStudioBaseURL := s.cfg.LMStudio.BaseURL
	lmStudioAllowPrivate := s.cfg.LMStudio.AllowPrivateHosts
	nvidiaNIMEnabled := s.cfg.NVIDIANIM.Enabled
	s.cfgMu.Unlock()
	if source == "" {
		source = config.DefaultModelsSource
	}
	nvidiaOptions := nvidiaNIMCatalogOptions{enabled: nvidiaNIMEnabled}
	if nvidiaNIMEnabled {
		if configDir, err := config.Dir(); err == nil {
			if apiKey, _, err := nvidianim.ResolveAPIKey(configDir); err == nil {
				nvidiaOptions.apiKey = apiKey
			}
		}
	}
	resp := buildModelsResponseWithNVIDIANIM(s.modelsCache, s.modelsRemoteCache, source, localCfg, ollamaBaseURL, lmStudioBaseURL, force, []bool{ollamaAllowPrivate, lmStudioAllowPrivate}, nvidiaOptions)
	writeJSON(w, resp)
}
