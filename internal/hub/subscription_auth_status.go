package hub

import (
	"context"
	"sync"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

const (
	usageAuthStatusTTL     = 15 * time.Second
	usageAuthStatusTimeout = 2 * time.Second
	usageAuthStatusWorkers = 3
)

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

// applyAuthStatuses enriches a snapshot with secret-free login state. The
// vendor CLIs are queried in parallel behind a small worker limit, and the
// result is cached so opening Usage does not repeatedly start every CLI.
func (s *subscriptionUsageStore) applyAuthStatuses(ctx context.Context, cfg *config.Config, configDir string, response *subscriptionUsageResponse, force bool) {
	if cfg == nil || response == nil {
		return
	}
	type pendingStatus struct {
		key      subscriptionUsageKey
		provider string
		id       string
	}
	pending := make([]pendingStatus, 0)
	states := make(map[subscriptionUsageKey]string)
	now := time.Now()

	s.mu.Lock()
	for _, provider := range response.Providers {
		for _, profile := range provider.Profiles {
			key := subscriptionUsageKey{provider: provider.Provider, id: config.NormalizeSubscriptionID(profile.ID)}
			cached, ok := s.auth[key]
			if force {
				delete(s.auth, key)
				ok = false
			}
			if ok && now.Sub(cached.checkedAt) < usageAuthStatusTTL {
				states[key] = cached.state
				continue
			}
			pending = append(pending, pendingStatus{key: key, provider: provider.Provider, id: key.id})
		}
	}
	s.mu.Unlock()

	var pendingMu sync.Mutex
	var wg sync.WaitGroup
	sem := make(chan struct{}, usageAuthStatusWorkers)
	for _, item := range pending {
		item := item
		wg.Add(1)
		go func() {
			defer wg.Done()
			select {
			case sem <- struct{}{}:
			case <-ctx.Done():
				pendingMu.Lock()
				states[item.key] = "status_unknown"
				pendingMu.Unlock()
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
			s.mu.Lock()
			if s.auth == nil {
				s.auth = map[subscriptionUsageKey]subscriptionAuthValue{}
			}
			s.auth[item.key] = subscriptionAuthValue{state: state, checkedAt: time.Now()}
			s.mu.Unlock()
			pendingMu.Lock()
			states[item.key] = state
			pendingMu.Unlock()
		}()
	}
	wg.Wait()

	for providerIndex := range response.Providers {
		for profileIndex := range response.Providers[providerIndex].Profiles {
			profile := &response.Providers[providerIndex].Profiles[profileIndex]
			key := subscriptionUsageKey{
				provider: response.Providers[providerIndex].Provider,
				id:       config.NormalizeSubscriptionID(profile.ID),
			}
			profile.AuthStatus = states[key]
		}
	}
}
