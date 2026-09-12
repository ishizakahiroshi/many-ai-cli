package hub

// relay.go: the relay loop state machine
// (docs/local/plan_orchestration-relay-loop.md).
//
// A relay drives plan → implementation → adversarial review → fix → next C
// without an AI conductor (D-1). The Hub decides every step from two signals
// only: the number of `## DONE <role>` lines in a child's progress file
// (D-2 / D-7 — the count, not the presence, so the same child can finish
// several Cs) and the reviewer's `verdict:` line (D-3 / D-14). Everything with
// a side effect — spawn, PTY injection, board bookkeeping, git, persistence —
// goes through relayDeps so the whole machine runs in unit tests without a
// process.
//
// Two implementers (D-22): the cheap `implementation` role does every C first.
// When a C fails review escalateAfter times in a row, or the plan marks the C
// `[strong]` and the cheap implementer reports `escalate=true`, the C is handed
// to the optional `implementation-strong` role. The next C goes back to the
// cheap one. The Hub never judges difficulty itself; the review result and the
// plan mark are the only triggers.
//
// One parent session may run several relays (D-21). Each relay allocates its
// own orchestration ID (`r<parent>-<time>`), board directory, children and
// worktree; the parent's own OrchestrationID is never reused. Every text the
// Hub injects into a child or sends to the parent starts with
// `[relay <short id> <plan basename>]` so nobody mixes two relays up (D-23).
//
// Locking: orchestrationManager.mu only guards the relays map. Each relayRun
// has its own mu that serialises its transitions; spawn / inject / git / save
// are called while run.mu is held, and those take sessionsMu or
// orchestration.mu briefly on their own. Nothing takes run.mu while holding
// either of those, so the order is always run.mu → (sessionsMu | orchestration.mu).

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

const (
	relayRoleImplementation       = "implementation"
	relayRoleImplementationStrong = "implementation-strong"
	relayRoleReview               = "review"

	relayModeWorktree = "worktree"
	relayModeSameTree = "same-tree"

	relayStateImplementing = "implementing"
	relayStateReviewing    = "reviewing"
	relayStateFixing       = "fixing"
	relayStateCompleted    = "completed"
	relayStateStopped      = "stopped"

	relayDefaultMaxRounds     = 3
	relayDefaultEscalateAfter = 2
	// relayChildrenPerRun is how many child sessions one relay always occupies
	// in the parent's max_children_per_parent budget (implementation +
	// review). The strong implementer is spawned on demand and only if a slot
	// is free at that moment.
	relayChildrenPerRun = 2
)

// Stop reasons (proto.RelayStatus.Reason).
const (
	relayReasonMaxRounds         = "max_rounds"
	relayReasonBlocked           = "blocked"
	relayReasonVerdictMissing    = "verdict_missing"
	relayReasonReviewFileMissing = "review_file_missing"
	relayReasonTimeout           = "timeout"
	relayReasonChildExited       = "child_exited"
	relayReasonUserStop          = "user_stop"
	relayReasonSpawnError        = "spawn_error"
	relayReasonHubRestart        = "hub_restart"
	// relayReasonStartupFailed: the awaited child never produced any progress
	// after its initial prompt was delivered
	// (plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md, D9/D10).
	relayReasonStartupFailed = "startup_failed"
)

// Event kinds (proto.RelayEvent.Kind).
const (
	relayEventStarted       = "started"
	relayEventCDone         = "c_done"
	relayEventReviewStarted = "review_started"
	relayEventVerdict       = "verdict"
	relayEventFixSent       = "fix_sent"
	relayEventProceedSent   = "proceed_sent"
	relayEventNudge         = "nudge"
	relayEventEscalated     = "escalated"
	relayEventCompleted     = "completed"
	relayEventStopped       = "stopped"
	relayEventRestored      = "restored"
	relayEventResumed       = "resumed"
)

// Escalation triggers (text of the escalated event).
const (
	relayEscalatePlanHint       = "plan_hint"
	relayEscalateReviewFailures = "review_failures"
)

var (
	errRelayParentNotFound = errors.New("parent session not found")
	errRelayNotFound       = errors.New("relay not found")
	errRelayAmbiguous      = errors.New("more than one relay is running for this session; specify orchestration_id")
	errRelayNotRunning     = errors.New("relay is not running")
	errRelayPlanPath       = errors.New("plan path must be an absolute path")
	errRelayMode           = errors.New("mode must be \"worktree\" or \"same-tree\"")
	errRelayNoChild        = errors.New("relay has no child for that role")
	errVerdictMissing      = errors.New("verdict line missing")
	errReviewFileMissing   = errors.New("review file missing")
)

// errRelayRolesMissing names the role whose provider / model was not supplied
// (or, for the optional strong role, was supplied but invalid).
type errRelayRolesMissing struct{ Role string }

func (e errRelayRolesMissing) Error() string {
	return fmt.Sprintf("role %q needs a provider and a model", e.Role)
}

// errRelayExecutionMode is an execution mode a role cannot run in — today only
// "headless was asked for and this provider has no headless definition". It is
// a start-time refusal on purpose (親 plan D2): the alternative, quietly running
// the role interactively, produces a relay that looks like it started and then
// waits forever for a child nobody is watching.
type errRelayExecutionMode struct {
	Role     string
	Provider string
	Err      error
}

func (e errRelayExecutionMode) Error() string {
	return fmt.Sprintf("role %q (%s): %v", e.Role, e.Provider, e.Err)
}

func (e errRelayExecutionMode) Unwrap() error { return e.Err }

// errOrchestrationLimit is returned when starting the relay would exceed one
// of the orchestration limits. Running is the number of relays already in
// progress for the parent so the caller can say "n in progress, budget max".
type errOrchestrationLimit struct {
	Limit   string // depth | children_per_parent | total_sessions
	Running int
	Max     int
}

func (e errOrchestrationLimit) Error() string {
	return fmt.Sprintf("orchestration limit %s reached (relays in progress=%d, max=%d)", e.Limit, e.Running, e.Max)
}

// relayDeps are the side effects of the state machine. A nil field means the
// production implementation (relayDep fills it in); tests replace them.
type relayDeps struct {
	// spawnChild starts a child for the relay. The default is dispatchSpawn —
	// deliberately not preparePromptAndWorktree, which would reuse the parent's
	// own OrchestrationID and make every relay of a parent share one board.
	spawnChild func(parentID int, parent *session, body spawnChildRequest, prep childSpawnPreparation) (childSpawnResult, error)
	// inject delivers an instruction to a live child (Enter is pressed).
	inject func(sessionID int, text string)
	// appendBoard writes a `## <role> <time>` section to board.md.
	appendBoard func(path, role, text string) error
	// notifyParent tells the conductor (queue-until-idle for conductors).
	notifyParent func(boardID string, parentID int, text string)
	gitHead      func(dir string) (string, error)
	// gitChangedFiles counts files changed since `from` (uncommitted files when
	// from is empty).
	gitChangedFiles func(dir, from string) (int, error)
	now             func() time.Time
	// save persists the run (relay_store.go). No-op until the store is wired.
	save func(run *relayRun) error
	// publishDone exposes a relay's terminal state through the normal Hub
	// completion channel. It deliberately uses the relay-specific publisher so
	// a relay completion is not mistaken for a user's own Git turn.
	publishDone func(summary proto.DoneSummary)
}

func (s *Server) relayDep() relayDeps {
	d := s.relay
	if d.spawnChild == nil {
		d.spawnChild = s.dispatchSpawn
	}
	if d.inject == nil {
		d.inject = func(id int, text string) { s.injectText(id, text, true, false) }
	}
	if d.appendBoard == nil {
		d.appendBoard = s.appendBoardSection
	}
	if d.notifyParent == nil {
		d.notifyParent = s.notifyBoardSession
	}
	if d.gitHead == nil {
		d.gitHead = relayGitHead
	}
	if d.gitChangedFiles == nil {
		d.gitChangedFiles = relayGitChangedFiles
	}
	if d.now == nil {
		d.now = time.Now
	}
	if d.save == nil {
		d.save = s.saveRelayFile
	}
	if d.publishDone == nil {
		d.publishDone = s.publishRelayDoneSummary
	}
	return d
}

// relayStartRequest is what startRelay needs after the caller (HTTP API or
// `orchestrate relay`) has validated the plan path.
type relayStartRequest struct {
	PlanPath      string // absolute, already validated by the caller
	MaxRounds     int    // reviews per C; 0 means relayDefaultMaxRounds
	EscalateAfter int    // failed rounds before the strong implementer takes over; 0 means relayDefaultEscalateAfter
	Mode          string // worktree (default) | same-tree
	ChildCWD      string // children's cwd; empty means the parent's cwd
	// Roles needs implementation and review; implementation-strong is optional.
	Roles map[string]orchestrationRoleAssignment
	Extra map[string]string // per-role free-text instructions appended to the prompts
}

// relayVerdict is the parsed `verdict:` line of the reviewer. It is persisted
// in relay.json (the resume prompt needs the last findings), hence the tags.
type relayVerdict struct {
	Kind   string `json:"kind"` // pass | findings | blocked
	Must   int    `json:"must"`
	Should int    `json:"should"`
	File   string `json:"file,omitempty"`
	Reason string `json:"reason,omitempty"`
}

