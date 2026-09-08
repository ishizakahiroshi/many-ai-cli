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
	UsedPercent      float64 `json:"used_percent"`
	RemainingPercent float64 `json:"remaining_percent"`
	WindowMinutes    int     `json:"window_minutes,omitempty"`
	ResetsAt         int64   `json:"resets_at,omitempty"`
}

type claudeSubscriptionUsage struct {
	FiveHour *usageWindow `json:"five_hour,omitempty"`
	SevenDay *usageWindow `json:"seven_day,omitempty"`
}

type codexSubscriptionUsage struct {
	Primary        *usageWindow  `json:"primary,omitempty"`
	Secondary      *usageWindow  `json:"secondary,omitempty"`
	PlanType       string        `json:"plan_type,omitempty"`
	Credits        *codexCredits `json:"credits,omitempty"`
	CreditsBalance string        `json:"credits_balance,omitempty"`
}

type codexCredits struct {
	HasCredits bool   `json:"has_credits"`
	Unlimited  bool   `json:"unlimited"`
	Balance    string `json:"balance,omitempty"`
}

type grokSubscriptionUsage struct {
	UsedPercent      float64 `json:"used_percent"`
	RemainingPercent float64 `json:"remaining_percent"`
	PeriodStart      string  `json:"period_start,omitempty"`
	PeriodEnd        string  `json:"period_end,omitempty"`
	PeriodType       string  `json:"period_type,omitempty"`
}

