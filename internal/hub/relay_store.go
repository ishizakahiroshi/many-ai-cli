package hub

// relay_store.go: persistence and restore of relays (D-18).
//
// Every transition writes `<orchestration dir>/<id>/relay.json`; it is the
// source of truth for the relay's state, counters, children and timeline. On
// start-up restoreRelays reads every relay.json and puts the non-terminal
// relays back into the manager in "awaiting reconnect" mode: the wrappers of
// their children survive a Hub restart (internal/wrapper reconnectSupervisor)
// and reattach within seconds of the Hub coming back, carrying the same
// --label they were spawned with. reattachLoop asks relayReattachMatch about
// every cold reattach; a match re-binds the child (or the conductor) to the
// relay, and once every recorded child is back the relay simply continues.
// Children that do not come back within relayReconnectGrace leave the relay
// stopped(hub_restart); resumeRelay re-spawns the implementer and continues
// from the git history (D-17 makes that possible).
//
// Relays that stopped for a resumable reason (hub_restart / child_exited /
// timeout) are restored as well, without a reconnect wait, so the user can
// resume them after the restart. They stay invisible to sessions until their
// conductor reattaches or a session resumes them by ID.

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
)

const (
	relayFileName    = "relay.json"
	relayFileVersion = 1
	// relayReconnectGrace bounds how long a restored relay waits for its
	// children to reattach. It is deliberately not hub.wrapper_reconnect_grace_sec
	// (default 3600 s): that setting is how long a *wrapper* waits for the Hub.
	// Once the Hub is up, surviving wrappers dial every 2 s (reconnectDialInterval),
	// so a child that has not reattached after two minutes is gone.
	relayReconnectGrace = 120 * time.Second
)

var (
	errRelayNotResumable    = errors.New("relay is not in a resumable state (stopped by hub_restart / child_exited / timeout)")
	errRelayWorktreeMissing = errors.New("relay worktree no longer exists; it cannot be resumed")
)

// relayFile is the on-disk form of relayRun. Field names are stable: the
// dashboard and doctor read `state`; everything else round-trips 1:1.
type relayFile struct {
	Version         int    `json:"version"`
	OrchestrationID string `json:"orchestration_id"`
	BoardPath       string `json:"board_path"`
	ParentSessionID int    `json:"parent_session_id"`
	ParentStartedAt string `json:"parent_started_at,omitempty"`
	ParentProvider  string `json:"parent_provider,omitempty"`
	ParentCWD       string `json:"parent_cwd,omitempty"`
	PlanPath        string `json:"plan_path"`
	Mode            string `json:"mode"`
	MaxRounds       int    `json:"max_rounds"`
	EscalateAfter   int    `json:"escalate_after"`

	CompletedCs       int    `json:"completed_cs"`
	Round             int    `json:"round"`
	FinalSeen         bool   `json:"final_seen"`
	State             string `json:"state"`
	Reason            string `json:"reason,omitempty"`
	ActiveImplementer string `json:"active_implementer"`

	Roles    map[string]orchestrationRoleAssignment `json:"roles"`
	Extra    map[string]string                      `json:"extra,omitempty"`
	ChildCWD string                                 `json:"child_cwd"`

	ImplementationLabel     string `json:"implementation_label,omitempty"`
	ImplementationSessionID int    `json:"implementation_session_id"`
	ImplementationProgress  int    `json:"implementation_progress_id,omitempty"`
	ImplDoneBaseline        int    `json:"impl_done_baseline"`
	StrongLabel             string `json:"strong_label,omitempty"`
	StrongSessionID         int    `json:"strong_session_id"`
	StrongProgress          int    `json:"strong_progress_id,omitempty"`
	StrongDoneBaseline      int    `json:"strong_done_baseline"`
	ReviewLabel             string `json:"review_label,omitempty"`
	ReviewSessionID         int    `json:"review_session_id"`
	ReviewProgress          int    `json:"review_progress_id,omitempty"`
	ReviewDoneBaseline      int    `json:"review_done_baseline"`

	ReviewPath         string        `json:"review_path,omitempty"`
	LastVerdict        *relayVerdict `json:"last_verdict,omitempty"`
	WorktreePath       string        `json:"worktree_path,omitempty"`
	Branch             string        `json:"branch,omitempty"`
	BaseCommit         string        `json:"base_commit,omitempty"`
	LastReviewedCommit string        `json:"last_reviewed_commit,omitempty"`

	Events    []proto.RelayEvent `json:"events"`
	UpdatedAt string             `json:"updated_at"`
}

