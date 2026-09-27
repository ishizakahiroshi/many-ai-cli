package hub

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"strconv"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/provider"
)

// TestCliUpdateFakeVersionHelper is "the CLI whose --version many-ai-cli
// would run" for update-job before/after checks, the same self-exec
// technique TestCliVersionFakeCLIHelper (cli_version_test.go) uses. It exits
// before any go test framework output, so a normal `go test` run (where
// MANY_AI_CLI_FAKE_UPDATE_VERSION_MODE is unset) just returns immediately.
func TestCliUpdateFakeVersionHelper(t *testing.T) {
	mode := os.Getenv("MANY_AI_CLI_FAKE_UPDATE_VERSION_MODE")
	if mode == "" {
		return
	}
	switch mode {
	case "dynamic":
		// "dynamic" simulates a CLI whose reported version actually changes
		// once the (fake) update command has run: it prints v1.1.0 once the
		// state file the update command writes exists, v1.0.0 otherwise.
		state := os.Getenv("MANY_AI_CLI_FAKE_UPDATE_STATE_FILE")
		if state != "" {
			if _, err := os.Stat(state); err == nil {
				fmt.Println("fakecli v1.1.0")
				os.Exit(0)
			}
		}
		fmt.Println("fakecli v1.0.0")
	case "static":
		fmt.Println("fakecli v1.0.0")
	case "error":
		os.Exit(1)
	}
	os.Exit(0)
}

// TestCliUpdateFakeCommandHelper is "the CLI update command many-ai-cli
// would run".
func TestCliUpdateFakeCommandHelper(t *testing.T) {
	mode := os.Getenv("MANY_AI_CLI_FAKE_UPDATE_COMMAND_MODE")
	if mode == "" {
		return
	}
	switch mode {
	case "ok":
		if state := os.Getenv("MANY_AI_CLI_FAKE_UPDATE_STATE_FILE"); state != "" {
			_ = os.WriteFile(state, []byte("updated"), 0o600)
		}
		fmt.Println("updating fakecli...")
	case "noop":
		fmt.Println("fakecli is already up to date")
	case "fail":
		fmt.Fprintln(os.Stderr, "update failed: boom")
		os.Exit(1)
	case "busy":
		fmt.Fprintln(os.Stderr, "EBUSY: resource busy or locked, rename 'fakecli.exe'")
		os.Exit(1)
	case "login":
		fmt.Println("Please sign-in required to continue")
	case "hang":
		time.Sleep(30 * time.Second)
	case "sleep":
		sleepMS := 300
		if v := os.Getenv("MANY_AI_CLI_FAKE_UPDATE_SLEEP_MS"); v != "" {
			if n, err := strconv.Atoi(v); err == nil {
				sleepMS = n
			}
		}
		time.Sleep(time.Duration(sleepMS) * time.Millisecond)
	}
	os.Exit(0)
}

// withFakeCLILookupNames is withFakeCLILookup (cli_version_test.go)
// generalized to more than one candidate name at once, needed by the
// concurrency test below which runs 3 fake providers side by side.
func withFakeCLILookupNames(t *testing.T, names ...string) {
	t.Helper()
	original := providerCommandLookPath
	t.Cleanup(func() { providerCommandLookPath = original })
	exe := os.Args[0]
	set := make(map[string]bool, len(names))
	for _, n := range names {
		set[n] = true
	}
	providerCommandLookPath = func(file string) (string, error) {
		if set[file] {
			return exe, nil
		}
		return "", fmt.Errorf("not found: %s", file)
	}
}

// fakeUpdateDefinition builds a provider.Definition whose version/update
// commands re-exec this test binary selecting versionTestFunc/commandTestFunc
// via -test.run, mirroring fakeCLIDefinition (cli_version_test.go).
func fakeUpdateDefinition(id, versionTestFunc, commandTestFunc string) provider.Definition {
	enabled := true
	return provider.Definition{
		SchemaVersion: provider.CurrentSchemaVersion,
		ID:            id,
		DisplayName:   "Fake CLI",
		Launch:        &provider.LaunchDefinition{Executable: id},
		Update: &provider.UpdateDefinition{
			VersionArgs: []string{"-test.run=" + versionTestFunc},
			Args:        []string{"-test.run=" + commandTestFunc},
			Enabled:     &enabled,
		},
	}
}

