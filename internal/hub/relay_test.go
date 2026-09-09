package hub

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

// relayHarness runs the relay state machine with every side effect replaced:
// spawn registers an in-memory child, inject / notify are recorded, git and
// the clock return fixed values. HOME is redirected so board files land in a
// temporary directory.
type relayHarness struct {
	t         *testing.T
	s         *Server
	parent    *session
	strong    bool // include the optional implementation-strong role
	nextChild int
	clock     time.Time
	injects   []relayInjectCall
	spawns    []spawnChildRequest
	preps     []childSpawnPreparation
	spawnErr  error
	saves     int
	notifies  []string
	head      string
	changed   int
}

type relayInjectCall struct {
	id   int
	text string
}

func newRelayHarness(t *testing.T) *relayHarness {
	t.Helper()
	return newRelayHarnessHome(t, t.TempDir())
}

// resolvedTempDir は t.TempDir() を EvalSymlinks 済みの形で返す。
// 比較対象になるパスは必ずこれを通す（newRelayHarnessHome のコメント参照）。
func resolvedTempDir(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	resolved, err := filepath.EvalSymlinks(dir)
	if err != nil {
		t.Fatalf("EvalSymlinks(%q): %v", dir, err)
	}
	return filepath.Clean(resolved)
}

// newRelayHarnessHome builds a harness on an explicit HOME so a second
// harness can play "the Hub after a restart" over the same relay.json files.
func newRelayHarnessHome(t *testing.T, home string) *relayHarness {
	t.Helper()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	s := newTestServer()
	s.cfg.Orchestration.MaxDepth = 1
	s.cfg.Orchestration.MaxChildrenPerParent = 4
	s.cfg.Orchestration.MaxTotalSessions = 16
	parent := registerTestSession(s, 1, "claude")
	// t.TempDir() の生値を使わない。resolveRelayPlanPath は EvalSymlinks を通した
	// パスを返すので、生値を期待値にすると環境によって食い違う。macOS は /var が
	// /private/var への symlink、GitHub Actions の Windows は TEMP が 8.3 短縮名
	// （RUNNER~1 → runneradmin）で、どちらも CI でだけ落ちる。開発機のユーザー
	// ディレクトリは短縮も symlink もされないので手元では緑になり、差が見えない。
	parent.CWD = resolvedTempDir(t)
	h := &relayHarness{t: t, s: s, parent: parent, nextChild: 9, clock: time.Date(2026, 8, 27, 15, 0, 0, 0, time.UTC), head: "base000"}
	s.relay = relayDeps{
		spawnChild: h.fakeSpawn,
		inject: func(id int, text string) {
			h.injects = append(h.injects, relayInjectCall{id: id, text: text})
		},
		notifyParent: func(boardID string, parentID int, text string) {
			h.notifies = append(h.notifies, text)
		},
		gitHead:         func(dir string) (string, error) { return h.head, nil },
		gitChangedFiles: func(dir, from string) (int, error) { return h.changed, nil },
		now: func() time.Time {
			h.clock = h.clock.Add(time.Second)
			return h.clock
		},
		save: func(run *relayRun) error {
			h.saves++
			return nil
		},
	}
	return h
}

func TestRelaySpawnPassesSubscriptionProfile(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	run := h.run(st.OrchestrationID)
	assignment := run.roles[relayRoleImplementation]
	assignment.Subscription = "work"
	run.roles[relayRoleImplementation] = assignment
	if err := h.s.relaySpawn(run, relayRoleImplementation, "prompt"); err != nil {
		t.Fatalf("relaySpawn: %v", err)
	}
	if len(h.spawns) < 2 || h.spawns[len(h.spawns)-1].SubscriptionProfileID != "work" {
		t.Fatalf("spawn subscription = %+v", h.spawns)
	}
}

func (h *relayHarness) fakeSpawn(parentID int, parent *session, body spawnChildRequest, prep childSpawnPreparation) (childSpawnResult, error) {
	if h.spawnErr != nil {
		return childSpawnResult{}, h.spawnErr
	}
	h.spawns = append(h.spawns, body)
	h.preps = append(h.preps, prep)
	h.nextChild++
	id := h.nextChild
	child := registerTestSession(h.s, id, body.Provider)
	child.ParentSessionID = parentID
	child.Role = body.Role
	child.OrchestrationID = prep.orchestrationID
	child.BoardPath = prep.boardPath
	child.CWD = prep.childCWD
	child.Label = fmt.Sprintf("orch-%s-%s-%d", safeToken(prep.orchestrationID), body.Role, id)
	h.s.sessionsMu.Lock()
	h.s.wrappers[id] = newWrapperConn(&websocket.Conn{})
	h.s.sessionsMu.Unlock()
	h.s.registerBoardSession(prep.orchestrationID, prep.boardPath, parentID, "conductor")
	h.s.registerBoardChild(prep.orchestrationID, prep.boardPath, id, parentID, body.Role, time.Now())
	return childSpawnResult{ID: id, BoardPath: prep.boardPath, CWD: prep.childCWD}, nil
}

func relayTestRoles(strong bool) map[string]orchestrationRoleAssignment {
	roles := map[string]orchestrationRoleAssignment{
		relayRoleImplementation: {Provider: "codex", Model: "gpt-5-mini"},
		relayRoleReview:         {Provider: "claude", Model: "opus"},
	}
	if strong {
		roles[relayRoleImplementationStrong] = orchestrationRoleAssignment{Provider: "claude", Model: "opus-strong"}
	}
	return roles
}

func (h *relayHarness) request() relayStartRequest {
	return relayStartRequest{
		PlanPath: filepath.Join(h.parent.CWD, "docs", "plan_x.md"),
		Mode:     relayModeSameTree,
		Roles:    relayTestRoles(h.strong),
	}
}

func (h *relayHarness) start() proto.RelayStatus {
	h.t.Helper()
	st, err := h.s.startRelay(h.parent.ID, h.request())
	if err != nil {
		h.t.Fatalf("startRelay: %v", err)
	}
	return *st
}

func (h *relayHarness) run(id string) *relayRun {
	h.t.Helper()
	run := h.s.relayByID(id)
	if run == nil {
		h.t.Fatalf("relay %q not found", id)
	}
	return run
}

func (h *relayHarness) status(id string) proto.RelayStatus {
	h.t.Helper()
	for _, st := range h.s.relayStatusesFor(h.parent.ID) {
		if st.OrchestrationID == id {
			return st
		}
	}
	h.t.Fatalf("relay %q missing from relayStatusesFor", id)
	return proto.RelayStatus{}
}

// progress writes the child's progress file and feeds it to the relay, the
// same way the board scan does after C2 wires the branch.
func (h *relayHarness) progress(id string, childID int, text string) {
	h.t.Helper()
	run := h.run(id)
	if err := os.WriteFile(childProgressPath(run.boardPath, childID), []byte(text), 0o600); err != nil {
		h.t.Fatal(err)
	}
	h.s.relayOnChildFileChange(id, childID, text)
}

func (h *relayHarness) writeReview(id string, c, round int, strong bool, text string) string {
	h.t.Helper()
	path := relayReviewFile(h.run(id).boardDir, c, round, strong)
	if err := os.WriteFile(path, []byte(text), 0o600); err != nil {
		h.t.Fatal(err)
	}
	return path
}

func (h *relayHarness) lastInject() relayInjectCall {
	h.t.Helper()
	if len(h.injects) == 0 {
		h.t.Fatal("no injection recorded")
	}
	return h.injects[len(h.injects)-1]
}

func (h *relayHarness) eventKinds(id string) []string {
	kinds := []string{}
	for _, ev := range h.s.relayEventsFor(h.parent.ID, id) {
		kinds = append(kinds, ev.Kind)
	}
	return kinds
}

func (h *relayHarness) board(id string) string {
	h.t.Helper()
	data, err := os.ReadFile(h.run(id).boardPath)
	if err != nil {
		h.t.Fatal(err)
	}
	return string(data)
}

func doneLines(role string, sessionID, n int, final bool) string {
	var b strings.Builder
	fmt.Fprintf(&b, "## %s session=%d 2026-08-27T15:00:00Z\nstatus: done\n", role, sessionID)
	for i := 0; i < n; i++ {
		fmt.Fprintf(&b, "## DONE %s session=%d", role, sessionID)
		if final && i == n-1 {
			b.WriteString(" final=true")
		}
		b.WriteString("\nsummary\n")
	}
	return b.String()
}

func reviewProgress(sessionID int, verdicts ...string) string {
	var b strings.Builder
	fmt.Fprintf(&b, "## review session=%d 2026-08-27T15:00:00Z\n", sessionID)
	for _, v := range verdicts {
		fmt.Fprintf(&b, "verdict: %s\n## DONE review session=%d\n", v, sessionID)
	}
	return b.String()
}

