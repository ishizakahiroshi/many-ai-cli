package hub

// relay_headless_test.go pins the headless relay route (子 plan:
// docs/local/plan_child_execution_modes_headless.md 内部 C4).
//
// The interactive route is pinned by relay_test.go and is not re-tested here:
// every test in that file still passes unchanged, which is what says the loop
// people already run was not disturbed.

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

// headlessRelayRoles are roles whose provider has a built-in headless
// definition. claude is the only one in this build, so both roles use it; the
// providers are what the definition table is keyed on, not who reviews whom.
func headlessRelayRoles(mode string) map[string]orchestrationRoleAssignment {
	return map[string]orchestrationRoleAssignment{
		relayRoleImplementation: {Provider: "claude", Model: "opus", ExecutionMode: mode},
		relayRoleReview:         {Provider: "claude", Model: "sonnet", ExecutionMode: mode},
	}
}

func (h *relayHarness) startHeadless(t *testing.T, roles map[string]orchestrationRoleAssignment) string {
	t.Helper()
	req := h.request()
	req.Roles = roles
	st, err := h.s.startRelay(h.parent.ID, req)
	if err != nil {
		t.Fatalf("startRelay: %v", err)
	}
	return st.OrchestrationID
}

// exit writes the child's progress file and reports the process exit, which is
// how a headless run ends: no DONE marker is required and no PTY is involved.
func (h *relayHarness) exit(id string, childID int, state, progress string) {
	h.t.Helper()
	run := h.run(id)
	if progress != "" {
		if err := os.WriteFile(childProgressPath(run.boardPath, childID), []byte(progress), 0o600); err != nil {
			h.t.Fatal(err)
		}
	}
	h.s.relayOnChildExit(id, childID, state)
}

// An explicit headless on a provider with no definition is refused when the
// relay starts, not quietly downgraded (親 plan D2). The API turns this into a
// 400 (writeRelayAPIError).
func TestRelay_startRelayRejectsHeadlessWithoutDefinition(t *testing.T) {
	h := newRelayHarness(t)
	req := h.request()
	// codex is the one built-in provider without a headless definition (the reason
	// is written out in internal/config/headless.go).
	req.Roles = map[string]orchestrationRoleAssignment{
		relayRoleImplementation: {Provider: "codex", Model: "gpt-5-mini", ExecutionMode: config.ExecutionModeHeadless},
		relayRoleReview:         {Provider: "claude", Model: "opus"},
	}
	_, err := h.s.startRelay(h.parent.ID, req)
	var modeErr errRelayExecutionMode
	if err == nil || !errors.As(err, &modeErr) {
		t.Fatalf("startRelay error = %v, want errRelayExecutionMode", err)
	}
	if modeErr.Role != relayRoleImplementation || modeErr.Provider != "codex" {
		t.Fatalf("error names %+v, want the implementation role on codex", modeErr)
	}
	if len(h.spawns) != 0 {
		t.Fatalf("a rejected relay spawned %d children", len(h.spawns))
	}
}

// orchestration.relay_execution_mode is the default for a role that names no
// mode, and auto resolves per provider: headless where there is a definition,
// interactive where there is not. The relay default is read instead of
// child_execution_mode, because the two kinds of child are driven differently.
func TestRelay_relayExecutionModeDefaultResolvesPerRole(t *testing.T) {
	h := newRelayHarness(t)
	h.s.cfg.Orchestration.RelayExecutionMode = config.ExecutionModeAuto
	h.s.cfg.Orchestration.ChildExecutionMode = config.ExecutionModeInteractive
	req := h.request()
	req.Roles = map[string]orchestrationRoleAssignment{
		relayRoleImplementation: {Provider: "claude", Model: "opus"},
		relayRoleReview:         {Provider: "codex", Model: "gpt-5-mini"},
	}
	st, err := h.s.startRelay(h.parent.ID, req)
	if err != nil {
		t.Fatalf("startRelay: %v", err)
	}
	run := h.run(st.OrchestrationID)
	run.mu.Lock()
	impl := run.roles[relayRoleImplementation].ExecutionMode
	review := run.roles[relayRoleReview].ExecutionMode
	run.mu.Unlock()
	if impl != config.ExecutionModeHeadless {
		t.Fatalf("implementation execution mode = %q, want headless (claude has a definition)", impl)
	}
	if review != config.ExecutionModeInteractive {
		t.Fatalf("review execution mode = %q, want interactive (codex has no definition)", review)
	}
	if got := h.spawns[0].ExecutionMode; got != config.ExecutionModeHeadless {
		t.Fatalf("first child launched with execution_mode %q, want headless", got)
	}
}

