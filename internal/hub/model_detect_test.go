package hub

import "testing"

// 起動時に指定した effort は、wrapper の register が申告してセッションへ載る
// （internal/hub/wrapper_loop.go）。その後に走るバナー検出が、その値を消して
// しまわないことを固定する（子 plan:
// docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C2）。
//
// 起動値が「消える」経路は 2 つありうる。どちらも起きない:
//  1. バナーからモデルだけ取れて effort が取れなかったとき（newEffort が空）
//  2. Hub が --model も渡していて、onlyIfEmpty の検出が早期に戻るとき
func TestLaunchEffortSurvivesBannerDetection(t *testing.T) {
	s := newTestServer()

	// 1. --model 無しで起動し、起動値の effort だけ持っているセッション。
	//    バナーからモデルだけ取れても effort は残る。
	ses := registerTestSession(s, 21, "claude")
	ses.Effort = "high"
	s.applyDetectedModel(21, "claude", "Opus 4.8 (1M context)", "", true)
	if ses.Model != "Opus 4.8 (1M context)" {
		t.Fatalf("Model = %q, want the detected model", ses.Model)
	}
	if ses.Effort != "high" {
		t.Fatalf("Effort = %q, 起動値が検出で消えてはならない", ses.Effort)
	}

	// 2. --model も渡した起動。onlyIfEmpty の検出は Model が入っている時点で
	//    早期に戻るので、effort にも触れない。
	withModel := registerTestSession(s, 22, "codex")
	withModel.Model = "gpt-5.5"
	withModel.Effort = "minimal"
	s.applyDetectedModel(22, "codex", "gpt-5.5-codex", "high", true)
	if withModel.Model != "gpt-5.5" {
		t.Fatalf("Model = %q, onlyIfEmpty で上書きされてはならない", withModel.Model)
	}
	if withModel.Effort != "minimal" {
		t.Fatalf("Effort = %q, 起動値が上書きされてはならない", withModel.Effort)
	}
}
