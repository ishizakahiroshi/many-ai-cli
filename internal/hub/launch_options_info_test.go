package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// /api/info は起動要求の共通 3 項目の候補値を返す（子 plan:
// docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C5）。
// 画面はこの応答だけを見て欄を出し分けるので、写像が無い provider は
// キーごと現れないことが「欄を出さない」の根拠になる。
func TestInfoReportsLaunchOptionChoices(t *testing.T) {
	s, _ := subsTestServer(t)
	w := httptest.NewRecorder()
	s.handleInfo(w, subsRequest(t, http.MethodGet, "/api/info", nil))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d body = %s", w.Code, w.Body.String())
	}
	var body struct {
		EffortLevels      map[string][]string `json:"effort_levels"`
		ExecutionModes    []string            `json:"execution_modes"`
		PermissionPresets []string            `json:"permission_presets"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
		t.Fatalf("decode /api/info: %v", err)
	}
	for _, provider := range []string{"claude", "codex", "opencode"} {
		if len(body.EffortLevels[provider]) == 0 {
			t.Errorf("effort_levels[%q] is empty", provider)
		}
	}
	for _, provider := range []string{"copilot", "grok", "cursor-agent", "command-code"} {
		if _, present := body.EffortLevels[provider]; present {
			t.Errorf("effort_levels must not have a key for %q (no mapping)", provider)
		}
	}
	// この build で選べる値を返す。headless は子 plan
	// plan_child_execution_modes_headless.md 内部 C1 で解禁したので 3 値とも入る
	// （provider ごとの可否は一覧ではなく起動ごとの解決が決める）。
	for _, mode := range []string{"auto", "interactive", "headless"} {
		if !launchOptionListHas(body.ExecutionModes, mode) {
			t.Errorf("execution_modes = %v, want %q included", body.ExecutionModes, mode)
		}
	}
	// 権限の段は 3 つとも選べる（子 plan
	// plan_derived-session-launch_c2_permission-tiers.md 内部 C3 で bounded を解禁）。
	for _, preset := range []string{"attended", "bounded", "full"} {
		if !launchOptionListHas(body.PermissionPresets, preset) {
			t.Errorf("permission_presets = %v, want %q included", body.PermissionPresets, preset)
		}
	}
}

// /api/info は画面から立てる子の実効権限も返す（子 plan:
// docs/local/plan_derived-session-launch_c3_derive-launch.md 内部 C2）。
// 派生ダイアログはこれを spawn 確認ダイアログと同じ描画へ渡すので、形（provider →
// 段 → 実効フラグ）と「段を選んでいないとき（内側の ""）は段 1」を固定する。
func TestInfoReportsChildPermissionPreviewForUIOrigin(t *testing.T) {
	s, _ := subsTestServer(t)
	w := httptest.NewRecorder()
	s.handleInfo(w, subsRequest(t, http.MethodGet, "/api/info", nil))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d body = %s", w.Code, w.Body.String())
	}
	var body struct {
		Preview map[string]map[string]struct {
			PermissionMode string `json:"permission_mode"`
			Sandbox        string `json:"sandbox"`
			AskForApproval string `json:"ask_for_approval"`
			RiskConfirmed  bool   `json:"risk_confirmed"`
			Tier           string `json:"tier"`
		} `json:"child_permission_preview"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
		t.Fatalf("decode /api/info: %v", err)
	}
	for _, provider := range orchestrationProviders {
		tiers, ok := body.Preview[provider]
		if !ok {
			t.Fatalf("child_permission_preview has no entry for %q", provider)
		}
		for _, tier := range []string{"", "attended", "bounded", "full"} {
			if _, ok := tiers[tier]; !ok {
				t.Errorf("child_permission_preview[%q] has no tier %q", provider, tier)
			}
		}
		unset := tiers[""]
		if unset.Tier != "attended" {
			t.Errorf("child_permission_preview[%q][\"\"].tier = %q, want attended (UI 起点の既定)", provider, unset.Tier)
		}
		if unset.PermissionMode != "" || unset.Sandbox != "" || unset.AskForApproval != "" || unset.RiskConfirmed {
			t.Errorf("child_permission_preview[%q][\"\"] = %+v, want 何も足さない", provider, unset)
		}
	}
}

func launchOptionListHas(list []string, want string) bool {
	for _, v := range list {
		if v == want {
			return true
		}
	}
	return false
}

// 役割ごとの effort の記憶（RoleProvider と同じ場所で読み書きする）。
func TestRoleEffortMemory(t *testing.T) {
	s, _ := subsTestServer(t)
	parent := registerTestSession(s, 1, "claude")

	// 覚えていない役割では effort は空のまま（従来どおりの起動）。
	body := spawnChildRequest{Role: "review", Provider: "claude"}
	s.resolveSpawnChildProvider(parent, &body)
	if body.Effort != "" {
		t.Fatalf("effort = %q, want empty before anything is remembered", body.Effort)
	}

	s.rememberRoleEffort("review", "claude", "high")
	body = spawnChildRequest{Role: "review", Provider: "claude"}
	s.resolveSpawnChildProvider(parent, &body)
	if body.Effort != "high" {
		t.Fatalf("effort = %q, want the remembered high", body.Effort)
	}

	// 明示指定は記憶に勝つ。
	body = spawnChildRequest{Role: "review", Provider: "claude", Effort: "low"}
	s.resolveSpawnChildProvider(parent, &body)
	if body.Effort != "low" {
		t.Fatalf("effort = %q, want the explicit low", body.Effort)
	}

	// 記憶が今の provider で使えないときは黙って捨てる（起動は失敗させない）。
	body = spawnChildRequest{Role: "review", Provider: "copilot"}
	s.resolveSpawnChildProvider(parent, &body)
	if body.Effort != "" {
		t.Fatalf("effort = %q, want empty for a provider without an effort mapping", body.Effort)
	}

	// 空で起こし直したら記憶も消える（次も effort 無しが既定になる）。
	s.rememberRoleEffort("review", "claude", "")
	body = spawnChildRequest{Role: "review", Provider: "claude"}
	s.resolveSpawnChildProvider(parent, &body)
	if body.Effort != "" {
		t.Fatalf("effort = %q, want empty after the memory was cleared", body.Effort)
	}

	// 表に無い値は覚えない。
	s.rememberRoleEffort("review", "claude", "turbo")
	s.cfgMu.Lock()
	remembered := s.cfg.UserPrefs.Spawn.RoleEffort["review"]
	s.cfgMu.Unlock()
	if remembered != "" {
		t.Fatalf("remembered = %q, want an invalid level to be rejected", remembered)
	}
}
