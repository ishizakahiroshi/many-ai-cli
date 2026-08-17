package doctor

import (
	"context"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

func subsDoctorConfig(t *testing.T) *config.Config {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	return &config.Config{}
}

// TestSubscriptionsCheckSilentWhenUnused は profile を 1 件も登録していない環境で
// doctor の出力が 1 行も増えないことを確認する。
func TestSubscriptionsCheckSilentWhenUnused(t *testing.T) {
	cfg := subsDoctorConfig(t)
	if got := subscriptions(context.Background(), cfg); len(got) != 0 {
		t.Fatalf("subscriptions() = %#v, want no rows", got)
	}
	if got := subscriptions(context.Background(), nil); len(got) != 0 {
		t.Fatalf("subscriptions(nil) = %#v, want no rows", got)
	}
}

// TestSubscriptionsCheckReportsLoginRequired は profile ディレクトリがまだ無い
// （＝未ログイン）profile が warning として出ることを確認する。
// ディレクトリが無い時点で判定できるので、公式 CLI は起動しない。
func TestSubscriptionsCheckReportsLoginRequired(t *testing.T) {
	cfg := subsDoctorConfig(t)
	cfg.Subscriptions = config.SubscriptionProfiles{"claude": {{ID: "main", Name: "Claude Max Main"}}}
	checks := subscriptions(context.Background(), cfg)
	if len(checks) != 1 {
		t.Fatalf("checks = %#v, want 1 row", checks)
	}
	if checks[0].Level != Warn || !strings.Contains(checks[0].Message, "Claude Max Main") {
		t.Fatalf("check = %#v, want a warning naming the profile", checks[0])
	}
	if !strings.Contains(checks[0].Message, "ログイン") {
		t.Fatalf("check message = %q, want it to say a login is required", checks[0].Message)
	}
}

// TestSubscriptionsCheckReportsBrokenConfig は設定が壊れた profile を Fail として
// 出すことを確認する。
func TestSubscriptionsCheckReportsBrokenConfig(t *testing.T) {
	cfg := subsDoctorConfig(t)
	cfg.Subscriptions = config.SubscriptionProfiles{"claude": {{ID: "../escape"}}}
	checks := subscriptions(context.Background(), cfg)
	if len(checks) != 1 || checks[0].Level != Fail {
		t.Fatalf("checks = %#v, want one FAIL row", checks)
	}
}

// TestSubscriptionsCheckReportsUnsupportedProvider は adapter を持たない provider の
// profile が「未対応」として出ることを確認する。
func TestSubscriptionsCheckReportsUnsupportedProvider(t *testing.T) {
	cfg := subsDoctorConfig(t)
	cfg.Subscriptions = config.SubscriptionProfiles{"cursor-agent": {{ID: "main", Name: "Personal"}}}
	checks := subscriptions(context.Background(), cfg)
	if len(checks) != 1 || checks[0].Level != Warn {
		t.Fatalf("checks = %#v, want one WARN row", checks)
	}
	if !strings.Contains(checks[0].Message, "未対応") {
		t.Fatalf("check message = %q, want it to say the provider is unsupported", checks[0].Message)
	}
}

// TestSubscriptionsCheckSkipsDisabledProfiles は無効化した profile で公式 CLI を
// 起動しないこと（＝ OK 行だけ出ること）を確認する。
func TestSubscriptionsCheckSkipsDisabledProfiles(t *testing.T) {
	cfg := subsDoctorConfig(t)
	off := false
	cfg.Subscriptions = config.SubscriptionProfiles{"claude": {{ID: "main", Name: "Main", Enabled: &off}}}
	checks := subscriptions(context.Background(), cfg)
	if len(checks) != 1 || checks[0].Level != OK {
		t.Fatalf("checks = %#v, want one OK row", checks)
	}
}

// TestSubscriptionsCheckOutputHasNoSecrets は診断出力に profile 名・パス以外の
// 秘密が混ざらないことを確認する。
func TestSubscriptionsCheckOutputHasNoSecrets(t *testing.T) {
	cfg := subsDoctorConfig(t)
	cfg.Token = "hub-token-should-not-appear"
	cfg.Subscriptions = config.SubscriptionProfiles{"claude": {{ID: "main", Name: "Main"}}}
	for _, c := range subscriptions(context.Background(), cfg) {
		if strings.Contains(c.Message+c.Fix, cfg.Token) {
			t.Fatalf("doctor output leaked the hub token: %#v", c)
		}
	}
}
