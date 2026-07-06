package sessionstore

import "testing"

func TestIsNoiseOutput(t *testing.T) {
	// 思考スピナー再描画フレームだけのメッセージは保存しない（noise）。
	noise := []string{
		"",
		"·",
		"Thinking",
		"Working",
		"✳ Imploring… (12s · ↑3.2k tokens · esc to interrupt)",
		"thinking with medium effort✳3thinking with medium effort\n✶✷✻ still thinking with medium effort",
		"1.8924  Opus 4.8  ↑111.0k ↓764\nauto mode on (shift+tab to cycle)",
	}
	for _, s := range noise {
		if !isNoiseOutput(s) {
			t.Errorf("isNoiseOutput(%q) = false, want true", s)
		}
	}
	// 実本文を含むメッセージは保存する（混在チャンクも 1 行でも本文があれば残す）。
	keep := []string{
		"Here is the summary of the changes.",
		"✳ thinking with medium effort\nDone. Updated server.go and added a test.",
		"● Read(internal/hub/server.go)",
	}
	for _, s := range keep {
		if isNoiseOutput(s) {
			t.Errorf("isNoiseOutput(%q) = true, want false", s)
		}
	}
}
