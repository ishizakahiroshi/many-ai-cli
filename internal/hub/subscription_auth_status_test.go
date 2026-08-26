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
