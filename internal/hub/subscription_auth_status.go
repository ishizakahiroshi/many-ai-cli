package hub

import (
	"context"
	"sync"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

const (
	usageAuthStatusTTL     = 60 * time.Second
	usageAuthStatusTimeout = 2 * time.Second
	usageAuthStatusWorkers = 3
)

// subscriptionAuthTarget is one profile whose login state has to be re-read
// from its vendor CLI.
type subscriptionAuthTarget struct {
	key      subscriptionUsageKey
	provider string
	id       string
}

func normalizeSubscriptionAuthStatus(status subscription.Status, err error) string {
	if err != nil {
		return "status_unknown"
	}
	if status.LoggedIn {
		return "ready"
	}
	return "login_required"
}

func (s *subscriptionUsageStore) setAuthStatus(provider, id, state string) {
	key := subscriptionUsageKey{provider: provider, id: config.NormalizeSubscriptionID(id)}
	if key.provider == "" || key.id == "" {
		return
	}
	s.mu.Lock()
	if s.auth == nil {
		s.auth = map[subscriptionUsageKey]subscriptionAuthValue{}
	}
	s.auth[key] = subscriptionAuthValue{state: state, checkedAt: time.Now()}
	s.mu.Unlock()
}

func (s *subscriptionUsageStore) invalidateAuthStatus(provider, id string) {
	key := subscriptionUsageKey{provider: provider, id: config.NormalizeSubscriptionID(id)}
	s.mu.Lock()
	delete(s.auth, key)
	s.mu.Unlock()
}

// applyAuthStatuses enriches a snapshot with secret-free login state without
// making the caller wait for it.
//
// Reading the state costs one vendor CLI process per profile (measured on
// Windows: `claude auth status` ~0.5s, `codex login status` ~0.2s, `grok
// models` ~1.2s). Doing that inline made GET /api/subscription-usage sit for
// seconds behind numbers that were already in memory, so the dropdown looked
// frozen every time it was opened. The snapshot therefore carries the last
// known state plus an auth_checking flag, and the re-read happens in the
// background; the UI polls this endpoint while anything is still checking.
//
// A profile is only reported as checking when there is nothing cached to show.
// A stale-but-known state is shown as-is and replaced once the background read
// lands, so the panel never flips a legible answer back to "checking".
func (s *subscriptionUsageStore) applyAuthStatuses(ctx context.Context, cfg *config.Config, configDir string, response *subscriptionUsageResponse, force bool) {
	if cfg == nil || response == nil {
		return
	}
	targets := make([]subscriptionAuthTarget, 0)
	states := make(map[subscriptionUsageKey]string)
	checking := make(map[subscriptionUsageKey]bool)
	now := time.Now()

	s.mu.Lock()
	if s.authChecking == nil {
		s.authChecking = map[subscriptionUsageKey]bool{}
	}
	for _, provider := range response.Providers {
		for _, profile := range provider.Profiles {
			key := subscriptionUsageKey{provider: provider.Provider, id: config.NormalizeSubscriptionID(profile.ID)}
			cached, ok := s.auth[key]
			if force {
				delete(s.auth, key)
				ok = false
			}
			if ok {
				states[key] = cached.state
				if now.Sub(cached.checkedAt) < usageAuthStatusTTL {
					continue
				}
			}
			if s.authChecking[key] {
				checking[key] = !ok
				continue
			}
			s.authChecking[key] = true
			checking[key] = !ok
			targets = append(targets, subscriptionAuthTarget{key: key, provider: provider.Provider, id: key.id})
		}
	}
	s.mu.Unlock()

	if len(targets) > 0 {
		// The read has to outlive this request: the browser gets the cached
		// answer now and picks up the fresh one on its next poll.
		go s.refreshAuthStatuses(context.WithoutCancel(ctx), cfg, configDir, targets)
	}

	for providerIndex := range response.Providers {
		for profileIndex := range response.Providers[providerIndex].Profiles {
			profile := &response.Providers[providerIndex].Profiles[profileIndex]
			key := subscriptionUsageKey{
				provider: response.Providers[providerIndex].Provider,
				id:       config.NormalizeSubscriptionID(profile.ID),
			}
			profile.AuthStatus = states[key]
			profile.AuthChecking = checking[key]
		}
	}
}

// refreshAuthStatuses queries the vendor CLIs behind a small worker limit and
// writes the result into the cache. It is the only place that starts those
// processes, so the worker limit is the whole concurrency budget.
func (s *subscriptionUsageStore) refreshAuthStatuses(ctx context.Context, cfg *config.Config, configDir string, targets []subscriptionAuthTarget) {
	var wg sync.WaitGroup
	sem := make(chan struct{}, usageAuthStatusWorkers)
	for _, item := range targets {
		item := item
		wg.Add(1)
		go func() {
			defer wg.Done()
			defer s.finishAuthCheck(item.key)
			select {
			case sem <- struct{}{}:
			case <-ctx.Done():
				return
			}
			defer func() { <-sem }()

			state := "status_unknown"
			adapter, ok := subscription.AdapterFor(item.provider)
			if ok {
				resolved, err := subscription.Resolve(cfg, configDir, item.provider, item.id)
				if err == nil && resolved != nil {
					statusCtx, cancel := context.WithTimeout(ctx, usageAuthStatusTimeout)
					status, statusErr := adapter.Status(statusCtx, resolved.ProfileDir)
					cancel()
					state = normalizeSubscriptionAuthStatus(status, statusErr)
				}
			}
			s.setAuthStatus(item.provider, item.id, state)
		}()
	}
	wg.Wait()
}

// finishAuthCheck clears the in-flight marker. It runs after setAuthStatus so
// a poll never sees "not checking" while the answer is still missing.
func (s *subscriptionUsageStore) finishAuthCheck(key subscriptionUsageKey) {
	s.mu.Lock()
	delete(s.authChecking, key)
	s.mu.Unlock()
}
