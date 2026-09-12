package hub

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"many-ai-cli/internal/config"
)

// 役割ごとの権限の段の記憶（子 plan:
// docs/local/plan_derived-session-launch_c2_permission-tiers.md 内部 C6）。
//
// provider / effort の記憶と違い、**人が「次回もこの段を使う」を明示したときだけ**書く。
// ここで固定するのは書き込みの規律だけで、誰がその指示を出せるかは
// TestRememberPermissionOnlyFromUIOrigin が別に固定している。
func TestRememberRolePermission(t *testing.T) {
	cases := []struct {
		name     string
		before   map[string]string
		role     string
		tier     string
		remember bool
		want     map[string]string
	}{
		{
			name:     "checked stores the tier for that role",
			role:     "review",
			tier:     config.PermissionPresetBounded,
			remember: true,
			want:     map[string]string{"review": "bounded"},
		},
		{
			name:     "unchecked clears the role's memory",
			before:   map[string]string{"review": "bounded"},
			role:     "review",
			tier:     config.PermissionPresetFull,
			remember: false,
			want:     map[string]string{},
		},
		{
			name:     "checked with no tier clears it too (指定なしは記憶の無い状態そのもの)",
			before:   map[string]string{"review": "bounded"},
			role:     "review",
			tier:     "",
			remember: true,
			want:     map[string]string{},
		},
		{
			name:     "a tier the build does not know is not stored",
			before:   map[string]string{"review": "bounded"},
			role:     "review",
			tier:     "yolo",
			remember: true,
			want:     map[string]string{"review": "bounded"},
		},
		{
			name:     "an empty role is ignored",
			before:   map[string]string{"review": "bounded"},
			role:     "   ",
			tier:     config.PermissionPresetFull,
			remember: true,
			want:     map[string]string{"review": "bounded"},
		},
		{
			name:     "unchecking a role that was never remembered writes nothing",
			role:     "implementation",
			tier:     "",
			remember: false,
			want:     map[string]string{},
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s, _ := subsTestServer(t)
			if tc.before != nil {
				s.cfgMu.Lock()
				s.cfg.UserPrefs.Spawn.RolePermission = map[string]string{}
				for role, tier := range tc.before {
					s.cfg.UserPrefs.Spawn.RolePermission[role] = tier
				}
				s.cfgMu.Unlock()
			}
			s.rememberRolePermission(tc.role, tc.tier, tc.remember)
			s.cfgMu.Lock()
			got := map[string]string{}
			for role, tier := range s.cfg.UserPrefs.Spawn.RolePermission {
				got[role] = tier
			}
			s.cfgMu.Unlock()
			if len(got) != len(tc.want) {
				t.Fatalf("role_permission = %v, want %v", got, tc.want)
			}
			for role, tier := range tc.want {
				if got[role] != tier {
					t.Fatalf("role_permission[%q] = %q, want %q (full map %v)", role, got[role], tier, got)
				}
			}
		})
	}
}

// 記憶を読む側。conductor（AI）起点の要求だけ Hub が段を埋め、確認ダイアログには
// 埋めた段が入って見える。画面起点はダイアログが見せていた段のまま送られてくるので
// 埋めない（設計 3）。
func TestRolePermissionPrefillByOrigin(t *testing.T) {
	cases := []struct {
		name       string
		remembered string
		origin     string
		requested  string
		want       string
	}{
		{name: "conductor origin takes the remembered tier", remembered: "bounded", origin: launchOriginConductor, want: "bounded"},
		{name: "ui origin is never pre-filled by the Hub", remembered: "bounded", origin: launchOriginUI, want: ""},
		{name: "an explicit tier wins over the memory", remembered: "bounded", origin: launchOriginConductor, requested: "attended", want: "attended"},
		{name: "no memory leaves the request exactly as it came", origin: launchOriginConductor, want: ""},
		{name: "a tier this build does not know is dropped, not launched with", remembered: "yolo", origin: launchOriginConductor, want: ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s, _ := subsTestServer(t)
			parent := registerTestSession(s, 1, "claude")
			if tc.remembered != "" {
				s.cfgMu.Lock()
				s.cfg.UserPrefs.Spawn.RolePermission = map[string]string{"review": tc.remembered}
				s.cfgMu.Unlock()
			}
			body := spawnChildRequest{Role: "review", Provider: "claude", Origin: tc.origin, PermissionPreset: tc.requested}
			s.resolveSpawnChildProvider(parent, &body)
			if body.PermissionPreset != tc.want {
				t.Fatalf("permission_preset = %q, want %q", body.PermissionPreset, tc.want)
			}
		})
	}
}