func TestRelay_countDoneLines(t *testing.T) {
	text := strings.Join([]string{
		"## implementation session=10 2026-08-27T15:00:00Z",
		"status: running",
		"## DONE implementation session=10",
		"## DONE review session=11",
		"## SUCCESS implementation",
		"## DONE implementation session=12",
		"## DONE implementation session=10 final=true",
		"",
	}, "\n")
	if n, final, esc := countDoneLines(text, "implementation", 10); n != 3 || !final || esc {
		t.Fatalf("implementation = (%d, %v, %v), want (3, true, false)", n, final, esc)
	}
	if n, final, _ := countDoneLines(text, "review", 11); n != 1 || final {
		t.Fatalf("review = (%d, %v), want (1, false)", n, final)
	}
	if n, _, _ := countDoneLines("## DONE implementation\n## DONE implementation\n", "implementation", 10); n != 2 {
		t.Fatalf("duplicate lines = %d, want 2 (no de-duplication)", n)
	}
	if n, final, _ := countDoneLines("## DONE implementation final=true\n## DONE implementation\n", "implementation", 10); n != 2 || final {
		t.Fatalf("final on an earlier line = (%d, %v), want (2, false)", n, final)
	}
	if n, final, esc := countDoneLines("## DONE implementation session=10 escalate=true final=true\n", "implementation", 10); n != 1 || !final || !esc {
		t.Fatalf("escalate line = (%d, %v, %v), want (1, true, true)", n, final, esc)
	}
	if n, _, _ := countDoneLines("## DONE implementation-strong session=12\n", "implementation-strong", 12); n != 1 {
		t.Fatalf("strong role = %d, want 1", n)
	}
	if n, _, _ := countDoneLines(text, "tester", 10); n != 0 {
		t.Fatalf("unrelated role = %d, want 0", n)
	}
}