func newCLIUpdateTestServer(t *testing.T) *Server {
	t.Helper()
	s := newTestServer()
	s.cfg.Token = "test-token"
	s.cliVersions = newCLIVersionState()
	return s
}

func buildFakeUpdateRegistry(t *testing.T, s *Server, defs ...provider.Definition) *provider.Registry {
	t.Helper()
	registry, diagnostics, err := buildProviderRegistry(s.cfg, defs)
	if err != nil {
		t.Fatalf("build provider registry: %v", err)
	}
	for _, d := range diagnostics {
		if d.IsError() {
			t.Fatalf("unexpected registry diagnostic: %#v", d)
		}
	}
	s.providers = registry
	return registry
}

// --- C1: 6 つの理由コード ---------------------------------------------------

func TestPlanCLIUpdateNotInstalledWhenMissingFromRegistry(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	registry := buildFakeUpdateRegistry(t, s, fakeUpdateDefinition("known", "TestCliUpdateFakeVersionHelper", "TestCliUpdateFakeCommandHelper"))
	plan := planCLIUpdate("unknown", registry)
	if plan.Eligible || plan.Reason != cliUpdateReasonNotInstalled {
		t.Fatalf("plan = %#v, want not_installed", plan)
	}
}

func TestPlanCLIUpdateNotInstalledWhenExecutableNotOnPath(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	withFakeCLILookupNames(t, "other-cli") // "missingcli" resolves to nothing
	def := fakeUpdateDefinition("missingcli", "TestCliUpdateFakeVersionHelper", "TestCliUpdateFakeCommandHelper")
	registry := buildFakeUpdateRegistry(t, s, def)
	plan := planCLIUpdate("missingcli", registry)
	if plan.Eligible || plan.Reason != cliUpdateReasonNotInstalled {
		t.Fatalf("plan = %#v, want not_installed", plan)
	}
}

func TestPlanCLIUpdateUpdateDisabled(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	withFakeCLILookupNames(t, "disabledcli")
	disabled := false
	def := provider.Definition{
		SchemaVersion: provider.CurrentSchemaVersion,
		ID:            "disabledcli",
		DisplayName:   "Disabled CLI",
		Launch:        &provider.LaunchDefinition{Executable: "disabledcli"},
		Update:        &provider.UpdateDefinition{Args: []string{"update"}, Enabled: &disabled},
	}
	registry := buildFakeUpdateRegistry(t, s, def)
	plan := planCLIUpdate("disabledcli", registry)
	if plan.Eligible || plan.Reason != cliUpdateReasonUpdateDisabled {
		t.Fatalf("plan = %#v, want update_disabled", plan)
	}
}

func TestPlanCLIUpdateLoginMayBeRequiredOverridesUpdateDisabled(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	withFakeCLILookupNames(t, "cursorish")
	disabled := false
	def := provider.Definition{
		SchemaVersion: provider.CurrentSchemaVersion,
		ID:            "cursorish",
		DisplayName:   "Cursorish CLI",
		Launch:        &provider.LaunchDefinition{Executable: "cursorish"},
		Update: &provider.UpdateDefinition{
			Args: []string{"update"}, Enabled: &disabled, LoginMayBeRequired: true,
		},
	}
	registry := buildFakeUpdateRegistry(t, s, def)
	plan := planCLIUpdate("cursorish", registry)
	if plan.Eligible || plan.Reason != cliUpdateReasonLoginMayBeRequired {
		t.Fatalf("plan = %#v, want login_may_be_required", plan)
	}
}

// TestPlanCLIUpdateNotConfiguredForSecondaryCandidate mirrors the copilot
// manifest: the primary launch candidate has an update method, its
// secondary ("gh") fallback does not.
func TestPlanCLIUpdateNotConfiguredForSecondaryCandidate(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	withFakeCLILookupNames(t, "gh") // only the fallback candidate resolves
	def := provider.Definition{
		SchemaVersion: provider.CurrentSchemaVersion,
		ID:            "copilotish",
		DisplayName:   "Copilotish CLI",
		Launch:        &provider.LaunchDefinition{ExecutableCandidates: []string{"copilotish", "gh"}},
		Update:        &provider.UpdateDefinition{Args: []string{"update"}},
	}
	registry := buildFakeUpdateRegistry(t, s, def)
	plan := planCLIUpdate("copilotish", registry)
	if plan.Eligible || plan.Reason != cliUpdateReasonNotConfigured {
		t.Fatalf("plan = %#v, want update_not_configured", plan)
	}
}