// 「次回もこの段」を出せるのは画面（origin: "ui"）だけ。conductor と relay の要求に
// 付いてきた欄は捨てる — AI が利用者の記憶を書き換えられないことが、この欄を
// 3 値にした理由そのもの（設計 2）。
func TestRememberPermissionOnlyFromUIOrigin(t *testing.T) {
	yes, no := true, false
	cases := []struct {
		name   string
		origin string
		in     *bool
		want   *bool
	}{
		{name: "ui origin keeps a checked box", origin: launchOriginUI, in: &yes, want: &yes},
		{name: "ui origin keeps an unchecked box", origin: launchOriginUI, in: &no, want: &no},
		{name: "conductor origin is discarded", origin: launchOriginConductor, in: &yes, want: nil},
		{name: "relay (same empty origin) is discarded", origin: "", in: &no, want: nil},
		{name: "a request without the field stays without it", origin: launchOriginUI, in: nil, want: nil},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := rememberPermissionForOrigin(tc.origin, tc.in)
			switch {
			case tc.want == nil && got != nil:
				t.Fatalf("remember_permission = %v, want nil (記憶を触らない)", *got)
			case tc.want != nil && got == nil:
				t.Fatalf("remember_permission = nil, want %v", *tc.want)
			case tc.want != nil && *got != *tc.want:
				t.Fatalf("remember_permission = %v, want %v", *got, *tc.want)
			}
		})
	}
}

// 上のガードが handleSpawnChild で実際に効いていること。conductor 起点の要求は
// 確認待ちとして Hub に残るので、そこに積まれた body を見れば、捨てられたことを
// 起動まで進めずに確認できる。
func TestHandleSpawnChildDropsRememberPermissionFromConductor(t *testing.T) {
	s, _ := subsTestServer(t)
	// 確認ダイアログを通す経路で観測する（テスト用サーバの既定は確認オフ）。
	s.cfg.Orchestration.SpawnConfirmMode = config.SpawnConfirmOn
	registerTestSession(s, 1, "claude")

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	req := subsRequest(t, http.MethodPost, "/api/sessions/1/spawn-child", map[string]any{
		"role":                "review",
		"provider":            "claude",
		"permission_preset":   "bounded",
		"remember_permission": true,
	}).WithContext(ctx)

	done := make(chan struct{})
	go func() {
		defer close(done)
		s.handleSpawnChild(httptest.NewRecorder(), req, 1)
	}()

	pending := waitForPendingConfirmation(t, s)
	if pending.Body.RememberPermission != nil {
		t.Fatalf("pending body carries remember_permission=%v; the conductor must not be able to write the memory", *pending.Body.RememberPermission)
	}
	// 段そのものは要求どおり残る（捨てるのは記憶の指示だけ）。
	if pending.Body.PermissionPreset != "bounded" {
		t.Fatalf("permission_preset = %q, want the requested bounded", pending.Body.PermissionPreset)
	}
	cancel()
	<-done
}

func waitForPendingConfirmation(t *testing.T, s *Server) *pendingSpawnConfirmation {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		s.orchestration.mu.Lock()
		for _, p := range s.orchestration.spawnConfirmations {
			cp := p
			s.orchestration.mu.Unlock()
			return cp
		}
		s.orchestration.mu.Unlock()
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatal("no spawn confirmation was registered")
	return nil
}