// An unset relay_execution_mode leaves every role exactly as it was before this
// existed: the launch request carries no execution mode at all.
func TestRelay_unsetRelayExecutionModeStaysInteractive(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	if got := h.spawns[0].ExecutionMode; got != "" {
		t.Fatalf("child execution_mode = %q, want empty", got)
	}
	run := h.run(st.OrchestrationID)
	run.mu.Lock()
	headless := run.anyHeadlessRoleLocked()
	run.mu.Unlock()
	if headless {
		t.Fatal("a relay with no execution mode reported a headless role")
	}
}

// The whole loop over the headless route: every instruction is one process, and
// each process's exit is what advances the machine. Nothing is ever injected.
func TestRelay_headlessLoopSpawnsOneProcessPerInstruction(t *testing.T) {
	h := newRelayHarness(t)
	id := h.startHeadless(t, headlessRelayRoles(config.ExecutionModeHeadless))
	implA := h.run(id).implID

	h.exit(id, implA, "completed", doneLines(relayRoleImplementation, implA, 1, false))
	run := h.run(id)
	run.mu.Lock()
	state, reviewID := run.state, run.reviewID
	run.mu.Unlock()
	if state != relayStateReviewing || reviewID == 0 {
		t.Fatalf("after the implementer exited: state=%q review=#%d, want reviewing with a reviewer", state, reviewID)
	}

	h.writeReview(id, 1, 1, false, "no findings")
	h.exit(id, reviewID, "completed", reviewProgress(reviewID, "pass"))
	run.mu.Lock()
	state, implB, completed := run.state, run.implID, run.completedCs
	run.mu.Unlock()
	if state != relayStateImplementing || completed != 1 {
		t.Fatalf("after a passing review: state=%q completed=%d, want implementing with 1 C done", state, completed)
	}
	if implB == implA {
		t.Fatalf("the next C reused child #%d; a headless instruction must get its own process", implB)
	}
	if len(h.injects) != 0 {
		t.Fatalf("headless children were injected into: %+v", h.injects)
	}
	// implementation ×2 + review ×1, and the replacement implementer took no new
	// admission slot (the child it replaced had already exited).
	if len(h.spawns) != 3 {
		t.Fatalf("spawns = %d, want 3 (impl, review, impl)", len(h.spawns))
	}
	for i, body := range h.spawns {
		if body.ExecutionMode != config.ExecutionModeHeadless {
			t.Fatalf("spawn %d execution_mode = %q, want headless", i, body.ExecutionMode)
		}
	}
	// A replacement process starts with no memory of the relay, so the standing
	// context travels with the instruction.
	last := h.spawns[len(h.spawns)-1].InitialPrompt
	if !strings.Contains(last, "no memory of earlier rounds") || !strings.Contains(last, h.run(id).planPath) {
		t.Fatalf("replacement prompt lost its preamble: %q", last)
	}
}

// Process exit is the completion signal (元設計 11 節): a headless run that
// forgot its DONE line still finishes its instruction, because the line can
// never arrive afterwards.
func TestRelay_headlessExitWithoutDoneStillCompletesTheUnit(t *testing.T) {
	h := newRelayHarness(t)
	id := h.startHeadless(t, headlessRelayRoles(config.ExecutionModeHeadless))
	impl := h.run(id).implID

	h.exit(id, impl, "completed", "## implementation session=1 2026-08-27T15:00:00Z\nstatus: done\n")

	run := h.run(id)
	run.mu.Lock()
	state, reviewID := run.state, run.reviewID
	run.mu.Unlock()
	if state != relayStateReviewing || reviewID == 0 {
		t.Fatalf("state=%q review=#%d, want reviewing with a reviewer", state, reviewID)
	}
}

// A reviewer that exits without a verdict is the existing "verdict missing"
// stop, not a new failure mode: the exit says the work unit is over, and the
// verdict rules then apply exactly as they do on the interactive route.
func TestRelay_headlessReviewWithoutVerdictStopsTheRelay(t *testing.T) {
	h := newRelayHarness(t)
	id := h.startHeadless(t, headlessRelayRoles(config.ExecutionModeHeadless))
	impl := h.run(id).implID
	h.exit(id, impl, "completed", doneLines(relayRoleImplementation, impl, 1, false))
	reviewID := h.run(id).reviewID

	h.exit(id, reviewID, "completed", "## review session=1 2026-08-27T15:00:00Z\nno verdict here\n")

	if got := h.status(id); got.State != relayStateStopped || got.Reason != relayReasonVerdictMissing {
		t.Fatalf("relay = %+v, want stopped(verdict_missing)", got)
	}
}