func TestRelay_parseRelayVerdict(t *testing.T) {
	boardDir := t.TempDir()
	existing := filepath.Join(boardDir, "review-c1-r1.md")
	if err := os.WriteFile(existing, []byte("1. must\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	defaultC2 := relayReviewFile(boardDir, 2, 1, false)

	tests := []struct {
		name      string
		text      string
		doneCount int
		c, round  int
		want      relayVerdict
		wantErr   error
	}{
		{name: "pass", text: "verdict: pass\n## DONE review\n", doneCount: 1, c: 1, round: 1, want: relayVerdict{Kind: "pass", File: existing}},
		{name: "findings with file", text: "verdict: findings must=2 should=1 file=" + existing + "\n## DONE review\n", doneCount: 1, c: 1, round: 1, want: relayVerdict{Kind: "findings", Must: 2, Should: 1, File: existing}},
		{name: "should only is must=0", text: "verdict: findings should=1\n", doneCount: 1, c: 1, round: 1, want: relayVerdict{Kind: "findings", Must: 0, Should: 1, File: existing}},
		{name: "bare findings is must>0", text: "verdict: findings\n", doneCount: 1, c: 1, round: 1, want: relayVerdict{Kind: "findings", Must: 1, File: existing}},
		{name: "blocked reason to end of line", text: "verdict: blocked reason=needs decision on API shape\n", doneCount: 1, c: 1, round: 1, want: relayVerdict{Kind: "blocked", Reason: "needs decision on API shape"}},
		{name: "fewer verdicts than DONE", text: "## DONE review\n## DONE review\nverdict: pass\n", doneCount: 2, c: 1, round: 1, wantErr: errVerdictMissing},
		{name: "no verdict", text: "## DONE review\n", doneCount: 1, c: 1, round: 1, wantErr: errVerdictMissing},
		{name: "file outside board dir", text: "verdict: findings must=1 file=" + filepath.Join(t.TempDir(), "x.md") + "\n", doneCount: 1, c: 1, round: 1, wantErr: errVerdictMissing},
		{name: "must>0 without file", text: "verdict: findings must=1\n", doneCount: 1, c: 2, round: 1, wantErr: errReviewFileMissing},
		{name: "last line wins", text: "verdict: findings must=1 file=" + existing + "\n## DONE review\nverdict: pass\n## DONE review\n", doneCount: 2, c: 1, round: 1, want: relayVerdict{Kind: "pass", File: existing}},
		{name: "relative file resolves against board dir", text: "verdict: findings must=1 file=review-c1-r1.md\n", doneCount: 1, c: 2, round: 1, want: relayVerdict{Kind: "findings", Must: 1, File: existing}},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got, err := parseRelayVerdict(tc.text, tc.doneCount, boardDir, relayReviewFile(boardDir, tc.c, tc.round, false))
			if tc.wantErr != nil {
				if !errors.Is(err, tc.wantErr) {
					t.Fatalf("err = %v, want %v", err, tc.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("err = %v", err)
			}
			if got != tc.want {
				t.Fatalf("verdict = %+v, want %+v", got, tc.want)
			}
		})
	}
	if v, err := parseRelayVerdict("verdict: findings must=1\n", 1, boardDir, defaultC2); !errors.Is(err, errReviewFileMissing) || v.File != defaultC2 {
		t.Fatalf("default file = %q err=%v, want %q / review_file_missing", v.File, err, defaultC2)
	}
}

func TestRelay_shortID(t *testing.T) {
	for in, want := range map[string]string{
		"r1-1756275600123456789": "r1-456789",
		"r12-987654":             "r12-987654",
		"r1-123":                 "r1-123",
		"plain":                  "plain",
	} {
		if got := relayShortID(in); got != want {
			t.Fatalf("relayShortID(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestRelay_startRelay_rolesMissing(t *testing.T) {
	h := newRelayHarness(t)
	req := h.request()
	delete(req.Roles, relayRoleReview)
	_, err := h.s.startRelay(h.parent.ID, req)
	var missing errRelayRolesMissing
	if !errors.As(err, &missing) || missing.Role != relayRoleReview {
		t.Fatalf("err = %v, want errRelayRolesMissing{review}", err)
	}
	req = h.request()
	req.Roles[relayRoleImplementation] = orchestrationRoleAssignment{Provider: "shell", Model: "x"}
	if _, err := h.s.startRelay(h.parent.ID, req); !errors.As(err, &missing) || missing.Role != relayRoleImplementation {
		t.Fatalf("invalid provider err = %v, want errRelayRolesMissing{implementation}", err)
	}
	// The strong role is optional, but when given it must be valid.
	req = h.request()
	req.Roles[relayRoleImplementationStrong] = orchestrationRoleAssignment{Provider: "nope", Model: "x"}
	if _, err := h.s.startRelay(h.parent.ID, req); !errors.As(err, &missing) || missing.Role != relayRoleImplementationStrong {
		t.Fatalf("invalid strong err = %v, want errRelayRolesMissing{implementation-strong}", err)
	}
	req = h.request()
	req.PlanPath = "docs/plan.md"
	if _, err := h.s.startRelay(h.parent.ID, req); !errors.Is(err, errRelayPlanPath) {
		t.Fatalf("relative plan err = %v, want errRelayPlanPath", err)
	}
	if _, err := h.s.startRelay(99, h.request()); !errors.Is(err, errRelayParentNotFound) {
		t.Fatalf("unknown parent err = %v, want errRelayParentNotFound", err)
	}
	if len(h.spawns) != 0 {
		t.Fatalf("spawned %d children on rejected requests", len(h.spawns))
	}
}

func TestRelay_startRelay_childBudget(t *testing.T) {
	h := newRelayHarness(t)
	h.s.cfg.Orchestration.MaxChildrenPerParent = 3
	first := h.start()
	if first.State != relayStateImplementing {
		t.Fatalf("first relay state = %q", first.State)
	}
	// One child is live and the first relay still reserves its reviewer:
	// 1 + 1 + 2 > 3.
	_, err := h.s.startRelay(h.parent.ID, h.request())
	var limit errOrchestrationLimit
	if !errors.As(err, &limit) || limit.Limit != "children_per_parent" || limit.Running != 1 || limit.Max != 3 {
		t.Fatalf("err = %v, want children_per_parent limit running=1 max=3", err)
	}
	h.s.cfg.Orchestration.MaxDepth = 0
	if _, err := h.s.startRelay(h.parent.ID, h.request()); !errors.As(err, &limit) || limit.Limit != "depth" {
		t.Fatalf("depth err = %v", err)
	}
}

func TestOrchestrationChildAdmissionConcurrentLimit(t *testing.T) {
	s := newTestServer()
	parent := registerTestSession(s, 1, "codex")
	cfg := s.cfg.Orchestration
	const attempts = 12
	start := make(chan struct{})
	ids := make(chan string, attempts)
	errs := make(chan error, attempts)
	var wg sync.WaitGroup
	for i := 0; i < attempts; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			<-start
			id, err := s.reserveOrchestrationChildren(parent.ID, 1, cfg, "test")
			if err == nil {
				ids <- id
			} else {
				errs <- err
			}
		}()
	}
	close(start)
	wg.Wait()
	close(ids)
	close(errs)

	admissionIDs := make([]string, 0, cfg.MaxChildrenPerParent)
	for id := range ids {
		admissionIDs = append(admissionIDs, id)
	}
	accepted := len(admissionIDs)
	if accepted != cfg.MaxChildrenPerParent {
		t.Fatalf("concurrent admissions = %d, want %d", accepted, cfg.MaxChildrenPerParent)
	}
	for err := range errs {
		var limit errOrchestrationLimit
		if !errors.As(err, &limit) || limit.Limit != "children_per_parent" {
			t.Fatalf("unexpected rejected admission: %v", err)
		}
	}
	s.orchestration.mu.Lock()
	if got := len(s.orchestration.childAdmissions); got != accepted {
		t.Fatalf("admission ledger entries = %d, want %d", got, accepted)
	}
	s.orchestration.mu.Unlock()
	for _, id := range admissionIDs {
		s.releaseOrchestrationChildren(id)
	}
}

func TestRelayAdmissionReleasedAfterSpawnFailure(t *testing.T) {
	h := newRelayHarness(t)
	h.s.cfg.Orchestration.MaxChildrenPerParent = relayChildrenPerRun
	h.spawnErr = errors.New("synthetic relay spawn failure")
	if _, err := h.s.startRelay(h.parent.ID, h.request()); err == nil {
		t.Fatal("failed relay start unexpectedly succeeded")
	}
	h.spawnErr = nil
	status, err := h.s.startRelay(h.parent.ID, h.request())
	if err != nil {
		t.Fatalf("relay start after failed admission cleanup: %v", err)
	}
	if status.State != relayStateImplementing {
		t.Fatalf("second relay state = %q, want %q", status.State, relayStateImplementing)
	}
	if len(h.spawns) != 1 {
		t.Fatalf("successful child spawns = %d, want 1", len(h.spawns))
	}
}

func TestRelayAdmissionSerializesConcurrentStarts(t *testing.T) {
	h := newRelayHarness(t)
	h.s.cfg.Orchestration.MaxChildrenPerParent = 3
	start := make(chan struct{})
	results := make(chan error, 2)
	var wg sync.WaitGroup
	for i := 0; i < 2; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			<-start
			_, err := h.s.startRelay(h.parent.ID, h.request())
			results <- err
		}()
	}
	close(start)
	wg.Wait()
	close(results)

	accepted := 0
	for err := range results {
		if err == nil {
			accepted++
			continue
		}
		var limit errOrchestrationLimit
		if !errors.As(err, &limit) || limit.Limit != "children_per_parent" {
			t.Fatalf("concurrent relay start error = %v, want child limit", err)
		}
	}
	if accepted != 1 || len(h.spawns) != 1 {
		t.Fatalf("concurrent relay starts accepted=%d child_spawns=%d, want one each", accepted, len(h.spawns))
	}
}

func TestRelayAdmissionCountsGlobalAcrossParents(t *testing.T) {
	h := newRelayHarness(t)
	h.s.cfg.Orchestration.MaxChildrenPerParent = 4
	h.s.cfg.Orchestration.MaxTotalSessions = 4 // two parents plus one relay's two slots
	other := registerTestSession(h.s, 2, "claude")
	other.CWD = h.parent.CWD
	if _, err := h.s.startRelay(h.parent.ID, h.request()); err != nil {
		t.Fatalf("first parent relay: %v", err)
	}
	request := h.request()
	request.PlanPath = filepath.Join(other.CWD, "docs", "other-plan.md")
	if _, err := h.s.startRelay(other.ID, request); err == nil {
		t.Fatal("second parent relay exceeded global admission budget")
	} else {
		var limit errOrchestrationLimit
		if !errors.As(err, &limit) || limit.Limit != "total_sessions" {
			t.Fatalf("global admission error = %v, want total_sessions limit", err)
		}
	}
}

func TestRelay_startRelay_preparationAndSecondRelay(t *testing.T) {
	h := newRelayHarness(t)
	first := h.start()
	if len(h.preps) != 1 {
		t.Fatalf("spawns = %d, want 1", len(h.preps))
	}
	prep := h.preps[0]
	if prep.orchestrationID != first.OrchestrationID || prep.childCWD != h.parent.CWD || prep.branch != "" {
		t.Fatalf("prep = %+v, want relay id %q cwd %q and no branch", prep, first.OrchestrationID, h.parent.CWD)
	}
	if !strings.HasPrefix(first.OrchestrationID, "r1-") {
		t.Fatalf("orchestration id = %q, want r<parent>-<time>", first.OrchestrationID)
	}
	if prep.boardPath != filepath.Join(h.run(first.OrchestrationID).boardDir, "board.md") {
		t.Fatalf("board path = %q", prep.boardPath)
	}
	if _, err := os.Stat(prep.boardPath); err != nil {
		t.Fatalf("board not created: %v", err)
	}
	body := h.spawns[0]
	if body.Role != relayRoleImplementation || body.Provider != "codex" || body.Model != "gpt-5-mini" || !body.Force || body.AskForApproval != "never" || !body.RiskConfirmed {
		t.Fatalf("spawn body = %+v, want implementation/codex with approval defaults", body)
	}
	if !strings.Contains(body.InitialPrompt, "Relay task: implement the plan at "+h.request().PlanPath) || !strings.Contains(body.InitialPrompt, "final=true") || !strings.Contains(body.InitialPrompt, "Ignore any [strong] marks") {
		t.Fatalf("implementation prompt = %q", body.InitialPrompt)
	}
	if first.ImplementationSessionID != 10 || first.ReviewSessionID != 0 || first.StrongSessionID != 0 || first.Round != 0 || first.CompletedCs != 0 || first.BaseCommit != "base000" {
		t.Fatalf("status = %+v", first)
	}
	if first.ActiveImplementer != relayRoleImplementation || first.EscalateAfter != relayDefaultEscalateAfter || first.MaxRounds != relayDefaultMaxRounds {
		t.Fatalf("status defaults = %+v", first)
	}
	if h.parent.OrchestrationID != first.OrchestrationID {
		t.Fatalf("parent not marked conductor: %q", h.parent.OrchestrationID)
	}

	second := h.start()
	if second.OrchestrationID == first.OrchestrationID {
		t.Fatalf("second relay reused orchestration id %q", first.OrchestrationID)
	}
	if h.run(second.OrchestrationID).boardDir == h.run(first.OrchestrationID).boardDir {
		t.Fatal("second relay shares the first relay's board directory")
	}
	statuses := h.s.relayStatusesFor(h.parent.ID)
	if len(statuses) != 2 || statuses[0].OrchestrationID != first.OrchestrationID || statuses[1].OrchestrationID != second.OrchestrationID {
		t.Fatalf("relayStatusesFor = %+v, want [first, second]", statuses)
	}
	if len(h.parent.Relays) != 2 || h.parent.Relays[0].OrchestrationID != first.OrchestrationID {
		t.Fatalf("parent.Relays = %+v", h.parent.Relays)
	}
	if !h.s.relayOwns(first.OrchestrationID) || !h.s.relayOwns(second.OrchestrationID) || h.s.relayOwns("s1") {
		t.Fatal("relayOwns does not follow the relays map")
	}
}

// relaySpawn は spawn body に承認系フィールドを一切入れず、承認の既定は
// orchestration.child_full_bypass だけで決まる。だから既定を off へ倒すと、relay の
// 子は承認プロンプトを出したまま止まり、無人で回すという relay の前提が崩れる。
// 既定反転の提案は見送っている（docs/local/reference/reference_declined-directions.md
// の D-12）。本テストは「その設定が relay に何を起こすか」を実物で固定しておくもので、
// 再提案するときはここが何を守っているかを先に読むこと。
func TestRelay_startRelay_childFullBypassOffLeavesApprovalUnset(t *testing.T) {
	h := newRelayHarness(t)
	off := false
	h.s.cfg.Orchestration.ChildFullBypass = &off
	_ = h.start()
	if len(h.spawns) != 1 {
		t.Fatalf("spawns = %d, want 1", len(h.spawns))
	}
	body := h.spawns[0]
	if body.AskForApproval != "" || body.Sandbox != "" || body.PermissionMode != "" || body.RiskConfirmed {
		t.Fatalf("child_full_bypass=false must not fill approval defaults: %+v", body)
	}
}

func TestRelay_startRelay_spawnError(t *testing.T) {
	h := newRelayHarness(t)
	h.spawnErr = errors.New("wrapper did not register")
	st, err := h.s.startRelay(h.parent.ID, h.request())
	if err == nil || !strings.Contains(err.Error(), "wrapper did not register") {
		t.Fatalf("err = %v", err)
	}
	if st == nil || st.State != relayStateStopped || st.Reason != relayReasonSpawnError {
		t.Fatalf("status = %+v, want stopped(spawn_error)", st)
	}
	if len(h.notifies) != 1 || !strings.Contains(h.notifies[0], "relay stopped") {
		t.Fatalf("parent notices = %q, want one stop notice", h.notifies)
	}
	if got := h.s.relayEventsFor(h.parent.ID, ""); len(got) != 0 {
		// resolveRelay with an empty ID only sees relays in progress.
		t.Fatalf("events for empty id = %v, want none (relay is terminal)", got)
	}
	if evs := h.s.relayEventsFor(h.parent.ID, st.OrchestrationID); len(evs) != 2 || evs[0].Kind != relayEventStarted || evs[1].Kind != relayEventStopped {
		t.Fatalf("events = %+v", evs)
	}
}

func TestRelay_transitions_twoCs(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id := st.OrchestrationID
	impl := st.ImplementationSessionID
	run := h.run(id)

	// C1 finished → reviewer spawned, round 1.
	h.head = "c1work"
	h.changed = 4
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	st = h.status(id)
	if st.State != relayStateReviewing || st.Round != 1 || st.ReviewSessionID != 11 || len(h.spawns) != 2 {
		t.Fatalf("after C1 DONE: %+v spawns=%d", st, len(h.spawns))
	}
	review := st.ReviewSessionID
	if p := h.spawns[1].InitialPrompt; !strings.Contains(p, "Relay review (C 1, round 1)") || !strings.Contains(p, relayReviewFile(run.boardDir, 1, 1, false)) || !strings.Contains(p, "`verdict: pass`") {
		t.Fatalf("review prompt = %q", p)
	}
	if st.ReviewPath != relayReviewFile(run.boardDir, 1, 1, false) {
		t.Fatalf("review path = %q", st.ReviewPath)
	}

	// findings must=1 → fix instruction to the implementation child.
	reviewFile := h.writeReview(id, 1, 1, false, "must:\n1. broken\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 should=2 file="+reviewFile))
	st = h.status(id)
	if st.State != relayStateFixing || st.Round != 1 {
		t.Fatalf("after findings: %+v", st)
	}
	if in := h.lastInject(); in.id != impl || !strings.Contains(in.text, "Relay fix (C 1, round 1)") || !strings.Contains(in.text, "1 must-fix and 2 should-fix items in "+reviewFile) {
		t.Fatalf("fix injection = %+v", in)
	}

	// Fix done → re-review, round 2.
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 2, false))
	st = h.status(id)
	if st.State != relayStateReviewing || st.Round != 2 {
		t.Fatalf("after fix DONE: %+v", st)
	}
	if in := h.lastInject(); in.id != review || !strings.Contains(in.text, "Relay review (C 1, round 2): re-review after the fixes for "+reviewFile) {
		t.Fatalf("re-review injection = %+v", in)
	}

	// pass → proceed to C2 (round back to 0), lastReviewedCommit advances and
	// the proceed text tells the implementer to re-sync from git (D-23 c).
	h.head = "c1pass"
	h.progress(id, review, reviewProgress(review, "findings must=1 should=2 file="+reviewFile, "pass"))
	st = h.status(id)
	if st.State != relayStateImplementing || st.CompletedCs != 1 || st.Round != 0 {
		t.Fatalf("after pass: %+v", st)
	}
	if in := h.lastInject(); in.id != impl || !strings.Contains(in.text, "the review of C 1 passed") || !strings.Contains(in.text, "git log --oneline c1pass..HEAD") || !strings.Contains(in.text, "trust the working tree over your own memory") {
		t.Fatalf("proceed injection = %+v", in)
	}
	run.mu.Lock()
	lastReviewed := run.lastReviewedCommit
	run.mu.Unlock()
	if lastReviewed != "c1pass" {
		t.Fatalf("lastReviewedCommit = %q, want c1pass", lastReviewed)
	}

	// C2 (final) done → review with the fresh C instruction, not a re-review.
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 3, true))
	st = h.status(id)
	if st.State != relayStateReviewing || !st.FinalSeen || st.Round != 1 || st.ReviewPath != relayReviewFile(run.boardDir, 2, 1, false) {
		t.Fatalf("after final DONE: %+v", st)
	}
	if in := h.lastInject(); in.id != review || !strings.Contains(in.text, "Relay review (C 2, round 1): adversarially review") {
		t.Fatalf("C2 review injection = %+v", in)
	}

	// pass → completed, parent notified once.
	h.progress(id, review, reviewProgress(review, "findings must=1 should=2 file="+reviewFile, "pass", "pass"))
	st = h.status(id)
	if st.State != relayStateCompleted || st.CompletedCs != 2 || st.Reason != "" {
		t.Fatalf("after final pass: %+v", st)
	}
	if len(h.notifies) != 1 || !strings.Contains(h.notifies[0], "relay completed") || !strings.Contains(h.notifies[0], "c=2") {
		t.Fatalf("parent notices = %q", h.notifies)
	}
	want := []string{
		relayEventStarted, relayEventCDone, relayEventReviewStarted, relayEventVerdict, relayEventFixSent,
		relayEventCDone, relayEventReviewStarted, relayEventVerdict, relayEventProceedSent,
		relayEventCDone, relayEventReviewStarted, relayEventVerdict, relayEventCompleted,
	}
	evs := h.s.relayEventsFor(h.parent.ID, id)
	kinds := make([]string, 0, len(evs))
	for _, ev := range evs {
		kinds = append(kinds, ev.Kind)
	}
	if strings.Join(kinds, ",") != strings.Join(want, ",") {
		t.Fatalf("events = %v\nwant     %v", kinds, want)
	}
	// same-tree mode: no commit to point at, only the working-tree change count.
	if evs[1].Commit != "" || evs[1].FilesChanged != 4 || evs[1].C != 1 {
		t.Fatalf("c_done event = %+v", evs[1])
	}
	if evs[3].Text != "findings must=1 should=2" || evs[3].ReviewPath != reviewFile {
		t.Fatalf("verdict event = %+v", evs[3])
	}
	// The children are never marked done by the relay itself.
	for _, cid := range []int{impl, review} {
		if s := h.s.sessions[cid].State; s != "running" {
			t.Fatalf("child %d state = %q, want running", cid, s)
		}
	}
	board := h.board(id)
	for _, needle := range []string{"relay started", "relay c=1 round=1 state=reviewing", "@implementation session=10 への指示:", "relay completed"} {
		if !strings.Contains(board, needle) {
			t.Fatalf("board missing %q:\n%s", needle, board)
		}
	}
}

