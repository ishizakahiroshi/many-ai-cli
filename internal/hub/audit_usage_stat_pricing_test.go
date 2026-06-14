package hub

import "testing"

// audit #31: lookupModelPricing は (1)完全一致 (2)スペース区切りの最初のトークンの完全一致 の
// 2 段のみで、真の前方一致（価格表に無い派生 ID を短いキーへ寄せる）は意図的に行わない。
// 表拡充（現行モデル ID を正規単価で追加）後も、この exact/space-prefix の挙動が保たれることを固定する。
func TestAuditLookupModelPricingExactAndSpacePrefix(t *testing.T) {
	cases := []struct {
		name    string
		modelID string
		wantOK  bool
	}{
		{"exact_codex", "gpt-4.1", true},
		{"exact_legacy_opus", "claude-opus-4", true},
		// 表拡充で追加した現行モデルは完全一致でヒットする。
		{"exact_current_opus", "claude-opus-4-8", true},
		{"exact_current_sonnet", "claude-sonnet-4-6", true},
		{"exact_fable", "claude-fable-5", true},
		// effort サフィックス（スペース区切り）の最初のトークンで完全一致する。
		{"space_prefix_codex", "gpt-4.1 medium", true},
		{"space_prefix_current_opus", "claude-opus-4-8 high", true},
		// 真の前方一致は行わない: 価格表に無い派生 ID は短いキーへ寄せず未知扱い。
		{"no_true_prefix_unregistered_opus", "claude-opus-4-99", false},
		{"no_true_prefix_codex_derived", "gpt-5.4-mini", false},
		// スペースなしの完全未登録 ID も未知扱い。
		{"unknown", "totally-unknown-model", false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, ok := lookupModelPricing(tc.modelID)
			if ok != tc.wantOK {
				t.Errorf("lookupModelPricing(%q) ok = %v, want %v", tc.modelID, ok, tc.wantOK)
			}
		})
	}
}

// audit #31: 表拡充で追加した現行モデルの単価が公式値（2026-06 時点）と一致することを固定する。
func TestAuditCurrentClaudePricing(t *testing.T) {
	cases := []struct {
		modelID    string
		inPerMTok  float64
		outPerMTok float64
	}{
		{"claude-fable-5", 10.00, 50.00},
		{"claude-opus-4-8", 5.00, 25.00},
		{"claude-opus-4-7", 5.00, 25.00},
		{"claude-opus-4-6", 5.00, 25.00},
		{"claude-sonnet-4-6", 3.00, 15.00},
		{"claude-haiku-4-5", 1.00, 5.00},
	}
	for _, tc := range cases {
		p, ok := lookupModelPricing(tc.modelID)
		if !ok {
			t.Errorf("lookupModelPricing(%q) ok = false, want true", tc.modelID)
			continue
		}
		if p.InputPerMTok != tc.inPerMTok || p.OutputPerMTok != tc.outPerMTok {
			t.Errorf("lookupModelPricing(%q) = in %v / out %v, want in %v / out %v",
				tc.modelID, p.InputPerMTok, p.OutputPerMTok, tc.inPerMTok, tc.outPerMTok)
		}
	}
}

// audit C6: claude-opus-4（4.0）と claude-opus-4-5（4.5）の単価固定テスト。
// claude-api スキル公式テーブル（2026-06-04 キャッシュ）にこれらの ID の価格記載なし（deprecated/legacy 扱い）。
// 確証が取れないため $15/$75 据え置き。値が変わる場合は usage_stat.go とともに更新すること。
func TestAuditLegacyOpusPricingHeld(t *testing.T) {
	cases := []struct {
		modelID    string
		inPerMTok  float64
		outPerMTok float64
		note       string
	}{
		{"claude-opus-4", 15.00, 75.00, "claude-opus-4-0: 確証なしのため据え置き"},
		{"claude-opus-4-5", 15.00, 75.00, "claude-opus-4-5: 確証なしのため据え置き"},
	}
	for _, tc := range cases {
		tc := tc
		t.Run(tc.modelID, func(t *testing.T) {
			p, ok := lookupModelPricing(tc.modelID)
			if !ok {
				t.Errorf("lookupModelPricing(%q) ok = false, want true (%s)", tc.modelID, tc.note)
				return
			}
			if p.InputPerMTok != tc.inPerMTok || p.OutputPerMTok != tc.outPerMTok {
				t.Errorf("lookupModelPricing(%q) = in %v / out %v, want in %v / out %v (%s)",
					tc.modelID, p.InputPerMTok, p.OutputPerMTok, tc.inPerMTok, tc.outPerMTok, tc.note)
			}
		})
	}
}

// audit #31: calcCostUSD は価格表未登録モデルで (0, false)、登録モデルで正の概算コストと true を返す。
func TestAuditCalcCostUSDKnownAndUnknown(t *testing.T) {
	// 価格表に無い派生 ID（前方一致を入れない限り未知扱い）。
	cost, known := calcCostUSD("claude-opus-4-99", 1000, 1000, 0)
	if known {
		t.Errorf("calcCostUSD(unregistered derived id) known = true, want false")
	}
	if cost != 0 {
		t.Errorf("calcCostUSD(unregistered) cost = %v, want 0", cost)
	}

	// 登録モデルは正のコストと known=true。
	cost, known = calcCostUSD("claude-opus-4-8", 1_000_000, 0, 0)
	if !known {
		t.Fatalf("calcCostUSD(claude-opus-4-8) known = false, want true")
	}
	if cost <= 0 {
		t.Errorf("calcCostUSD(claude-opus-4-8, 1M in) cost = %v, want > 0", cost)
	}
}