// relayRun is one relay loop. Fields are guarded by mu (see the file comment).
type relayRun struct {
	mu  sync.Mutex
	seq uint64 // start order across all relays of the Hub process

	orchestrationID string
	// admissionID holds the relay's implementation/review reservation while
	// the run is active. Each successful child consumes one slot; a terminal
	// run releases whatever remains.
	admissionID   string
	boardPath     string
	boardDir      string
	parentID      int
	planPath      string
	mode          string
	maxRounds     int
	escalateAfter int

	// Parent identity for re-identification after a Hub restart
	// (relay_store.go). parentAttached is false for a restored relay until the
	// conductor reattaches (or another session resumes and adopts it); while
	// it is false nothing is sent to parentID, which may now belong to a
	// different session.
	parentStartedAt string
	parentProvider  string
	parentCWD       string
	parentAttached  bool
	// awaitingReconnect is set by restoreRelays until every child recorded in
	// relay.json has reattached; restoredAt bounds that wait.
	awaitingReconnect bool
	restoredAt        time.Time
	reconnected       map[string]bool // role → reattached since restore

	completedCs int
	round       int
	finalSeen   bool
	state       string
	reason      string
	// activeImpl is the implementer role working on the current C
	// (relayRoleImplementation or relayRoleImplementationStrong). It goes back
	// to the cheap role whenever a new C starts.
	activeImpl string

	roles    map[string]orchestrationRoleAssignment
	extra    map[string]string
	childCWD string

	implID, strongID, reviewID                               int
	implLabel, strongLabel, reviewLabel                      string
	implDoneBaseline, strongDoneBaseline, reviewDoneBaseline int
	// *ProgressID is the session ID a child uses in its progress file name
	// and DONE lines. It equals the live ID unless the child was renumbered
	// when it reattached after a Hub restart; 0 means "same as the live ID".
	implProgressID, strongProgressID, reviewProgressID int
	// nudged remembers, per child, whether the single idle reminder for the
	// current work unit has been sent. It is reset whenever a new instruction
	// is injected into that child.
	nudged map[int]bool

	reviewPath  string
	lastVerdict *relayVerdict

	worktreePath, branch, baseCommit, lastReviewedCommit string

	events    []proto.RelayEvent
	updatedAt time.Time
}

func (run *relayRun) terminalLocked() bool {
	return run.state == relayStateCompleted || run.state == relayStateStopped
}

// currentCLocked is the 1-based C the relay is working on (or just finished).
func (run *relayRun) currentCLocked() int {
	if run.terminalLocked() {
		return run.completedCs
	}
	return run.completedCs + 1
}

func (run *relayRun) hasStrongRoleLocked() bool {
	_, ok := run.roles[relayRoleImplementationStrong]
	return ok
}

func (run *relayRun) childIDLocked(role string) int {
	switch role {
	case relayRoleImplementation:
		return run.implID
	case relayRoleImplementationStrong:
		return run.strongID
	case relayRoleReview:
		return run.reviewID
	}
	return 0
}

func (run *relayRun) childIDsLocked() []int {
	seen := make(map[int]struct{}, 3)
	ids := make([]int, 0, 3)
	for _, id := range []int{run.implID, run.strongID, run.reviewID} {
		if id <= 0 {
			continue
		}
		if _, exists := seen[id]; exists {
			continue
		}
		seen[id] = struct{}{}
		ids = append(ids, id)
	}
	return ids
}

func (run *relayRun) roleOfLocked(childID int) string {
	switch {
	case childID == 0:
		return ""
	case childID == run.implID:
		return relayRoleImplementation
	case childID == run.strongID:
		return relayRoleImplementationStrong
	case childID == run.reviewID:
		return relayRoleReview
	}
	return ""
}

func relayIsImplementerRole(role string) bool {
	return role == relayRoleImplementation || role == relayRoleImplementationStrong
}

// relayResolveRoleExecutionMode decides the execution mode one relay role runs
// in, once, at start (子 plan 内部 C4).
//
// The role's own value wins; an unset role falls back to
// orchestration.relay_execution_mode, which is unset by default and therefore
// leaves the role exactly as it ran before this mode existed.
//
// unattended is always true here, and that is the whole difference from
// applyChildExecutionMode: a relay worker has nobody watching it no matter who
// pressed the button that started the relay, so `auto` on a headless-capable
// provider means headless even when a human started the run from the screen.
func relayResolveRoleExecutionMode(requested, provider string, cfg *config.Config) (string, error) {
	mode := config.NormalizeExecutionMode(requested)
	if mode == config.ExecutionModeUnset && cfg != nil {
		mode = cfg.Orchestration.RelayExecutionModeDefault()
	}
	return config.ResolveExecutionMode(mode, headlessCapableProvider(provider, cfg), config.LaunchOriginConductor, true)
}

// roleHeadlessLocked reports whether this role's children run non-interactively.
// It reads the resolved mode startRelay stored on the assignment, so there is
// one answer per role for the life of the relay — including after a restore,
// because the roles are what relay.json persists.
func (run *relayRun) roleHeadlessLocked(role string) bool {
	return config.IsHeadlessExecutionMode(run.roles[role].ExecutionMode)
}

// anyHeadlessRoleLocked reports whether the relay has any headless role at all.
// Used by the restore path, which cannot wait for children that have no way to
// come back.
func (run *relayRun) anyHeadlessRoleLocked() bool {
	for role := range run.roles {
		if run.roleHeadlessLocked(role) {
			return true
		}
	}
	return false
}

// awaitedChildLocked is the child whose DONE the current state waits for.
// Idle / timeout handling applies to this child only: the others are
// legitimately silent while they wait for their next instruction.
func (run *relayRun) awaitedChildLocked() int {
	switch run.state {
	case relayStateImplementing, relayStateFixing:
		return run.childIDLocked(run.activeImpl)
	case relayStateReviewing:
		return run.reviewID
	}
	return 0
}

// progressIDLocked is the ID the child of role writes into its progress file.
func (run *relayRun) progressIDLocked(role string) int {
	switch role {
	case relayRoleImplementation:
		if run.implProgressID != 0 {
			return run.implProgressID
		}
	case relayRoleImplementationStrong:
		if run.strongProgressID != 0 {
			return run.strongProgressID
		}
	case relayRoleReview:
		if run.reviewProgressID != 0 {
			return run.reviewProgressID
		}
	}
	return run.childIDLocked(role)
}

func (run *relayRun) doneBaselineLocked(role string) int {
	switch role {
	case relayRoleImplementation:
		return run.implDoneBaseline
	case relayRoleImplementationStrong:
		return run.strongDoneBaseline
	case relayRoleReview:
		return run.reviewDoneBaseline
	}
	return 0
}

func (run *relayRun) setDoneBaselineLocked(role string, count int) {
	switch role {
	case relayRoleImplementation:
		run.implDoneBaseline = count
	case relayRoleImplementationStrong:
		run.strongDoneBaseline = count
	case relayRoleReview:
		run.reviewDoneBaseline = count
	}
}

// reviewFileLocked is the review file for the given round of the current C.
// After a hand-over to the strong implementer the round counter restarts at 0
// (D-22), so those files carry a `strong` segment to keep the earlier rounds'
// files (and the timeline that points at them) intact.
func (run *relayRun) reviewFileLocked(round int) string {
	return relayReviewFile(run.boardDir, run.currentCLocked(), round, run.activeImpl == relayRoleImplementationStrong)
}

func (run *relayRun) statusLocked() proto.RelayStatus {
	st := proto.RelayStatus{
		OrchestrationID:         run.orchestrationID,
		PlanPath:                run.planPath,
		Mode:                    run.mode,
		State:                   run.state,
		Reason:                  run.reason,
		CompletedCs:             run.completedCs,
		Round:                   run.round,
		MaxRounds:               run.maxRounds,
		FinalSeen:               run.finalSeen,
		ImplementationSessionID: run.implID,
		StrongSessionID:         run.strongID,
		ActiveImplementer:       run.activeImpl,
		EscalateAfter:           run.escalateAfter,
		ReviewSessionID:         run.reviewID,
		ReviewPath:              run.reviewPath,
		WorktreePath:            run.worktreePath,
		Branch:                  run.branch,
		BaseCommit:              run.baseCommit,
	}
	if !run.updatedAt.IsZero() {
		st.UpdatedAt = run.updatedAt.Format(time.RFC3339)
	}
	return st
}

func (run *relayRun) eventsLocked() []proto.RelayEvent {
	return append([]proto.RelayEvent(nil), run.events...)
}

// relayReviewFile is the conventional review file for C c, round r.
func relayReviewFile(boardDir string, c, round int, strong bool) string {
	if strong {
		return filepath.Join(boardDir, fmt.Sprintf("review-c%d-strong-r%d.md", c, round))
	}
	return filepath.Join(boardDir, fmt.Sprintf("review-c%d-r%d.md", c, round))
}

// relayShortID shortens `r<parent>-<unix nanoseconds>` to `r<parent>-<last 6
// digits>` for the message tag; the full ID stays the key everywhere else.
func relayShortID(orchestrationID string) string {
	i := strings.LastIndex(orchestrationID, "-")
	if i < 0 {
		return orchestrationID
	}
	tail := orchestrationID[i+1:]
	if len(tail) <= 6 {
		return orchestrationID
	}
	return orchestrationID[:i+1] + tail[len(tail)-6:]
}

// tagLocked is the prefix of every text the Hub sends for this relay (D-23).
func (run *relayRun) tagLocked() string {
	return fmt.Sprintf("[relay %s %s]", relayShortID(run.orchestrationID), filepath.Base(run.planPath))
}

// ---- manager access -------------------------------------------------------

// relayMeta is the part of a run that listing and the child budget need
// without taking the run's mutex. relayPublishMetaLocked mirrors it under
// orchestration.mu whenever the source fields change (start, spawn, every
// save, attach); a transition that holds run.mu can therefore consult the
// budget of the parent's other relays without touching their mutexes.
type relayMeta struct {
	seq      uint64
	parentID int
	attached bool
	active   bool
	spawned  int // implementation + review children already spawned
}

