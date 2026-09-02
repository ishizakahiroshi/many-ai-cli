package hub

// relay_api.go exposes the relay state machine to the browser and to the
// `many-ai-cli orchestrate relay` wrapper. The API owns input validation and
// error translation; relay.go remains a side-effect-injectable state machine.

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/proto"
)

const relayPlanMaxBytes = 1 << 20

type relayStartJSON struct {
	PlanPath      string                                  `json:"plan_path"`
	MaxRounds     int                                     `json:"max_rounds"`
	Mode          string                                  `json:"mode"`
	Roles         map[string]*orchestrationRoleAssignment `json:"roles"`
	EscalateAfter int                                     `json:"escalate_after"`
	Extra         map[string]string                       `json:"extra"`
}

type relayControlAPIRequest struct {
	OrchestrationID string `json:"orchestration_id"`
}

type relayAPIItem struct {
	Relay  proto.RelayStatus  `json:"relay"`
	Events []proto.RelayEvent `json:"events"`
}

// handleRelayStart handles POST /api/sessions/:id/relay.
func (s *Server) handleRelayStart(w http.ResponseWriter, r *http.Request, parentID int) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body relayStartJSON
	if !decodeJSON(w, r, &body) {
		return
	}
	parent, _, _ := s.orchestrationParentState(parentID)
	if parent == nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "parent session not found")
		return
	}

	planPath, err := resolveRelayPlanPath(parent.CWD, body.PlanPath)
	if err != nil {
		s.writeRelayAPIError(w, http.StatusBadRequest, "", err, parentID)
		return
	}
	maxRounds, err := relayAPIPositiveRange(body.MaxRounds, relayDefaultMaxRounds, 1, 9, "max_rounds")
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
		return
	}
	escalateAfter, err := relayAPIPositiveRange(body.EscalateAfter, relayDefaultEscalateAfter, 1, 5, "escalate_after")
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
		return
	}
	mode := strings.TrimSpace(body.Mode)
	if mode == "" {
		mode = relayModeWorktree
	}
	if mode != relayModeWorktree && mode != relayModeSameTree {
		writeJSONError(w, http.StatusBadRequest, "bad_request", `mode must be "worktree" or "same-tree"`)
		return
	}

	roles := s.relayRolesForStart(parentID, parent.OrchestrationID, body.Roles)
	status, err := s.startRelay(parentID, relayStartRequest{
		PlanPath:      planPath,
		MaxRounds:     maxRounds,
		EscalateAfter: escalateAfter,
		Mode:          mode,
		Roles:         roles,
		Extra:         copyStringMap(body.Extra),
	})
	if err != nil {
		s.writeRelayAPIError(w, http.StatusInternalServerError, "", err, parentID)
		return
	}
	run := s.relayByID(status.OrchestrationID)
	if run == nil {
		writeJSONError(w, http.StatusInternalServerError, "spawn_error", "relay was started but its state is unavailable")
		return
	}
	st, boardPath := relaySnapshotWithBoard(run)
	writeJSON(w, map[string]any{
		"ok":                        true,
		"orchestration_id":          st.OrchestrationID,
		"board_path":                boardPath,
		"implementation_session_id": st.ImplementationSessionID,
		"worktree_path":             st.WorktreePath,
		"branch":                    st.Branch,
		"relay":                     st,
	})
}

// handleRelayStop handles POST /api/sessions/:id/relay-stop. With no ID it
// is shorthand only when the parent has exactly one active relay.
func (s *Server) handleRelayStop(w http.ResponseWriter, r *http.Request, parentID int) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body relayControlAPIRequest
	if !decodeOptionalRelayJSON(w, r, &body) {
		return
	}
	run, err := s.resolveRelay(parentID, body.OrchestrationID)
	if err != nil {
		s.writeRelayAPIError(w, http.StatusNotFound, "", err, parentID)
		return
	}
	if err := s.stopRelay(parentID, run.orchestrationID, relayReasonUserStop); err != nil {
		s.writeRelayAPIError(w, http.StatusConflict, "", err, parentID)
		return
	}
	st, _ := relaySnapshotWithBoard(run)
	writeJSON(w, map[string]any{"ok": true, "relay": st})
}