func TestBeginProviderUpdateRunningSessions(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	registerTestSession(s, 1, "claude")
	ok, reason, running := s.beginProviderUpdate("claude")
	if ok || reason != cliUpdateReasonRunningSessions || running != 1 {
		t.Fatalf("beginProviderUpdate = (%v, %q, %d), want (false, running_sessions, 1)", ok, reason, running)
	}
}

func TestBeginProviderUpdateAlreadyUpdating(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	ok, _, _ := s.beginProviderUpdate("claude")
	if !ok {
		t.Fatal("first beginProviderUpdate should succeed")
	}
	ok, reason, _ := s.beginProviderUpdate("claude")
	if ok || reason != cliUpdateReasonAlreadyUpdating {
		t.Fatalf("second beginProviderUpdate = (%v, %q), want (false, already_updating)", ok, reason)
	}
}

func TestBeginAndEndProviderUpdateRoundTrip(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	ok, _, _ := s.beginProviderUpdate("claude")
	if !ok || !s.providerUpdating("claude") {
		t.Fatal("provider should be marked updating after beginProviderUpdate")
	}
	s.endProviderUpdate("claude")
	if s.providerUpdating("claude") {
		t.Fatal("provider should not be marked updating after endProviderUpdate")
	}
}

// --- C1: 起動の一時停止（3 入口） -------------------------------------------

func TestHandleSpawnBlockedWhileProviderUpdating(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	ok, _, _ := s.beginProviderUpdate("claude")
	if !ok {
		t.Fatal("beginProviderUpdate setup failed")
	}
	w := httptest.NewRecorder()
	s.handleSpawn(w, subsRequest(t, http.MethodPost, "/api/spawn", map[string]any{
		"provider": "claude",
		"cwd":      s.hubCWD,
	}))
	if w.Code != http.StatusConflict {
		t.Fatalf("code = %d, want 409: %s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "provider_updating") {
		t.Fatalf("body = %s, want provider_updating", w.Body.String())
	}
}

func TestHandleSpawnGridBlockedWhileProviderUpdating(t *testing.T) {
	s, _ := subsTestServer(t)
	s.hubCWD = t.TempDir()
	ok, _, _ := s.beginProviderUpdate("claude")
	if !ok {
		t.Fatal("beginProviderUpdate setup failed")
	}
	w := httptest.NewRecorder()
	s.handleSpawnGrid(w, subsRequest(t, http.MethodPost, "/api/spawn-grid", map[string]any{
		"preset":   "ai+shell",
		"provider": "claude",
		"count":    2,
		"cwd":      s.hubCWD,
	}))
	if w.Code != http.StatusConflict {
		t.Fatalf("code = %d, want 409: %s", w.Code, w.Body.String())
	}
	if !strings.Contains(w.Body.String(), "provider_updating") {
		t.Fatalf("body = %s, want provider_updating", w.Body.String())
	}
}

func TestSpawnWrappedSessionBlockedWhileProviderUpdating(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	ok, _, _ := s.beginProviderUpdate("claude")
	if !ok {
		t.Fatal("beginProviderUpdate setup failed")
	}
	_, err := s.spawnWrappedSession(spawnWrappedSpec{
		Context:  context.Background(),
		Provider: "claude",
		CWD:      t.TempDir(),
		Label:    "test-blocked",
	}, 10*time.Millisecond)
	if err == nil {
		t.Fatal("spawnWrappedSession should refuse a provider mid-update")
	}
	if !strings.Contains(err.Error(), "updating") {
		t.Fatalf("err = %v, want a message mentioning the provider is updating", err)
	}
}

// --- C2: 6 種の結果分類 ------------------------------------------------------

func runSingleFakeUpdate(t *testing.T, s *Server, def provider.Definition) (cliUpdateProviderStatus, *cliUpdateJob) {
	t.Helper()
	registry := buildFakeUpdateRegistry(t, s, def)
	plan := planCLIUpdate(def.ID, registry)
	if !plan.Eligible {
		t.Fatalf("plan for %q not eligible: %s", def.ID, plan.Reason)
	}
	job := newCLIUpdateJob("job-"+def.ID, false, []string{def.ID}, nil, time.Now())
	s.runCLIUpdateJobProvider(context.Background(), job, registry, plan)
	providers, _ := job.snapshot()
	if len(providers) != 1 {
		t.Fatalf("providers = %#v, want exactly 1", providers)
	}
	return providers[0], job
}

func TestRunCLIUpdateJobProviderUpdated(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	withFakeCLILookupNames(t, "updatedcli")
	stateFile := t.TempDir() + "/state"
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_STATE_FILE", stateFile)
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_VERSION_MODE", "dynamic")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_COMMAND_MODE", "ok")

	def := fakeUpdateDefinition("updatedcli", "TestCliUpdateFakeVersionHelper", "TestCliUpdateFakeCommandHelper")
	status, job := runSingleFakeUpdate(t, s, def)
	if status.State != cliUpdateStateUpdated {
		t.Fatalf("State = %q, want %q (status=%#v)", status.State, cliUpdateStateUpdated, status)
	}
	if status.VersionBefore == "" || status.VersionAfter == "" || status.VersionBefore == status.VersionAfter {
		t.Fatalf("expected differing before/after versions, got before=%q after=%q", status.VersionBefore, status.VersionAfter)
	}

	// ログファイルが provider ごとに 1 つでき、argv と終了コードが書かれている。
	logPath, ok := job.logPathFor("updatedcli")
	if !ok {
		t.Fatal("expected a log path for updatedcli")
	}
	content, err := os.ReadFile(logPath)
	if err != nil {
		t.Fatalf("read log: %v", err)
	}
	text := string(content)
	if !strings.Contains(text, "argv: updatedcli") {
		t.Fatalf("log missing argv line: %s", text)
	}
	if !strings.Contains(text, "exit_code: 0") {
		t.Fatalf("log missing exit_code line: %s", text)
	}
}