// relayPublishMetaLocked mirrors the run into orchestration.relayMeta. The
// caller holds run.mu.
func (s *Server) relayPublishMetaLocked(run *relayRun) {
	spawned := 0
	if run.implID != 0 {
		spawned++
	}
	if run.reviewID != 0 {
		spawned++
	}
	s.orchestration.mu.Lock()
	s.orchestration.relayMeta[run.orchestrationID] = relayMeta{
		seq:      run.seq,
		parentID: run.parentID,
		attached: run.parentAttached,
		active:   !run.terminalLocked(),
		spawned:  spawned,
	}
	s.orchestration.mu.Unlock()
}

// relaysForParent returns the parent's relays in start order. A restored relay
// whose conductor has not re-identified itself yet is left out: its parentID
// is a number from the previous Hub run and may belong to someone else now.
func (s *Server) relaysForParent(parentID int) []*relayRun {
	type entry struct {
		run *relayRun
		seq uint64
	}
	s.orchestration.mu.Lock()
	entries := make([]entry, 0, len(s.orchestration.relays))
	for id, meta := range s.orchestration.relayMeta {
		if meta.parentID != parentID || !meta.attached {
			continue
		}
		if run := s.orchestration.relays[id]; run != nil {
			entries = append(entries, entry{run: run, seq: meta.seq})
		}
	}
	s.orchestration.mu.Unlock()
	sort.Slice(entries, func(i, j int) bool { return entries[i].seq < entries[j].seq })
	runs := make([]*relayRun, 0, len(entries))
	for _, e := range entries {
		runs = append(runs, e.run)
	}
	return runs
}

// relayRolesForParent returns the role mapping of an attached relay belonging
// to parentID. The parent session's OrchestrationID points at only the most
// recently started relay, so the mapping must not rely on that mutable field
// when a parent starts a second relay.
func (s *Server) relayRolesForParent(parentID int) map[string]orchestrationRoleAssignment {
	for _, run := range s.relaysForParent(parentID) {
		run.mu.Lock()
		roles := make(map[string]orchestrationRoleAssignment, len(run.roles))
		for role, assignment := range run.roles {
			roles[role] = assignment
		}
		run.mu.Unlock()
		if len(roles) > 0 {
			return roles
		}
	}
	return nil
}

func (s *Server) relayByID(orchestrationID string) *relayRun {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	return s.orchestration.relays[orchestrationID]
}

// relayOwns reports whether the board belongs to a relay. It is the single
// branch point of the generic orchestration scan / timers / session-end
// handling (D-6: no generic DONE processing and no automatic notices there).
func (s *Server) relayOwns(boardID string) bool {
	return s.relayByID(boardID) != nil
}

func (s *Server) relayTerminal(boardID string) bool {
	run := s.relayByID(boardID)
	if run == nil {
		return false
	}
	run.mu.Lock()
	defer run.mu.Unlock()
	return run.terminalLocked()
}

// relayBudgetForParent returns how many relays are in progress for the parent
// and how many child slots they still have to spawn. exclude (may be nil) is
// the run the caller is working on (and holds); its own reservation is left
// out. Only relayMeta is read, so no run mutex is taken.
func (s *Server) relayBudgetForParent(parentID int, exclude *relayRun) (running, reserved int) {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	for id, meta := range s.orchestration.relayMeta {
		if meta.parentID != parentID || !meta.attached || !meta.active {
			continue
		}
		if exclude != nil && id == exclude.orchestrationID {
			continue
		}
		running++
		if left := relayChildrenPerRun - meta.spawned; left > 0 {
			reserved += left
		}
	}
	return running, reserved
}

// resolveRelay finds the relay addressed by orchestrationID. An empty ID is
// accepted only when the parent has exactly one relay in progress.
func (s *Server) resolveRelay(parentID int, orchestrationID string) (*relayRun, error) {
	if id := strings.TrimSpace(orchestrationID); id != "" {
		s.orchestration.mu.Lock()
		run := s.orchestration.relays[id]
		meta := s.orchestration.relayMeta[id]
		s.orchestration.mu.Unlock()
		if run == nil || meta.parentID != parentID || !meta.attached {
			return nil, errRelayNotFound
		}
		return run, nil
	}
	var active []*relayRun
	for _, run := range s.relaysForParent(parentID) {
		run.mu.Lock()
		terminal := run.terminalLocked()
		run.mu.Unlock()
		if !terminal {
			active = append(active, run)
		}
	}
	switch len(active) {
	case 0:
		return nil, errRelayNotFound
	case 1:
		return active[0], nil
	default:
		return nil, errRelayAmbiguous
	}
}

// relayStatusesFor returns the parent's relays (all states) in start order.
func (s *Server) relayStatusesFor(parentID int) []proto.RelayStatus {
	runs := s.relaysForParent(parentID)
	out := make([]proto.RelayStatus, 0, len(runs))
	for _, run := range runs {
		run.mu.Lock()
		out = append(out, run.statusLocked())
		run.mu.Unlock()
	}
	return out
}

// relayEventsFor returns the timeline of one relay. With an empty ID the
// parent's only in-progress relay is used.
func (s *Server) relayEventsFor(parentID int, orchestrationID string) []proto.RelayEvent {
	run, err := s.resolveRelay(parentID, orchestrationID)
	if err != nil {
		return nil
	}
	run.mu.Lock()
	defer run.mu.Unlock()
	return run.eventsLocked()
}

// ---- start / stop ---------------------------------------------------------

func (s *Server) startRelay(parentID int, req relayStartRequest) (*proto.RelayStatus, error) {
	deps := s.relayDep()
	planPath := strings.TrimSpace(req.PlanPath)
	if planPath == "" || !filepath.IsAbs(planPath) {
		return nil, errRelayPlanPath
	}
	mode := strings.TrimSpace(req.Mode)
	if mode == "" {
		mode = relayModeWorktree
	}
	if mode != relayModeWorktree && mode != relayModeSameTree {
		return nil, errRelayMode
	}
	maxRounds := req.MaxRounds
	if maxRounds <= 0 {
		maxRounds = relayDefaultMaxRounds
	}
	escalateAfter := req.EscalateAfter
	if escalateAfter <= 0 {
		escalateAfter = relayDefaultEscalateAfter
	}
	fullCfg := s.snapshotCfg()
	roles := map[string]orchestrationRoleAssignment{}
	for _, role := range []string{relayRoleImplementation, relayRoleReview, relayRoleImplementationStrong} {
		ra, ok := req.Roles[role]
		if !ok && role == relayRoleImplementationStrong {
			continue // optional (D-22)
		}
		ra.Provider = strings.TrimSpace(ra.Provider)
		ra.Model = strings.TrimSpace(ra.Model)
		if !ok || !validOrchestrationProvider(ra.Provider) || !spawnValidModelLabel(ra.Model) || strings.HasPrefix(ra.Model, "-") {
			return nil, errRelayRolesMissing{Role: role}
		}
		mode, modeErr := relayResolveRoleExecutionMode(ra.ExecutionMode, ra.Provider, fullCfg)
		if modeErr != nil {
			return nil, errRelayExecutionMode{Role: role, Provider: ra.Provider, Err: modeErr}
		}
		// The **resolved** mode is stored back onto the assignment, which is what
		// every later decision reads: which prompt route a spawn takes, whether an
		// exit means "the relay's child died" or "that instruction finished", and —
		// because roles are persisted in relay.json — what a restored relay knows
		// about children that cannot come back.
		ra.ExecutionMode = mode
		roles[role] = ra
	}
	parent, _, _ := s.orchestrationParentState(parentID)
	if parent == nil {
		return nil, errRelayParentNotFound
	}
	cfg := fullCfg.Orchestration
	running, _ := s.relayBudgetForParent(parentID, nil)
	if parent.Depth >= cfg.MaxDepth {
		return nil, errOrchestrationLimit{Limit: "depth", Running: running, Max: cfg.MaxDepth}
	}
	admissionID, admissionErr := s.reserveOrchestrationChildren(parentID, relayChildrenPerRun, cfg, "relay-start")
	if admissionErr != nil {
		if errors.Is(admissionErr, errOrchestrationAdmissionParentNotFound) {
			return nil, errRelayParentNotFound
		}
		return nil, admissionErr
	}
	admissionKept := false
	defer func() {
		if !admissionKept {
			s.releaseOrchestrationChildren(admissionID)
		}
	}()

	now := deps.now()
	// The relay allocates its own orchestration ID (D-21). The parent's
	// existing OrchestrationID is never reused: it would make two relays of the
	// same parent share one board directory.
	orchestrationID := fmt.Sprintf("r%d-%d", parentID, now.UnixNano())
	childCWD := strings.TrimSpace(req.ChildCWD)
	if childCWD == "" {
		childCWD = parent.CWD
	}
	worktreePath, branch, base := "", "", ""
	if mode == relayModeWorktree {
		// One shared worktree per relay (D-16), created before the board so a
		// git failure leaves nothing behind. A non-git cwd is an error, never a
		// silent fall-back to the parent's tree: the user chooses same-tree.
		var wtErr error
		worktreePath, branch, base, wtErr = s.prepareRelayWorktree(parent.CWD, orchestrationID, cfg)
		if wtErr != nil {
			return nil, wtErr
		}
		childCWD = worktreePath
	}
	boardPath, err := s.ensureOrchestrationBoard(orchestrationID, parent, spawnChildRequest{InitialPrompt: "relay: " + filepath.Base(planPath)})
	if err != nil {
		return nil, fmt.Errorf("board error: %w", err)
	}
	// markConductor makes the parent card a conductor. With several relays the
	// parent's OrchestrationID ends up as the last one started; relays are
	// identified by their own ID, never through that field.
	s.markConductor(parentID, orchestrationID, boardPath)
	s.registerBoardSession(orchestrationID, boardPath, parentID, "conductor")

	run := &relayRun{
		orchestrationID: orchestrationID,
		admissionID:     admissionID,
		boardPath:       boardPath,
		boardDir:        filepath.Dir(boardPath),
		parentID:        parentID,
		parentStartedAt: parent.StartedAt,
		parentProvider:  parent.Provider,
		parentCWD:       parent.CWD,
		parentAttached:  true,
		reconnected:     map[string]bool{},
		planPath:        planPath,
		mode:            mode,
		maxRounds:       maxRounds,
		escalateAfter:   escalateAfter,
		activeImpl:      relayRoleImplementation,
		roles:           roles,
		extra:           copyStringMap(req.Extra),
		childCWD:        childCWD,
		worktreePath:    worktreePath,
		branch:          branch,
		baseCommit:      base,
		nudged:          map[int]bool{},
		updatedAt:       now,
	}
	// The base commit is recorded in both modes (the review scope and the
	// resume prompt refer to it). A failure (not a git repository) leaves it
	// empty and the relay proceeds with working-tree scopes.
	if run.baseCommit == "" {
		if head, headErr := deps.gitHead(childCWD); headErr == nil {
			run.baseCommit = strings.TrimSpace(head)
		}
	}
	run.lastReviewedCommit = run.baseCommit

	s.orchestration.mu.Lock()
	s.orchestration.relaySeq++
	run.seq = s.orchestration.relaySeq
	s.orchestration.relays[orchestrationID] = run
	s.orchestration.mu.Unlock()

	run.mu.Lock()
	defer run.mu.Unlock()
	s.relayPublishMetaLocked(run)
	impl, review := roles[relayRoleImplementation], roles[relayRoleReview]
	strongText := "none"
	if strong, ok := roles[relayRoleImplementationStrong]; ok {
		strongText = strong.Provider + "/" + strong.Model
	}
	s.relayBoardLocked(run, fmt.Sprintf("relay started plan=%s mode=%s impl=%s/%s strong=%s review=%s/%s max_rounds=%d escalate_after=%d base=%s",
		planPath, mode, impl.Provider, impl.Model, strongText, review.Provider, review.Model, maxRounds, escalateAfter, run.baseCommit))
	s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventStarted, C: 1, Text: filepath.Base(planPath), Commit: run.baseCommit})
	if err := s.relaySpawn(run, relayRoleImplementation, s.relayImplementationPrompt(run)); err != nil {
		s.relayFinishLocked(run, relayStateStopped, relayReasonSpawnError, err.Error())
		st := run.statusLocked()
		return &st, fmt.Errorf("relay spawn: %w", err)
	}
	s.relayTransitionLocked(run, relayStateImplementing, "")
	admissionKept = true
	st := run.statusLocked()
	return &st, nil
}