// handleRelayResume handles POST /api/sessions/:id/relay-resume. A restored
// relay may be addressed by ID even before it is attached to the new parent
// session, so explicit IDs are resolved from the global relay map.
func (s *Server) handleRelayResume(w http.ResponseWriter, r *http.Request, parentID int) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body relayControlAPIRequest
	if !decodeOptionalRelayJSON(w, r, &body) {
		return
	}
	run, err := s.resumableRelayCandidate(parentID, body.OrchestrationID)
	if err != nil {
		s.writeRelayAPIError(w, http.StatusNotFound, "", err, parentID)
		return
	}
	if err := s.resumeRelay(parentID, run.orchestrationID); err != nil {
		s.writeRelayAPIError(w, http.StatusConflict, "", err, parentID)
		return
	}
	st, _ := relaySnapshotWithBoard(run)
	writeJSON(w, map[string]any{"ok": true, "relay": st})
}

// handleRelayCleanup handles POST /api/sessions/:id/relay-cleanup. It removes
// only the relay worktree; the result branch remains available for merging.
func (s *Server) handleRelayCleanup(w http.ResponseWriter, r *http.Request, parentID int) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body relayControlAPIRequest
	if !decodeOptionalRelayJSON(w, r, &body) {
		return
	}
	if strings.TrimSpace(body.OrchestrationID) == "" {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "orchestration_id is required")
		return
	}
	run, err := s.resolveRelay(parentID, body.OrchestrationID)
	if err != nil {
		s.writeRelayAPIError(w, http.StatusNotFound, "", err, parentID)
		return
	}

	run.mu.Lock()
	if !run.terminalLocked() {
		run.mu.Unlock()
		writeJSONError(w, http.StatusConflict, "relay_active", "active relay worktree cannot be cleaned up")
		return
	}
	if run.mode != relayModeWorktree {
		run.mu.Unlock()
		writeJSONError(w, http.StatusBadRequest, "relay_no_worktree", "same-tree relay has no worktree to clean up")
		return
	}
	path, branch, parentCWD := run.worktreePath, run.branch, run.parentCWD
	if strings.TrimSpace(path) == "" {
		run.mu.Unlock()
		writeJSONError(w, http.StatusConflict, "relay_worktree_missing", "relay worktree has already been cleaned up")
		return
	}
	if err := validateRelayCleanupPath(parentCWD, path, s.snapshotCfg().Orchestration); err != nil {
		run.mu.Unlock()
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
		return
	}
	childIDs := run.childIDsLocked()
	run.mu.Unlock()
	if remaining := s.closeRelayChildren(childIDs); len(remaining) > 0 {
		writeJSONError(w, http.StatusConflict, "relay_children_active", fmt.Sprintf("relay child sessions are still active: %v; close them before cleaning up the worktree", remaining))
		return
	}
	s.markRelayChildrenDone(body.OrchestrationID, childIDs)

	// Another cleanup request can run while the child sessions are closing.
	// Re-lock and verify that this request still owns the same worktree handle.
	run.mu.Lock()
	if run.worktreePath != path {
		run.mu.Unlock()
		writeJSONError(w, http.StatusConflict, "relay_worktree_missing", "relay worktree has already been cleaned up")
		return
	}
	if info, statErr := os.Stat(path); statErr != nil || !info.IsDir() {
		run.mu.Unlock()
		writeJSONError(w, http.StatusConflict, "relay_worktree_missing", "relay worktree is no longer present")
		return
	}
	if err := s.cleanupRelayWorktree(parentCWD, path, branch, true); err != nil {
		run.mu.Unlock()
		writeJSONError(w, http.StatusInternalServerError, "relay_cleanup_error", errorDetail("relay worktree cleanup failed", err))
		return
	}
	run.worktreePath = ""
	run.updatedAt = s.relayDep().now()
	s.relayBoardLocked(run, fmt.Sprintf("relay worktree cleaned path=%s branch=%s", path, branch))
	s.relaySaveLocked(run)
	s.syncRelaySession(run)
	st := run.statusLocked()
	run.mu.Unlock()
	writeJSON(w, map[string]any{"ok": true, "relay": st})
}

