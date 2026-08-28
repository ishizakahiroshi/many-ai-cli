package hub

import (
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/proto"
)

// relayStoreFixture starts a relay that really writes relay.json, gives the
// parent a start time (the conductor is re-identified by it after a restart)
// and advances the relay to `reviewing` (C1 done, reviewer spawned).
func relayStoreFixture(t *testing.T) (*relayHarness, proto.RelayStatus) {
	t.Helper()
	h := newRelayHarness(t)
	h.s.relay.save = nil // production save: relay.json under HOME
	h.parent.StartedAt = "2026-08-27T15:00:00+09:00"
	st := h.start()
	h.progress(st.OrchestrationID, st.ImplementationSessionID, doneLines(relayRoleImplementation, st.ImplementationSessionID, 1, false))
	return h, h.status(st.OrchestrationID)
}

func TestRelayStore_roundTrip(t *testing.T) {
	h, st := relayStoreFixture(t)
	run := h.run(st.OrchestrationID)
	data, err := os.ReadFile(relayFilePath(run.boardDir))
	if err != nil {
		t.Fatalf("relay.json not written: %v", err)
	}
	if !strings.Contains(string(data), `"state": "reviewing"`) || !strings.Contains(string(data), `"implementation_label"`) {
		t.Fatalf("relay.json content unexpected:\n%s", data)
	}
	files, err := h.s.loadRelayFiles()
	if err != nil || len(files) != 1 {
		t.Fatalf("loadRelayFiles = %d files, err=%v", len(files), err)
	}
	run.mu.Lock()
	want := relayFileFromRun(run)
	wantStatus := run.statusLocked()
	run.mu.Unlock()
	got := relayRunFromFile(files[0])
	got.mu.Lock()
	gotFile := relayFileFromRun(got)
	gotStatus := got.statusLocked()
	got.mu.Unlock()
	if gotStatus != wantStatus {
		t.Fatalf("status after round trip = %+v\nwant %+v", gotStatus, wantStatus)
	}
	if !reflect.DeepEqual(gotFile, want) {
		t.Fatalf("file after round trip = %+v\nwant %+v", gotFile, want)
	}
	if len(got.events) != len(run.events) || got.implLabel == "" || got.reviewLabel == "" || got.parentStartedAt != "2026-08-27T15:00:00+09:00" {
		t.Fatalf("round trip lost fields: events=%d labels=%q/%q parentStartedAt=%q", len(got.events), got.implLabel, got.reviewLabel, got.parentStartedAt)
	}

	// A corrupt relay.json and a conductor board without one are skipped.
	orchDir := filepath.Dir(run.boardDir)
	bad := filepath.Join(orchDir, "bad")
	if err := os.MkdirAll(bad, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(bad, relayFileName), []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	plain := filepath.Join(orchDir, "s1")
	if err := os.MkdirAll(plain, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(plain, "board.md"), []byte("# board\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	if files, err := h.s.loadRelayFiles(); err != nil || len(files) != 1 {
		t.Fatalf("loadRelayFiles with corrupt/plain dirs = %d files, err=%v", len(files), err)
	}
}

func TestRelayStore_restoreOnlyResumable(t *testing.T) {
	h, st := relayStoreFixture(t)
	run := h.run(st.OrchestrationID)
	home := os.Getenv("HOME")
	run.mu.Lock()
	base := relayFileFromRun(run)
	run.mu.Unlock()
	orchDir := filepath.Dir(run.boardDir)
	write := func(id, state, reason string) {
		f := base
		f.OrchestrationID = id
		f.BoardPath = filepath.Join(orchDir, id, "board.md")
		f.State, f.Reason = state, reason
		if err := writeRelayFile(relayFilePath(filepath.Dir(f.BoardPath)), f); err != nil {
			t.Fatal(err)
		}
	}
	write("r1-completed", relayStateCompleted, "")
	write("r1-userstop", relayStateStopped, relayReasonUserStop)
	write("r1-hubrestart", relayStateStopped, relayReasonHubRestart)

	h2 := newRelayHarnessHome(t, home)
	h2.s.restoreRelays()

	live := h2.s.relayByID(st.OrchestrationID)
	if live == nil {
		t.Fatal("non-terminal relay not restored")
	}
	live.mu.Lock()
	awaiting, attached, state, last := live.awaitingReconnect, live.parentAttached, live.state, live.events[len(live.events)-1]
	live.mu.Unlock()
	if !awaiting || attached || state != relayStateReviewing || last.Kind != relayEventRestored {
		t.Fatalf("restored relay: awaiting=%v attached=%v state=%q last=%+v", awaiting, attached, state, last)
	}
	resumable := h2.s.relayByID("r1-hubrestart")
	if resumable == nil {
		t.Fatal("stopped(hub_restart) relay not restored")
	}
	resumable.mu.Lock()
	if resumable.awaitingReconnect || resumable.parentAttached {
		t.Fatalf("resumable relay must be idle and unattached: %+v", resumable.statusLocked())
	}
	resumable.mu.Unlock()
	if h2.s.relayByID("r1-completed") != nil || h2.s.relayByID("r1-userstop") != nil {
		t.Fatal("completed / user-stopped relays were restored")
	}
	// Unattached relays are invisible to the new session #1 until the
	// conductor re-identifies itself or a session adopts them.
	if got := h2.s.relayStatusesFor(1); len(got) != 0 {
		t.Fatalf("restored relays leaked to session 1: %+v", got)
	}
	h2.s.orchestration.mu.Lock()
	b := h2.s.orchestration.boards[st.OrchestrationID]
	var children []int
	if b != nil {
		for id := range b.Children {
			children = append(children, id)
		}
	}
	h2.s.orchestration.mu.Unlock()
	if len(children) != 2 {
		t.Fatalf("board children after restore = %v, want impl and review", children)
	}
	// Restoring twice does not duplicate.
	h2.s.restoreRelays()
	h2.s.orchestration.mu.Lock()
	n := len(h2.s.orchestration.relays)
	h2.s.orchestration.mu.Unlock()
	if n != 2 {
		t.Fatalf("relays after second restore = %d, want 2", n)
	}
}

func TestRelayStore_reconnectAndContinue(t *testing.T) {
	h, st := relayStoreFixture(t)
	id, impl, review := st.OrchestrationID, st.ImplementationSessionID, st.ReviewSessionID
	f1 := h.writeReview(id, 1, 1, false, "must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1))
	if got := h.status(id); got.State != relayStateFixing {
		t.Fatalf("fixture state = %q, want fixing", got.State)
	}
	run := h.run(id)
	run.mu.Lock()
	implLabel, reviewLabel := run.implLabel, run.reviewLabel
	run.mu.Unlock()
	home := os.Getenv("HOME")

	h2 := newRelayHarnessHome(t, home)
	h2.s.restoreRelays()
	run2 := h2.run(id)

	// The implementer finished its fix while the Hub was down.
	fixed := doneLines(relayRoleImplementation, impl, 2, false)
	if err := os.WriteFile(childProgressPath(run2.boardPath, impl), []byte(fixed), 0o600); err != nil {
		t.Fatal(err)
	}
	h2.s.relayOnChildFileChange(id, impl, fixed)
	if got := h2.s.relayByID(id); got != nil {
		got.mu.Lock()
		state := got.state
		got.mu.Unlock()
		if state != relayStateFixing || len(h2.injects) != 0 {
			t.Fatalf("file change acted on while awaiting reconnect: state=%q injects=%d", state, len(h2.injects))
		}
	}

	// The implementer reattaches renumbered (#21); it keeps writing child-10.md.
	m := h2.s.relayReattachMatch(proto.Message{Label: implLabel})
	if m == nil || m.role != relayRoleImplementation || m.orchestrationID != id {
		t.Fatalf("implementer match = %+v", m)
	}
	ses := registerTestSession(h2.s, 21, "codex")
	h2.s.sessionsMu.Lock()
	m.applyLocked(ses)
	h2.s.sessionsMu.Unlock()
	if ses.Role != relayRoleImplementation || ses.ParentSessionID != 1 || ses.OrchestrationID != id || ses.BoardPath != run2.boardPath || !ses.Auto {
		t.Fatalf("session fields after apply = %+v", ses)
	}
	h2.s.relayNoteReattached(m, 21)
	run2.mu.Lock()
	implID, progressID := run2.implID, run2.progressIDLocked(relayRoleImplementation)
	run2.mu.Unlock()
	if implID != 21 || progressID != impl {
		t.Fatalf("after reattach implID=%d progressID=%d, want 21 / %d", implID, progressID, impl)
	}
	h2.s.orchestration.mu.Lock()
	b := h2.s.orchestration.boards[id]
	_, oldPresent := b.Children[impl]
	newChild := b.Children[21]
	h2.s.orchestration.mu.Unlock()
	if oldPresent || newChild == nil || newChild.FilePath != childProgressPath(run2.boardPath, impl) {
		t.Fatalf("board child not re-keyed: old=%v new=%+v", oldPresent, newChild)
	}
	if again := h2.s.relayReattachMatch(proto.Message{Label: implLabel}); again != nil {
		t.Fatal("the same label matched twice")
	}

	// Still waiting for the reviewer.
	run2.mu.Lock()
	restoredAt := run2.restoredAt
	run2.mu.Unlock()
	h2.s.checkRelayReconnect(restoredAt.Add(time.Second))
	run2.mu.Lock()
	awaiting := run2.awaitingReconnect
	run2.mu.Unlock()
	if !awaiting {
		t.Fatal("relay resumed before every child reattached")
	}

	// The conductor reattaches with its old ID and is matched by start time.
	pm := h2.s.relayReattachMatch(proto.Message{SessionID: 1, StartedAt: "2026-08-27T15:00:00+09:00", CWD: h.parent.CWD, Provider: "claude"})
	if pm == nil || pm.role != "" {
		t.Fatalf("conductor match = %+v", pm)
	}
	h2.s.relayNoteReattached(pm, 1)
	if got := h2.s.relayStatusesFor(1); len(got) != 1 || got[0].OrchestrationID != id {
		t.Fatalf("relay not visible to the reattached conductor: %+v", got)
	}
	if h2.s.relayReattachMatch(proto.Message{SessionID: 1, StartedAt: "2026-08-27T15:00:00+09:00", CWD: h.parent.CWD, Provider: "claude"}) != nil {
		t.Fatal("conductor matched twice")
	}

	// The reviewer reattaches with the same ID; the relay continues and the
	// fix DONE written during the outage is picked up: fixing → reviewing.
	rm := h2.s.relayReattachMatch(proto.Message{Label: reviewLabel})
	if rm == nil || rm.role != relayRoleReview {
		t.Fatalf("reviewer match = %+v", rm)
	}
	registerTestSession(h2.s, review, "claude")
	h2.s.relayNoteReattached(rm, review)
	h2.s.checkRelayReconnect(restoredAt.Add(time.Second))
	got := h2.status(id)
	if got.State != relayStateReviewing || got.Round != 2 || got.ImplementationSessionID != 21 || got.ReviewSessionID != review {
		t.Fatalf("after reconnect: %+v", got)
	}
	if in := h2.lastInject(); in.id != review || !strings.Contains(in.text, "re-review after the fixes for "+f1) {
		t.Fatalf("re-review after reconnect = %+v", in)
	}
	kinds := h2.eventKinds(id)
	if strings.Join(kinds[len(kinds)-4:], ",") != strings.Join([]string{relayEventRestored, relayEventResumed, relayEventCDone, relayEventReviewStarted}, ",") {
		t.Fatalf("events after reconnect = %v", kinds)
	}
}

func TestRelayStore_reconnectGraceExpires(t *testing.T) {
	h, st := relayStoreFixture(t)
	id := st.OrchestrationID
	home := os.Getenv("HOME")
	h2 := newRelayHarnessHome(t, home)
	h2.s.relay.save = nil // the stop below must reach relay.json
	h2.s.restoreRelays()
	run2 := h2.run(id)
	run2.mu.Lock()
	restoredAt := run2.restoredAt
	run2.mu.Unlock()

	h2.s.checkRelayReconnect(restoredAt.Add(relayReconnectGrace - time.Second))
	run2.mu.Lock()
	state := run2.state
	run2.mu.Unlock()
	if state != relayStateReviewing {
		t.Fatalf("relay stopped before the grace elapsed: %q", state)
	}
	h2.s.checkRelayReconnect(restoredAt.Add(relayReconnectGrace + time.Second))
	run2.mu.Lock()
	state, reason := run2.state, run2.reason
	run2.mu.Unlock()
	if state != relayStateStopped || reason != relayReasonHubRestart {
		t.Fatalf("after grace: state=%q reason=%q, want stopped(hub_restart)", state, reason)
	}
	if len(h2.notifies) != 0 {
		t.Fatalf("unattached relay notified someone: %q", h2.notifies)
	}
	files, err := h2.s.loadRelayFiles()
	if err != nil || len(files) != 1 || files[0].State != relayStateStopped || files[0].Reason != relayReasonHubRestart {
		t.Fatalf("relay.json after stop = %+v err=%v", files, err)
	}
	_ = h
}

func TestRelayStore_resumeRelay(t *testing.T) {
	h, st := relayStoreFixture(t)
	id, review := st.OrchestrationID, st.ReviewSessionID
	f1 := h.writeReview(id, 1, 1, false, "must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1))
	home := os.Getenv("HOME")

	h2 := newRelayHarnessHome(t, home)
	h2.s.restoreRelays()
	if err := h2.s.resumeRelay(1, id); !errors.Is(err, errRelayNotResumable) {
		t.Fatalf("resume while awaiting reconnect = %v, want errRelayNotResumable", err)
	}
	run2 := h2.run(id)
	run2.mu.Lock()
	restoredAt := run2.restoredAt
	run2.mu.Unlock()
	h2.s.checkRelayReconnect(restoredAt.Add(relayReconnectGrace + time.Second))
	if err := h2.s.resumeRelay(1, "nope"); !errors.Is(err, errRelayNotFound) {
		t.Fatalf("resume unknown id = %v, want errRelayNotFound", err)
	}

	// Session #1 (new after the restart) adopts and resumes the relay.
	if err := h2.s.resumeRelay(1, id); err != nil {
		t.Fatalf("resumeRelay: %v", err)
	}
	got := h2.status(id)
	if got.State != relayStateImplementing || got.ImplementationSessionID != 10 || got.ReviewSessionID != 0 || got.ActiveImplementer != relayRoleImplementation || got.Round != 0 || got.CompletedCs != 0 {
		t.Fatalf("after resume: %+v", got)
	}
	if len(h2.spawns) != 1 || h2.spawns[0].Role != relayRoleImplementation {
		t.Fatalf("resume spawns = %+v", h2.spawns)
	}
	p := h2.spawns[0].InitialPrompt
	for _, needle := range []string{"[relay " + relayShortID(id) + " plan_x.md]", "Relay resume", "git log --oneline base000..HEAD", "first fix the must items in " + f1, "trust the working tree over your own memory"} {
		if !strings.Contains(p, needle) {
			t.Fatalf("resume prompt missing %q: %q", needle, p)
		}
	}
	if h2.parent.OrchestrationID != id || len(h2.parent.Relays) != 1 {
		t.Fatalf("adopting parent not marked conductor: orch=%q relays=%d", h2.parent.OrchestrationID, len(h2.parent.Relays))
	}
	kinds := h2.eventKinds(id)
	if kinds[len(kinds)-1] != relayEventResumed {
		t.Fatalf("last event = %v, want resumed", kinds)
	}
	if !strings.Contains(h2.board(id), "relay adopted by session=1") {
		t.Fatalf("board lacks the adoption record:\n%s", h2.board(id))
	}
	// A running relay cannot be resumed again; a missing worktree blocks a resume.
	if err := h2.s.resumeRelay(1, id); !errors.Is(err, errRelayNotResumable) {
		t.Fatalf("resume while running = %v, want errRelayNotResumable", err)
	}
	run2.mu.Lock()
	run2.state, run2.reason = relayStateStopped, relayReasonChildExited
	run2.mode, run2.worktreePath = relayModeWorktree, filepath.Join(t.TempDir(), "gone")
	run2.mu.Unlock()
	if err := h2.s.resumeRelay(1, id); !errors.Is(err, errRelayWorktreeMissing) {
		t.Fatalf("resume with missing worktree = %v, want errRelayWorktreeMissing", err)
	}
}