// relayFileFromRun snapshots the run. The caller holds run.mu.
func relayFileFromRun(run *relayRun) relayFile {
	f := relayFile{
		Version:         relayFileVersion,
		OrchestrationID: run.orchestrationID,
		BoardPath:       run.boardPath,
		ParentSessionID: run.parentID,
		ParentStartedAt: run.parentStartedAt,
		ParentProvider:  run.parentProvider,
		ParentCWD:       run.parentCWD,
		PlanPath:        run.planPath,
		Mode:            run.mode,
		MaxRounds:       run.maxRounds,
		EscalateAfter:   run.escalateAfter,

		CompletedCs:       run.completedCs,
		Round:             run.round,
		FinalSeen:         run.finalSeen,
		State:             run.state,
		Reason:            run.reason,
		ActiveImplementer: run.activeImpl,

		Roles:    map[string]orchestrationRoleAssignment{},
		Extra:    copyStringMap(run.extra),
		ChildCWD: run.childCWD,

		ImplementationLabel:     run.implLabel,
		ImplementationSessionID: run.implID,
		ImplementationProgress:  run.implProgressID,
		ImplDoneBaseline:        run.implDoneBaseline,
		StrongLabel:             run.strongLabel,
		StrongSessionID:         run.strongID,
		StrongProgress:          run.strongProgressID,
		StrongDoneBaseline:      run.strongDoneBaseline,
		ReviewLabel:             run.reviewLabel,
		ReviewSessionID:         run.reviewID,
		ReviewProgress:          run.reviewProgressID,
		ReviewDoneBaseline:      run.reviewDoneBaseline,

		ReviewPath:         run.reviewPath,
		WorktreePath:       run.worktreePath,
		Branch:             run.branch,
		BaseCommit:         run.baseCommit,
		LastReviewedCommit: run.lastReviewedCommit,
		Events:             run.eventsLocked(),
	}
	for role, ra := range run.roles {
		f.Roles[role] = ra
	}
	if run.lastVerdict != nil {
		v := *run.lastVerdict
		f.LastVerdict = &v
	}
	if !run.updatedAt.IsZero() {
		f.UpdatedAt = run.updatedAt.Format(time.RFC3339)
	}
	return f
}