// A non-zero exit is a failure, with the same reason code a dead interactive
// child produces — and what the child managed to write reaches the board before
// the relay stops, because nothing reads that file afterwards.
func TestRelay_headlessNonZeroExitStopsTheRelayAndKeepsTheEvidence(t *testing.T) {
	h := newRelayHarness(t)
	id := h.startHeadless(t, headlessRelayRoles(config.ExecutionModeHeadless))
	impl := h.run(id).implID

	h.exit(id, impl, "error", "status: blocked\nthe repository has no plan file\n")

	if got := h.status(id); got.State != relayStateStopped || got.Reason != relayReasonChildExited {
		t.Fatalf("relay = %+v, want stopped(child_exited)", got)
	}
	if board := h.board(id); !strings.Contains(board, "the repository has no plan file") {
		t.Fatalf("the failed child's progress file never reached the board:\n%s", board)
	}
}

// An exit from a child that has already been replaced changes nothing. Both
// routes into a completion (this one and the board scan) resolve the role by the
// run's current ID, so whichever arrives second finds a stranger.
func TestRelay_headlessStaleExitIsIgnored(t *testing.T) {
	h := newRelayHarness(t)
	id := h.startHeadless(t, headlessRelayRoles(config.ExecutionModeHeadless))
	implA := h.run(id).implID
	h.exit(id, implA, "completed", doneLines(relayRoleImplementation, implA, 1, false))
	spawnsAfterFirst := len(h.spawns)

	h.exit(id, implA, "completed", doneLines(relayRoleImplementation, implA, 2, false))

	if got := h.status(id); got.State != relayStateReviewing {
		t.Fatalf("a stale exit moved the relay to %+v", got)
	}
	if len(h.spawns) != spawnsAfterFirst {
		t.Fatalf("a stale exit spawned another child (%d → %d)", spawnsAfterFirst, len(h.spawns))
	}
}

// The idle reminder is for a CLI sitting at its prompt. A headless process has
// no prompt, nothing would read the text, and quiet means "still working".
func TestRelay_headlessChildIsNeverNudged(t *testing.T) {
	h := newRelayHarness(t)
	id := h.startHeadless(t, headlessRelayRoles(config.ExecutionModeHeadless))
	impl := h.run(id).implID

	h.s.relayOnChildIdle(id, impl)

	if len(h.injects) != 0 {
		t.Fatalf("a headless child was nudged: %+v", h.injects)
	}
	for _, kind := range h.eventKinds(id) {
		if kind == relayEventNudge {
			t.Fatal("a nudge event was recorded for a headless child")
		}
	}
}

// A headless child is a process the Hub read over one WebSocket; after a Hub
// restart there is nothing to reattach to, so the relay stops at once with a
// resumable reason instead of spending the reconnect grace to reach the same
// place.
func TestRelay_restoreStopsARelayWithHeadlessChildren(t *testing.T) {
	home := t.TempDir()
	h := newRelayHarnessHome(t, home)
	h.s.relay.save = nil // use the real store so the restore has a file to read
	id := h.startHeadless(t, headlessRelayRoles(config.ExecutionModeHeadless))
	if _, err := os.Stat(relayFilePath(h.run(id).boardDir)); err != nil {
		t.Fatalf("relay.json was not written: %v", err)
	}

	restarted := newRelayHarnessHome(t, home)
	restarted.s.restoreRelays()
	run := restarted.s.relayByID(id)
	if run == nil {
		t.Fatal("the relay was not restored")
	}
	run.mu.Lock()
	state, reason, awaiting := run.state, run.reason, run.awaitingReconnect
	run.mu.Unlock()
	if state != relayStateStopped || reason != relayReasonHubRestart {
		t.Fatalf("restored relay = %s(%s), want stopped(hub_restart)", state, reason)
	}
	if awaiting {
		t.Fatal("a headless relay was left waiting for children that cannot reattach")
	}
	if !relayResumableReason(reason) {
		t.Fatalf("reason %q is not resumable; the user could not pick the relay back up", reason)
	}
	data, err := os.ReadFile(filepath.Join(run.boardDir, "board.md"))
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "cannot reattach") {
		t.Fatalf("the board does not say why the relay stopped:\n%s", data)
	}
}