func TestRelay_mustZeroCountsAsPass(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	review := h.status(id).ReviewSessionID
	h.progress(id, review, reviewProgress(review, "findings must=0 should=2"))
	st = h.status(id)
	if st.State != relayStateImplementing || st.CompletedCs != 1 {
		t.Fatalf("must=0 did not pass: %+v", st)
	}
	if in := h.lastInject(); in.id != impl || !strings.Contains(in.text, "passed") {
		t.Fatalf("injection = %+v, want proceed", in)
	}
}

func TestRelay_blocked(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	review := h.status(id).ReviewSessionID
	h.progress(id, review, reviewProgress(review, "blocked reason=needs decision on API shape"))
	st = h.status(id)
	if st.State != relayStateStopped || st.Reason != "blocked: needs decision on API shape" {
		t.Fatalf("status = %+v, want stopped(blocked: ...)", st)
	}
	if len(h.notifies) != 1 || !strings.Contains(h.notifies[0], "needs decision on API shape") {
		t.Fatalf("notices = %q", h.notifies)
	}
}

func TestRelay_maxRounds(t *testing.T) {
	h := newRelayHarness(t)
	req := h.request()
	req.MaxRounds = 1
	st, err := h.s.startRelay(h.parent.ID, req)
	if err != nil {
		t.Fatal(err)
	}
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	review := h.status(id).ReviewSessionID
	h.writeReview(id, 1, 1, false, "must:\n1. x\n")
	h.progress(id, review, reviewProgress(review, "findings must=1"))
	if got := h.status(id); got.State != relayStateStopped || got.Reason != relayReasonMaxRounds || got.MaxRounds != 1 {
		t.Fatalf("status = %+v, want stopped(max_rounds)", got)
	}
	// verdict_missing / review_file_missing stop as well.
	h2 := newRelayHarness(t)
	st2 := h2.start()
	h2.progress(st2.OrchestrationID, st2.ImplementationSessionID, doneLines(relayRoleImplementation, st2.ImplementationSessionID, 1, false))
	review2 := h2.status(st2.OrchestrationID).ReviewSessionID
	h2.progress(st2.OrchestrationID, review2, "## review\n## DONE review\n")
	if got := h2.status(st2.OrchestrationID); got.State != relayStateStopped || got.Reason != relayReasonVerdictMissing {
		t.Fatalf("missing verdict status = %+v", got)
	}
	h3 := newRelayHarness(t)
	st3 := h3.start()
	h3.progress(st3.OrchestrationID, st3.ImplementationSessionID, doneLines(relayRoleImplementation, st3.ImplementationSessionID, 1, false))
	review3 := h3.status(st3.OrchestrationID).ReviewSessionID
	h3.progress(st3.OrchestrationID, review3, reviewProgress(review3, "findings must=2"))
	if got := h3.status(st3.OrchestrationID); got.State != relayStateStopped || got.Reason != relayReasonReviewFileMissing {
		t.Fatalf("missing review file status = %+v", got)
	}
}

func TestRelay_doneBaseline(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	// Progress without DONE does not advance.
	h.progress(id, impl, "## implementation session=10\nstatus: running\n")
	if got := h.status(id); got.State != relayStateImplementing {
		t.Fatalf("progress without DONE moved to %q", got.State)
	}
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	review := h.status(id).ReviewSessionID
	reviewFile := h.writeReview(id, 1, 1, false, "1. must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+reviewFile))
	if got := h.status(id); got.State != relayStateFixing {
		t.Fatalf("state = %q, want fixing", got.State)
	}
	injectsBefore := len(h.injects)
	// The first DONE line is still there; more notes without a new DONE keep the relay in fixing.
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false)+"## implementation session=10\nstill fixing\n")
	if got := h.status(id); got.State != relayStateFixing || len(h.injects) != injectsBefore {
		t.Fatalf("unchanged DONE count moved the relay: %+v injects=%d", got, len(h.injects))
	}
	// A reviewer write during fixing is ignored as well.
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+reviewFile, "pass"))
	if got := h.status(id); got.State != relayStateFixing {
		t.Fatalf("reviewer write during fixing moved the relay to %q", got.State)
	}
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 2, false))
	if got := h.status(id); got.State != relayStateReviewing || got.Round != 2 {
		t.Fatalf("second DONE: %+v", got)
	}
}