// stopRelay stops a relay on the user's request. The children stay alive.
func (s *Server) stopRelay(parentID int, orchestrationID, reason string) error {
	run, err := s.resolveRelay(parentID, orchestrationID)
	if err != nil {
		return err
	}
	if strings.TrimSpace(reason) == "" {
		reason = relayReasonUserStop
	}
	if !s.relayFinish(run, relayStateStopped, reason, "") {
		return errRelayNotRunning
	}
	return nil
}

// ---- children -------------------------------------------------------------

// relaySpawn starts the child for role and records it on the run. The child's
// worktree is the relay's shared cwd (never the per-role worktree of the AI
// conductor mode); the branch line of buildChildInitialPrompt is left out and
// the relay prompt names the branch itself.
func (s *Server) relaySpawn(run *relayRun, role, prompt string) error {
	admissionID := run.admissionID
	if !s.orchestrationAdmissionMatches(admissionID, run.parentID, 1) {
		var err error
		admissionID, err = s.reserveOrchestrationChildren(run.parentID, 1, s.snapshotCfg().Orchestration, "relay-child")
		if err != nil {
			return err
		}
	}
	if err := s.relaySpawnWithAdmission(run, role, prompt, admissionID); err != nil {
		s.releaseOrchestrationChildren(admissionID)
		return err
	}
	return nil
}

func (s *Server) relaySpawnWithAdmission(run *relayRun, role, prompt, admissionID string) error {
	deps := s.relayDep()
	ra := run.roles[role]
	// 起動要求の共通 3 項目は役割の割り当てから写すだけ（子 plan:
	// docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C3）。
	// 承認系フィールドはここでは埋めない: relay の子は無人なので、権限の段は
	// 役割の permission_preset か orchestration.child_permission_default から
	// resolveChildPermission が決める（internal/hub/child_permission.go）。
	// Origin は空＝無人（画面から人が起動したものではない）。
	body := spawnChildRequest{
		Role: role, Provider: ra.Provider, Model: ra.Model, InitialPrompt: prompt,
		CWD: run.childCWD, Force: true, SubscriptionProfileID: ra.Subscription,
		Effort: ra.Effort, ExecutionMode: ra.ExecutionMode, PermissionPreset: ra.PermissionPreset,
	}
	applyChildPermission(&body, s.snapshotCfg().Orchestration)
	prep := childSpawnPreparation{orchestrationID: run.orchestrationID, boardPath: run.boardPath, childCWD: run.childCWD}
	parent, _, _ := s.orchestrationParentState(run.parentID)
	if parent == nil {
		return errRelayParentNotFound
	}
	res, err := deps.spawnChild(run.parentID, parent, body, prep)
	if err != nil {
		return err
	}
	if strings.TrimSpace(admissionID) != "" {
		s.consumeOrchestrationChildren(admissionID, 1)
	}
	label := s.sessionLabel(res.ID)
	// The progress ID is cleared along with the rest: a child that has just been
	// started writes child-<its own live id>.md. A non-zero progress ID only ever
	// means "this child was renumbered when it reattached", which cannot be true
	// of one that did not exist a moment ago.
	switch role {
	case relayRoleImplementation:
		run.implID, run.implLabel, run.implDoneBaseline, run.implProgressID = res.ID, label, 0, 0
	case relayRoleImplementationStrong:
		run.strongID, run.strongLabel, run.strongDoneBaseline, run.strongProgressID = res.ID, label, 0, 0
	case relayRoleReview:
		run.reviewID, run.reviewLabel, run.reviewDoneBaseline, run.reviewProgressID = res.ID, label, 0, 0
	}
	run.nudged[res.ID] = false
	s.relayPublishMetaLocked(run)
	s.relayBoardLocked(run, fmt.Sprintf("relay spawned role=%s session=%d provider=%s model=%s cwd=%s", role, res.ID, body.Provider, body.Model, run.childCWD))
	return nil
}

// relayInject sends the next instruction to a live child. The DONE baseline
// is taken from the progress file right before injecting, so only DONE lines
// written after this instruction count as its completion (D-7).
func (s *Server) relayInject(run *relayRun, role, text string) error {
	deps := s.relayDep()
	id := run.childIDLocked(role)
	if id == 0 {
		return errRelayNoChild
	}
	progressID := run.progressIDLocked(role)
	count, _, _ := countDoneLines(readFileOrEmpty(childProgressPath(run.boardPath, progressID)), role, progressID)
	run.setDoneBaselineLocked(role, count)
	run.nudged[id] = false
	safe := sanitizeInjectText(text)
	// The board record comes first so a child that reads the board on
	// notification already finds the instruction there (same order as send-child).
	s.relayBoardLocked(run, fmt.Sprintf("@%s session=%d への指示:\n%s", role, id, safe))
	s.relayRearmChildTimers(run.orchestrationID, id)
	deps.inject(id, "\n"+safe+"\n")
	return nil
}

// relayDispatch hands a role its next instruction, whichever shape its children
// have (子 plan 内部 C4).
//
//	interactive  the child is alive and waiting; the text is typed into it.
//	headless     the child ran one process for one instruction and has already
//	             exited. The next instruction starts a **new** process with that
//	             text as its prompt — one instruction, one process, and its exit
//	             is what says the instruction is over.
//
// The interactive branch is relayInject unchanged, so a relay with no headless
// role behaves byte for byte as it did before this existed.
func (s *Server) relayDispatch(run *relayRun, role, text string) error {
	if !run.roleHeadlessLocked(role) {
		return s.relayInject(run, role, text)
	}
	prompt := s.relayHeadlessInstruction(run, text)
	previous := run.childIDLocked(role)
	s.relayBoardLocked(run, fmt.Sprintf("@%s への指示（headless: 新しいプロセスを立てる。前の子 #%d は終了済み）:\n%s",
		role, previous, sanitizeInjectText(text)))
	if previous == 0 {
		return s.relaySpawn(run, role, prompt)
	}
	return s.relayRespawn(run, role, prompt)
}

// relayRespawn replaces the process of a role the relay already holds, without
// taking an admission slot.
//
// The slot was taken when that role's first child started, and the child being
// replaced has already exited — which is the only reason a replacement is
// happening at all. So a replacement adds no live session, and live sessions are
// what the children-per-parent budget is there to bound.
//
// Asking reserveOrchestrationChildren instead would compare against every child
// this parent has ever had, finished ones included (sessions stay in the list
// until they are dismissed). A headless relay runs one process per instruction,
// so that count only goes up, and a relay would run out of budget after a
// handful of rounds while never having more than one worker alive.
func (s *Server) relayRespawn(run *relayRun, role, prompt string) error {
	return s.relaySpawnWithAdmission(run, role, prompt, "")
}

// relayHeadlessInstruction wraps one instruction for a process that is about to
// be started for it alone.
//
// The relay's continuation texts (proceed / fix / re-review / hand-over) were
// written for a child that has been in the conversation since the relay
// started: they say "the same rules" and "continue" because the standing rules
// arrived once, in the spawn prompt. A headless run has no such history — it is
// a new process with an empty context — so what an interactive child was told
// at spawn has to travel with every instruction instead.
//
// The instruction itself is not rewritten. Two sets of relay texts, one per
// execution mode, would drift apart; one preamble in front of the existing text
// cannot.
func (s *Server) relayHeadlessInstruction(run *relayRun, text string) string {
	preamble := relayJoin(
		run.tagLocked(),
		"You are a relay worker started for this one instruction. You have no memory of earlier rounds of this relay, and you will exit when this instruction is finished — that exit is how the relay knows you are done.",
		"The plan is at "+run.planPath+".",
		run.worktreeLineLocked(),
		"Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the plan before changing anything.",
		run.resyncLineLocked(),
		"Your instruction follows.",
	)
	return sanitizeInjectText(preamble) + "\n\n" + text
}

