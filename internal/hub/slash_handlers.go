package hub

import (
	"encoding/json"
	"net/http"
	"time"

	"ai-cli-hub/internal/config"
)

func (s *Server) invalidateSlashCache(provider string) {
	s.slashCmdMu.Lock()
	delete(s.slashCmdCache, provider)
	s.slashCmdMu.Unlock()
}

// handleSlashCmdSources は provider ごとのソース URL を GET/POST で管理する。
func (s *Server) handleSlashCmdSources(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	switch r.Method {
	case http.MethodGet:
		s.mu.Lock()
		src := config.EffectiveSlashCmdSources(s.cfg.SlashCmdSources)
		s.mu.Unlock()
		_ = json.NewEncoder(w).Encode(src)
	case http.MethodPost:
		var body config.SlashCmdSources
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(w, "bad request", http.StatusBadRequest)
			return
		}
		s.mu.Lock()
		prev := s.cfg.SlashCmdSources
		s.cfg.SlashCmdSources = body
		s.mu.Unlock()
		if body.Claude != prev.Claude {
			s.invalidateSlashCache("claude")
		}
		if body.Codex != prev.Codex {
			s.invalidateSlashCache("codex")
		}
		if err := config.Save(s.cfg); err != nil {
			http.Error(w, "save failed", http.StatusInternalServerError)
			return
		}
		_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

// handleSlashCommands は provider のスラッシュコマンド一覧を返す。
// GET: キャッシュがあれば返す（24h TTL）、なければ fetch。
// POST: キャッシュを強制リフレッシュ。
func (s *Server) handleSlashCommands(w http.ResponseWriter, r *http.Request) {
	if r.URL.Query().Get("token") != s.cfg.Token {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if r.Method != http.MethodGet && r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	provider := r.URL.Query().Get("provider")
	if provider != "claude" && provider != "codex" {
		http.Error(w, "invalid provider", http.StatusBadRequest)
		return
	}

	forceRefresh := r.Method == http.MethodPost

	s.slashCmdMu.Lock()
	entry := s.slashCmdCache[provider]
	s.slashCmdMu.Unlock()

	if !forceRefresh && entry != nil && time.Since(entry.fetchedAt) < slashCmdCacheTTL {
		w.Header().Set("Content-Type", "application/json")
		writeSlashCmdsResp(w, entry)
		return
	}

	s.mu.Lock()
	src := config.EffectiveSlashCmdSources(s.cfg.SlashCmdSources)
	var sourceURL string
	switch provider {
	case "claude":
		sourceURL = src.Claude
	case "codex":
		sourceURL = src.Codex
	}
	s.mu.Unlock()

	if sourceURL == "" {
		http.Error(w, "source URL not configured", http.StatusNotFound)
		return
	}

	cmds, err := fetchAndParseSlashCmds(sourceURL)
	if err != nil {
		s.logger.Warn("slash cmd fetch failed", "provider", provider, "err", err)
		if entry != nil {
			w.Header().Set("Content-Type", "application/json")
			writeSlashCmdsResp(w, entry)
			return
		}
		http.Error(w, "fetch failed: "+err.Error(), http.StatusBadGateway)
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

	w.Header().Set("Content-Type", "application/json")
	writeSlashCmdsResp(w, newEntry)
}

func writeSlashCmdsResp(w http.ResponseWriter, entry *slashCmdCacheEntry) {
	_ = json.NewEncoder(w).Encode(map[string]any{
		"cmds":       entry.cmds,
		"fetched_at": entry.fetchedAt.UTC().Format(time.RFC3339),
		"source_url": entry.sourceURL,
	})
}
