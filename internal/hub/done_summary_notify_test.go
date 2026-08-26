package hub

import (
	"strings"
	"testing"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
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

// フォールバックは通知が有効でも外部へ出さない。
//
// マーカーの規約は「タスク完了時のみ出力」なので、会話ターンにマーカーが無いのは
// 正常動作。ここを外すと、質問へ答えただけのターンごとに push が飛び、本物の完了
// 通知が埋もれる（2026-08-26 実測: Codex は 3 セッション連続でマーカー 0 件）。
func TestFallbackDoneSummaryIsNeverNotifiedExternally(t *testing.T) {
	s := newDoneSummaryServer(ptrBool(true), 3)
	if !s.doneSummaryNotifyEnabled() {
		t.Fatal("前提が崩れている: 明示 true なのに通知が無効")
	}
	if s.shouldNotifyDoneExternally(proto.DoneSummary{Text: "終わりました", Fallback: true}) {
		t.Fatal("フォールバックが外部通知へ流れた")
	}
	if !s.shouldNotifyDoneExternally(proto.DoneSummary{Text: "終わりました"}) {
		t.Fatal("AI が出した本物の完了サマリーまで止まった")
	}
}

// フォールバックが needs_action を名乗らないことを固定する。
// success 側（既定）へ寄せないことも同時に見る。どちらへ寄っても「AI が状態を
// 報告した」と誤読させるため。
func TestFallbackDoneSummaryKindIsUnknown(t *testing.T) {
	if fallbackDoneSummaryKind != "unknown" {
		t.Fatalf("fallbackDoneSummaryKind = %q, want unknown", fallbackDoneSummaryKind)
	}
	if got := classifyDoneSummary(fallbackDoneSummaryText("")); got == fallbackDoneSummaryKind {
		t.Fatal("前提が崩れている: 定型文が classifyDoneSummary でも unknown になるなら固定値を持つ意味がない")
	}
}

// 文面に警告語彙（検出できませんでした・確認してください）を戻さない。
// 語彙が戻ると、色を弱めても読んだ側には異常として届く。
func TestFallbackDoneSummaryTextIsNotAlarming(t *testing.T) {
	for _, text := range []string{fallbackDoneSummaryText(""), fallbackDoneSummaryText("テストは 3 件とも成功しました。")} {
		for _, ng := range []string{"検出できませんでした", "確認してください"} {
			if strings.Contains(text, ng) {
				t.Fatalf("フォールバック文面に警告語彙 %q が入っている: %q", ng, text)
			}
		}
	}
	if got, want := fallbackDoneSummaryText("直しました。"), "ターン終了（完了サマリーなし）。最後の出力: 直しました。"; got != want {
		t.Fatalf("fallbackDoneSummaryText() = %q, want %q", got, want)
	}
}