func (s *Server) closeRelayChildren(ids []int) []int {
	for _, id := range ids {
		s.handleDismiss(proto.Message{Type: "session_dismiss", SessionID: id})
	}
	remaining := make([]int, 0)
	s.sessionsMu.Lock()
	for _, id := range ids {
		if s.sessions[id] != nil {
			remaining = append(remaining, id)
		}
	}
	s.sessionsMu.Unlock()
	return remaining
}

func (s *Server) markRelayChildrenDone(boardID string, ids []int) {
	s.orchestration.mu.Lock()
	defer s.orchestration.mu.Unlock()
	board := s.orchestration.boards[boardID]
	if board == nil {
		return
	}
	for _, id := range ids {
		board.Done[id] = true
		if child := board.Children[id]; child != nil {
			child.Done = true
		}
		delete(board.IdleWarned, id)
		delete(board.TimedOut, id)
		delete(board.StartupFailed, id)
	}
}

// handleRelayGet handles GET /api/sessions/:id/relay and includes each relay's
// full event timeline. Start order is retained by relaysForParent.
func (s *Server) handleRelayGet(w http.ResponseWriter, r *http.Request, parentID int) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	parent, _, _ := s.orchestrationParentState(parentID)
	if parent == nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "parent session not found")
		return
	}
	items := make([]relayAPIItem, 0)
	for _, run := range s.relaysForParent(parentID) {
		st, events := relaySnapshot(run)
		items = append(items, relayAPIItem{Relay: st, Events: events})
	}
	writeJSON(w, map[string]any{"ok": true, "relays": items})
}

func decodeOptionalRelayJSON(w http.ResponseWriter, r *http.Request, dst any) bool {
	if r.Body == nil || r.Body == http.NoBody || r.ContentLength == 0 {
		return true
	}
	return decodeJSON(w, r, dst)
}

func relayAPIPositiveRange(value, defaultValue, min, max int, name string) (int, error) {
	if value == 0 {
		return defaultValue, nil
	}
	if value < min || value > max {
		return 0, fmt.Errorf("%s must be between %d and %d", name, min, max)
	}
	return value, nil
}

func (s *Server) relayRolesForStart(parentID int, orchestrationID string, supplied map[string]*orchestrationRoleAssignment) map[string]orchestrationRoleAssignment {
	roles := map[string]orchestrationRoleAssignment{}
	for _, source := range []map[string]orchestrationRoleAssignment{
		s.orchestrationRolesFor(orchestrationID),
		s.relayRolesForParent(parentID),
	} {
		for role, assignment := range source {
			if _, exists := roles[role]; !exists {
				roles[role] = assignment
			}
		}
	}
	// The request is authoritative for fields it explicitly supplies. Keeping
	// the other field from the mapping makes provider-only CLI overrides useful
	// without weakening the required provider/model validation in startRelay.
	for role, suppliedAssignment := range supplied {
		if suppliedAssignment == nil {
			continue
		}
		assignment := *suppliedAssignment
		current := roles[role]
		if strings.TrimSpace(assignment.Provider) != "" {
			current.Provider = assignment.Provider
		}
		if strings.TrimSpace(assignment.Model) != "" {
			current.Model = assignment.Model
		}
		roles[role] = current
	}
	return roles
}