func TestRunCLIUpdateJobProviderLatest(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	withFakeCLILookupNames(t, "latestcli")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_VERSION_MODE", "static")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_COMMAND_MODE", "noop")

	def := fakeUpdateDefinition("latestcli", "TestCliUpdateFakeVersionHelper", "TestCliUpdateFakeCommandHelper")
	status, _ := runSingleFakeUpdate(t, s, def)
	if status.State != cliUpdateStateLatest {
		t.Fatalf("State = %q, want %q (status=%#v)", status.State, cliUpdateStateLatest, status)
	}
}

func TestRunCLIUpdateJobProviderUnknown(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	withFakeCLILookupNames(t, "unknowncli")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_VERSION_MODE", "error")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_COMMAND_MODE", "ok")

	def := fakeUpdateDefinition("unknowncli", "TestCliUpdateFakeVersionHelper", "TestCliUpdateFakeCommandHelper")
	status, _ := runSingleFakeUpdate(t, s, def)
	if status.State != cliUpdateStateUnknown {
		t.Fatalf("State = %q, want %q (status=%#v)", status.State, cliUpdateStateUnknown, status)
	}
}

func TestRunCLIUpdateJobProviderFailed(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	withFakeCLILookupNames(t, "failedcli")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_VERSION_MODE", "static")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_COMMAND_MODE", "fail")

	def := fakeUpdateDefinition("failedcli", "TestCliUpdateFakeVersionHelper", "TestCliUpdateFakeCommandHelper")
	status, _ := runSingleFakeUpdate(t, s, def)
	if status.State != cliUpdateStateFailed {
		t.Fatalf("State = %q, want %q (status=%#v)", status.State, cliUpdateStateFailed, status)
	}
}

func TestRunCLIUpdateJobProviderFileInUse(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	withFakeCLILookupNames(t, "busycli")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_VERSION_MODE", "static")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_COMMAND_MODE", "busy")

	def := fakeUpdateDefinition("busycli", "TestCliUpdateFakeVersionHelper", "TestCliUpdateFakeCommandHelper")
	status, _ := runSingleFakeUpdate(t, s, def)
	if status.State != cliUpdateStateFileInUse {
		t.Fatalf("State = %q, want %q (status=%#v)", status.State, cliUpdateStateFileInUse, status)
	}
}

