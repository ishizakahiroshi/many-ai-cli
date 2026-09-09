package hub

import (
	"strings"
	"testing"

	"many-ai-cli/internal/sessionlog"
)

func TestClassifyDoneSummary(t *testing.T) {
	cases := map[string]string{
		"変更を完了しました":                      "success",
		"テスト 3 件失敗のため未完了です":              "failure",
		"ユーザーがキャンセルしたため中断しました":           "aborted",
		"migration の適用可否についてユーザー判断が必要です": "needs_action",
	}
	for text, want := range cases {
		if got := classifyDoneSummary(text); got != want {
			t.Errorf("classifyDoneSummary(%q) = %q, want %q", text, got, want)
		}
	}
}

// TestExtractIntentFromDoneText covers the C2 extraction (子 plan
// docs/local/plan_session-handoff-board_c3_intent-layer.md 内部 C2): both
// labels, either alone, and neither present ("空の行を作らない" → ok=false).
func TestExtractIntentFromDoneText(t *testing.T) {
	cases := []struct {
		name           string
		text           string
		wantNext       string
		wantUnverified string
		wantOK         bool
	}{
		{
			name:           "both labels",
			text:           "作業完了。 次: 次のPRをレビューする。 未検証: DBスキーマ変更の影響範囲",
			wantNext:       "次のPRをレビューする。",
			wantUnverified: "DBスキーマ変更の影響範囲",
			wantOK:         true,
		},
		{
			name:     "next only",
			text:     "作業完了。 次: 次のPRをレビューする。",
			wantNext: "次のPRをレビューする。",
			wantOK:   true,
		},
		{
			name:           "unverified only",
			text:           "作業完了。 未検証: DBスキーマ変更の影響範囲",
			wantUnverified: "DBスキーマ変更の影響範囲",
			wantOK:         true,
		},
		{
			name:   "neither label",
			text:   "作業完了しました",
			wantOK: false,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			next, unverified, ok := extractIntentFromDoneText(tc.text)
			if ok != tc.wantOK {
				t.Fatalf("ok = %v, want %v", ok, tc.wantOK)
			}
			if next != tc.wantNext {
				t.Errorf("next = %q, want %q", next, tc.wantNext)
			}
			if unverified != tc.wantUnverified {
				t.Errorf("unverified = %q, want %q", unverified, tc.wantUnverified)
			}
		})
	}
}

func TestLastUsefulDoneLine(t *testing.T) {
	screen := []string{
		"⏺ 調べています",
		"",
		"⏺ 変更: internal/hub/server.go。テスト: go test は成功しました。",
		"",
	}
	if got, want := lastUsefulDoneLine(screen), "変更: internal/hub/server.go。テスト: go test は成功しました。"; got != want {
		t.Fatalf("lastUsefulDoneLine() = %q, want %q", got, want)
	}
}

// 画面下端の入力ボックスと、その下にぶら下がる案内行は AI の出力ではない。
// ボックスの中には利用者が打ちかけた入力が居るので、拾うと「AI が言った」と
// 誤読させる。
func TestLastUsefulDoneLineSkipsInputBoxAndHints(t *testing.T) {
	screen := []string{
		"⏺ push はしていません（指示に含まれていなかったため）",
		"",
		"╭──────────────────────────────────────────────╮",
		"│ > やってください                             │",
		"╰──────────────────────────────────────────────╯",
		"  ? for shortcuts",
		"",
	}
	if got, want := lastUsefulDoneLine(screen), "push はしていません（指示に含まれていなかったため）"; got != want {
		t.Fatalf("lastUsefulDoneLine() = %q, want %q", got, want)
	}
}

// 入力ボックスの上辺が画面外へ流れていても、縦罫線で始まる行は中身として落とす。
func TestLastUsefulDoneLineSkipsBoxInteriorWithoutTopBorder(t *testing.T) {
	screen := []string{
		"⏺ テストは 3 件とも成功しました。",
		"│ > やってください                             │",
		"╰──────────────────────────────────────────────╯",
	}
	if got, want := lastUsefulDoneLine(screen), "テストは 3 件とも成功しました。"; got != want {
		t.Fatalf("lastUsefulDoneLine() = %q, want %q", got, want)
	}
}

func TestLastUsefulDoneLineEmptyWhenNothingReadable(t *testing.T) {
	screen := []string{"", "  ", "────────────", "> "}
	if got := lastUsefulDoneLine(screen); got != "" {
		t.Fatalf("lastUsefulDoneLine() = %q, want empty", got)
	}
}

// 回帰テスト（root cause）: TUI が同じ行を消して描き直すと、StripANSI 済みの
// ストリームでは前後の文字列が改行なしで連結する。表示元を vt の画面行に
// することでしか、この連結は解消できない。
func TestLastUsefulDoneLineUsesRenderedScreenNotStream(t *testing.T) {
	chunks := [][]byte{
		[]byte("push はしていません（指示に含まれていなかったため）"),
		[]byte("\x1b[1;1H\x1b[K"),
		[]byte("やってください"),
	}

	var stream strings.Builder
	vt := newVTBuffer(80, 24)
	for _, chunk := range chunks {
		vt.Write(chunk)
		stream.WriteString(sessionlog.StripANSI(string(chunk)))
	}

	// 前提の確認: ストリーム側は実際に壊れた 1 行になっている。
	// ここが連結しなくなったら、このテストは症状を再現できていない。
	if !strings.Contains(stream.String(), "ため）やってください") {
		t.Fatalf("fixture does not reproduce the stitched stream: %q", stream.String())
	}

	if got, want := lastUsefulDoneLine(vt.Lines()), "やってください"; got != want {
		t.Fatalf("lastUsefulDoneLine() = %q, want %q", got, want)
	}
}