func TestRelay_nudgeOnce(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	h.s.relayOnChildIdle(id, impl)
	if in := h.lastInject(); in.id != impl || !strings.Contains(in.text, "This reminder is sent only once") || !strings.Contains(in.text, "## DONE implementation") {
		t.Fatalf("nudge = %+v", in)
	}
	n := len(h.injects)
	h.s.relayOnChildIdle(id, impl)
	if len(h.injects) != n {
		t.Fatal("second idle nudged again")
	}
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	review := h.status(id).ReviewSessionID
	n = len(h.injects)
	// The implementation child is now waiting for its next instruction: no nudge.
	h.s.relayOnChildIdle(id, impl)
	if len(h.injects) != n {
		t.Fatal("non-awaited implementation child was nudged")
	}
	h.s.relayOnChildIdle(id, review)
	if in := h.lastInject(); in.id != review || !strings.Contains(in.text, "## DONE review") || !strings.Contains(in.text, "verdict") {
		t.Fatalf("review nudge = %+v", in)
	}
	n = len(h.injects)
	h.s.relayOnChildIdle(id, review)
	if len(h.injects) != n {
		t.Fatal("reviewer nudged twice")
	}
	// A new instruction re-arms the single nudge.
	reviewFile := h.writeReview(id, 1, 1, false, "1. must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+reviewFile))
	n = len(h.injects)
	h.s.relayOnChildIdle(id, impl)
	if len(h.injects) != n+1 || h.lastInject().id != impl {
		t.Fatalf("nudge after fix instruction: injects=%d", len(h.injects))
	}
	if kinds := h.eventKinds(id); strings.Count(strings.Join(kinds, ","), relayEventNudge) != 3 {
		t.Fatalf("nudge events = %v, want 3", kinds)
	}
}

func TestRelay_exitAndTimeout(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	h.s.relayOnChildExit(id, 99, "completed")
	h.s.relayOnChildTimeout(id, 99)
	h.s.relayOnChildExit("other-board", impl, "completed")
	if got := h.status(id); got.State != relayStateImplementing {
		t.Fatalf("foreign child changed the relay: %+v", got)
	}
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	// While reviewing, a timeout of the waiting implementation child is ignored.
	h.s.relayOnChildTimeout(id, impl)
	if got := h.status(id); got.State != relayStateReviewing {
		t.Fatalf("timeout of the non-awaited child stopped the relay: %+v", got)
	}
	review := h.status(id).ReviewSessionID
	h.s.relayOnChildTimeout(id, review)
	if got := h.status(id); got.State != relayStateStopped || got.Reason != relayReasonTimeout {
		t.Fatalf("awaited child timeout: %+v", got)
	}

	h2 := newRelayHarness(t)
	st2 := h2.start()
	h2.s.relayOnChildExit(st2.OrchestrationID, st2.ImplementationSessionID, "error")
	if got := h2.status(st2.OrchestrationID); got.State != relayStateStopped || got.Reason != relayReasonChildExited {
		t.Fatalf("child exit: %+v", got)
	}
	// Terminal relays ignore further signals.
	before := len(h2.notifies)
	h2.s.relayOnChildTimeout(st2.OrchestrationID, st2.ImplementationSessionID)
	h2.s.relayOnChildIdle(st2.OrchestrationID, st2.ImplementationSessionID)
	if len(h2.notifies) != before || len(h2.injects) != 0 {
		t.Fatal("terminal relay reacted to timeout / idle")
	}
}

func TestRelay_stopRelay(t *testing.T) {
	h := newRelayHarness(t)
	if err := h.s.stopRelay(h.parent.ID, "", ""); !errors.Is(err, errRelayNotFound) {
		t.Fatalf("stop without relay = %v, want errRelayNotFound", err)
	}
	first := h.start()
	if err := h.s.stopRelay(h.parent.ID, "", ""); err != nil {
		t.Fatalf("stop = %v", err)
	}
	if got := h.status(first.OrchestrationID); got.State != relayStateStopped || got.Reason != relayReasonUserStop {
		t.Fatalf("stopped status = %+v", got)
	}
	if s := h.s.sessions[first.ImplementationSessionID].State; s != "running" {
		t.Fatalf("child state after stop = %q, want running (children stay alive)", s)
	}
	if err := h.s.stopRelay(h.parent.ID, first.OrchestrationID, ""); !errors.Is(err, errRelayNotRunning) {
		t.Fatalf("second stop = %v, want errRelayNotRunning", err)
	}
	// The stopped relay's child stays alive and keeps its slot in the child
	// budget, so widen the budget for two more relays.
	h.s.cfg.Orchestration.MaxChildrenPerParent = 8
	a := h.start()
	b := h.start()
	if err := h.s.stopRelay(h.parent.ID, "", ""); !errors.Is(err, errRelayAmbiguous) {
		t.Fatalf("stop with two running = %v, want errRelayAmbiguous", err)
	}
	if err := h.s.stopRelay(2, a.OrchestrationID, ""); !errors.Is(err, errRelayNotFound) {
		t.Fatalf("stop from another parent = %v, want errRelayNotFound", err)
	}
	if err := h.s.stopRelay(h.parent.ID, b.OrchestrationID, ""); err != nil {
		t.Fatalf("stop by id = %v", err)
	}
	if got := h.status(a.OrchestrationID); got.State != relayStateImplementing {
		t.Fatalf("stopping b touched a: %+v", got)
	}
	if err := h.s.stopRelay(h.parent.ID, "", ""); err != nil {
		t.Fatalf("stop remaining = %v", err)
	}
}

func TestRelay_multipleRelaysAreIndependent(t *testing.T) {
	h := newRelayHarness(t)
	a := h.start()
	b := h.start()
	h.progress(a.OrchestrationID, a.ImplementationSessionID, doneLines(relayRoleImplementation, a.ImplementationSessionID, 1, false))
	if got := h.status(a.OrchestrationID); got.State != relayStateReviewing {
		t.Fatalf("a = %+v", got)
	}
	if got := h.status(b.OrchestrationID); got.State != relayStateImplementing || got.ReviewSessionID != 0 {
		t.Fatalf("b changed with a's DONE: %+v", got)
	}
	// b's implementation child writing into a's board id is not a's child.
	h.s.relayOnChildFileChange(a.OrchestrationID, b.ImplementationSessionID, doneLines(relayRoleImplementation, b.ImplementationSessionID, 1, false))
	if got := h.status(a.OrchestrationID); got.State != relayStateReviewing || got.Round != 1 {
		t.Fatalf("a reacted to b's child: %+v", got)
	}
	if err := h.s.stopRelay(h.parent.ID, b.OrchestrationID, ""); err != nil {
		t.Fatal(err)
	}
	if got := h.status(a.OrchestrationID); got.State != relayStateReviewing {
		t.Fatalf("stopping b changed a: %+v", got)
	}
	if ka, kb := h.eventKinds(a.OrchestrationID), h.s.relayEventsFor(h.parent.ID, b.OrchestrationID); len(ka) != 3 || len(kb) != 2 || kb[1].Kind != relayEventStopped {
		t.Fatalf("events a=%v b=%+v", ka, kb)
	}
	if h.run(a.OrchestrationID).boardPath == h.run(b.OrchestrationID).boardPath {
		t.Fatal("relays share a board")
	}
	if len(h.parent.Relays) != 2 || h.parent.Relays[0].OrchestrationID != a.OrchestrationID || h.parent.Relays[1].State != relayStateStopped {
		t.Fatalf("parent.Relays = %+v", h.parent.Relays)
	}
}

func TestRelay_sessionUpdateCarriesRelaysAndSaves(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	savesAfterStart := h.saves
	if savesAfterStart == 0 {
		t.Fatal("deps.save not called on start")
	}
	h.s.sessionsMu.Lock()
	msg := sessionUpdateMessage(h.parent)
	h.s.sessionsMu.Unlock()
	if len(msg.Relays) != 1 || msg.Relays[0].OrchestrationID != st.OrchestrationID || msg.Relays[0].State != relayStateImplementing {
		t.Fatalf("session_update relays = %+v", msg.Relays)
	}
	data, err := json.Marshal(msg)
	if err != nil {
		t.Fatal(err)
	}
	for _, needle := range []string{`"relays":[{`, `"orchestration_id":"` + st.OrchestrationID + `"`, `"state":"implementing"`, `"plan_path":`, `"active_implementer":"implementation"`} {
		if !strings.Contains(string(data), needle) {
			t.Fatalf("json missing %s: %s", needle, data)
		}
	}
	h.progress(st.OrchestrationID, st.ImplementationSessionID, doneLines(relayRoleImplementation, st.ImplementationSessionID, 1, false))
	if h.saves <= savesAfterStart {
		t.Fatal("deps.save not called on transition")
	}
	h.s.sessionsMu.Lock()
	msg = sessionUpdateMessage(h.parent)
	h.s.sessionsMu.Unlock()
	if msg.Relays[0].State != relayStateReviewing || msg.Relays[0].ReviewSessionID != 11 {
		t.Fatalf("session_update after transition = %+v", msg.Relays[0])
	}
	if msg.Relays[0].UpdatedAt == "" {
		t.Fatal("updated_at empty")
	}
}

// ---- D-22: strong implementer -------------------------------------------

func TestRelay_escalatesAfterReviewFailures(t *testing.T) {
	h := newRelayHarness(t)
	h.strong = true
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	run := h.run(id)
	if st.StrongSessionID != 0 || st.ActiveImplementer != relayRoleImplementation || st.EscalateAfter != 2 {
		t.Fatalf("initial status = %+v", st)
	}
	if !strings.Contains(h.spawns[0].InitialPrompt, "marked [strong]") || !strings.Contains(h.spawns[0].InitialPrompt, "escalate=true") {
		t.Fatalf("implementation prompt lacks the [strong] rule: %q", h.spawns[0].InitialPrompt)
	}

	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	review := h.status(id).ReviewSessionID
	f1 := h.writeReview(id, 1, 1, false, "must:\n1. a\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1))
	if got := h.status(id); got.State != relayStateFixing || got.ActiveImplementer != relayRoleImplementation || got.StrongSessionID != 0 {
		t.Fatalf("round 1 findings should stay with the cheap implementer: %+v", got)
	}
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 2, false))
	f2 := h.writeReview(id, 1, 2, false, "must:\n1. a still\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1, "findings must=1 file="+f2))

	// Second failed round → the C is handed to the strong implementer.
	st = h.status(id)
	if st.State != relayStateFixing || st.ActiveImplementer != relayRoleImplementationStrong || st.StrongSessionID != 12 || st.Round != 0 {
		t.Fatalf("after escalation: %+v", st)
	}
	if len(h.spawns) != 3 || h.spawns[2].Role != relayRoleImplementationStrong || h.spawns[2].Provider != "claude" || h.spawns[2].Model != "opus-strong" {
		t.Fatalf("strong spawn = %+v (spawns=%d)", h.spawns[len(h.spawns)-1], len(h.spawns))
	}
	strongPrompt := h.spawns[2].InitialPrompt
	for _, needle := range []string{"Relay escalation", "C 1 only", "1 must-fix items in " + f2, "trust the working tree over your own memory", "git log --oneline base000..HEAD", "## DONE implementation-strong"} {
		if !strings.Contains(strongPrompt, needle) {
			t.Fatalf("strong prompt missing %q: %q", needle, strongPrompt)
		}
	}
	if in := h.lastInject(); in.id != impl || !strings.Contains(in.text, "another worker has taken over") {
		t.Fatalf("cheap implementer was not told to wait: %+v", in)
	}
	evs := h.s.relayEventsFor(h.parent.ID, id)
	if last := evs[len(evs)-1]; last.Kind != relayEventEscalated || last.Text != "reason=review_failures" || last.ReviewPath != f2 {
		t.Fatalf("escalated event = %+v", last)
	}

	// A DONE from the waiting cheap implementer is recorded and ignored.
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 3, false))
	if got := h.status(id); got.State != relayStateFixing || got.ActiveImplementer != relayRoleImplementationStrong {
		t.Fatalf("waiting implementer's DONE moved the relay: %+v", got)
	}
	if !strings.Contains(h.board(id), "ignored DONE from waiting implementer role=implementation") {
		t.Fatalf("board lacks the ignored-DONE record:\n%s", h.board(id))
	}

	// The strong implementer's DONE → re-review, round 1, strong review file.
	strong := st.StrongSessionID
	h.progress(id, strong, doneLines(relayRoleImplementationStrong, strong, 1, false))
	st = h.status(id)
	strongReview := relayReviewFile(run.boardDir, 1, 1, true)
	if st.State != relayStateReviewing || st.Round != 1 || st.ReviewPath != strongReview {
		t.Fatalf("after strong DONE: %+v", st)
	}
	if in := h.lastInject(); in.id != review || !strings.Contains(in.text, "re-review after the fixes for "+f2) || !strings.Contains(in.text, strongReview) {
		t.Fatalf("re-review after escalation = %+v", in)
	}

	// pass → the next C goes back to the cheap implementer.
	h.head = "c1pass"
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1, "findings must=1 file="+f2, "pass"))
	st = h.status(id)
	if st.State != relayStateImplementing || st.CompletedCs != 1 || st.ActiveImplementer != relayRoleImplementation || st.Round != 0 {
		t.Fatalf("after pass: %+v", st)
	}
	if in := h.lastInject(); in.id != impl || !strings.Contains(in.text, "the review of C 1 passed") {
		t.Fatalf("proceed went to %d: %+v", in.id, in)
	}

	// C2 fails twice as well → the strong implementer is reused by injection.
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 4, false))
	g1 := h.writeReview(id, 2, 1, false, "must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1, "findings must=1 file="+f2, "pass", "findings must=1 file="+g1))
	if got := h.status(id); got.State != relayStateFixing || got.ActiveImplementer != relayRoleImplementation {
		t.Fatalf("C2 round 1: %+v", got)
	}
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 5, false))
	g2 := h.writeReview(id, 2, 2, false, "must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1, "findings must=1 file="+f2, "pass", "findings must=1 file="+g1, "findings must=1 file="+g2))
	st = h.status(id)
	if st.State != relayStateFixing || st.ActiveImplementer != relayRoleImplementationStrong || st.StrongSessionID != strong || len(h.spawns) != 3 {
		t.Fatalf("C2 escalation: %+v spawns=%d", st, len(h.spawns))
	}
	found := false
	for _, in := range h.injects {
		if in.id == strong && strings.Contains(in.text, "Relay escalation: C 2:") && strings.Contains(in.text, "must-fix items in "+g2) {
			found = true
		}
	}
	if !found {
		t.Fatalf("strong implementer did not receive the C2 fix instruction: %+v", h.injects)
	}
	// The awaited child is now the strong one: idle on the cheap one is ignored.
	n := len(h.injects)
	h.s.relayOnChildIdle(id, impl)
	if len(h.injects) != n {
		t.Fatal("idle on the waiting cheap implementer nudged it")
	}
	h.s.relayOnChildIdle(id, strong)
	if in := h.lastInject(); len(h.injects) != n+1 || in.id != strong || !strings.Contains(in.text, "## DONE implementation-strong") {
		t.Fatalf("strong nudge = %+v", in)
	}
}