// relayRunFromFile rebuilds a run from its file. Nothing is attached yet:
// the caller decides about parentAttached / awaitingReconnect.
func relayRunFromFile(f relayFile) *relayRun {
	run := &relayRun{
		orchestrationID: f.OrchestrationID,
		boardPath:       f.BoardPath,
		boardDir:        filepath.Dir(f.BoardPath),
		parentID:        f.ParentSessionID,
		parentStartedAt: f.ParentStartedAt,
		parentProvider:  f.ParentProvider,
		parentCWD:       f.ParentCWD,
		planPath:        f.PlanPath,
		mode:            f.Mode,
		maxRounds:       f.MaxRounds,
		escalateAfter:   f.EscalateAfter,

		completedCs: f.CompletedCs,
		round:       f.Round,
		finalSeen:   f.FinalSeen,
		state:       f.State,
		reason:      f.Reason,
		activeImpl:  f.ActiveImplementer,

		roles:    map[string]orchestrationRoleAssignment{},
		extra:    copyStringMap(f.Extra),
		childCWD: f.ChildCWD,

		implID:             f.ImplementationSessionID,
		strongID:           f.StrongSessionID,
		reviewID:           f.ReviewSessionID,
		implLabel:          f.ImplementationLabel,
		strongLabel:        f.StrongLabel,
		reviewLabel:        f.ReviewLabel,
		implDoneBaseline:   f.ImplDoneBaseline,
		strongDoneBaseline: f.StrongDoneBaseline,
		reviewDoneBaseline: f.ReviewDoneBaseline,
		implProgressID:     f.ImplementationProgress,
		strongProgressID:   f.StrongProgress,
		reviewProgressID:   f.ReviewProgress,
		nudged:             map[int]bool{},
		reconnected:        map[string]bool{},

		reviewPath:         f.ReviewPath,
		worktreePath:       f.WorktreePath,
		branch:             f.Branch,
		baseCommit:         f.BaseCommit,
		lastReviewedCommit: f.LastReviewedCommit,
		events:             append([]proto.RelayEvent(nil), f.Events...),
	}
	for role, ra := range f.Roles {
		run.roles[role] = ra
	}
	if f.LastVerdict != nil {
		v := *f.LastVerdict
		run.lastVerdict = &v
	}
	if run.activeImpl == "" {
		run.activeImpl = relayRoleImplementation
	}
	if run.maxRounds <= 0 {
		run.maxRounds = relayDefaultMaxRounds
	}
	if run.escalateAfter <= 0 {
		run.escalateAfter = relayDefaultEscalateAfter
	}
	if t, err := time.Parse(time.RFC3339, f.UpdatedAt); err == nil {
		run.updatedAt = t
	}
	return run
}

func relayFilePath(boardDir string) string {
	return filepath.Join(boardDir, relayFileName)
}

// saveRelayFile is the default relayDeps.save: write to a temp file in the
// same directory and rename, so a crash mid-write never leaves a torn file.
// The caller holds run.mu.
func (s *Server) saveRelayFile(run *relayRun) error {
	return writeRelayFile(relayFilePath(run.boardDir), relayFileFromRun(run))
}

func writeRelayFile(path string, f relayFile) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, sessionlog.PrivateDirMode); err != nil {
		return fmt.Errorf("mkdir relay dir: %w", err)
	}
	data, err := json.MarshalIndent(f, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal relay file: %w", err)
	}
	tmp, err := os.CreateTemp(dir, "relay-*.json.tmp")
	if err != nil {
		return fmt.Errorf("create temp relay file: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // no-op after a successful rename
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write temp relay file: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("sync temp relay file: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close temp relay file: %w", err)
	}
	if err := os.Chmod(tmpName, sessionlog.PrivateFileMode); err != nil {
		return fmt.Errorf("chmod temp relay file: %w", err)
	}
	return os.Rename(tmpName, path)
}

// loadRelayFiles reads every <orchestration dir>/*/relay.json. A directory
// without relay.json (AI-conductor boards) is not a relay; a file that does
// not parse is skipped with a warning.
func (s *Server) loadRelayFiles() ([]relayFile, error) {
	base, err := orchestrationDir()
	if err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(base)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var files []relayFile
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		path := filepath.Join(base, e.Name(), relayFileName)
		data, err := os.ReadFile(path)
		if err != nil {
			continue
		}
		var f relayFile
		if err := json.Unmarshal(data, &f); err != nil || f.OrchestrationID == "" {
			s.logger.Warn("relay file skipped", "path", path, "err", err)
			continue
		}
		if f.BoardPath == "" {
			f.BoardPath = filepath.Join(base, e.Name(), "board.md")
		}
		files = append(files, f)
	}
	sort.Slice(files, func(i, j int) bool { return files[i].OrchestrationID < files[j].OrchestrationID })
	return files, nil
}

// relayResumableReason reports whether a stopped relay may be resumed.
func relayResumableReason(reason string) bool {
	switch reason {
	case relayReasonHubRestart, relayReasonChildExited, relayReasonTimeout:
		return true
	}
	return false
}

