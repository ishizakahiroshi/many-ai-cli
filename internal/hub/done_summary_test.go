package hub

import "testing"

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

func TestLastUsefulDoneLine(t *testing.T) {
	tail := "progress...\n\n変更: internal/hub/server.go。テスト: go test は成功しました。\n"
	if got, want := lastUsefulDoneLine(tail), "変更: internal/hub/server.go。テスト: go test は成功しました。"; got != want {
		t.Fatalf("lastUsefulDoneLine() = %q, want %q", got, want)
	}
}