func resolveRelayPlanPath(parentCWD, raw string) (string, error) {
	if strings.TrimSpace(raw) == "" {
		return "", fmt.Errorf("%w: plan_path is required", errRelayPlanPath)
	}
	base := strings.TrimSpace(parentCWD)
	if base == "" {
		return "", fmt.Errorf("%w: parent session has no cwd", errRelayPlanPath)
	}
	base, err := filepath.Abs(base)
	if err != nil {
		return "", fmt.Errorf("%w: invalid parent cwd", errRelayPlanPath)
	}
	base, err = filepath.EvalSymlinks(filepath.Clean(base))
	if err != nil {
		return "", fmt.Errorf("%w: parent cwd is unavailable", errRelayPlanPath)
	}
	if info, statErr := os.Stat(base); statErr != nil || !info.IsDir() {
		return "", fmt.Errorf("%w: parent cwd is not a directory", errRelayPlanPath)
	}

	plan := strings.TrimSpace(raw)
	if !filepath.IsAbs(plan) {
		plan = filepath.Join(base, plan)
	}
	plan = filepath.Clean(plan)
	resolved, err := filepath.EvalSymlinks(plan)
	if err != nil {
		return "", fmt.Errorf("%w: plan file does not exist", errRelayPlanPath)
	}
	resolved = filepath.Clean(resolved)
	info, err := os.Stat(resolved)
	if err != nil || !info.Mode().IsRegular() {
		return "", fmt.Errorf("%w: plan_path must be a regular file", errRelayPlanPath)
	}
	if !strings.EqualFold(filepath.Ext(resolved), ".md") {
		return "", fmt.Errorf("%w: plan_path must point to a .md file", errRelayPlanPath)
	}
	if info.Size() > relayPlanMaxBytes {
		return "", fmt.Errorf("%w: plan_path must be at most %d bytes", errRelayPlanPath, relayPlanMaxBytes)
	}
	gitRoot := relayGitRoot(base)
	allowed, err := isPathUnderAllowedRoots(resolved, base, gitRoot)
	if err != nil {
		return "", fmt.Errorf("%w: plan_path is outside the parent cwd and git root", errRelayPlanPath)
	}
	if !allowed {
		// Repository-local plan queues may be directory junctions whose physical
		// target is outside the repository. Permit that layout when the lexical
		// path is still under an allowed root, but do not extend this exception
		// to a plan file that is itself a symlink.
		lexicallyAllowed := false
		if planInfo, lstatErr := os.Lstat(plan); lstatErr == nil && planInfo.Mode()&os.ModeSymlink == 0 {
			lexicallyAllowed = isUnder(plan, base) || isUnder(plan, gitRoot)
		}
		if !lexicallyAllowed {
			return "", fmt.Errorf("%w: plan_path is outside the parent cwd and git root", errRelayPlanPath)
		}
	}
	return resolved, nil
}

func relayGitRoot(cwd string) string {
	ctx, cancel := context.WithTimeout(context.Background(), gitCommandTimeout)
	defer cancel()
	out, err := runGit(ctx, cwd, "rev-parse", "--show-toplevel")
	if err == nil {
		if root := strings.TrimSpace(string(out)); root != "" {
			if resolved, resolveErr := filepath.EvalSymlinks(root); resolveErr == nil {
				return filepath.Clean(resolved)
			}
		}
	}
	return findGitRoot(cwd)
}

func (s *Server) resumableRelayCandidate(parentID int, orchestrationID string) (*relayRun, error) {
	if id := strings.TrimSpace(orchestrationID); id != "" {
		run := s.relayByID(id)
		if run == nil {
			return nil, errRelayNotFound
		}
		return run, nil
	}
	var candidates []*relayRun
	for _, run := range s.relaysForParent(parentID) {
		run.mu.Lock()
		resumable := run.state == relayStateStopped && relayResumableReason(run.reason)
		run.mu.Unlock()
		if resumable {
			candidates = append(candidates, run)
		}
	}
	switch len(candidates) {
	case 0:
		return nil, errRelayNotFound
	case 1:
		return candidates[0], nil
	default:
		return nil, errRelayAmbiguous
	}
}

