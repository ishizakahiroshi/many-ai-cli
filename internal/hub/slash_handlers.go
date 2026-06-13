package hub

import (
	"net/http"
	"strings"
	"time"

	"many-ai-cli/internal/config"
)

func (s *Server) invalidateSlashCache(provider string) {
	s.slashCmdMu.Lock()
	delete(s.slashCmdCache, provider)
	s.slashCmdMu.Unlock()
}

// handleSlashCmdSources は provider ごとのソース URL を GET/POST で管理する。
func (s *Server) handleSlashCmdSources(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.cfgMu.Lock()
		src := config.EffectiveSlashCmdSources(s.cfg.SlashCmdSources)
		s.cfgMu.Unlock()
		writeJSON(w, src)
	case http.MethodPost:
		var body config.SlashCmdSources
		if !decodeJSON(w, r, &body) {
			return
		}
		body.Claude = strings.TrimSpace(body.Claude)
		body.Codex = strings.TrimSpace(body.Codex)
		body.Copilot = strings.TrimSpace(body.Copilot)
		body.CursorAgent = strings.TrimSpace(body.CursorAgent)
		if err := validateSlashCmdSource(body.Claude); err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("invalid claude source", err))
			return
		}
		if err := validateSlashCmdSource(body.Codex); err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("invalid codex source", err))
			return
		}
		if err := validateSlashCmdSource(body.Copilot); err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("invalid copilot source", err))
			return
		}
		if err := validateSlashCmdSource(body.CursorAgent); err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("invalid cursor-agent source", err))
			return
		}
		s.cfgMu.Lock()
		prev := s.cfg.SlashCmdSources
		s.cfg.SlashCmdSources = body
		s.cfgMu.Unlock()
		if body.Claude != prev.Claude {
			s.invalidateSlashCache("claude")
		}
		if body.Codex != prev.Codex {
			s.invalidateSlashCache("codex")
		}
		if body.Copilot != prev.Copilot {
			s.invalidateSlashCache("copilot")
		}
		if body.CursorAgent != prev.CursorAgent {
			s.invalidateSlashCache("cursor-agent")
		}
		if err := s.persistConfig(); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "save_failed", "save failed")
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	}
}

// handleSlashCommands は provider のスラッシュコマンド一覧を返す。
// GET: キャッシュがあれば返す（24h TTL）、なければ fetch。
// POST: キャッシュを強制リフレッシュ。
func (s *Server) handleSlashCommands(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPost) {
		return
	}

	provider := r.URL.Query().Get("provider")
	if provider != "claude" && provider != "codex" && provider != "copilot" && provider != "cursor-agent" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid provider")
		return
	}

	forceRefresh := r.Method == http.MethodPost

	s.slashCmdMu.Lock()
	entry := s.slashCmdCache[provider]
	s.slashCmdMu.Unlock()

	if !forceRefresh && entry != nil && time.Since(entry.fetchedAt) < slashCmdCacheTTL {
		writeSlashCmdsResp(w, entry)
		return
	}

	s.cfgMu.Lock()
	src := config.EffectiveSlashCmdSources(s.cfg.SlashCmdSources)
	var sourceURL string
	switch provider {
	case "claude":
		sourceURL = src.Claude
	case "codex":
		sourceURL = src.Codex
	case "copilot":
		sourceURL = src.Copilot
	case "cursor-agent":
		sourceURL = src.CursorAgent
	}
	s.cfgMu.Unlock()

	if sourceURL == "" {
		writeJSONError(w, http.StatusNotFound, "not_found", "source URL not configured")
		return
	}

	cmds, err := fetchAndParseSlashCmds(sourceURL)
	if err != nil {
		s.logger.Warn("slash cmd fetch failed", "provider", provider, "err", err)
		if entry != nil {
			writeSlashCmdsResp(w, entry)
			return
		}
		writeJSONError(w, http.StatusBadGateway, "fetch_failed", errorDetail("fetch failed", err))
		return
	}

	newEntry := &slashCmdCacheEntry{
		cmds:      cmds,
		fetchedAt: time.Now(),
		sourceURL: sourceURL,
	}
	s.slashCmdMu.Lock()
	s.slashCmdCache[provider] = newEntry
	s.slashCmdMu.Unlock()

	writeSlashCmdsResp(w, newEntry)
}

func writeSlashCmdsResp(w http.ResponseWriter, entry *slashCmdCacheEntry) {
	writeJSON(w, map[string]any{
		"cmds":       entry.cmds,
		"fetched_at": entry.fetchedAt.UTC().Format(time.RFC3339),
		"source_url": entry.sourceURL,
	})
}

// handleUsageLinkDefaults は全 provider の usage リンクデフォルト URL を返す。
// GitHub の resources/usage-links/defaults.json から TTL 24h でキャッシュして提供し、
// 取得失敗時はハードコード値にフォールバックする。
func (s *Server) handleUsageLinkDefaults(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	defaults := s.usageLinkCache.get(config.DefaultUsageLinkSource)
	writeJSON(w, defaults)
}