// restoreRelays runs once at Hub start, before the board loop and before the
// first wrapper can reattach. Non-terminal relays come back awaiting their
// children; resumable stopped relays come back idle; completed and other
// stopped relays stay on disk only.
func (s *Server) restoreRelays() {
	files, err := s.loadRelayFiles()
	if err != nil {
		s.logger.Warn("relay restore skipped", "err", err)
		return
	}
	now := s.relayDep().now()
	restored := 0
	for _, f := range files {
		if f.State == relayStateCompleted {
			continue
		}
		if f.State == relayStateStopped && !relayResumableReason(f.Reason) {
			continue
		}
		run := relayRunFromFile(f)
		run.parentAttached = false
		s.orchestration.mu.Lock()
		if _, exists := s.orchestration.relays[run.orchestrationID]; exists {
			s.orchestration.mu.Unlock()
			continue
		}
		s.orchestration.relaySeq++
		run.seq = s.orchestration.relaySeq
		s.orchestration.relays[run.orchestrationID] = run
		s.orchestration.mu.Unlock()
		s.registerBoardSession(run.orchestrationID, run.boardPath, run.parentID, "conductor")
		run.mu.Lock()
		s.relayPublishMetaLocked(run)
		for _, child := range []struct {
			id   int
			role string
		}{{run.implID, relayRoleImplementation}, {run.strongID, relayRoleImplementationStrong}, {run.reviewID, relayRoleReview}} {
			if child.id != 0 {
				s.registerBoardChild(run.orchestrationID, run.boardPath, child.id, run.parentID, child.role, now)
				s.relaySetChildProgressPath(run.orchestrationID, child.id, run.progressIDLocked(child.role))
			}
		}
		if !run.terminalLocked() {
			run.awaitingReconnect = true
			run.restoredAt = now
			s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventRestored, C: run.currentCLocked(), Round: run.round, Text: "hub restarted; waiting for the children to reattach"})
			s.relayBoardLocked(run, fmt.Sprintf("relay restored after hub restart state=%s c=%d round=%d; waiting for impl=#%d strong=#%d review=#%d to reattach", run.state, run.currentCLocked(), run.round, run.implID, run.strongID, run.reviewID))
			s.relaySaveLocked(run)
		}
		run.mu.Unlock()
		restored++
	}
	if restored > 0 {
		s.logger.Info("relays restored", "count", restored)
	}
}

// relaySetChildProgressPath points the board child at the progress file the
// child actually writes (child-<progress id>.md), which differs from the live
// ID after a renumbered reattach.
func (s *Server) relaySetChildProgressPath(boardID string, childID, progressID int) {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	if b := s.orchestration.boards[boardID]; b != nil {
		if child := b.Children[childID]; child != nil {
			child.FilePath = childProgressPath(b.Path, progressID)
		}
	}
}

// relayReconnectedLocked reports whether every child recorded in the file has
// reattached since the restore.
func (run *relayRun) relayReconnectedLocked() bool {
	for _, child := range []struct {
		id   int
		role string
	}{{run.implID, relayRoleImplementation}, {run.strongID, relayRoleImplementationStrong}, {run.reviewID, relayRoleReview}} {
		if child.id != 0 && !run.reconnected[child.role] {
			return false
		}
	}
	return true
}

