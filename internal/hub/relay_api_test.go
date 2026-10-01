package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

type relayAPIResponse struct {
	OK              bool              `json:"ok"`
	Error           string            `json:"error"`
	Detail          string            `json:"detail"`
	OrchestrationID string            `json:"orchestration_id"`
	Relay           proto.RelayStatus `json:"relay"`
	Relays          []relayAPIItem    `json:"relays"`
	MaxChildren     int               `json:"max_children_per_parent"`
}

func relayAPICall(t *testing.T, s *Server, method, path string, body any) (int, relayAPIResponse) {
	t.Helper()
	rr := httptest.NewRecorder()
	s.handleSessionAPI(rr, orchestrationRequest(method, path, body))
	var got relayAPIResponse
	if err := json.Unmarshal(rr.Body.Bytes(), &got); err != nil {
		t.Fatalf("decode response status=%d body=%s: %v", rr.Code, rr.Body.String(), err)
	}
	return rr.Code, got
}

func writeRelayPlan(t *testing.T, h *relayHarness, name string) string {
	t.Helper()
	dir := filepath.Join(h.parent.CWD, "docs")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, []byte("# plan\n\n- C1\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

func relayAPIStartBody(plan string, mode string) relayStartJSON {
	roles := relayTestRoles(false)
	rolePointers := make(map[string]*orchestrationRoleAssignment, len(roles))
	for role, assignment := range roles {
		assignment := assignment
		rolePointers[role] = &assignment
	}
	return relayStartJSON{
		PlanPath:                   plan,
		Mode:                       mode,
		Roles:                      rolePointers,
		AcknowledgeChildFullBypass: true,
	}
}

func TestResolveRelayPlanPathValidation(t *testing.T) {
	h := newRelayHarness(t)
	plan := writeRelayPlan(t, h, "plan.md")
	got, err := resolveRelayPlanPath(h.parent.CWD, filepath.Join("docs", "plan.md"))
	if err != nil || got != plan {
		t.Fatalf("relative plan = %q, %v; want %q", got, err, plan)
	}

	outside := filepath.Join(t.TempDir(), "outside.md")
	if err := os.WriteFile(outside, []byte("# outside\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	for name, raw := range map[string]string{
		"outside": outside,
		"text":    filepath.Join(h.parent.CWD, "docs", "plan.txt"),
		"missing": filepath.Join(h.parent.CWD, "docs", "missing.md"),
	} {
		if name == "text" {
			if err := os.WriteFile(raw, []byte("not a plan"), 0o600); err != nil {
				t.Fatal(err)
			}
		}
		if _, err := resolveRelayPlanPath(h.parent.CWD, raw); err == nil {
			t.Errorf("%s plan unexpectedly accepted", name)
		}
	}

	large := filepath.Join(h.parent.CWD, "docs", "large.md")
	if err := os.WriteFile(large, make([]byte, relayPlanMaxBytes+1), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := resolveRelayPlanPath(h.parent.CWD, large); err == nil || !strings.Contains(err.Error(), "at most") {
		t.Fatalf("large plan error = %v, want size validation", err)
	}

	link := filepath.Join(h.parent.CWD, "docs", "outside-link.md")
	if err := os.Symlink(outside, link); err != nil {
		t.Skipf("symlink creation unavailable: %v", err)
	}
	if _, err := resolveRelayPlanPath(h.parent.CWD, link); err == nil {
		t.Fatal("symlink escaping the cwd unexpectedly accepted")
	}
}

func TestResolveRelayPlanPathAllowsPlanBelowLinkedDirectory(t *testing.T) {
	h := newRelayHarness(t)
	outDir := t.TempDir()
	want := filepath.Join(outDir, "junction-plan.md")
	if err := os.WriteFile(want, []byte("# linked plan\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	linkDir := filepath.Join(h.parent.CWD, "docs", "local")
	if err := os.MkdirAll(filepath.Dir(linkDir), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outDir, linkDir); err != nil {
		t.Skipf("directory symlink creation unavailable: %v", err)
	}

	got, err := resolveRelayPlanPath(h.parent.CWD, filepath.Join("docs", "local", "junction-plan.md"))
	if err != nil {
		t.Fatalf("plan below linked directory rejected: %v", err)
	}
	resolvedWant, err := filepath.EvalSymlinks(want)
	if err != nil {
		t.Fatal(err)
	}
	if got != filepath.Clean(resolvedWant) {
		t.Fatalf("resolved plan = %q, want %q", got, resolvedWant)
	}
}

func TestHandleRelayStartValidationAndDefaults(t *testing.T) {
	h := newRelayHarness(t)
	h.s.cfg.Hub.AllowLoopbackWithoutToken = true
	plan := writeRelayPlan(t, h, "start.md")

	code, got := relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay", relayAPIStartBody(plan, relayModeSameTree))
	if code != http.StatusOK || !got.OK {
		t.Fatalf("start status=%d response=%+v", code, got)
	}
	if got.Relay.Mode != relayModeSameTree || got.Relay.MaxRounds != relayDefaultMaxRounds || got.Relay.EscalateAfter != relayDefaultEscalateAfter {
		t.Fatalf("defaults/status = %+v", got.Relay)
	}
	if got.OrchestrationID == "" || got.Relay.ImplementationSessionID == 0 {
		t.Fatalf("start response omitted relay identity: %+v", got)
	}

	badMode := relayAPIStartBody(plan, "invalid")
	code, got = relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay", badMode)
	if code != http.StatusBadRequest || got.Error != "bad_request" {
		t.Fatalf("bad mode status=%d response=%+v", code, got)
	}
	badRounds := relayAPIStartBody(plan, relayModeSameTree)
	badRounds.MaxRounds = 10
	code, got = relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay", badRounds)
	if code != http.StatusBadRequest || got.Error != "bad_request" {
		t.Fatalf("bad max rounds status=%d response=%+v", code, got)
	}
	badEscalation := relayAPIStartBody(plan, relayModeSameTree)
	badEscalation.EscalateAfter = 6
	code, got = relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay", badEscalation)
	if code != http.StatusBadRequest || got.Error != "bad_request" {
		t.Fatalf("bad escalation status=%d response=%+v", code, got)
	}

	hMissing := newRelayHarness(t)
	hMissing.s.cfg.Hub.AllowLoopbackWithoutToken = true
	missingPlan := writeRelayPlan(t, hMissing, "missing.md")
	missing := relayAPIStartBody(missingPlan, relayModeSameTree)
	missing.Roles = nil
	code, got = relayAPICall(t, hMissing.s, http.MethodPost, "/api/sessions/1/relay", missing)
	if code != http.StatusBadRequest || got.Error != "relay_roles_missing" || !strings.Contains(got.Detail, relayRoleImplementation) {
		t.Fatalf("missing roles status=%d response=%+v", code, got)
	}
}

func TestHandleRelayMultipleStopGetAndLimits(t *testing.T) {
	h := newRelayHarness(t)
	h.s.cfg.Hub.AllowLoopbackWithoutToken = true
	plan := writeRelayPlan(t, h, "multiple.md")
	body := relayAPIStartBody(plan, relayModeSameTree)
	code, first := relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay", body)
	if code != http.StatusOK {
		t.Fatalf("first start status=%d response=%+v", code, first)
	}
	code, second := relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay", body)
	if code != http.StatusOK || first.OrchestrationID == second.OrchestrationID {
		t.Fatalf("second start status=%d response=%+v first=%+v", code, second, first)
	}
	code, limited := relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay", body)
	if code != http.StatusTooManyRequests || limited.Error != "orchestration_limit" || !strings.Contains(limited.Detail, "running=2") || !strings.Contains(limited.Detail, "max_children_per_parent=4") {
		t.Fatalf("limit status=%d response=%+v", code, limited)
	}

	code, ambiguous := relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay-stop", relayControlAPIRequest{})
	if code != http.StatusBadRequest || ambiguous.Error != "relay_ambiguous" || !strings.Contains(ambiguous.Detail, first.OrchestrationID) || !strings.Contains(ambiguous.Detail, second.OrchestrationID) {
		t.Fatalf("ambiguous stop status=%d response=%+v", code, ambiguous)
	}
	code, stopped := relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay-stop", relayControlAPIRequest{OrchestrationID: first.OrchestrationID})
	if code != http.StatusOK || stopped.Relay.State != relayStateStopped {
		t.Fatalf("explicit stop status=%d response=%+v", code, stopped)
	}
	code, stillRunning := relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay-resume", relayControlAPIRequest{OrchestrationID: second.OrchestrationID})
	if code != http.StatusConflict || stillRunning.Error != "relay_not_resumable" {
		t.Fatalf("resume active status=%d response=%+v", code, stillRunning)
	}
	code, activeCleanup := relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay-cleanup", relayControlAPIRequest{OrchestrationID: second.OrchestrationID})
	if code != http.StatusConflict || activeCleanup.Error != "relay_active" {
		t.Fatalf("cleanup active status=%d response=%+v", code, activeCleanup)
	}
	code, stopped = relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay-stop", relayControlAPIRequest{})
	if code != http.StatusOK || stopped.Relay.OrchestrationID != second.OrchestrationID {
		t.Fatalf("single remaining stop status=%d response=%+v", code, stopped)
	}
	code, got := relayAPICall(t, h.s, http.MethodGet, "/api/sessions/1/relay", nil)
	if code != http.StatusOK || len(got.Relays) != 2 || len(got.Relays[0].Events) == 0 || len(got.Relays[1].Events) == 0 {
		t.Fatalf("get relays status=%d response=%+v", code, got)
	}
	code, none := relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay-stop", relayControlAPIRequest{})
	if code != http.StatusNotFound || none.Error != "relay_not_found" {
		t.Fatalf("stop with no active status=%d response=%+v", code, none)
	}
}

func TestCloseRelayChildrenDismissesAndStopsBoardWatch(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	if !h.s.relayFinish(h.run(id), relayStateCompleted, "", "finished by test") {
		t.Fatal("relayFinish returned false")
	}
	// The harness websocket is only a send sentinel and cannot be closed.
	// Lifecycle tests cover connected-wrapper shutdown; here we exercise the
	// no-wrapper teardown path used when a child process already exited.
	h.s.sessionsMu.Lock()
	delete(h.s.wrappers, impl)
	h.s.sessionsMu.Unlock()

	if remaining := h.s.closeRelayChildren([]int{impl}); len(remaining) != 0 {
		t.Fatalf("remaining children = %v", remaining)
	}
	h.s.markRelayChildrenDone(id, []int{impl})
	h.s.sessionsMu.Lock()
	_, sessionExists := h.s.sessions[impl]
	_, wrapperExists := h.s.wrappers[impl]
	h.s.sessionsMu.Unlock()
	if sessionExists || wrapperExists {
		t.Fatalf("child retained after cleanup: session=%v wrapper=%v", sessionExists, wrapperExists)
	}
	h.s.orchestration.mu.Lock()
	child := h.s.orchestration.boards[id].Children[impl]
	done := h.s.orchestration.boards[id].Done[impl]
	h.s.orchestration.mu.Unlock()
	if child == nil || !child.Done || !done {
		t.Fatalf("board child not finalized: child=%+v done=%v", child, done)
	}
}

func TestHandleRelayStartUsesRoleMappingAndReportsNotGit(t *testing.T) {
	h := newRelayHarness(t)
	h.s.cfg.Hub.AllowLoopbackWithoutToken = true
	plan := writeRelayPlan(t, h, "mapping.md")
	h.parent.OrchestrationID = "mapped"
	h.s.orchestration.roles["mapped"] = relayTestRoles(false)
	body := relayStartJSON{PlanPath: plan, Mode: relayModeSameTree, AcknowledgeChildFullBypass: true}
	code, got := relayAPICall(t, h.s, http.MethodPost, "/api/sessions/1/relay", body)
	if code != http.StatusOK || !got.OK {
		t.Fatalf("mapped role start status=%d response=%+v", code, got)
	}

	h2 := newRelayHarness(t)
	h2.s.cfg.Hub.AllowLoopbackWithoutToken = true
	plan2 := writeRelayPlan(t, h2, "non-git.md")
	code, got = relayAPICall(t, h2.s, http.MethodPost, "/api/sessions/1/relay", relayAPIStartBody(plan2, relayModeWorktree))
	if code != http.StatusBadRequest || got.Error != "relay_not_git" {
		t.Fatalf("non-git worktree status=%d response=%+v", code, got)
	}
}

func TestRelayRolesForStartMergesSubscriptionOverrides(t *testing.T) {
	s := newTestServer()
	s.orchestration.roles["mapped"] = map[string]orchestrationRoleAssignment{
		relayRoleImplementation: {Provider: "codex", Model: "gpt-5", Subscription: "default"},
	}

	roles := s.relayRolesForStart(0, "mapped", map[string]*orchestrationRoleAssignment{
		relayRoleImplementation: {Subscription: "work"},
	})
	if got := roles[relayRoleImplementation]; got.Subscription != "work" || got.Provider != "codex" || got.Model != "gpt-5" {
		t.Fatalf("subscription override merged incorrectly: %+v", got)
	}

	roles = s.relayRolesForStart(0, "mapped", map[string]*orchestrationRoleAssignment{
		relayRoleImplementation: {Provider: "claude"},
	})
	if got := roles[relayRoleImplementation]; got.Subscription != "default" || got.Provider != "claude" {
		t.Fatalf("existing subscription was not preserved: %+v", got)
	}
}

// 役割の起動要求 3 項目が、役割対応表とのマージで落ちないことを固定する
// （子 plan: docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C3）。
// relayRolesForStart はフィールド単位の allowlist なので、足し忘れると
// `orchestrate relay --impl claude/opus@high` の effort が黙って捨てられる。
func TestRelayRolesForStartMergesLaunchOptions(t *testing.T) {
	s := newTestServer()
	s.orchestration.roles["mapped"] = map[string]orchestrationRoleAssignment{
		relayRoleImplementation: {Provider: "codex", Model: "gpt-5"},
		relayRoleReview:         {Provider: "claude", Model: "opus", Effort: "medium"},
	}

	roles := s.relayRolesForStart(0, "mapped", map[string]*orchestrationRoleAssignment{
		relayRoleImplementation: {Provider: "claude", Model: "opus", Effort: "high", ExecutionMode: "interactive", PermissionPreset: "attended"},
	})
	if got := roles[relayRoleImplementation]; got.Effort != "high" || got.ExecutionMode != "interactive" || got.PermissionPreset != "attended" {
		t.Fatalf("launch options were dropped by the merge: %+v", got)
	}
	// 送らなかった項目は対応表の値のまま（空で上書きしない）。
	if got := roles[relayRoleReview]; got.Effort != "medium" {
		t.Fatalf("mapped effort was overwritten by an absent request field: %+v", got)
	}
}

func TestRelayFinishPublishesRelayDoneSummary(t *testing.T) {
	h := newRelayHarness(t)
	var got proto.DoneSummary
	h.s.relay.publishDone = func(summary proto.DoneSummary) { got = summary }
	st := h.start()
	run := h.run(st.OrchestrationID)
	if !h.s.relayFinish(run, relayStateStopped, relayReasonUserStop, "stopped by test") {
		t.Fatal("relayFinish returned false")
	}
	if got.SessionID != h.parent.ID || got.Provider != h.parent.Provider || got.Kind != "relay" || !strings.Contains(got.Text, "stopped (c=0, round=0") || got.Fallback {
		t.Fatalf("relay completion summary = %+v", got)
	}
}

func TestHandleOrchestrationConfigExposesRelayBudget(t *testing.T) {
	s := newTestServer()
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	s.cfg.Orchestration.MaxChildrenPerParent = 7
	rr := httptest.NewRecorder()
	s.handleOrchestrationConfig(rr, orchestrationRequest(http.MethodGet, "/api/orchestration-config", nil))
	var got relayAPIResponse
	if err := json.Unmarshal(rr.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if rr.Code != http.StatusOK || got.MaxChildren != 7 {
		t.Fatalf("config status=%d response=%+v", rr.Code, got)
	}
}

func TestRelayRequiresFullBypassAck(t *testing.T) {
	fullRoles := map[string]orchestrationRoleAssignment{
		"implementation": {Provider: "codex", Model: "gpt"},
		"review":         {Provider: "claude", Model: "opus"},
	}
	presetRoles := map[string]orchestrationRoleAssignment{
		"implementation": {Provider: "codex", Model: "gpt", PermissionPreset: config.PermissionPresetBounded},
		"review":         {Provider: "claude", Model: "opus", PermissionPreset: config.PermissionPresetAttended},
	}

	cfg := config.OrchestrationConfig{}
	if !relayRequiresFullBypassAck(cfg, fullRoles) {
		t.Fatal("default full bypass must require ack")
	}
	if !relayRequiresFullBypassAck(cfg, nil) {
		t.Fatal("empty roles with default full bypass must require ack")
	}
	if relayRequiresFullBypassAck(cfg, presetRoles) {
		t.Fatal("every role with an explicit preset must not require ack")
	}

	off := false
	cfg.ChildFullBypass = &off
	if relayRequiresFullBypassAck(cfg, fullRoles) {
		t.Fatal("child_full_bypass=false must not require ack")
	}

	cfg = config.OrchestrationConfig{ChildPermissionDefault: config.PermissionPresetBounded}
	if relayRequiresFullBypassAck(cfg, fullRoles) {
		t.Fatal("safer child_permission_default must not require ack")
	}
}

func TestHandleRelayStartRequiresFullBypassAck(t *testing.T) {
	s := newTestServer()
	s.cfg.Hub.AllowLoopbackWithoutToken = true
	parent := registerTestSession(s, 1, "codex")
	parent.CWD = t.TempDir()
	plan := filepath.Join(parent.CWD, "plan.md")
	if err := os.WriteFile(plan, []byte("# plan\n"), 0o644); err != nil {
		t.Fatal(err)
	}

	code, got := relayAPICall(t, s, http.MethodPost, "/api/sessions/1/relay", map[string]any{
		"plan_path": plan,
		"roles": map[string]any{
			"implementation": map[string]any{"provider": "codex", "model": "gpt-5"},
			"review":         map[string]any{"provider": "claude", "model": "opus"},
		},
	})
	if code != http.StatusBadRequest {
		t.Fatalf("status = %d, want 400, detail=%s", code, got.Detail)
	}
	if got.Error != "full_bypass_acknowledgment_required" {
		t.Fatalf("error = %q, want full_bypass_acknowledgment_required", got.Error)
	}
}