func TestRelay_escalatesOnPlanHint(t *testing.T) {
	h := newRelayHarness(t)
	h.strong = true
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	run := h.run(id)
	h.progress(id, impl, "## implementation session=10\nC1 is marked [strong]\n## DONE implementation session=10 escalate=true\n")
	st = h.status(id)
	if st.State != relayStateImplementing || st.ActiveImplementer != relayRoleImplementationStrong || st.StrongSessionID != 11 || len(h.spawns) != 2 {
		t.Fatalf("after escalate=true: %+v spawns=%d", st, len(h.spawns))
	}
	if p := h.spawns[1].InitialPrompt; h.spawns[1].Role != relayRoleImplementationStrong || !strings.Contains(p, "C 1 only: implement it from the plan") {
		t.Fatalf("strong spawn = %+v", h.spawns[1])
	}
	if in := h.lastInject(); in.id != impl || !strings.Contains(in.text, "another worker has taken over") {
		t.Fatalf("wait text = %+v", in)
	}
	evs := h.s.relayEventsFor(h.parent.ID, id)
	if last := evs[len(evs)-1]; last.Kind != relayEventEscalated || last.Text != "reason=plan_hint" {
		t.Fatalf("escalated event = %+v", last)
	}
	// The strong implementer finishes the (final) C → fresh review, strong file.
	strong := st.StrongSessionID
	h.progress(id, strong, doneLines(relayRoleImplementationStrong, strong, 1, true))
	st = h.status(id)
	if st.State != relayStateReviewing || !st.FinalSeen || st.Round != 1 || st.ReviewPath != relayReviewFile(run.boardDir, 1, 1, true) || len(h.spawns) != 3 {
		t.Fatalf("after strong DONE: %+v spawns=%d", st, len(h.spawns))
	}
	if !strings.Contains(h.spawns[2].InitialPrompt, relayReviewFile(run.boardDir, 1, 1, true)) {
		t.Fatalf("review prompt = %q", h.spawns[2].InitialPrompt)
	}
	review := st.ReviewSessionID
	h.progress(id, review, reviewProgress(review, "pass"))
	if got := h.status(id); got.State != relayStateCompleted || got.CompletedCs != 1 {
		t.Fatalf("after pass: %+v", got)
	}

	// Without a strong role the cheap implementer is told to do it itself.
	h2 := newRelayHarness(t)
	st2 := h2.start()
	id2, impl2 := st2.OrchestrationID, st2.ImplementationSessionID
	h2.progress(id2, impl2, "## implementation session=10\n## DONE implementation session=10 escalate=true\n")
	if got := h2.status(id2); got.State != relayStateImplementing || got.ActiveImplementer != relayRoleImplementation || len(h2.spawns) != 1 {
		t.Fatalf("escalate without strong role: %+v spawns=%d", got, len(h2.spawns))
	}
	if in := h2.lastInject(); in.id != impl2 || !strings.Contains(in.text, "no stronger implementer is available for C 1") {
		t.Fatalf("self-implement text = %+v", in)
	}
	h2.progress(id2, impl2, "## implementation session=10\n## DONE implementation session=10 escalate=true\n## DONE implementation session=10\n")
	if got := h2.status(id2); got.State != relayStateReviewing {
		t.Fatalf("plain DONE after self-implement: %+v", got)
	}
}

