package hub

import (
	"sort"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
	"many-ai-cli/internal/usagelocal"
)

type usageWindow struct {
	UsedPercent   float64 `json:"used_percent"`
	WindowMinutes int     `json:"window_minutes,omitempty"`
	ResetsAt      int64   `json:"resets_at,omitempty"`
}

type claudeSubscriptionUsage struct {
	FiveHour *usageWindow `json:"five_hour,omitempty"`
	SevenDay *usageWindow `json:"seven_day,omitempty"`
}

type codexSubscriptionUsage struct {
	Primary        *usageWindow `json:"primary,omitempty"`
	Secondary      *usageWindow `json:"secondary,omitempty"`
	PlanType       string       `json:"plan_type,omitempty"`
	CreditsBalance string       `json:"credits_balance,omitempty"`
}

type grokSubscriptionUsage struct {
	UsedPercent float64 `json:"used_percent"`
	PeriodStart string  `json:"period_start,omitempty"`
	PeriodEnd   string  `json:"period_end,omitempty"`
	PeriodType  string  `json:"period_type,omitempty"`
}

type subscriptionUsageProfile struct {
	ID             string                   `json:"id"`
	Name           string                   `json:"name,omitempty"`
	Plan           string                   `json:"plan,omitempty"`
	RetrievedAt    string                   `json:"retrieved_at,omitempty"`
	ProbeAvailable bool                     `json:"probe_available,omitempty"`
	Claude         *claudeSubscriptionUsage `json:"claude,omitempty"`
	Codex          *codexSubscriptionUsage  `json:"codex,omitempty"`
	Grok           *grokSubscriptionUsage   `json:"grok,omitempty"`
}

type subscriptionUsageProvider struct {
	Provider string                     `json:"provider"`
	Profiles []subscriptionUsageProfile `json:"profiles"`
}

type subscriptionUsageResponse struct {
	Providers []subscriptionUsageProvider `json:"providers"`
}

type subscriptionUsageKey struct {
	provider string
	id       string
}

type subscriptionUsageValue struct {
	claude      *claudeSubscriptionUsage
	codex       *codexSubscriptionUsage
	grok        *grokSubscriptionUsage
	retrievedAt time.Time
	source      string
}

type subscriptionUsageStore struct {
	mu      sync.Mutex
	entries map[subscriptionUsageKey]subscriptionUsageValue
}

func (s *Server) subscriptionUsageStoreForServer() *subscriptionUsageStore {
	s.subscriptionUsageMu.Lock()
	defer s.subscriptionUsageMu.Unlock()
	if s.subscriptionUsage == nil {
		s.subscriptionUsage = newSubscriptionUsageStore()
	}
	return s.subscriptionUsage
}

func (s *Server) refreshSubscriptionUsage() *subscriptionUsageStore {
	store := s.subscriptionUsageStoreForServer()
	if dir := subscriptionConfigDir(); dir != "" {
		store.refreshLocal(s.snapshotCfg(), dir, time.Now())
	}
	return store
}

func (s *Server) recordSessionSubscriptionUsage(sessionID int, stat *usageStat, at time.Time) {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	provider, profileID := "", ""
	if ses != nil {
		provider = ses.Provider
		profileID = ses.SubscriptionProfileID
	}
	s.sessionsMu.Unlock()
	if provider == "" || profileID == "" {
		return
	}
	s.subscriptionUsageStoreForServer().recordSession(provider, profileID, stat, at)
}

func newSubscriptionUsageStore() *subscriptionUsageStore {
	return &subscriptionUsageStore{entries: map[subscriptionUsageKey]subscriptionUsageValue{}}
}

func (s *subscriptionUsageStore) putLive(provider, id string, value subscriptionUsageValue, at time.Time) {
	provider = strings.TrimSpace(provider)
	id = config.NormalizeSubscriptionID(id)
	if provider == "" || id == "" {
		return
	}
	value.retrievedAt = at
	value.source = "live"
	s.mu.Lock()
	defer s.mu.Unlock()
	s.entries[subscriptionUsageKey{provider: provider, id: id}] = value
}