// relayRearmChildTimers clears the idle / timeout latches of a child that is
// being handed new work. While a child waits for its next instruction the
// generic timers fire for it and latch (TimedOut / IdleWarned); the relay
// ignores those for the non-awaited child, so the latches must be released
// when that child becomes the awaited one again. LastBoardWrite is bumped so
// the timeout clock starts from the instruction, not from the child's last
// activity before the wait.
func (s *Server) relayRearmChildTimers(boardID string, childID int) {
	now := time.Now()
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	b := s.orchestration.boards[boardID]
	if b == nil {
		return
	}
	delete(b.TimedOut, childID)
	delete(b.IdleWarned, childID)
	if child := b.Children[childID]; child != nil {
		child.LastBoardWrite = now
	}
}

func (s *Server) sessionLabel(id int) string {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if ses := s.sessions[id]; ses != nil {
		return ses.Label
	}
	return ""
}

func readFileOrEmpty(path string) string {
	data, err := os.ReadFile(path)
	if err != nil {
		return ""
	}
	return string(data)
}

// ---- progress file parsing ------------------------------------------------

// countDoneLines counts the `## DONE <role>` / `## SUCCESS <role>` lines of one
// child's progress file. Unlike detectBoardDoneEvents it does not de-duplicate:
// the relay judges completion by the count growing past the baseline taken
// when the instruction was sent. A line whose session= names another session
// is ignored; a line without session= belongs to the file's owner. lastFinal
// and lastEscalate report the `final=true` / `escalate=true` tokens of the
// last counted line.
func countDoneLines(text, role string, sessionID int) (count int, lastFinal, lastEscalate bool) {
	role = sanitizeRole(role)
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		var payload string
		switch {
		case strings.HasPrefix(line, "## DONE "):
			payload = strings.TrimPrefix(line, "## DONE ")
		case strings.HasPrefix(line, "## SUCCESS "):
			payload = strings.TrimPrefix(line, "## SUCCESS ")
		default:
			continue
		}
		fields := strings.Fields(payload)
		if len(fields) == 0 || sanitizeRole(fields[0]) != role {
			continue
		}
		if id := parseBoardSessionID(fields[1:]); id != 0 && id != sessionID {
			continue
		}
		count++
		lastFinal, lastEscalate = false, false
		for _, f := range fields[1:] {
			switch {
			case strings.EqualFold(f, "final=true"):
				lastFinal = true
			case strings.EqualFold(f, "escalate=true"):
				lastEscalate = true
			}
		}
	}
	return count, lastFinal, lastEscalate
}

// parseRelayVerdict reads the reviewer's `verdict:` line (D-14).
//
//	verdict: pass
//	verdict: findings must=<n> should=<m> file=<path>
//	verdict: blocked reason=<text to end of line>
//
// There must be at least as many verdict lines as DONE lines (one per review);
// the last one is used. `file=` runs to the end of the line (paths may contain
// spaces), a relative path is resolved against boardDir, and the file must lie
// inside boardDir. A `findings` line without must= is read as must>0 unless it
// names should= only (then must=0, which counts as pass). With must>0 the
// review file must exist. defaultFile is used when `file=` is absent.
func parseRelayVerdict(text string, doneCount int, boardDir, defaultFile string) (relayVerdict, error) {
	var lines []string
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if len(line) >= len("verdict:") && strings.EqualFold(line[:len("verdict:")], "verdict:") {
			lines = append(lines, strings.TrimSpace(line[len("verdict:"):]))
		}
	}
	if len(lines) == 0 || len(lines) < doneCount {
		return relayVerdict{}, errVerdictMissing
	}
	raw := lines[len(lines)-1]
	fields := strings.Fields(raw)
	if len(fields) == 0 {
		return relayVerdict{}, errVerdictMissing
	}
	v := relayVerdict{Kind: strings.ToLower(fields[0])}
	switch v.Kind {
	case "pass":
	case "findings":
		mustSeen, shouldSeen := false, false
		for _, f := range fields[1:] {
			lf := strings.ToLower(f)
			switch {
			case strings.HasPrefix(lf, "must="):
				if n, err := strconv.Atoi(strings.TrimPrefix(lf, "must=")); err == nil && n >= 0 {
					v.Must, mustSeen = n, true
				}
			case strings.HasPrefix(lf, "should="):
				if n, err := strconv.Atoi(strings.TrimPrefix(lf, "should=")); err == nil && n >= 0 {
					v.Should, shouldSeen = n, true
				}
			}
		}
		if !mustSeen {
			// Safe side: findings without a must count are treated as blocking
			// unless the reviewer explicitly reported should-only items.
			if shouldSeen {
				v.Must = 0
			} else {
				v.Must = 1
			}
		}
	case "blocked":
		if i := strings.Index(strings.ToLower(raw), "reason="); i >= 0 {
			v.Reason = strings.TrimSpace(raw[i+len("reason="):])
		}
		return v, nil
	default:
		return relayVerdict{}, errVerdictMissing
	}
	v.File = defaultFile
	if i := strings.Index(strings.ToLower(raw), "file="); i >= 0 {
		file := strings.TrimSpace(raw[i+len("file="):])
		// Tolerate file= not being the last token.
		for _, stop := range []string{" must=", " should="} {
			if j := strings.Index(strings.ToLower(file), stop); j >= 0 {
				file = strings.TrimSpace(file[:j])
			}
		}
		file = strings.Trim(file, "`\"'")
		if file != "" {
			if !filepath.IsAbs(file) {
				file = filepath.Join(boardDir, file)
			}
			v.File = filepath.Clean(file)
		}
	}
	if !relayPathInside(boardDir, v.File) {
		return relayVerdict{}, errVerdictMissing
	}
	if v.Kind == "findings" && v.Must > 0 {
		if info, err := os.Stat(v.File); err != nil || info.IsDir() {
			return v, errReviewFileMissing
		}
	}
	return v, nil
}

// relayPathInside reports whether path is boardDir or below it.
func relayPathInside(boardDir, path string) bool {
	rel, err := filepath.Rel(filepath.Clean(boardDir), filepath.Clean(path))
	if err != nil || filepath.IsAbs(rel) {
		return false
	}
	return rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator))
}

// ---- transitions ----------------------------------------------------------

// relayOnChildFileChange is called by the board scan (orchestration.go) with
// the full text of a relay child's progress file whenever it changed. Only the
// awaited role advances the machine; writes of the other children are
// recorded on the board (a DONE from the waiting implementer) or ignored.
func (s *Server) relayOnChildFileChange(boardID string, childID int, text string) {
	run := s.relayByID(boardID)
	if run == nil {
		return
	}
	run.mu.Lock()
	defer run.mu.Unlock()
	// While a restored relay waits for its children to reattach it cannot
	// inject anything; the file is re-read once they are back (checkRelayReconnect).
	if run.terminalLocked() || run.awaitingReconnect {
		return
	}
	s.relayProcessChildTextLocked(run, childID, text)
}

// relayProcessChildTextLocked is the transition body of relayOnChildFileChange.
func (s *Server) relayProcessChildTextLocked(run *relayRun, childID int, text string) {
	s.relayAdvanceChildLocked(run, childID, text, false)
}

// relayAdvanceChildLocked advances the machine from one child's progress file.
//
// exited is set when the child's process has already ended, which only happens
// on the headless route: there, the exit *is* the completion signal (元設計 11
// 節), so a work unit that produced no new `## DONE` line still counts as
// finished rather than leaving the relay waiting for a line that can never
// arrive. On the interactive route exited is always false and every branch below
// behaves exactly as it did before.
func (s *Server) relayAdvanceChildLocked(run *relayRun, childID int, text string, exited bool) {
	role := run.roleOfLocked(childID)
	if role == "" {
		return
	}
	count, final, escalate := countDoneLines(text, role, run.progressIDLocked(role))
	if count <= run.doneBaselineLocked(role) {
		if !exited {
			return
		}
		// `escalate=true` is a token on a DONE line. There is no such line here,
		// so the hand-over request cannot be one that was actually made.
		escalate = false
	}
	switch run.state {
	case relayStateImplementing, relayStateFixing:
		if !relayIsImplementerRole(role) {
			return
		}
		if role != run.activeImpl {
			// The implementer that is waiting for its next instruction wrote a
			// DONE anyway. Record it so the timeline explains the file, but the
			// C belongs to the active implementer (D-22).
			run.setDoneBaselineLocked(role, count)
			s.relayBoardLocked(run, fmt.Sprintf("ignored DONE from waiting implementer role=%s session=%d (active=%s)", role, childID, run.activeImpl))
			return
		}
		run.setDoneBaselineLocked(role, count)
		if run.state == relayStateImplementing && role == relayRoleImplementation && escalate {
			// The plan marks this C [strong]: hand it over before anything is
			// implemented. finalSeen is kept so a [strong] last C still
			// completes the relay after it passes.
			run.finalSeen = run.finalSeen || final
			if s.relayHandToStrong(run, "implement", relayVerdict{}) {
				s.relayTransitionLocked(run, relayStateImplementing, "")
				return
			}
			if err := s.relayDispatch(run, relayRoleImplementation, s.relaySelfImplementText(run)); err != nil {
				s.relayFinishLocked(run, relayStateStopped, relayReasonSpawnError, err.Error())
				return
			}
			s.relayTransitionLocked(run, relayStateImplementing, "")
			return
		}
		s.relayImplDoneLocked(run, final)
	case relayStateReviewing:
		if role != relayRoleReview {
			return
		}
		run.setDoneBaselineLocked(role, count)
		s.relayReviewDoneLocked(run, text, count)
	}
}