func TestRunCLIUpdateJobProviderLoginRequired(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	withFakeCLILookupNames(t, "logincli")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_VERSION_MODE", "static")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_COMMAND_MODE", "login")

	def := fakeUpdateDefinition("logincli", "TestCliUpdateFakeVersionHelper", "TestCliUpdateFakeCommandHelper")
	status, _ := runSingleFakeUpdate(t, s, def)
	if status.State != cliUpdateStateLoginRequired {
		t.Fatalf("State = %q, want %q (status=%#v)", status.State, cliUpdateStateLoginRequired, status)
	}
}

// --- C2: 同時実行 vs serial --------------------------------------------------

// TestRunCLIUpdateJobConcurrentIsFasterThanSerial pins the C2 completion
// condition: 3 providers running concurrently finish in roughly 1x the fake
// command's sleep duration, while the same 3 with serial:true take roughly
// 3x — proving the concurrent job actually overlaps the 3 processes instead
// of accidentally serializing them, and that serial actually waits for each
// one before starting the next.
func TestRunCLIUpdateJobConcurrentIsFasterThanSerial(t *testing.T) {
	s := newCLIUpdateTestServer(t)
	ids := []string{"conc-a", "conc-b", "conc-c"}
	withFakeCLILookupNames(t, ids...)
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_VERSION_MODE", "static")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_COMMAND_MODE", "sleep")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_SLEEP_MS", "300")

	defs := make([]provider.Definition, 0, len(ids))
	for _, id := range ids {
		defs = append(defs, fakeUpdateDefinition(id, "TestCliUpdateFakeVersionHelper", "TestCliUpdateFakeCommandHelper"))
	}
	registry := buildFakeUpdateRegistry(t, s, defs...)
	plans := make(map[string]cliUpdatePlan, len(ids))
	for _, id := range ids {
		plan := planCLIUpdate(id, registry)
		if !plan.Eligible {
			t.Fatalf("plan for %q not eligible: %s", id, plan.Reason)
		}
		plans[id] = plan
	}

	start := time.Now()
	concurrentJob := newCLIUpdateJob("job-concurrent", false, ids, nil, time.Now())
	s.runCLIUpdateJob(concurrentJob, registry, plans)
	concurrentElapsed := time.Since(start)

	start = time.Now()
	serialJob := newCLIUpdateJob("job-serial", true, ids, nil, time.Now())
	s.runCLIUpdateJob(serialJob, registry, plans)
	serialElapsed := time.Since(start)

	if concurrentElapsed >= serialElapsed {
		t.Fatalf("concurrent (%v) should be faster than serial (%v)", concurrentElapsed, serialElapsed)
	}
	if concurrentElapsed > 700*time.Millisecond {
		t.Fatalf("concurrent run took %v, want well under 3x the 300ms sleep (proves overlap)", concurrentElapsed)
	}
	if serialElapsed < 800*time.Millisecond {
		t.Fatalf("serial run took %v, want roughly 3x the 300ms sleep (proves serialization)", serialElapsed)
	}
}

// --- C3: POST の excluded / cleanup ------------------------------------------

func waitForCLIUpdateJobTerminal(t *testing.T, job *cliUpdateJob, providerID string, timeout time.Duration) cliUpdateProviderStatus {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for {
		providers, _ := job.snapshot()
		for _, p := range providers {
			if p.Provider != providerID {
				continue
			}
			if p.State != cliUpdateStateQueued && p.State != cliUpdateStateRunning {
				return p
			}
		}
		if time.Now().After(deadline) {
			t.Fatalf("provider %q did not reach a terminal state within %v", providerID, timeout)
		}
		time.Sleep(10 * time.Millisecond)
	}
}