func validateRelayCleanupPath(parentCWD, path string, cfg config.OrchestrationConfig) error {
	root := filepath.Clean(relayWorktreeRoot(parentCWD, cfg))
	cleanPath := filepath.Clean(path)
	if cleanPath == root {
		return errors.New("relay worktree path cannot be the worktree root")
	}
	ok, err := isPathUnderAllowedRoots(cleanPath, root)
	if err != nil || !ok {
		return errors.New("relay worktree path is outside the configured worktree root")
	}
	return nil
}

func relaySnapshot(run *relayRun) (proto.RelayStatus, []proto.RelayEvent) {
	run.mu.Lock()
	defer run.mu.Unlock()
	return run.statusLocked(), run.eventsLocked()
}

func relaySnapshotWithBoard(run *relayRun) (proto.RelayStatus, string) {
	run.mu.Lock()
	defer run.mu.Unlock()
	return run.statusLocked(), run.boardPath
}

func (s *Server) relayAmbiguousDetail(parentID int) string {
	ids := make([]string, 0)
	for _, run := range s.relaysForParent(parentID) {
		run.mu.Lock()
		if !run.terminalLocked() {
			ids = append(ids, run.orchestrationID)
		}
		run.mu.Unlock()
	}
	sort.Strings(ids)
	if len(ids) == 0 {
		return "more than one relay is running; specify orchestration_id"
	}
	return fmt.Sprintf("more than one relay is running; specify orchestration_id (active: %s)", strings.Join(ids, ", "))
}

func (s *Server) writeRelayAPIError(w http.ResponseWriter, status int, code string, err error, parentID int) {
	if err == nil {
		writeJSONError(w, status, code, "relay request failed")
		return
	}
	if errors.Is(err, errRelayAmbiguous) {
		if code == "" {
			code = "relay_ambiguous"
		}
		writeJSONError(w, http.StatusBadRequest, code, s.relayAmbiguousDetail(parentID))
		return
	}
	var missing errRelayRolesMissing
	if errors.As(err, &missing) {
		writeJSONError(w, http.StatusBadRequest, "relay_roles_missing", fmt.Sprintf("missing role: %s", missing.Role))
		return
	}
	var limit errOrchestrationLimit
	if errors.As(err, &limit) {
		detail := fmt.Sprintf("relay running=%d max_children_per_parent=%d; raise orchestration.max_children_per_parent to run more", limit.Running, limit.Max)
		if limit.Limit != "children_per_parent" {
			detail = fmt.Sprintf("%s; %s", limit.Error(), detail)
		}
		writeJSONError(w, http.StatusTooManyRequests, "orchestration_limit", detail)
		return
	}
	var worktreeErr errRelayWorktree
	switch {
	case errors.Is(err, errRelayParentNotFound):
		writeJSONError(w, http.StatusNotFound, "not_found", "parent session not found")
	case errors.Is(err, errRelayNotFound):
		writeJSONError(w, http.StatusNotFound, "relay_not_found", "relay not found")
	case errors.Is(err, errRelayPlanPath), errors.Is(err, errRelayMode):
		writeJSONError(w, http.StatusBadRequest, "bad_request", err.Error())
	case errors.Is(err, errRelayNotGit):
		writeJSONError(w, http.StatusBadRequest, "relay_not_git", err.Error())
	case errors.As(err, &worktreeErr):
		writeJSONError(w, http.StatusInternalServerError, "relay_worktree_error", "relay worktree setup failed")
	case errors.Is(err, errRelayNotRunning):
		writeJSONError(w, http.StatusConflict, "relay_not_running", err.Error())
	case errors.Is(err, errRelayNotResumable):
		writeJSONError(w, http.StatusConflict, "relay_not_resumable", err.Error())
	case errors.Is(err, errRelayWorktreeMissing):
		writeJSONError(w, http.StatusConflict, "relay_worktree_missing", err.Error())
	default:
		if code == "" {
			code = "spawn_error"
		}
		writeJSONError(w, status, code, errorDetail("relay request failed", err))
	}
}
