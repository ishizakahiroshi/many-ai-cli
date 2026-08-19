package hub

import (
	"net/http"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/config"
)

func (s *Server) invalidateSlashCache(provider string) {
	s.slashCmdMu.Lock()
	for key := range s.slashCmdCache {
		if strings.HasPrefix(key, provider+"|") || key == provider {
			delete(s.slashCmdCache, key)
		}
	}
	s.slashCmdMu.Unlock()
}

// slashCmdSourcesPatch keeps omitted provider fields intact. The settings UI
// predates the opencode/grok fields, so decoding directly into the complete
// struct would turn those omitted fields into empty strings and overwrite
// user-configured sources.
type slashCmdSourcesPatch struct {
	Claude      *string `json:"claude"`
	Codex       *string `json:"codex"`
	Copilot     *string `json:"copilot"`
	CursorAgent *string `json:"cursor-agent"`
	Opencode    *string `json:"opencode"`
	Grok        *string `json:"grok"`
}

func applySlashCmdSourcesPatch(base config.SlashCmdSources, patch slashCmdSourcesPatch) config.SlashCmdSources {
	if patch.Claude != nil {
		base.Claude = strings.TrimSpace(*patch.Claude)
	}
	if patch.Codex != nil {
		base.Codex = strings.TrimSpace(*patch.Codex)
	}
	if patch.Copilot != nil {
		base.Copilot = strings.TrimSpace(*patch.Copilot)
	}
	if patch.CursorAgent != nil {
		base.CursorAgent = strings.TrimSpace(*patch.CursorAgent)
	}
	if patch.Opencode != nil {
		base.Opencode = strings.TrimSpace(*patch.Opencode)
	}
	if patch.Grok != nil {
		base.Grok = strings.TrimSpace(*patch.Grok)
	}
	return base
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
		var patch slashCmdSourcesPatch
		if !decodeJSON(w, r, &patch) {
			return
		}
		s.cfgMu.Lock()
		prev := s.cfg.SlashCmdSources
		body := applySlashCmdSourcesPatch(prev, patch)
		s.cfgMu.Unlock()
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
		if err := validateSlashCmdSource(body.Opencode); err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("invalid opencode source", err))
			return
		}
		if err := validateSlashCmdSource(body.Grok); err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("invalid grok source", err))
			return
		}
		s.cfgMu.Lock()
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
		if body.Opencode != prev.Opencode {
			s.invalidateSlashCache("opencode")
		}
		if body.Grok != prev.Grok {
			s.invalidateSlashCache("grok")
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
	if provider != "claude" && provider != "codex" && provider != "copilot" && provider != "cursor-agent" && provider != "opencode" && provider != "grok" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid provider")
		return
	}

	forceRefresh := r.Method == http.MethodPost
	searchCtx := s.skillSearchContextForRequest(provider, r)
	cacheKey := slashCmdCacheKey(provider, searchCtx)

	s.slashCmdMu.Lock()
	entry := s.slashCmdCache[cacheKey]
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
	case "opencode":
		sourceURL = src.Opencode
	case "grok":
		sourceURL = src.Grok
	}
	s.cfgMu.Unlock()

	if sourceURL == "" {
		writeJSONError(w, http.StatusNotFound, "not_found", "source URL not configured")
		return
	}

	skills := discoverSkillSlashCmds(provider, searchCtx)
	cmds, err := fetchAndParseSlashCmds(sourceURL)
	if err != nil {
		s.logger.Warn("slash cmd fetch failed", "provider", provider, "err", err)
		if entry != nil {
			fallback := *entry
			fallback.cmds = mergeSkillSlashCmds(provider, entry.cmds, skills)
			writeSlashCmdsResp(w, &fallback)
			return
		}
		if len(skills) > 0 {
			newEntry := &slashCmdCacheEntry{
				cmds:      dedupeSlashCmds(skills),
				fetchedAt: time.Now(),
				sourceURL: sourceURL,
			}
			s.slashCmdMu.Lock()
			s.slashCmdCache[cacheKey] = newEntry
			s.slashCmdMu.Unlock()
			writeSlashCmdsResp(w, newEntry)
			return
		}
		writeJSONError(w, http.StatusBadGateway, "fetch_failed", errorDetail("fetch failed", err))
		return
	}
	cmds = mergeSkillSlashCmds(provider, cmds, skills)

	newEntry := &slashCmdCacheEntry{
		cmds:      cmds,
		fetchedAt: time.Now(),
		sourceURL: sourceURL,
	}
	s.slashCmdMu.Lock()
	s.slashCmdCache[cacheKey] = newEntry
	s.slashCmdMu.Unlock()

	writeSlashCmdsResp(w, newEntry)
}

func slashCmdCacheKey(provider string, ctx skillSearchContext) string {
	return provider + "|" + filepathCleanForCache(ctx.HomeDir) + "|" + filepathCleanForCache(ctx.CodexHome) + "|" + filepathCleanForCache(ctx.ClaudeDir)
}

func filepathCleanForCache(path string) string {
	path = strings.TrimSpace(path)
	if path == "" {
		return ""
	}
	return strings.ToLower(path)
}

func (s *Server) skillSearchContextForRequest(provider string, r *http.Request) skillSearchContext {
	raw := strings.TrimSpace(r.URL.Query().Get("session_id"))
	if raw == "" {
		raw = strings.TrimSpace(r.URL.Query().Get("session"))
	}
	if raw == "" {
		return skillSearchContext{}
	}
	id, err := strconv.Atoi(raw)
	if err != nil || id <= 0 {
		return skillSearchContext{}
	}
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[id]
	if ses == nil || ses.Provider != provider {
		return skillSearchContext{}
	}
	return skillSearchContext{
		HomeDir:   ses.HomeDir,
		CodexHome: ses.CodexHome,
		ClaudeDir: ses.ClaudeDir,
	}
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
