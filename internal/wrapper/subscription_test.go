package wrapper

import (
	"testing"

	"many-ai-cli/internal/subscription"
)

// TestActiveSubscriptionIDReadsHubEnv は wrapper が Hub の指定した profile ID を
// env から読み戻すことを確認する。register でこの値を申告するので、Hub は
// 「起動しようとした profile」ではなく「実際に起動した profile」を記録できる。
func TestActiveSubscriptionIDReadsHubEnv(t *testing.T) {
	t.Setenv(subscription.SessionEnvVar, "  claude-main  ")
	if got := activeSubscriptionID(); got != "claude-main" {
		t.Fatalf("activeSubscriptionID() = %q, want claude-main", got)
	}
	t.Setenv(subscription.SessionEnvVar, "")
	if got := activeSubscriptionID(); got != "" {
		t.Fatalf("activeSubscriptionID() = %q, want empty for a default-login session", got)
	}
}

// TestSubscriptionLoginArgsAreOfficialSubcommands は login セッションが公式 CLI の
// ログインサブコマンドだけを起動することを確認する。many-ai-cli が独自の OAuth
// フローや token 回収を持たないことの担保。
func TestSubscriptionLoginArgsAreOfficialSubcommands(t *testing.T) {
	adapter, ok := subscription.AdapterFor("claude")
	if !ok {
		t.Fatal("claude adapter must be registered")
	}
	args := adapter.LoginArgs()
	if len(args) == 0 {
		t.Fatal("LoginArgs must not be empty")
	}
	for _, a := range args {
		if len(a) > 0 && a[0] == '-' {
			t.Fatalf("LoginArgs contains a flag (%q); login sessions must not pass session flags", a)
		}
	}
}