// checkRelayReconnect is polled by scanOrchestrationBoards. A restored relay
// whose children are all back resumes where it was — the awaited child's
// progress file is re-read so a DONE written while the Hub was down is not
// lost — and one that is still incomplete after relayReconnectGrace stops
// with hub_restart.
func (s *Server) checkRelayReconnect(now time.Time) {
	s.orchestration.mu.Lock()
	runs := make([]*relayRun, 0, len(s.orchestration.relays))
	for _, run := range s.orchestration.relays {
		runs = append(runs, run)
	}
	s.orchestration.mu.Unlock()
	for _, run := range runs {
		run.mu.Lock()
		if !run.awaitingReconnect || run.terminalLocked() {
			run.mu.Unlock()
			continue
		}
		switch {
		case run.relayReconnectedLocked():
			run.awaitingReconnect = false
			s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventResumed, C: run.currentCLocked(), Round: run.round, Text: "children reattached; continuing"})
			s.relayBoardLocked(run, fmt.Sprintf("relay continues after hub restart state=%s impl=#%d strong=#%d review=#%d", run.state, run.implID, run.strongID, run.reviewID))
			s.relaySaveLocked(run)
			s.syncRelaySession(run)
			if awaited := run.awaitedChildLocked(); awaited != 0 {
				role := run.roleOfLocked(awaited)
				s.relayRearmChildTimers(run.orchestrationID, awaited)
				s.relayProcessChildTextLocked(run, awaited, readFileOrEmpty(childProgressPath(run.boardPath, run.progressIDLocked(role))))
			}
		case now.Sub(run.restoredAt) > relayReconnectGrace:
			run.awaitingReconnect = false
			s.relayFinishLocked(run, relayStateStopped, relayReasonHubRestart, fmt.Sprintf("children not reconnected within %s", relayReconnectGrace))
		}
		run.mu.Unlock()
	}
}

// relayReattachMatch is what reattachLoop learns about a cold reattach: the
// relay the session belongs to and, for a child, its role. The copied fields
// let reattachLoop fill the session under sessionsMu without touching run.mu.
type relayReattachMatch struct {
	run             *relayRun
	role            string // "" for the conductor
	parentID        int
	orchestrationID string
	boardPath       string
}

// relayReattachMatch identifies a reattaching wrapper. Children match by
// label (unique per spawn, carried on every reattach); the conductor matches
// by start time, cwd and provider, never by session ID alone, because IDs
// restart at 1 after a Hub restart and may already be taken.
func (s *Server) relayReattachMatch(req proto.Message) *relayReattachMatch {
	s.orchestration.mu.Lock()
	runs := make([]*relayRun, 0, len(s.orchestration.relays))
	for _, run := range s.orchestration.relays {
		runs = append(runs, run)
	}
	s.orchestration.mu.Unlock()
	label := strings.TrimSpace(req.Label)
	for _, run := range runs {
		run.mu.Lock()
		var m *relayReattachMatch
		if label != "" {
			for _, child := range []struct {
				label string
				role  string
			}{{run.implLabel, relayRoleImplementation}, {run.strongLabel, relayRoleImplementationStrong}, {run.reviewLabel, relayRoleReview}} {
				if child.label == label && !run.reconnected[child.role] && run.childIDLocked(child.role) != 0 {
					m = &relayReattachMatch{run: run, role: child.role, parentID: run.parentID, orchestrationID: run.orchestrationID, boardPath: run.boardPath}
					break
				}
			}
		}
		if m == nil && !run.parentAttached && run.parentStartedAt != "" &&
			req.StartedAt == run.parentStartedAt && req.CWD == run.parentCWD && req.Provider == run.parentProvider {
			m = &relayReattachMatch{run: run, parentID: run.parentID, orchestrationID: run.orchestrationID, boardPath: run.boardPath}
		}
		run.mu.Unlock()
		if m != nil {
			return m
		}
	}
	return nil
}

// applyLocked fills the reattached session's orchestration fields. The
// caller holds sessionsMu.
func (m *relayReattachMatch) applyLocked(ses *session) {
	if m == nil || ses == nil {
		return
	}
	ses.OrchestrationID = m.orchestrationID
	ses.BoardPath = m.boardPath
	if m.role != "" {
		ses.ParentSessionID = m.parentID
		ses.Role = m.role
		ses.Auto = true
		if ses.Depth == 0 {
			ses.Depth = 1
		}
	}
}