// relayImplDoneLocked: the active implementer finished a C (implementing) or a
// fix (fixing). Either way the reviewer gets the diff.
func (s *Server) relayImplDoneLocked(run *relayRun, final bool) {
	deps := s.relayDep()
	fixed := run.state == relayStateFixing
	if fixed {
		run.round++
		run.finalSeen = run.finalSeen || final
	} else {
		run.round = 1
		run.finalSeen = run.finalSeen || final
	}
	c := run.currentCLocked()
	// Timeline detail (D-20): in worktree mode the C is a commit range, so the
	// event carries HEAD and the files changed since the last reviewed commit.
	// In same-tree mode nothing is committed; only the working-tree change
	// count is meaningful.
	commit, files := "", 0
	if run.mode == relayModeWorktree {
		if head, err := deps.gitHead(run.childCWD); err == nil {
			commit = strings.TrimSpace(head)
		}
		if n, err := deps.gitChangedFiles(run.childCWD, run.lastReviewedCommit); err == nil {
			files = n
		}
	} else if n, err := deps.gitChangedFiles(run.childCWD, ""); err == nil {
		files = n
	}
	text := fmt.Sprintf("C%d done by %s", c, run.activeImpl)
	if fixed {
		text = fmt.Sprintf("C%d fix done by %s (round %d)", c, run.activeImpl, run.round-1)
	}
	if run.finalSeen {
		text += " final=true"
	}
	s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventCDone, C: c, Round: run.round, Text: text, Commit: commit, FilesChanged: files})
	run.reviewPath = run.reviewFileLocked(run.round)
	if run.reviewID == 0 {
		if err := s.relaySpawn(run, relayRoleReview, s.relayReviewPrompt(run)); err != nil {
			s.relayFinishLocked(run, relayStateStopped, relayReasonSpawnError, err.Error())
			return
		}
	} else {
		// The reviewer is reused across Cs and rounds (D-7). A fresh C gets the
		// full review instruction again; only a fix round gets the re-review text
		// that points at the previous review file.
		text := s.relayReviewPrompt(run)
		if fixed {
			text = s.relayReReviewText(run)
		}
		if err := s.relayDispatch(run, relayRoleReview, text); err != nil {
			s.relayFinishLocked(run, relayStateStopped, relayReasonSpawnError, err.Error())
			return
		}
	}
	s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventReviewStarted, C: c, Round: run.round, ReviewPath: run.reviewPath})
	s.relayTransitionLocked(run, relayStateReviewing, "")
}

// relayReviewDoneLocked: the reviewer wrote its DONE; read the verdict.
func (s *Server) relayReviewDoneLocked(run *relayRun, text string, doneCount int) {
	deps := s.relayDep()
	c := run.currentCLocked()
	v, err := parseRelayVerdict(text, doneCount, run.boardDir, run.reviewFileLocked(run.round))
	if err != nil {
		reason := relayReasonVerdictMissing
		if errors.Is(err, errReviewFileMissing) {
			reason = relayReasonReviewFileMissing
			run.reviewPath = v.File
		}
		s.relayFinishLocked(run, relayStateStopped, reason, err.Error())
		return
	}
	if v.File != "" {
		run.reviewPath = v.File
	}
	vv := v
	run.lastVerdict = &vv
	verdictText := v.Kind
	if v.Kind == "findings" {
		verdictText = fmt.Sprintf("findings must=%d should=%d", v.Must, v.Should)
	} else if v.Kind == "blocked" {
		verdictText = "blocked reason=" + v.Reason
	}
	s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventVerdict, C: c, Round: run.round, Text: verdictText, ReviewPath: run.reviewPath})
	switch {
	case v.Kind == "pass" || (v.Kind == "findings" && v.Must == 0):
		if head, headErr := deps.gitHead(run.childCWD); headErr == nil && strings.TrimSpace(head) != "" {
			run.lastReviewedCommit = strings.TrimSpace(head)
		}
		run.completedCs++
		if run.finalSeen {
			s.relayFinishLocked(run, relayStateCompleted, "", fmt.Sprintf("C%d passed (final)", c))
			return
		}
		// The next C always starts with the cheap implementer; the strong one
		// (if any) waits until it is needed again (D-22).
		run.round = 0
		run.activeImpl = relayRoleImplementation
		if err := s.relayDispatch(run, relayRoleImplementation, s.relayProceedText(run)); err != nil {
			s.relayFinishLocked(run, relayStateStopped, relayReasonSpawnError, err.Error())
			return
		}
		s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventProceedSent, C: run.currentCLocked(), Round: run.round, Text: fmt.Sprintf("C%d passed; proceed to C%d", c, run.currentCLocked())})
		s.relayTransitionLocked(run, relayStateImplementing, "")
	case v.Kind == "findings":
		if run.activeImpl == relayRoleImplementation && run.round >= run.escalateAfter && s.relayHandToStrong(run, "fix", v) {
			s.relayTransitionLocked(run, relayStateFixing, "")
			return
		}
		if run.round >= run.maxRounds {
			s.relayFinishLocked(run, relayStateStopped, relayReasonMaxRounds, fmt.Sprintf("must=%d still open after round %d of %d (%s)", v.Must, run.round, run.maxRounds, run.activeImpl))
			return
		}
		if err := s.relayDispatch(run, run.activeImpl, s.relayFixText(run, v)); err != nil {
			s.relayFinishLocked(run, relayStateStopped, relayReasonSpawnError, err.Error())
			return
		}
		s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventFixSent, C: c, Round: run.round, Text: verdictText + " → " + run.activeImpl, ReviewPath: run.reviewPath})
		s.relayTransitionLocked(run, relayStateFixing, "")
	default: // blocked
		s.relayFinishLocked(run, relayStateStopped, relayReasonBlocked, v.Reason)
	}
}

// relayHandToStrong hands the current C to the strong implementer (D-22).
// kind is "implement" (plan hint) or "fix" (review failures; v is the verdict).
// It returns false — and the caller falls back to the ordinary path — when no
// strong role is configured, when the child budget has no free slot for it, or
// when its spawn fails. The board records why.
func (s *Server) relayHandToStrong(run *relayRun, kind string, v relayVerdict) bool {
	if !run.hasStrongRoleLocked() {
		return false
	}
	reason := relayEscalateReviewFailures
	if kind == "implement" {
		reason = relayEscalatePlanHint
	}
	if run.strongID == 0 {
		cfg := s.snapshotCfg().Orchestration
		strongAdmissionID, admissionErr := s.reserveOrchestrationChildren(run.parentID, 1, cfg, "relay-strong")
		if admissionErr != nil {
			var limit errOrchestrationLimit
			if errors.As(admissionErr, &limit) {
				if limit.Limit == "total_sessions" {
					s.relayBoardLocked(run, fmt.Sprintf("escalation skipped: total session limit (max=%d reason=%s)", cfg.MaxTotalSessions, reason))
				} else {
					s.relayBoardLocked(run, fmt.Sprintf("escalation skipped: child limit (max=%d reason=%s)", cfg.MaxChildrenPerParent, reason))
				}
				return false
			}
			s.relayBoardLocked(run, fmt.Sprintf("escalation skipped: admission error (%v) reason=%s", admissionErr, reason))
			return false
		}
		if err := s.relaySpawnWithAdmission(run, relayRoleImplementationStrong, s.relayStrongPrompt(run, kind, v), strongAdmissionID); err != nil {
			s.releaseOrchestrationChildren(strongAdmissionID)
			s.relayBoardLocked(run, fmt.Sprintf("escalation skipped: spawn error (%v) reason=%s", err, reason))
			return false
		}
	} else if err := s.relayDispatch(run, relayRoleImplementationStrong, s.relayStrongText(run, kind, v)); err != nil {
		s.relayBoardLocked(run, fmt.Sprintf("escalation skipped: inject error (%v) reason=%s", err, reason))
		return false
	}
	run.activeImpl = relayRoleImplementationStrong
	run.round = 0
	s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventEscalated, C: run.currentCLocked(), Round: 0, Text: "reason=" + reason, ReviewPath: v.File})
	// "Stand down, somebody else has this C" is only worth saying to a child that
	// is sitting there waiting. A headless implementer already exited when it
	// wrote its DONE, so telling it would mean starting a whole process to say
	// one sentence to something with no next turn; the board records it instead.
	if run.roleHeadlessLocked(relayRoleImplementation) {
		s.relayBoardLocked(run, fmt.Sprintf("hand-over to %s: no wait notice sent (the headless implementer's process already exited)", relayRoleImplementationStrong))
	} else if err := s.relayInject(run, relayRoleImplementation, s.relayWaitText(run)); err != nil {
		s.logger.Warn("relay wait text not delivered", "orchestration_id", run.orchestrationID, "err", err)
	}
	return true
}

// relayOnChildIdle is called when the generic idle detection fired for a relay
// child. The awaited child gets one reminder per work unit (D-15); the other
// children are waiting for their next instruction and are left alone.
//
// A headless child is never nudged. The reminder exists because an interactive
// CLI can be sitting at its prompt having forgotten to write its DONE line; a
// headless process has no prompt to sit at, nothing would read the text, and
// "quiet" there means "still working" — which is what the timeout is for.
func (s *Server) relayOnChildIdle(boardID string, childID int) {
	run := s.relayByID(boardID)
	if run == nil {
		return
	}
	run.mu.Lock()
	defer run.mu.Unlock()
	if run.terminalLocked() || run.awaitingReconnect || childID != run.awaitedChildLocked() || run.nudged[childID] {
		return
	}
	role := run.roleOfLocked(childID)
	if run.roleHeadlessLocked(role) {
		return
	}
	run.nudged[childID] = true
	s.relayDep().inject(childID, "\n"+s.relayNudgeText(run, role)+"\n")
	s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventNudge, C: run.currentCLocked(), Round: run.round, Text: fmt.Sprintf("%s #%d", role, childID)})
	s.relaySaveLocked(run)
}