func (s *subscriptionUsageStore) putLocal(provider, id string, value subscriptionUsageValue, at time.Time) {
	provider = strings.TrimSpace(provider)
	id = config.NormalizeSubscriptionID(id)
	if provider == "" || id == "" {
		return
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	key := subscriptionUsageKey{provider: provider, id: id}
	if current, ok := s.entries[key]; ok && current.source == "live" {
		// A live relay value is the more direct source and must not be replaced
		// by an offline reread on every UI refresh.
		return
	}
	value.retrievedAt = at
	value.source = "local"
	s.entries[key] = value
}

func (s *subscriptionUsageStore) recordSession(provider, id string, stat *usageStat, at time.Time) {
	if stat == nil || strings.TrimSpace(id) == "" {
		return
	}
	value := subscriptionUsageValue{}
	switch provider {
	case "claude":
		if stat.RateLimit5hPct == 0 && stat.RateLimit5hReset == 0 && stat.RateLimit7dPct == 0 && stat.RateLimit7dReset == 0 {
			return
		}
		value.claude = &claudeSubscriptionUsage{
			FiveHour: &usageWindow{UsedPercent: stat.RateLimit5hPct, ResetsAt: stat.RateLimit5hReset},
			SevenDay: &usageWindow{UsedPercent: stat.RateLimit7dPct, ResetsAt: stat.RateLimit7dReset},
		}
	case "codex":
		if !stat.CodexRateLimitsPresent {
			return
		}
		value.codex = &codexSubscriptionUsage{
			Primary:        toUsageWindow(stat.CodexPrimaryUsedPct, stat.CodexPrimaryWindowMinutes, stat.CodexPrimaryReset),
			Secondary:      toUsageWindow(stat.CodexSecondaryUsedPct, stat.CodexSecondaryWindowMinutes, stat.CodexSecondaryReset),
			PlanType:       stat.CodexPlanType,
			CreditsBalance: stat.CodexCreditsBalance,
		}
	default:
		return
	}
	s.putLive(provider, id, value, at)
}

func toUsageWindow(pct float64, minutes int, reset int64) *usageWindow {
	if pct == 0 && minutes == 0 && reset == 0 {
		return nil
	}
	return &usageWindow{UsedPercent: pct, WindowMinutes: minutes, ResetsAt: reset}
}

func (s *subscriptionUsageStore) refreshLocal(cfg *config.Config, configDir string, at time.Time) {
	if cfg == nil || configDir == "" {
		return
	}
	for provider, profiles := range cfg.Subscriptions {
		if provider != "codex" && provider != "grok" {
			continue
		}
		if _, ok := subscription.AdapterFor(provider); !ok {
			continue
		}
		for _, profile := range profiles {
			if !profile.IsEnabled() {
				continue
			}
			id := config.NormalizeSubscriptionID(profile.ID)
			if config.ValidateSubscriptionID(id) != nil {
				continue
			}
			dir, err := config.ResolveSubscriptionProfileDir(configDir, provider, profile)
			if err != nil {
				continue
			}
			switch provider {
			case "codex":
				if usage, ok := usagelocal.ReadCodexProfile(dir); ok {
					s.putLocal(provider, id, subscriptionUsageValue{codex: codexUsageFromLocal(usage)}, at)
				}
			case "grok":
				if usage, ok := usagelocal.ReadGrokProfile(dir); ok {
					s.putLocal(provider, id, subscriptionUsageValue{grok: &grokSubscriptionUsage{
						UsedPercent: usage.UsedPercent,
						PeriodStart: usage.PeriodStart,
						PeriodEnd:   usage.PeriodEnd,
						PeriodType:  usage.PeriodType,
					}}, at)
				}
			}
		}
	}
}

func codexUsageFromLocal(usage usagelocal.CodexUsage) *codexSubscriptionUsage {
	return &codexSubscriptionUsage{
		Primary:        localWindow(usage.Primary),
		Secondary:      localWindow(usage.Secondary),
		PlanType:       usage.PlanType,
		CreditsBalance: usage.CreditsBalance,
	}
}

func localWindow(window *usagelocal.RateLimitWindow) *usageWindow {
	if window == nil {
		return nil
	}
	return &usageWindow{UsedPercent: window.UsedPercent, WindowMinutes: window.WindowMinutes, ResetsAt: window.ResetsAt}
}

func (s *subscriptionUsageStore) snapshot(cfg *config.Config) subscriptionUsageResponse {
	if cfg == nil {
		return subscriptionUsageResponse{Providers: []subscriptionUsageProvider{}}
	}
	providers := make([]string, 0, 3)
	for _, provider := range []string{"claude", "codex", "grok"} {
		if len(cfg.Subscriptions[provider]) > 0 {
			providers = append(providers, provider)
		}
	}
	sort.Strings(providers)

	s.mu.Lock()
	defer s.mu.Unlock()
	result := subscriptionUsageResponse{Providers: make([]subscriptionUsageProvider, 0, len(providers))}
	for _, provider := range providers {
		item := subscriptionUsageProvider{Provider: provider, Profiles: []subscriptionUsageProfile{}}
		for _, profile := range cfg.Subscriptions[provider] {
			if !profile.IsEnabled() {
				continue
			}
			id := config.NormalizeSubscriptionID(profile.ID)
			if config.ValidateSubscriptionID(id) != nil {
				continue
			}
			row := subscriptionUsageProfile{
				ID:             id,
				Name:           strings.TrimSpace(profile.Name),
				Plan:           strings.TrimSpace(profile.Plan),
				ProbeAvailable: provider == "claude",
			}
			if row.Name == "" {
				row.Name = id
			}
			if value, ok := s.entries[subscriptionUsageKey{provider: provider, id: id}]; ok {
				row.RetrievedAt = value.retrievedAt.Format(time.RFC3339)
				row.Claude = value.claude
				row.Codex = value.codex
				row.Grok = value.grok
			}
			item.Profiles = append(item.Profiles, row)
		}
		if len(item.Profiles) > 0 {
			result.Providers = append(result.Providers, item)
		}
	}
	return result
}