type subscriptionUsageProfile struct {
	ID             string                   `json:"id"`
	Name           string                   `json:"name,omitempty"`
	Plan           string                   `json:"plan,omitempty"`
	RetrievedAt    string                   `json:"retrieved_at,omitempty"`
	ProbeAvailable bool                     `json:"probe_available,omitempty"`
	ProbeState     string                   `json:"probe_state,omitempty"`
	AuthStatus     string                   `json:"auth_status,omitempty"`
	AuthChecking   bool                     `json:"auth_checking,omitempty"`
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

type subscriptionAuthValue struct {
	state     string
	checkedAt time.Time
}

type subscriptionUsageStore struct {
	mu      sync.Mutex
	entries map[subscriptionUsageKey]subscriptionUsageValue
	auth    map[subscriptionUsageKey]subscriptionAuthValue
	// authChecking marks the profiles whose login state is being re-read in the
	// background, so repeated polls do not start the same vendor CLI again.
	authChecking map[subscriptionUsageKey]bool
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
	return &subscriptionUsageStore{
		entries:      map[subscriptionUsageKey]subscriptionUsageValue{},
		auth:         map[subscriptionUsageKey]subscriptionAuthValue{},
		authChecking: map[subscriptionUsageKey]bool{},
	}
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
	// A provider absent from the usage-source table (子 plan: docs/local/plan_session-handoff-board_c6_usage-dispatch.md)
	// cannot be reasoned about at all, so it is rejected before the
	// provider-specific parsing below even starts. This does not narrow which
	// of claude/codex the switch accepts today — Codex's table Kind is
	// local-file, not pushed, but it still keeps a pushed channel here for a
	// session whose CLI hook posts rate_limits directly — the table only
	// answers "is this a real provider", not "which of its channels exist".
	if subscription.UsageSourceFor(provider).Kind == subscription.UsageSourceNone {
		return
	}
	value := subscriptionUsageValue{}
	switch provider {
	case "claude":
		claudeObserved := stat.ClaudeRateLimitsPresent || stat.RateLimit5hPct != 0 || stat.RateLimit5hReset != 0 || stat.RateLimit7dPct != 0 || stat.RateLimit7dReset != 0
		if !claudeObserved {
			return
		}
		value.claude = &claudeSubscriptionUsage{}
		fiveHourPresent := stat.ClaudeFiveHourPresent || stat.RateLimit5hPct != 0 || stat.RateLimit5hReset != 0
		sevenDayPresent := stat.ClaudeSevenDayPresent || stat.RateLimit7dPct != 0 || stat.RateLimit7dReset != 0
		if fiveHourPresent {
			value.claude.FiveHour = toUsageWindow(stat.RateLimit5hPct, 0, stat.RateLimit5hReset)
		}
		if sevenDayPresent {
			value.claude.SevenDay = toUsageWindow(stat.RateLimit7dPct, 0, stat.RateLimit7dReset)
		}
	case "codex":
		if !stat.CodexRateLimitsPresent {
			return
		}
		value.codex = &codexSubscriptionUsage{
			PlanType: stat.CodexPlanType,
		}
		primaryPresent := stat.CodexPrimaryPresent || stat.CodexPrimaryUsedPct != 0 || stat.CodexPrimaryWindowMinutes != 0 || stat.CodexPrimaryReset != 0
		secondaryPresent := stat.CodexSecondaryPresent || stat.CodexSecondaryUsedPct != 0 || stat.CodexSecondaryWindowMinutes != 0 || stat.CodexSecondaryReset != 0
		if primaryPresent {
			value.codex.Primary = toUsageWindow(stat.CodexPrimaryUsedPct, stat.CodexPrimaryWindowMinutes, stat.CodexPrimaryReset)
		}
		if secondaryPresent {
			value.codex.Secondary = toUsageWindow(stat.CodexSecondaryUsedPct, stat.CodexSecondaryWindowMinutes, stat.CodexSecondaryReset)
		}
		if stat.CodexCreditsPresent {
			value.codex.Credits = &codexCredits{
				HasCredits: stat.CodexHasCredits,
				Unlimited:  stat.CodexCreditsUnlimited,
				Balance:    stat.CodexCreditsBalance,
			}
			if stat.CodexHasCredits && !stat.CodexCreditsUnlimited {
				value.codex.CreditsBalance = stat.CodexCreditsBalance
			}
		}
	default:
		return
	}
	s.putLive(provider, id, value, at)
}

func toUsageWindow(pct float64, minutes int, reset int64) *usageWindow {
	return usageWindowFromSource(pct, "used", minutes, reset)
}

func usageWindowFromSource(value float64, semantics string, minutes int, reset int64) *usageWindow {
	remaining, used, ok := normalizeUsagePercent(value, semantics)
	if !ok {
		return nil
	}
	return &usageWindow{UsedPercent: used, RemainingPercent: remaining, WindowMinutes: minutes, ResetsAt: reset}
}

func normalizeUsagePercent(value float64, semantics string) (remaining, used float64, ok bool) {
	if value != value {
		return 0, 0, false
	}
	if value < 0 {
		value = 0
	}
	if value > 100 {
		value = 100
	}
	switch strings.ToLower(strings.TrimSpace(semantics)) {
	case "used":
		return 100 - value, value, true
	case "remaining":
		return value, 100 - value, true
	default:
		return 0, 0, false
	}
}

func (s *subscriptionUsageStore) refreshLocal(cfg *config.Config, configDir string, at time.Time) {
	if cfg == nil || configDir == "" {
		return
	}
	for provider, profiles := range cfg.Subscriptions {
		// Which providers get a local file resolved and read at all now comes
		// from the usage-source table (子 plan: docs/local/plan_session-handoff-board_c6_usage-dispatch.md
		// 内部 C1), not a hand-written provider comparison. Adding a
		// local-file provider means adding its row there plus its parsing
		// case in usagelocal.ReadProfile — this loop needs no further edit.
		if subscription.UsageSourceFor(provider).Kind != subscription.UsageSourceLocalFile {
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
			local, ok := usagelocal.ReadProfile(provider, dir)
			if !ok {
				continue
			}
			fetched := at
			if !local.ObservedAt.IsZero() {
				fetched = local.ObservedAt
			}
			// The switch below only decides which of the Hub's own JSON
			// shapes (codexSubscriptionUsage / grokSubscriptionUsage) to
			// fill in — it branches on which field ReadProfile populated,
			// not on the provider id, so it stays correct even though the
			// participation decision above no longer names providers.
			switch {
			case local.Codex != nil:
				s.putLocal(provider, id, subscriptionUsageValue{codex: codexUsageFromLocal(*local.Codex)}, fetched)
			case local.Grok != nil:
				s.putLocal(provider, id, subscriptionUsageValue{grok: &grokSubscriptionUsage{
					UsedPercent:      clampUsagePercent(local.Grok.UsedPercent),
					RemainingPercent: 100 - clampUsagePercent(local.Grok.UsedPercent),
					PeriodStart:      local.Grok.PeriodStart,
					PeriodEnd:        local.Grok.PeriodEnd,
					PeriodType:       local.Grok.PeriodType,
				}}, fetched)
			}
		}
	}
}

func codexUsageFromLocal(usage usagelocal.CodexUsage) *codexSubscriptionUsage {
	value := &codexSubscriptionUsage{
		Primary:   localWindow(usage.Primary),
		Secondary: localWindow(usage.Secondary),
		PlanType:  usage.PlanType,
	}
	if usage.Credits.Present {
		value.Credits = &codexCredits{
			HasCredits: usage.Credits.HasCredits,
			Unlimited:  usage.Credits.Unlimited,
			Balance:    usage.Credits.Balance,
		}
		if usage.Credits.HasCredits && !usage.Credits.Unlimited {
			value.CreditsBalance = usage.Credits.Balance
		}
	}
	return value
}

func localWindow(window *usagelocal.RateLimitWindow) *usageWindow {
	if window == nil {
		return nil
	}
	return usageWindowFromSource(window.UsedPercent, "used", window.WindowMinutes, window.ResetsAt)
}

func clampUsagePercent(value float64) float64 {
	if value != value || value < 0 {
		return 0
	}
	if value > 100 {
		return 100
	}
	return value
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