func TestRelay_escalationSkippedOnChildLimit(t *testing.T) {
	h := newRelayHarness(t)
	h.strong = true
	h.s.cfg.Orchestration.MaxChildrenPerParent = 2
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	review := h.status(id).ReviewSessionID
	f1 := h.writeReview(id, 1, 1, false, "must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1))
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 2, false))
	f2 := h.writeReview(id, 1, 2, false, "must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1, "findings must=1 file="+f2))
	st = h.status(id)
	if st.State != relayStateFixing || st.ActiveImplementer != relayRoleImplementation || st.StrongSessionID != 0 || len(h.spawns) != 2 || st.Round != 2 {
		t.Fatalf("escalation should have been skipped: %+v spawns=%d", st, len(h.spawns))
	}
	if !strings.Contains(h.board(id), "escalation skipped: child limit") {
		t.Fatalf("board lacks the skip record:\n%s", h.board(id))
	}
	if in := h.lastInject(); in.id != impl || !strings.Contains(in.text, "Relay fix (C 1, round 2)") {
		t.Fatalf("fix injection = %+v", in)
	}
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 3, false))
	f3 := h.writeReview(id, 1, 3, false, "must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1, "findings must=1 file="+f2, "findings must=1 file="+f3))
	if got := h.status(id); got.State != relayStateStopped || got.Reason != relayReasonMaxRounds {
		t.Fatalf("third failure should stop on max_rounds: %+v", got)
	}
}

// ---- D-23: message tag ----------------------------------------------------

func TestRelay_messagesCarryTag(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	tag := "[relay " + relayShortID(id) + " plan_x.md]"
	if !strings.HasPrefix(h.spawns[0].InitialPrompt, tag+" ") {
		t.Fatalf("implementation prompt does not start with %q: %q", tag, h.spawns[0].InitialPrompt)
	}
	h.s.relayOnChildIdle(id, impl)
	if in := h.lastInject(); !strings.HasPrefix(in.text, "\n"+tag+" ") {
		t.Fatalf("nudge does not start with the tag: %q", in.text)
	}
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	if !strings.HasPrefix(h.spawns[1].InitialPrompt, tag+" ") {
		t.Fatalf("review prompt does not start with the tag: %q", h.spawns[1].InitialPrompt)
	}
	review := h.status(id).ReviewSessionID
	f1 := h.writeReview(id, 1, 1, false, "must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1))
	if in := h.lastInject(); !strings.HasPrefix(in.text, "\n"+tag+" Relay fix") {
		t.Fatalf("fix injection does not start with the tag: %q", in.text)
	}
	if err := h.s.stopRelay(h.parent.ID, id, ""); err != nil {
		t.Fatal(err)
	}
	if len(h.notifies) != 1 || !strings.HasPrefix(h.notifies[0], "\n"+tag+" relay stopped") {
		t.Fatalf("parent notice does not start with the tag: %q", h.notifies)
	}
	for _, in := range h.injects {
		if !strings.HasPrefix(in.text, "\n"+tag+" ") {
			t.Fatalf("injection without tag: %q", in.text)
		}
	}
}

// ---- D-16 / D-17: shared worktree ---------------------------------------

// newRelayTestRepo creates a git repository with one commit. Settings are kept
// repository-local so the user's global gitconfig cannot fail the test.
func newRelayTestRepo(t *testing.T) string {
	t.Helper()
	repo := t.TempDir()
	if resolved, err := filepath.EvalSymlinks(repo); err == nil {
		repo = resolved
	}
	runWorktreeTestGit(t, repo, "init")
	runWorktreeTestGit(t, repo, "config", "user.name", "relay test")
	runWorktreeTestGit(t, repo, "config", "user.email", "relay-test@example.com")
	runWorktreeTestGit(t, repo, "config", "commit.gpgsign", "false")
	if err := os.WriteFile(filepath.Join(repo, "README.md"), []byte("base\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	runWorktreeTestGit(t, repo, "add", "README.md")
	runWorktreeTestGit(t, repo, "commit", "-m", "base")
	return repo
}

func gitOutput(t *testing.T, dir string, args ...string) string {
	t.Helper()
	head, err := relayGitHead(dir)
	if len(args) == 0 {
		if err != nil {
			t.Fatalf("git rev-parse HEAD: %v", err)
		}
		return head
	}
	out, err := runGit(context.Background(), dir, args...)
	if err != nil {
		t.Fatalf("git %v: %v", args, err)
	}
	return string(out)
}

func TestRelay_prepareRelayWorktree(t *testing.T) {
	s := newTestServer()
	repo := newRelayTestRepo(t)
	cfg := config.OrchestrationConfig{}
	head := gitOutput(t, repo)

	path, branch, base, err := s.prepareRelayWorktree(repo, "r1-123", cfg)
	if err != nil {
		t.Fatalf("prepareRelayWorktree: %v", err)
	}
	wantPath := filepath.Join(repo, ".many-ai-cli", "worktrees", "r1-123", "relay")
	if path != wantPath || branch != proto.RelayBranchPrefix+"r1-123" || base != head {
		t.Fatalf("got path=%q branch=%q base=%q, want %q / %q / %q", path, branch, base, wantPath, proto.RelayBranchPrefix+"r1-123", head)
	}
	if list := gitOutput(t, repo, "worktree", "list", "--porcelain"); !strings.Contains(filepath.ToSlash(list), filepath.ToSlash(path)) || !strings.Contains(list, "branch refs/heads/"+branch) {
		t.Fatalf("worktree list does not show the relay worktree:\n%s", list)
	}
	if got := gitOutput(t, path); got != head {
		t.Fatalf("worktree HEAD = %q, want base %q", got, head)
	}
	exclude, err := os.ReadFile(filepath.Join(repo, ".git", "info", "exclude"))
	if err != nil || !strings.Contains(string(exclude), "/.many-ai-cli/worktrees/") {
		t.Fatalf("info/exclude missing the worktree root (err=%v):\n%s", err, exclude)
	}
	// Second call: the directory exists → reuse, base left for the caller.
	path2, branch2, base2, err := s.prepareRelayWorktree(repo, "r1-123", cfg)
	if err != nil || path2 != path || branch2 != branch || base2 != "" {
		t.Fatalf("reuse = %q %q %q err=%v", path2, branch2, base2, err)
	}
	// Two relays → two worktrees and branches.
	pathB, branchB, _, err := s.prepareRelayWorktree(repo, "r1-456", cfg)
	if err != nil || pathB == path || branchB == branch {
		t.Fatalf("second relay worktree = %q %q err=%v", pathB, branchB, err)
	}
	// Cleanup keeps the branch (the user's result) and removes the tree.
	if err := s.cleanupRelayWorktree(repo, path, branch, true); err != nil {
		t.Fatalf("cleanupRelayWorktree: %v", err)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("worktree still present after cleanup (err=%v)", err)
	}
	if branches := gitOutput(t, repo, "branch", "--list", branch); !strings.Contains(branches, branch) {
		t.Fatalf("branch %q was deleted by cleanup(keepBranch=true): %q", branch, branches)
	}
	// The branch survived: preparing again attaches to it instead of failing.
	path3, _, base3, err := s.prepareRelayWorktree(repo, "r1-123", cfg)
	if err != nil || path3 != path || base3 != head {
		t.Fatalf("re-attach = %q base=%q err=%v", path3, base3, err)
	}
	if err := s.cleanupRelayWorktree(repo, path, branch, false); err != nil {
		t.Fatalf("cleanupRelayWorktree(delete branch): %v", err)
	}
	if branches := gitOutput(t, repo, "branch", "--list", branch); strings.Contains(branches, branch) {
		t.Fatalf("branch %q survived cleanup(keepBranch=false): %q", branch, branches)
	}

	if _, _, _, err := s.prepareRelayWorktree(t.TempDir(), "r1-789", cfg); !errors.Is(err, errRelayNotGit) {
		t.Fatalf("non-git err = %v, want errRelayNotGit", err)
	}
	if n, err := relayGitChangedFiles(repo, ""); err != nil || n != 0 {
		t.Fatalf("changed files on clean tree = %d err=%v", n, err)
	}
}

func TestRelay_prepareRelayWorktree_rejectsChangedBranch(t *testing.T) {
	s := newTestServer()
	repo := newRelayTestRepo(t)
	cfg := config.OrchestrationConfig{}
	path, branch, _, err := s.prepareRelayWorktree(repo, "r1-identity", cfg)
	if err != nil {
		t.Fatalf("prepareRelayWorktree create: %v", err)
	}
	t.Cleanup(func() { _ = s.cleanupRelayWorktree(repo, path, branch, true) })
	runWorktreeTestGit(t, path, "switch", "-c", "manual-relay-branch")
	before, err := os.ReadFile(filepath.Join(path, "README.md"))
	if err != nil {
		t.Fatalf("read reused worktree before rejection: %v", err)
	}
	if _, _, _, err := s.prepareRelayWorktree(repo, "r1-identity", cfg); !errors.Is(err, errWorktreeIdentityMismatch) {
		t.Fatalf("changed branch error = %v, want identity mismatch", err)
	}
	if got := strings.TrimSpace(gitOutput(t, path, "branch", "--show-current")); got != "manual-relay-branch" {
		t.Fatalf("branch after rejected reuse = %q, want manual-relay-branch", got)
	}
	after, err := os.ReadFile(filepath.Join(path, "README.md"))
	if err != nil || string(after) != string(before) {
		t.Fatalf("worktree content changed on rejected reuse: before=%q after=%q err=%v", before, after, err)
	}
	runWorktreeTestGit(t, path, "switch", branch)
}

func TestRelay_prepareRelayWorktree_rejectsUnregisteredDirectory(t *testing.T) {
	s := newTestServer()
	repo := newRelayTestRepo(t)
	path := filepath.Join(repo, ".many-ai-cli", "worktrees", "r1-ordinary", "relay")
	if err := os.MkdirAll(path, 0o700); err != nil {
		t.Fatal(err)
	}
	if _, _, _, err := s.prepareRelayWorktree(repo, "r1-ordinary", config.OrchestrationConfig{}); !errors.Is(err, errWorktreeIdentityMismatch) {
		t.Fatalf("ordinary directory error = %v, want identity mismatch", err)
	}
}

func TestRelay_cleanupRelayWorktreeRecoversUnregisteredEmptyDirectory(t *testing.T) {
	s := newTestServer()
	repo := newRelayTestRepo(t)
	path, branch, _, err := s.prepareRelayWorktree(repo, "r1-partial", config.OrchestrationConfig{})
	if err != nil {
		t.Fatal(err)
	}
	if registered, err := relayWorktreeRegistered(context.Background(), repo, path); err != nil || !registered {
		t.Fatalf("registered before removal = %v, %v", registered, err)
	}
	runWorktreeTestGit(t, repo, "worktree", "remove", "--force", path)
	if err := os.MkdirAll(path, 0o700); err != nil {
		t.Fatal(err)
	}
	if registered, err := relayWorktreeRegistered(context.Background(), repo, path); err != nil || registered {
		t.Fatalf("registered after manual removal = %v, %v", registered, err)
	}

	if err := s.cleanupRelayWorktree(repo, path, branch, true); err != nil {
		t.Fatalf("cleanup partial worktree: %v", err)
	}
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("partial worktree directory remains: %v", err)
	}
	if _, err := os.Stat(filepath.Dir(path)); !os.IsNotExist(err) {
		t.Fatalf("empty orchestration directory remains: %v", err)
	}
	if branches := gitOutput(t, repo, "branch", "--list", branch); !strings.Contains(branches, branch) {
		t.Fatalf("branch %q was deleted during recovery: %q", branch, branches)
	}
}

func TestRelay_worktreeMode(t *testing.T) {
	h := newRelayHarness(t)
	repo := newRelayTestRepo(t)
	h.parent.CWD = repo
	head := gitOutput(t, repo)
	req := h.request()
	req.Mode = relayModeWorktree
	st, err := h.s.startRelay(h.parent.ID, req)
	if err != nil {
		t.Fatalf("startRelay(worktree): %v", err)
	}
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	wantPath := filepath.Join(repo, ".many-ai-cli", "worktrees", safeToken(id), "relay")
	if st.Mode != relayModeWorktree || st.WorktreePath != wantPath || st.Branch != proto.RelayBranchPrefix+safeToken(id) || st.BaseCommit != head {
		t.Fatalf("status = %+v (want path %q base %q)", st, wantPath, head)
	}
	if h.preps[0].childCWD != wantPath || h.spawns[0].CWD != wantPath {
		t.Fatalf("implementation child cwd = %q / %q, want the relay worktree", h.preps[0].childCWD, h.spawns[0].CWD)
	}
	p := h.spawns[0].InitialPrompt
	for _, needle := range []string{"git add -A && git commit -m \"relay: C<n> <short summary>\"", "relay worktree " + wantPath, "on branch " + st.Branch, "never push"} {
		if !strings.Contains(p, needle) {
			t.Fatalf("worktree-mode implementation prompt missing %q: %q", needle, p)
		}
	}
	// The reviewer is pointed at the commit range since the base commit and
	// shares the same worktree.
	h.head = "c1head"
	h.changed = 2
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	if len(h.spawns) != 2 || h.preps[1].childCWD != wantPath {
		t.Fatalf("reviewer cwd = %q, want %q", h.preps[1].childCWD, wantPath)
	}
	rp := h.spawns[1].InitialPrompt
	for _, needle := range []string{"git log --oneline " + head + "..HEAD", "git diff " + head + "..HEAD", "git status --short"} {
		if !strings.Contains(rp, needle) {
			t.Fatalf("worktree-mode review prompt missing %q: %q", needle, rp)
		}
	}
	evs := h.s.relayEventsFor(h.parent.ID, id)
	if evs[1].Kind != relayEventCDone || evs[1].Commit != "c1head" || evs[1].FilesChanged != 2 {
		t.Fatalf("c_done event in worktree mode = %+v", evs[1])
	}
	// Fix instruction carries the commit line as well.
	review := h.status(id).ReviewSessionID
	f1 := h.writeReview(id, 1, 1, false, "must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1))
	if in := h.lastInject(); !strings.Contains(in.text, "relay: C<n> fix r<r>") {
		t.Fatalf("worktree-mode fix text lacks the commit line: %q", in.text)
	}
	// After a pass the scope moves to the reviewed commit.
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 2, false))
	h.head = "c1pass"
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1, "pass"))
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 3, false))
	if in := h.lastInject(); in.id != review || !strings.Contains(in.text, "git diff c1pass..HEAD") {
		t.Fatalf("C2 review scope = %+v", in)
	}

	// A non-git parent in worktree mode is refused, nothing is spawned.
	h2 := newRelayHarness(t)
	req2 := h2.request()
	req2.Mode = relayModeWorktree
	if _, err := h2.s.startRelay(h2.parent.ID, req2); !errors.Is(err, errRelayNotGit) {
		t.Fatalf("worktree mode on non-git cwd err = %v, want errRelayNotGit", err)
	}
	if len(h2.spawns) != 0 || len(h2.s.relayStatusesFor(h2.parent.ID)) != 0 {
		t.Fatalf("refused relay left spawns=%d relays=%d", len(h2.spawns), len(h2.s.relayStatusesFor(h2.parent.ID)))
	}
}