// relayOnChildExit is called when one of the relay's children ends (EOF). What
// that means depends on the role's execution mode, and this is the only place
// the two shapes of child part company:
//
//	interactive  the child was supposed to stay alive for the whole relay, so
//	             its exit is the relay losing a worker: stop (today's behaviour,
//	             unchanged).
//	headless     one process was started for one instruction, so its exit is
//	             that instruction finishing. Read the progress file it left and
//	             advance — or, on a non-zero exit, stop with the same reason
//	             code, after putting what the child did manage to write on the
//	             board.
//
// A late exit from a child that has already been replaced is ignored, because
// roleOfLocked matches on the run's current ID for the role and an old ID
// matches nothing. That is also what keeps the two routes into a completion —
// this one and the board scan — from running it twice: whichever arrives first
// spawns the next child, and the other then finds a stranger's ID.
func (s *Server) relayOnChildExit(boardID string, childID int, state string) {
	run := s.relayByID(boardID)
	if run == nil {
		return
	}
	run.mu.Lock()
	defer run.mu.Unlock()
	role := run.roleOfLocked(childID)
	if run.terminalLocked() || run.awaitingReconnect || role == "" {
		return
	}
	if !run.roleHeadlessLocked(role) {
		s.relayFinishLocked(run, relayStateStopped, relayReasonChildExited, fmt.Sprintf("%s #%d state=%s", role, childID, state))
		return
	}
	text := readFileOrEmpty(childProgressPath(run.boardPath, run.progressIDLocked(role)))
	if state != "completed" {
		// The exit code is the verdict (元設計 11 節), so a failed run stops the
		// relay just like a dead interactive child does. The progress file goes on
		// the board first: it is the only place the child could say why, and after
		// relayFinishLocked nothing reads it again.
		if strings.TrimSpace(text) != "" {
			s.relayBoardLocked(run, fmt.Sprintf("headless %s session=%d exited state=%s; its progress file said:\n%s", role, childID, state, text))
		}
		s.relayFinishLocked(run, relayStateStopped, relayReasonChildExited, fmt.Sprintf("%s #%d state=%s", role, childID, state))
		return
	}
	s.relayBoardLocked(run, fmt.Sprintf("headless %s session=%d finished its instruction (process exit, state=%s)", role, childID, state))
	s.relayAdvanceChildLocked(run, childID, text, true)
}

// relayOnChildTimeout stops the relay when the awaited child timed out. A
// timeout of another (waiting) child is ignored; its latch is released when
// it receives its next instruction (relayRearmChildTimers).
func (s *Server) relayOnChildTimeout(boardID string, childID int) {
	run := s.relayByID(boardID)
	if run == nil {
		return
	}
	run.mu.Lock()
	defer run.mu.Unlock()
	if run.terminalLocked() || run.awaitingReconnect || childID != run.awaitedChildLocked() {
		return
	}
	s.relayFinishLocked(run, relayStateStopped, relayReasonTimeout, fmt.Sprintf("%s #%d", run.roleOfLocked(childID), childID))
}

// relayOnChildStartupFailed stops the relay when the awaited child never
// produced any progress after its initial prompt was delivered (C2/C3,
// plan_spawn-orchestration-backlog-closeout_c3_child-fast-fail.md). Mirrors
// relayOnChildTimeout: a startup failure of a child the relay is not
// currently awaiting is ignored (it is not blocking any relay progress).
// The board record, latch and evidence capture happen in
// handleChildStartupFailed (orchestration.go) before this is called;
// relayFinishLocked owns the relay's own board/event/parent-notification path
// (D10 — no new injection route is introduced here).
func (s *Server) relayOnChildStartupFailed(boardID string, childID int) {
	run := s.relayByID(boardID)
	if run == nil {
		return
	}
	run.mu.Lock()
	defer run.mu.Unlock()
	if run.terminalLocked() || run.awaitingReconnect || childID != run.awaitedChildLocked() {
		return
	}
	s.relayFinishLocked(run, relayStateStopped, relayReasonStartupFailed, fmt.Sprintf("%s #%d", run.roleOfLocked(childID), childID))
}

// relayTransitionLocked records a non-terminal state change.
func (s *Server) relayTransitionLocked(run *relayRun, state, reason string) {
	run.state = state
	run.reason = reason
	run.updatedAt = s.relayDep().now()
	s.relayBoardLocked(run, fmt.Sprintf("relay c=%d round=%d state=%s reason=%s impl=#%d strong=#%d active=%s review=#%d review_file=%s",
		run.currentCLocked(), run.round, run.state, run.reason, run.implID, run.strongID, run.activeImpl, run.reviewID, run.reviewPath))
	s.relaySaveLocked(run)
	s.syncRelaySession(run)
}

// relayFinish ends a relay from outside a transition (stop API / restore
// paths). It reports false when the relay was already terminal.
func (s *Server) relayFinish(run *relayRun, state, reason, detail string) bool {
	run.mu.Lock()
	defer run.mu.Unlock()
	if run.terminalLocked() {
		return false
	}
	s.relayFinishLocked(run, state, reason, detail)
	return true
}

// relayFinishLocked is the common completed / stopped path: event, board,
// save, broadcast and the notification to the parent.
func (s *Server) relayFinishLocked(run *relayRun, state, reason, detail string) {
	deps := s.relayDep()
	run.state = state
	run.reason = reason
	if reason == relayReasonBlocked && detail != "" {
		run.reason = reason + ": " + detail
	}
	if run.admissionID != "" {
		s.releaseOrchestrationChildren(run.admissionID)
		run.admissionID = ""
	}
	run.updatedAt = deps.now()
	kind := relayEventStopped
	if state == relayStateCompleted {
		kind = relayEventCompleted
	}
	text := detail
	if text == "" {
		text = run.reason
	}
	s.relayEventLocked(run, proto.RelayEvent{Kind: kind, C: run.currentCLocked(), Round: run.round, Text: text, ReviewPath: run.reviewPath})
	s.relayBoardLocked(run, fmt.Sprintf("relay %s reason=%s c=%d round=%d review_file=%s branch=%s %s",
		state, run.reason, run.completedCs, run.round, run.reviewPath, run.branch, detail))
	s.relaySaveLocked(run)
	s.syncRelaySession(run)
	if run.parentAttached {
		notice := fmt.Sprintf("\n%s relay %s: plan=%s c=%d round=%d reason=%s review=%s branch=%s\n",
			run.tagLocked(), state, run.planPath, run.completedCs, run.round, run.reason, run.reviewPath, run.branch)
		if state == relayStateCompleted && strings.TrimSpace(run.branch) != "" {
			notice += fmt.Sprintf("Merge when ready (not run automatically):\n  git merge %s\n", run.branch)
		}
		deps.notifyParent(run.orchestrationID, run.parentID, notice)
	}
	text = fmt.Sprintf("%s: %s (c=%d, round=%d", filepath.Base(run.planPath), state, run.completedCs, run.round)
	if strings.TrimSpace(run.reason) != "" {
		text += ", reason=" + run.reason
	}
	text += ")"
	if strings.TrimSpace(run.branch) != "" {
		text += " branch=" + run.branch
	}
	deps.publishDone(proto.DoneSummary{
		SessionID: run.parentID,
		Provider:  run.parentProvider,
		Title:     "relay " + state,
		Text:      text,
		Kind:      "relay",
		At:        run.updatedAt.Format(time.RFC3339),
		Fallback:  false,
	})
}

func (s *Server) relayBoardLocked(run *relayRun, text string) {
	if err := s.relayDep().appendBoard(run.boardPath, "hub", text+"\n"); err != nil {
		s.logger.Warn("relay board append failed", "board", run.boardPath, "err", err)
	}
}

func (s *Server) relayEventLocked(run *relayRun, ev proto.RelayEvent) {
	if ev.At == "" {
		ev.At = s.relayDep().now().Format(time.RFC3339)
	}
	run.events = append(run.events, ev)
}

func (s *Server) relaySaveLocked(run *relayRun) {
	s.relayPublishMetaLocked(run)
	if err := s.relayDep().save(run); err != nil {
		s.logger.Warn("relay save failed", "orchestration_id", run.orchestrationID, "err", err)
	}
}

// syncRelaySession replaces the relay's snapshot on the parent session and
// broadcasts the session update. The Relays slice is rebuilt, never edited in
// place, because session copies are marshalled outside sessionsMu.
func (s *Server) syncRelaySession(run *relayRun) {
	if !run.parentAttached {
		return
	}
	st := run.statusLocked()
	s.sessionsMu.Lock()
	ses := s.sessions[run.parentID]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	next := make([]*proto.RelayStatus, 0, len(ses.Relays)+1)
	replaced := false
	for _, cur := range ses.Relays {
		if cur != nil && cur.OrchestrationID == st.OrchestrationID {
			next = append(next, &st)
			replaced = true
			continue
		}
		next = append(next, cur)
	}
	if !replaced {
		next = append(next, &st)
	}
	ses.Relays = next
	msg := sessionUpdateMessage(ses)
	s.sessionsMu.Unlock()
	s.broadcast(msg)
}

// ---- prompts (D-10: fixed English templates; the plan is passed by path) ---
//
// Every text starts with the relay tag (D-23 b). proceed / hand-over / resume
// texts also carry the "trust the working tree" line (D-23 c), because the
// tree may have been changed by another worker since that child last looked.

