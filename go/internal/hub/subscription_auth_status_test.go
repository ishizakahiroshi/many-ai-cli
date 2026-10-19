package hub

import (
	"context"
	"errors"
	"testing"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

func TestNormalizeSubscriptionAuthStatus(t *testing.T) {
	cases := []struct {
		name   string
		status subscription.Status
		err    error
		want   string
	}{
		{name: "ready", status: subscription.Status{LoggedIn: true}, want: "ready"},
		{name: "login required", status: subscription.Status{LoggedIn: false}, want: "login_required"},
		{name: "status error", status: subscription.Status{LoggedIn: true}, err: errors.New("status failed"), want: "status_unknown"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := normalizeSubscriptionAuthStatus(tc.status, tc.err); got != tc.want {
				t.Fatalf("normalizeSubscriptionAuthStatus=%q want %q", got, tc.want)
			}
		})
	}
}

func TestUsageAuthStatusPolicyConstants(t *testing.T) {
	if usageAuthStatusTTL <= 0 || usageAuthStatusTimeout <= 0 || usageAuthStatusWorkers < 1 {
		t.Fatalf("invalid auth status policy: ttl=%v timeout=%v workers=%d", usageAuthStatusTTL, usageAuthStatusTimeout, usageAuthStatusWorkers)
	}
}

func TestApplyAuthStatusesUsesFreshCacheWithoutRunningProviderCLI(t *testing.T) {
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"claude": {{ID: "cached-profile", Name: "Cached"}},
	}}
	store := newSubscriptionUsageStore()
	store.setAuthStatus("claude", "cached-profile", "ready")
	response := store.snapshot(cfg)
	store.applyAuthStatuses(context.Background(), cfg, "", &response, false)
	if got := response.Providers[0].Profiles[0].AuthStatus; got != "ready" {
		t.Fatalf("cached AuthStatus=%q want ready", got)
	}
	if checked := store.auth[subscriptionUsageKey{provider: "claude", id: "cached-profile"}].checkedAt; time.Since(checked) < 0 {
		t.Fatalf("auth cache timestamp is in the future: %v", checked)
	}
}

// The Usage dropdown must open at the speed of the numbers already in memory,
// never at the speed of the slowest vendor CLI. A profile with nothing cached
// is reported as checking and the read is left to the background.
func TestApplyAuthStatusesDefersUnknownProfilesToBackground(t *testing.T) {
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{}}
	store := newSubscriptionUsageStore()
	response := subscriptionUsageResponse{Providers: []subscriptionUsageProvider{{
		Provider: "claude",
		Profiles: []subscriptionUsageProfile{{ID: "unknown-profile", Name: "Unknown"}},
	}}}
	store.applyAuthStatuses(context.Background(), cfg, "", &response, false)

	profile := response.Providers[0].Profiles[0]
	if profile.AuthStatus != "" {
		t.Fatalf("AuthStatus=%q want empty while the check is still running", profile.AuthStatus)
	}
	if !profile.AuthChecking {
		t.Fatalf("AuthChecking=false want true for a profile with no cached state")
	}

	key := subscriptionUsageKey{provider: "claude", id: "unknown-profile"}
	waitForAuthState(t, store, key, "status_unknown")
	store.mu.Lock()
	stillChecking := store.authChecking[key]
	store.mu.Unlock()
	if stillChecking {
		t.Fatalf("authChecking[%v] stayed set after the background read finished", key)
	}
}

// A cached state older than the TTL is refreshed in the background, but the
// response keeps showing the known answer instead of falling back to checking.
func TestApplyAuthStatusesKeepsStaleStateVisibleWhileRefreshing(t *testing.T) {
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{}}
	store := newSubscriptionUsageStore()
	key := subscriptionUsageKey{provider: "claude", id: "stale-profile"}
	store.auth[key] = subscriptionAuthValue{state: "ready", checkedAt: time.Now().Add(-2 * usageAuthStatusTTL)}
	response := subscriptionUsageResponse{Providers: []subscriptionUsageProvider{{
		Provider: "claude",
		Profiles: []subscriptionUsageProfile{{ID: "stale-profile", Name: "Stale"}},
	}}}
	store.applyAuthStatuses(context.Background(), cfg, "", &response, false)

	profile := response.Providers[0].Profiles[0]
	if profile.AuthStatus != "ready" {
		t.Fatalf("AuthStatus=%q want the stale but legible ready", profile.AuthStatus)
	}
	if profile.AuthChecking {
		t.Fatalf("AuthChecking=true want false while a known state is on screen")
	}
	waitForAuthState(t, store, key, "status_unknown")
}

func waitForAuthState(t *testing.T, store *subscriptionUsageStore, key subscriptionUsageKey, want string) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		store.mu.Lock()
		value, ok := store.auth[key]
		store.mu.Unlock()
		if ok && value.state == want {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("background auth read never stored %q for %v", want, key)
}