// relayNoteReattached binds the reattached session (possibly renumbered) to
// the relay and re-keys the board bookkeeping. Called outside sessionsMu.
func (s *Server) relayNoteReattached(m *relayReattachMatch, sessionID int) {
	if m == nil {
		return
	}
	run := m.run
	run.mu.Lock()
	defer run.mu.Unlock()
	if m.role == "" {
		old := run.parentID
		run.parentID = sessionID
		run.parentAttached = true
		if old != sessionID {
			s.relayRebindBoardSession(run.orchestrationID, old, sessionID)
		}
		s.relayBoardLocked(run, fmt.Sprintf("conductor reattached session=%d (was #%d)", sessionID, old))
	} else {
		old := run.childIDLocked(m.role)
		progressID := run.progressIDLocked(m.role)
		switch m.role {
		case relayRoleImplementation:
			run.implID, run.implProgressID = sessionID, progressID
		case relayRoleImplementationStrong:
			run.strongID, run.strongProgressID = sessionID, progressID
		case relayRoleReview:
			run.reviewID, run.reviewProgressID = sessionID, progressID
		}
		run.reconnected[m.role] = true
		if old != sessionID {
			s.relayRebindBoardChild(run.orchestrationID, old, sessionID, run.parentID, m.role, progressID)
		}
		s.relayBoardLocked(run, fmt.Sprintf("child reattached role=%s session=%d (was #%d, progress file child-%d.md)", m.role, sessionID, old, progressID))
	}
	s.relaySaveLocked(run)
	s.syncRelaySession(run)
}

func (s *Server) relayRebindBoardSession(boardID string, oldID, newID int) {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	b := s.orchestration.boards[boardID]
	if b == nil {
		return
	}
	delete(b.Sessions, oldID)
	b.Sessions[newID] = "conductor"
	for _, child := range b.Children {
		if child.ParentID == oldID {
			child.ParentID = newID
		}
	}
}

func (s *Server) relayRebindBoardChild(boardID string, oldID, newID, parentID int, role string, progressID int) {
	now := time.Now()
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	b := s.orchestration.boards[boardID]
	if b == nil {
		return
	}
	delete(b.Children, oldID)
	delete(b.Sessions, oldID)
	delete(b.Done, oldID)
	delete(b.IdleWarned, oldID)
	delete(b.TimedOut, oldID)
	b.Sessions[newID] = role
	b.Children[newID] = &orchestrationChild{
		ID:             newID,
		ParentID:       parentID,
		Role:           role,
		SpawnedAt:      now,
		LastBoardWrite: now,
		FilePath:       childProgressPath(b.Path, progressID),
	}
}