func TestRelay_completedWorktreeNotifiesMergeCommandWithoutMerging(t *testing.T) {
	h := newRelayHarness(t)
	repo := newRelayTestRepo(t)
	h.parent.CWD = repo
	baseHead := gitOutput(t, repo)
	baseBranch := strings.TrimSpace(gitOutput(t, repo, "branch", "--show-current"))
	req := h.request()
	req.Mode = relayModeWorktree
	st, err := h.s.startRelay(h.parent.ID, req)
	if err != nil {
		t.Fatal(err)
	}
	if !h.s.relayFinish(h.run(st.OrchestrationID), relayStateCompleted, "", "finished by test") {
		t.Fatal("relayFinish returned false")
	}
	if len(h.notifies) != 1 || !strings.Contains(h.notifies[0], "git merge "+st.Branch) || !strings.Contains(h.notifies[0], "not run automatically") {
		t.Fatalf("completion notice = %q", h.notifies)
	}
	if got := gitOutput(t, repo); got != baseHead {
		t.Fatalf("parent HEAD changed: got %q want %q", got, baseHead)
	}
	if got := strings.TrimSpace(gitOutput(t, repo, "branch", "--show-current")); got != baseBranch {
		t.Fatalf("parent branch changed: got %q want %q", got, baseBranch)
	}
}

func TestRelay_sameTreeHasNoCommitLine(t *testing.T) {
	h := newRelayHarness(t)
	st := h.start()
	id, impl := st.OrchestrationID, st.ImplementationSessionID
	if p := h.spawns[0].InitialPrompt; strings.Contains(p, "git commit") || strings.Contains(p, "relay worktree") {
		t.Fatalf("same-tree implementation prompt mentions commits / worktree: %q", p)
	}
	if st.WorktreePath != "" || st.Branch != "" {
		t.Fatalf("same-tree status has worktree fields: %+v", st)
	}
	h.progress(id, impl, doneLines(relayRoleImplementation, impl, 1, false))
	if rp := h.spawns[1].InitialPrompt; !strings.Contains(rp, "changes that pre-date the relay are also visible") || strings.Contains(rp, "..HEAD") {
		t.Fatalf("same-tree review prompt = %q", rp)
	}
	review := h.status(id).ReviewSessionID
	f1 := h.writeReview(id, 1, 1, false, "must\n")
	h.progress(id, review, reviewProgress(review, "findings must=1 file="+f1))
	if in := h.lastInject(); strings.Contains(in.text, "git commit") {
		t.Fatalf("same-tree fix text has a commit line: %q", in.text)
	}
}
