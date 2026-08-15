package hub

import (
	"testing"

	"many-ai-cli/internal/config"
)

func ptrBool(v bool) *bool { return &v }

func newDoneSummaryServer(enabled *bool, backends int) *Server {
	cfg := &config.Config{}
	cfg.UserPrefs.DoneSummaryNotify.Enabled = enabled
	for i := 0; i < backends; i++ {
		cfg.Notify.Backends = append(cfg.Notify.Backends, config.NotifyBackendConfig{})
	}
	return &Server{cfg: cfg}
}

// 明示設定があればそれに従う。通知チャネルの有無で覆さない。
func TestDoneSummaryNotifyRespectsExplicitValue(t *testing.T) {
	if got := newDoneSummaryServer(ptrBool(false), 3).doneSummaryNotifyEnabled(); got {
		t.Fatal("明示 false なのに有効になった（通知チャネルがあっても優先しない）")
	}
	if got := newDoneSummaryServer(ptrBool(true), 0).doneSummaryNotifyEnabled(); !got {
		t.Fatal("明示 true なのに無効になった")
	}
}

// 未設定 + 通知チャネルあり → 既定 ON（v0.7.0 の変更点）。
func TestDoneSummaryNotifyDefaultsOnWithBackend(t *testing.T) {
	if got := newDoneSummaryServer(nil, 1).doneSummaryNotifyEnabled(); !got {
		t.Fatal("notify.backends が 1 件あるのに既定 ON にならなかった")
	}
}

// 未設定 + 通知チャネル無し → OFF のまま。
// **通知手段を持たない利用者の挙動は変えない**のが本変更の前提。ここが崩れると
// 送り先の無い通知が増えるだけになる。
func TestDoneSummaryNotifyStaysOffWithoutChannel(t *testing.T) {
	if got := newDoneSummaryServer(nil, 0).doneSummaryNotifyEnabled(); got {
		t.Fatal("通知チャネルが無いのに既定 ON になった")
	}
}

// push manager 未初期化でも落ちない（テスト構成・push 無効構成）。
func TestHasPushSubscriptionWithoutManager(t *testing.T) {
	if newDoneSummaryServer(nil, 0).hasPushSubscription() {
		t.Fatal("push manager が nil なのに購読ありと返した")
	}
}

// Clone が *bool を共有すると、複製側の書き換えが元の設定へ波及する。
func TestUserPrefsCloneDoesNotSharePointer(t *testing.T) {
	var prefs config.UserPrefs
	prefs.DoneSummaryNotify.Enabled = ptrBool(true)
	clone := prefs.Clone()
	*clone.DoneSummaryNotify.Enabled = false
	if !*prefs.DoneSummaryNotify.Enabled {
		t.Fatal("Clone がポインタを共有しており、複製の書き換えが元へ波及した")
	}
}