// 承認ダイアログの決定が記憶に届くまで。決定は 3 値で、要求へ重ねた後の body が
// そのまま起動に使われるので、記憶の反映もその body 1 つから決まる。
//
// 起動そのもの（performSpawn の worktree 準備と wrapper 起動）は通さない: この
// テストが固定したいのは「決定 → body → 記憶」の写り方で、子が実際に立つかは
// 別のテストの担当。performSpawn が成功後に呼ぶのと同じ applyRolePermissionMemory を
// 呼んで確かめている。
func TestSpawnConfirmationDecisionWritesRolePermission(t *testing.T) {
	yes, no := true, false
	bounded, unset := config.PermissionPresetBounded, ""
	cases := []struct {
		name     string
		before   string
		decision spawnConfirmationDecision
		want     string
	}{
		{
			name:     "checked stores the decided tier",
			decision: spawnConfirmationDecision{PermissionPreset: &bounded, RememberPermission: &yes},
			want:     "bounded",
		},
		{
			name:     "unchecked clears what was remembered",
			before:   "bounded",
			decision: spawnConfirmationDecision{PermissionPreset: &bounded, RememberPermission: &no},
			want:     "",
		},
		{
			name:     "checked with 指定なし clears it as well",
			before:   "bounded",
			decision: spawnConfirmationDecision{PermissionPreset: &unset, RememberPermission: &yes},
			want:     "",
		},
		{
			name:     "a decision without the field leaves the memory alone",
			before:   "bounded",
			decision: spawnConfirmationDecision{PermissionPreset: &unset},
			want:     "bounded",
		},
		{
			name:     "an empty decision (a UI that knows none of this) leaves it alone",
			before:   "bounded",
			decision: spawnConfirmationDecision{},
			want:     "bounded",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s, _ := subsTestServer(t)
			if tc.before != "" {
				s.cfgMu.Lock()
				s.cfg.UserPrefs.Spawn.RolePermission = map[string]string{"review": tc.before}
				s.cfgMu.Unlock()
			}
			requested := spawnChildRequest{Role: "review", Provider: "claude"}
			launch := applySpawnConfirmationDecision(requested, tc.decision)
			s.applyRolePermissionMemory(launch)
			s.cfgMu.Lock()
			got := s.cfg.UserPrefs.Spawn.RolePermission["review"]
			s.cfgMu.Unlock()
			if got != tc.want {
				t.Fatalf("role_permission[review] = %q, want %q", got, tc.want)
			}
		})
	}
}

// 確認ダイアログのチェックボックスの初期状態は Hub が載せる（記憶があれば ON）。
func TestSpawnConfirmationMessageCarriesRememberPermission(t *testing.T) {
	s, _ := subsTestServer(t)
	pending := &pendingSpawnConfirmation{ID: "sc-1", ParentID: 1, Role: "review", Body: spawnChildRequest{Role: "review", Provider: "claude"}}

	if msg := s.spawnConfirmationRequestedMessage(pending); msg.RememberPermission {
		t.Fatal("remember_permission is true with nothing remembered")
	}
	s.cfgMu.Lock()
	s.cfg.UserPrefs.Spawn.RolePermission = map[string]string{"review": "bounded"}
	s.cfgMu.Unlock()
	if msg := s.spawnConfirmationRequestedMessage(pending); !msg.RememberPermission {
		t.Fatal("remember_permission is false even though the role has a remembered tier")
	}
}

// /api/info は役割 → 段の記憶を返す。派生ダイアログはこれだけを見て段の初期値を
// 決めるので、記憶が無いときに null ではなく {} が返ることまで固定する。
func TestInfoReportsRolePermissionMemory(t *testing.T) {
	s, _ := subsTestServer(t)

	empty := infoRolePermission(t, s)
	if len(empty) != 0 {
		t.Fatalf("role_permission = %v, want empty before anything is remembered", empty)
	}

	s.cfgMu.Lock()
	s.cfg.UserPrefs.Spawn.RolePermission = map[string]string{
		"review":         "bounded",
		"implementation": "full",
		// config.yaml を手で書き換えれば未知の値も入る。画面の選択肢に無い段は返さない。
		"test": "yolo",
	}
	s.cfgMu.Unlock()

	got := infoRolePermission(t, s)
	if got["review"] != "bounded" || got["implementation"] != "full" {
		t.Fatalf("role_permission = %v, want the remembered tiers", got)
	}
	if _, present := got["test"]; present {
		t.Fatalf("role_permission = %v, want the unknown tier dropped", got)
	}
}

func infoRolePermission(t *testing.T, s *Server) map[string]string {
	t.Helper()
	w := httptest.NewRecorder()
	s.handleInfo(w, subsRequest(t, http.MethodGet, "/api/info", nil))
	if w.Code != http.StatusOK {
		t.Fatalf("status = %d body = %s", w.Code, w.Body.String())
	}
	var body struct {
		RolePermission map[string]string `json:"role_permission"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
		t.Fatalf("decode /api/info: %v", err)
	}
	if body.RolePermission == nil {
		t.Fatal("/api/info has no role_permission object (null ではなく {} を返すこと)")
	}
	return body.RolePermission
}