// commitLineLocked is the D-17 instruction: in worktree mode every C and every
// fix is committed to the relay branch; in same-tree mode nothing is committed.
func (run *relayRun) commitLineLocked() string {
	if run.mode != relayModeWorktree {
		return ""
	}
	return "commit everything for this C on the current branch with `git add -A && git commit -m \"relay: C<n> <short summary>\"` (a fix round is `relay: C<n> fix r<r>`; never switch branches, never push), then"
}

// scopeLineLocked tells the reviewer what to look at (D-13): the commit range
// since the last reviewed commit in worktree mode, the whole working tree in
// same-tree mode.
func (run *relayRun) scopeLineLocked() string {
	if run.mode == relayModeWorktree && run.lastReviewedCommit != "" {
		from := run.lastReviewedCommit
		return fmt.Sprintf("the commits %s..HEAD — run `git log --oneline %s..HEAD`, `git diff %s..HEAD`, and `git status --short` for anything left uncommitted", from, from, from)
	}
	return "run `git status --short` and `git diff` in the working directory; changes that pre-date the relay are also visible, judge them against the plan"
}

// worktreeLineLocked names the shared worktree and branch (worktree mode).
func (run *relayRun) worktreeLineLocked() string {
	if run.mode != relayModeWorktree || run.worktreePath == "" {
		return ""
	}
	return fmt.Sprintf("Your working directory is the relay worktree %s on branch %s (forked from %s); stay on that branch.", run.worktreePath, run.branch, run.baseCommit)
}

// resyncLineLocked is the D-23 (c) sentence.
func (run *relayRun) resyncLineLocked() string {
	logCmd := "git log --oneline -20"
	if run.lastReviewedCommit != "" {
		logCmd = "git log --oneline " + run.lastReviewedCommit + "..HEAD"
	}
	return "The working tree may have been changed by another worker. Before you continue, run `" + logCmd + "` and `git status --short` to see the current state, and trust the working tree over your own memory."
}

func (run *relayRun) extraLocked(role string) string {
	if text := strings.TrimSpace(run.extra[role]); text != "" {
		return "\n\nAdditional instructions from the user:\n" + text
	}
	return ""
}

// relayJoin joins the non-empty parts with single spaces so an empty
// <commitLine> does not leave a double space behind.
func relayJoin(parts ...string) string {
	kept := parts[:0:0]
	for _, p := range parts {
		if p = strings.TrimSpace(p); p != "" {
			kept = append(kept, p)
		}
	}
	return strings.Join(kept, " ")
}

func (s *Server) relayImplementationPrompt(run *relayRun) string {
	strongRule := "Ignore any [strong] marks in the plan and implement every C yourself."
	if run.hasStrongRoleLocked() {
		strongRule = "If a C's row in the plan's context table is marked [strong], do not implement it: append `## DONE implementation escalate=true` with the C number to your progress file and wait (add `final=true` as well if it is the last C). When the relay tells you another worker has taken over a C, wait until you are told to continue."
	}
	text := relayJoin(
		run.tagLocked(),
		"Relay task: implement the plan at "+run.planPath+".",
		run.worktreeLineLocked(),
		"Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the whole plan before changing anything.",
		"Work through the plan's C entries in order, one at a time. After finishing EACH C:",
		run.commitLineLocked(),
		"append a summary of changed files and verification results, then `## DONE implementation`, to your progress file — and then WAIT for the next relay instruction; do not start the next C on your own.",
		"On the LAST C of the plan write `## DONE implementation final=true` instead.",
		strongRule,
		"Do not push, and do not touch branches other than the current one.",
	)
	return sanitizeInjectText(text + run.extraLocked(relayRoleImplementation))
}

// relayStrongKindLine describes the strong implementer's job for kind.
func relayStrongKindLine(kind string, v relayVerdict) string {
	if kind == "fix" {
		return fmt.Sprintf("the review found %d must-fix items in %s; fix every must item and re-check the C against the plan", v.Must, v.File)
	}
	return "implement it from the plan"
}

func (s *Server) relayStrongPrompt(run *relayRun, kind string, v relayVerdict) string {
	text := relayJoin(
		run.tagLocked(),
		"Relay escalation: you are the stronger implementer for the plan at "+run.planPath+".",
		run.worktreeLineLocked(),
		"Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the plan.",
		fmt.Sprintf("Your job now is C %d only: %s.", run.currentCLocked(), relayStrongKindLine(kind, v)),
		run.resyncLineLocked(),
		run.commitLineLocked(),
		"When finished, append a summary and `## DONE implementation-strong` to your progress file (add `final=true` if this is the last C of the plan), then wait; the next C goes back to the other implementer, so do nothing until the relay calls you again.",
		"Do not push, and do not touch branches other than the current one.",
	)
	return sanitizeInjectText(text + run.extraLocked(relayRoleImplementationStrong))
}

func (s *Server) relayStrongText(run *relayRun, kind string, v relayVerdict) string {
	text := relayJoin(
		run.tagLocked(),
		fmt.Sprintf("Relay escalation: C %d: %s.", run.currentCLocked(), relayStrongKindLine(kind, v)),
		run.resyncLineLocked(),
		run.commitLineLocked(),
		"Then append `## DONE implementation-strong` (add `final=true` if this is the last C of the plan) and wait.",
	)
	return sanitizeInjectText(text + run.extraLocked(relayRoleImplementationStrong))
}

func (s *Server) relayWaitText(run *relayRun) string {
	return sanitizeInjectText(relayJoin(
		run.tagLocked(),
		"Relay: another worker has taken over the current C. Do nothing until the relay tells you to continue.",
	))
}

// relaySelfImplementText answers an escalate=true DONE when no strong
// implementer could take the C (no role, no free slot, spawn failure).
func (s *Server) relaySelfImplementText(run *relayRun) string {
	text := relayJoin(
		run.tagLocked(),
		fmt.Sprintf("Relay: no stronger implementer is available for C %d, so implement it yourself now. Same rules:", run.currentCLocked()),
		run.commitLineLocked(),
		"append `## DONE implementation` when the C is finished (`final=true` on the last C) and wait.",
	)
	return sanitizeInjectText(text + run.extraLocked(relayRoleImplementation))
}

func (s *Server) relayReviewPrompt(run *relayRun) string {
	c := run.currentCLocked()
	text := relayJoin(
		run.tagLocked(),
		fmt.Sprintf("Relay review (C %d, round %d): adversarially review the changes for the C that was just finished against the plan at %s.", c, run.round, run.planPath),
		run.worktreeLineLocked(),
		"Scope: "+run.scopeLineLocked()+".",
		"Assume the implementation is wrong until proven otherwise; inspect deleted lines as carefully as added lines. Do not modify any file.",
		fmt.Sprintf("Write your findings to %s as two numbered lists, \"must\" (blocks acceptance: broken behaviour, safety, deviation from the plan) and \"should\" (worth fixing, not blocking); each item needs file:line, what is wrong, and how to verify the fix.", run.reviewFileLocked(run.round)),
		"If there are no findings write the single line `verdict: pass`.",
		"Then, in your progress file, write ONE line: `verdict: pass` or `verdict: findings must=<n> should=<m> file=<that path>` or `verdict: blocked reason=<why a human must decide>`, followed by `## DONE review`.",
	)
	return sanitizeInjectText(text + run.extraLocked(relayRoleReview))
}

func (s *Server) relayFixText(run *relayRun, v relayVerdict) string {
	doneLine := "`## DONE " + run.activeImpl + "`"
	text := relayJoin(
		run.tagLocked(),
		fmt.Sprintf("Relay fix (C %d, round %d): the review found %d must-fix and %d should-fix items in %s.", run.currentCLocked(), run.round, v.Must, v.Should, v.File),
		"Fix every must item. Fix should items when cheap; otherwise record the item number and the reason in your progress file. Do not touch unrelated code.",
		"When finished:",
		run.commitLineLocked(),
		"append "+doneLine+" to your progress file and wait.",
	)
	return sanitizeInjectText(text + run.extraLocked(run.activeImpl))
}

func (s *Server) relayReReviewText(run *relayRun) string {
	c := run.currentCLocked()
	previous := run.reviewFileLocked(run.round - 1)
	if run.lastVerdict != nil && run.lastVerdict.File != "" {
		previous = run.lastVerdict.File
	}
	text := relayJoin(
		run.tagLocked(),
		fmt.Sprintf("Relay review (C %d, round %d): re-review after the fixes for %s.", c, run.round, previous),
		"Verify each earlier must item is resolved and look for regressions.",
		"Scope: "+run.scopeLineLocked()+".",
		fmt.Sprintf("Write %s the same way, then the `verdict:` line and `## DONE review` in your progress file.", run.reviewFileLocked(run.round)),
	)
	return sanitizeInjectText(text + run.extraLocked(relayRoleReview))
}

func (s *Server) relayProceedText(run *relayRun) string {
	text := relayJoin(
		run.tagLocked(),
		fmt.Sprintf("Relay: the review of C %d passed. Continue with the next C of the plan.", run.completedCs),
		run.resyncLineLocked(),
		"Same rules:",
		run.commitLineLocked(),
		"append `## DONE implementation` when the C is finished (`final=true` on the last C) and wait.",
	)
	return sanitizeInjectText(text + run.extraLocked(relayRoleImplementation))
}

func (s *Server) relayNudgeText(run *relayRun, role string) string {
	return sanitizeInjectText(relayJoin(
		run.tagLocked(),
		"Relay: no update to your progress file has been seen for a while. If your current work unit is finished, append `## DONE "+role+"` now (review: write the `verdict:` line first). If you are still working, continue. This reminder is sent only once.",
	))
}

func copyStringMap(m map[string]string) map[string]string {
	out := make(map[string]string, len(m))
	for k, v := range m {
		out[k] = v
	}
	return out
}