// resumeRelay re-spawns the implementer of a stopped relay and continues from
// the git history (D-18). A restored relay whose conductor never came back is
// adopted by the calling session. The reviewer is spawned again when the next
// review is due; the strong implementer likewise.
func (s *Server) resumeRelay(parentID int, orchestrationID string) error {
	var run *relayRun
	if id := strings.TrimSpace(orchestrationID); id != "" {
		run = s.relayByID(id)
		if run == nil {
			return errRelayNotFound
		}
	} else {
		// No run mutex is held here yet, so locking the candidates one by one
		// is safe (run.mu is never nested with another run.mu).
		var candidates []*relayRun
		for _, r := range s.relaysForParent(parentID) {
			r.mu.Lock()
			ok := r.state == relayStateStopped && relayResumableReason(r.reason)
			r.mu.Unlock()
			if ok {
				candidates = append(candidates, r)
			}
		}
		switch len(candidates) {
		case 0:
			return errRelayNotFound
		case 1:
			run = candidates[0]
		default:
			return errRelayAmbiguous
		}
	}
	parent, _, _ := s.orchestrationParentState(parentID)
	if parent == nil {
		return errRelayParentNotFound
	}
	cfg := s.snapshotCfg().Orchestration

	run.mu.Lock()
	defer run.mu.Unlock()
	if run.parentAttached && run.parentID != parentID {
		return errRelayNotFound
	}
	if run.awaitingReconnect || run.state != relayStateStopped || !relayResumableReason(run.reason) {
		return errRelayNotResumable
	}
	if run.mode == relayModeWorktree {
		if info, err := os.Stat(run.worktreePath); err != nil || !info.IsDir() {
			return errRelayWorktreeMissing
		}
		parentCWD := strings.TrimSpace(run.parentCWD)
		if parentCWD == "" {
			parentCWD = parent.CWD
		}
		if err := validateWorktreeIdentity(parentCWD, run.worktreePath, run.branch); err != nil {
			return err
		}
	}
	admissionID, admissionErr := s.reserveOrchestrationChildren(parentID, relayChildrenPerRun, cfg, "relay-resume")
	if admissionErr != nil {
		if errors.Is(admissionErr, errOrchestrationAdmissionParentNotFound) {
			return errRelayParentNotFound
		}
		return admissionErr
	}
	run.admissionID = admissionID
	if !run.parentAttached || run.parentID != parentID {
		old := run.parentID
		run.parentID = parentID
		run.parentAttached = true
		run.parentStartedAt, run.parentProvider, run.parentCWD = parent.StartedAt, parent.Provider, parent.CWD
		s.relayRebindBoardSession(run.orchestrationID, old, parentID)
		s.markConductor(parentID, run.orchestrationID, run.boardPath)
		s.relayBoardLocked(run, fmt.Sprintf("relay adopted by session=%d (was #%d)", parentID, old))
	}
	s.registerBoardSession(run.orchestrationID, run.boardPath, parentID, "conductor")
	// The previous children are gone or stale: start over with a fresh
	// implementer and spawn the others again when they are needed.
	run.implID, run.strongID, run.reviewID = 0, 0, 0
	run.implLabel, run.strongLabel, run.reviewLabel = "", "", ""
	run.implProgressID, run.strongProgressID, run.reviewProgressID = 0, 0, 0
	run.implDoneBaseline, run.strongDoneBaseline, run.reviewDoneBaseline = 0, 0, 0
	run.reconnected = map[string]bool{}
	run.nudged = map[int]bool{}
	run.activeImpl = relayRoleImplementation
	run.round = 0
	run.reason = ""
	s.relayBoardLocked(run, fmt.Sprintf("relay resume requested by session=%d c=%d", parentID, run.currentCLocked()))
	if err := s.relaySpawn(run, relayRoleImplementation, s.relayResumePrompt(run)); err != nil {
		s.relayFinishLocked(run, relayStateStopped, relayReasonSpawnError, err.Error())
		return fmt.Errorf("relay resume spawn: %w", err)
	}
	s.relayEventLocked(run, proto.RelayEvent{Kind: relayEventResumed, C: run.currentCLocked(), Round: 0, Text: fmt.Sprintf("resumed by session #%d", parentID), ReviewPath: run.reviewPath})
	s.relayTransitionLocked(run, relayStateImplementing, "")
	return nil
}

// relayResumePrompt is the implementer's prompt after a resume: what is
// already committed, what the last review still wants, and the usual rules.
func (s *Server) relayResumePrompt(run *relayRun) string {
	logCmd := "git log --oneline -20"
	if run.baseCommit != "" {
		logCmd = "git log --oneline " + run.baseCommit + "..HEAD"
	}
	pendingReview := ""
	if run.lastVerdict != nil && run.lastVerdict.Kind == "findings" && run.lastVerdict.Must > 0 && run.reviewPath != "" {
		pendingReview = "; first fix the must items in " + run.reviewPath
	}
	text := relayJoin(
		run.tagLocked(),
		"Relay resume: the plan at "+run.planPath+" was partly implemented before an interruption.",
		run.worktreeLineLocked(),
		"Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the whole plan.",
		"Run `"+logCmd+"` to see which C entries are already committed"+pendingReview+".",
		run.resyncLineLocked(),
		"Continue from the next unfinished C with the same rules: after finishing EACH C:",
		run.commitLineLocked(),
		"append a summary and `## DONE implementation` to your progress file (`final=true` on the last C) and wait for the next relay instruction.",
		"Do not push, and do not touch branches other than the current one.",
	)
	return sanitizeInjectText(text + run.extraLocked(relayRoleImplementation))
}