// TestHandleCLIUpdatesCreateExcludesIneligibleProviders pins the C3
// completion condition: providers that are not eligible right now land in
// the response's "excluded" list with a reason code and are never accepted
// (never run), while an eligible provider is accepted and actually runs.
func TestHandleCLIUpdatesCreateExcludesIneligibleProviders(t *testing.T) {
	s, _ := subsTestServer(t)
	s.cliVersions = newCLIVersionState()
	withFakeCLILookupNames(t, "enabledcli", "disabledcli")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_VERSION_MODE", "static")
	t.Setenv("MANY_AI_CLI_FAKE_UPDATE_COMMAND_MODE", "noop")

	disabled := false
	disabledDef := provider.Definition{
		SchemaVersion: provider.CurrentSchemaVersion,
		ID:            "disabledcli",
		DisplayName:   "Disabled CLI",
		Launch:        &provider.LaunchDefinition{Executable: "disabledcli"},
		Update:        &provider.UpdateDefinition{Args: []string{"update"}, Enabled: &disabled},
	}
	enabledDef := fakeUpdateDefinition("enabledcli", "TestCliUpdateFakeVersionHelper", "TestCliUpdateFakeCommandHelper")
	buildFakeUpdateRegistry(t, s, disabledDef, enabledDef)

	w := httptest.NewRecorder()
	s.handleCLIUpdatesCreate(w, subsRequest(t, http.MethodPost, "/api/cli-updates", map[string]any{
		"providers": []string{"enabledcli", "disabledcli", "unknowncli"},
	}))
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d, want 200: %s", w.Code, w.Body.String())
	}
	var resp cliUpdateCreateResponse
	if err := json.Unmarshal(w.Body.Bytes(), &resp); err != nil {
		t.Fatalf("decode response: %v", err)
	}
	if len(resp.Accepted) != 1 || resp.Accepted[0] != "enabledcli" {
		t.Fatalf("Accepted = %#v, want exactly [enabledcli]", resp.Accepted)
	}
	excludedReasons := map[string]string{}
	for _, e := range resp.Excluded {
		excludedReasons[e.Provider] = e.Reason
	}
	if excludedReasons["disabledcli"] != cliUpdateReasonUpdateDisabled {
		t.Fatalf("disabledcli reason = %q, want update_disabled", excludedReasons["disabledcli"])
	}
	if excludedReasons["unknowncli"] != cliUpdateReasonNotInstalled {
		t.Fatalf("unknowncli reason = %q, want not_installed", excludedReasons["unknowncli"])
	}

	job, ok := s.cliUpdateJobLookup(resp.JobID)
	if !ok {
		t.Fatalf("job %q not registered", resp.JobID)
	}
	providers, excluded := job.snapshot()
	for _, p := range providers {
		if p.Provider == "disabledcli" || p.Provider == "unknowncli" {
			t.Fatalf("excluded provider %q must not have a live job status: %#v", p.Provider, p)
		}
	}
	if len(excluded) != 2 {
		t.Fatalf("job.excluded = %#v, want 2 entries", excluded)
	}

	waitForCLIUpdateJobTerminal(t, job, "enabledcli", 5*time.Second)
}

func TestCleanCliUpdateLogsRemovesOnlyExpiredLogs(t *testing.T) {
	s := newTestServer()
	s.cfg.Log.SessionRetentionDays = 7
	dir := t.TempDir()
	s.cfg.Hub.LogDir = dir
	logsDir := dir + "/cli-updates"
	if err := os.MkdirAll(logsDir, 0o700); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	fresh := logsDir + "/fresh.log"
	stale := logsDir + "/stale.log"
	for _, p := range []string{fresh, stale} {
		if err := os.WriteFile(p, []byte("x"), 0o600); err != nil {
			t.Fatalf("write %s: %v", p, err)
		}
	}
	oldTime := time.Now().Add(-10 * 24 * time.Hour)
	if err := os.Chtimes(stale, oldTime, oldTime); err != nil {
		t.Fatalf("chtimes: %v", err)
	}

	s.cleanCliUpdateLogs()

	if _, err := os.Stat(fresh); err != nil {
		t.Fatalf("fresh log should survive: %v", err)
	}
	if _, err := os.Stat(stale); !os.IsNotExist(err) {
		t.Fatalf("stale log should have been removed, stat err = %v", err)
	}
}

func TestCleanCliUpdateLogsDisabledWhenRetentionIsZero(t *testing.T) {
	s := newTestServer()
	s.cfg.Log.SessionRetentionDays = 0
	dir := t.TempDir()
	s.cfg.Hub.LogDir = dir
	logsDir := dir + "/cli-updates"
	if err := os.MkdirAll(logsDir, 0o700); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	stale := logsDir + "/stale.log"
	if err := os.WriteFile(stale, []byte("x"), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}
	oldTime := time.Now().Add(-30 * 24 * time.Hour)
	if err := os.Chtimes(stale, oldTime, oldTime); err != nil {
		t.Fatalf("chtimes: %v", err)
	}

	s.cleanCliUpdateLogs()

	if _, err := os.Stat(stale); err != nil {
		t.Fatalf("retention=0 should disable cleanup entirely: %v", err)
	}
}
