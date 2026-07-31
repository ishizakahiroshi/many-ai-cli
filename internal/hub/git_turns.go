package hub

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

const maxGitTurnSnapshots = 100

// gitTurnSnapshot is kept only in memory for the lifetime of a live session.
// Tree object IDs are intentionally not serialized in the public session
// snapshot; callers select a turn by its monotonic number.
type gitTurnSnapshot struct {
	Turn      int
	StartedAt time.Time
	EndedAt   time.Time
	StartTree string
	EndTree   string
	Files     int
	Added     int
	Removed   int
}

type gitTurnSummary struct {
	Turn      int    `json:"turn"`
	StartedAt string `json:"started_at"`
	EndedAt   string `json:"ended_at"`
	Files     int    `json:"files_changed"`
	Added     int    `json:"added"`
	Removed   int    `json:"removed"`
}

type gitTurnsResp struct {
	OK       bool             `json:"ok"`
	GitRoot  string           `json:"git_root"`
	RepoName string           `json:"repo_name"`
	Turns    []gitTurnSummary `json:"turns"`
}

// captureGitTurnStart records the worktree immediately before a confirmed
// user turn is delivered to the provider. It is deliberately synchronous:
// sending the input first would allow the provider to edit files before the
// baseline tree is complete.
func (s *Server) captureGitTurnStart(sessionID int) {
	var ses *session
	var captureDone chan struct{}
	for {
		s.sessionsMu.Lock()
		ses = s.sessions[sessionID]
		if ses == nil {
			s.sessionsMu.Unlock()
			return
		}
		if ses.gitTurnCaptureInFlight {
			// A previous turn end is still materializing. Waiting here is
			// intentional: handleInput calls us before submitInput, so provider
			// delivery cannot overtake the next turn's baseline snapshot.
			waitFor := ses.gitTurnCaptureDone
			if waitFor == nil {
				// Defensive recovery for an impossible/legacy partial state.
				ses.gitTurnCaptureInFlight = false
				s.sessionsMu.Unlock()
				continue
			}
			ses.gitTurnCaptureWaiters++
			s.sessionsMu.Unlock()
			<-waitFor
			s.sessionsMu.Lock()
			if ses.gitTurnCaptureWaiters > 0 {
				ses.gitTurnCaptureWaiters--
			}
			s.sessionsMu.Unlock()
			continue
		}
		if ses.gitTurnStartTree != "" {
			s.sessionsMu.Unlock()
			return
		}
		captureDone = make(chan struct{})
		ses.gitTurnCaptureInFlight = true
		ses.gitTurnCaptureDone = captureDone
		s.sessionsMu.Unlock()
		break
	}

	gitRoot, _, err := s.resolveGitRoot(sessionID)
	if err != nil {
		s.finishGitTurnCaptureFailure(sessionID, ses, captureDone, false)
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), gitCommandTimeout)
	defer cancel()
	tree, err := writeGitWorktreeTree(ctx, gitRoot)

	s.sessionsMu.Lock()
	if s.sessions[sessionID] == ses && ses.gitTurnCaptureDone == captureDone {
		if err != nil {
			s.logger.Debug("git turn start snapshot skipped", "session_id", sessionID, "err", err)
		} else {
			ses.gitTurnStartTree = tree
			ses.gitTurnStartedAt = time.Now()
		}
		ses.gitTurnCaptureInFlight = false
		ses.gitTurnCaptureDone = nil
	}
	close(captureDone)
	s.sessionsMu.Unlock()
}

// captureGitTurnEnd closes the pending turn at the existing DONE boundary.
// It marks the capture as pending synchronously, then runs Git I/O in a
// goroutine so PTY processing is not blocked.
func (s *Server) captureGitTurnEnd(sessionID int, endedAtText string) {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	if ses == nil || ses.gitTurnStartTree == "" || ses.gitTurnCaptureInFlight {
		s.sessionsMu.Unlock()
		return
	}
	captureDone := make(chan struct{})
	ses.gitTurnCaptureInFlight = true
	ses.gitTurnCaptureDone = captureDone
	startTree := ses.gitTurnStartTree
	startedAt := ses.gitTurnStartedAt
	s.sessionsMu.Unlock()

	go s.captureGitTurnEndWorker(sessionID, endedAtText, ses, captureDone, startTree, startedAt)
}

func (s *Server) captureGitTurnEndWorker(sessionID int, endedAtText string, ses *session, captureDone chan struct{}, startTree string, startedAt time.Time) {
	gitRoot, _, err := s.resolveGitRoot(sessionID)
	if err != nil {
		s.finishGitTurnCaptureFailure(sessionID, ses, captureDone, true)
		return
	}
	snapshotCtx, snapshotCancel := context.WithTimeout(context.Background(), gitCommandTimeout)
	endTree, err := writeGitWorktreeTree(snapshotCtx, gitRoot)
	snapshotCancel()
	if err != nil {
		s.logger.Warn("git turn end snapshot failed", "session_id", sessionID, "err", err)
		s.finishGitTurnCaptureFailure(sessionID, ses, captureDone, true)
		return
	}

	diffCtx, diffCancel := context.WithTimeout(context.Background(), gitCommandTimeout)
	diff, diffErr := gitTreeDiff(diffCtx, gitRoot, startTree, endTree)
	diffCancel()
	if diffErr != nil {
		s.logger.Warn("git turn summary failed", "session_id", sessionID, "err", diffErr)
		diff = gitDiffResp{OK: true, GitRoot: gitRoot, RepoName: filepath.Base(gitRoot)}
	}
	endedAt, err := time.Parse(time.RFC3339, endedAtText)
	if err != nil {
		endedAt = time.Now()
	}

	s.sessionsMu.Lock()
	if s.sessions[sessionID] != ses || ses.gitTurnCaptureDone != captureDone {
		close(captureDone)
		s.sessionsMu.Unlock()
		return
	}
	turnNo := 1
	if n := len(ses.gitTurns); n > 0 {
		turnNo = ses.gitTurns[n-1].Turn + 1
	}
	turn := gitTurnSnapshot{
		Turn:      turnNo,
		StartedAt: startedAt,
		EndedAt:   endedAt,
		StartTree: startTree,
		EndTree:   endTree,
		Files:     diff.Summary.FilesChanged,
		Added:     diff.Summary.Added,
		Removed:   diff.Summary.Removed,
	}
	ses.gitTurns = append(ses.gitTurns, turn)
	if len(ses.gitTurns) > maxGitTurnSnapshots {
		ses.gitTurns = append([]gitTurnSnapshot(nil), ses.gitTurns[len(ses.gitTurns)-maxGitTurnSnapshots:]...)
	}
	ses.gitTurnStartTree = ""
	ses.gitTurnStartedAt = time.Time{}
	ses.gitTurnCaptureInFlight = false
	ses.gitTurnCaptureDone = nil
	close(captureDone)
	s.sessionsMu.Unlock()

	// A compact event lets the active session show its completion card without
	// polling. Reloaded clients recover the same state from /api/git-turns.
	s.broadcast(map[string]any{
		"type":          "git_turn",
		"session_id":    sessionID,
		"turn":          turn.Turn,
		"started_at":    turn.StartedAt.Format(time.RFC3339),
		"ended_at":      turn.EndedAt.Format(time.RFC3339),
		"files_changed": turn.Files,
		"added":         turn.Added,
		"removed":       turn.Removed,
	})
}

func (s *Server) finishGitTurnCaptureFailure(sessionID int, expected *session, captureDone chan struct{}, clearStart bool) {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	if ses == expected && ses.gitTurnCaptureDone == captureDone {
		ses.gitTurnCaptureInFlight = false
		ses.gitTurnCaptureDone = nil
		if clearStart {
			ses.gitTurnStartTree = ""
			ses.gitTurnStartedAt = time.Time{}
		}
	}
	close(captureDone)
	s.sessionsMu.Unlock()
}

// writeGitWorktreeTree materializes staged, unstaged, and untracked content in
// a temporary index. It writes ordinary loose Git objects only; the temporary
// index is removed immediately and the objects remain eligible for normal gc.
func writeGitWorktreeTree(ctx context.Context, gitRoot string) (string, error) {
	tmpDir, err := os.MkdirTemp("", "many-ai-cli-git-turn-")
	if err != nil {
		return "", fmt.Errorf("create temporary git index directory: %w", err)
	}
	defer os.RemoveAll(tmpDir)
	indexPath := filepath.Join(tmpDir, "index")
	env := []string{"GIT_INDEX_FILE=" + indexPath}

	if _, err := runGitEnv(ctx, gitRoot, env, "rev-parse", "--verify", "HEAD^{tree}"); err == nil {
		if _, err := runGitEnv(ctx, gitRoot, env, "read-tree", "HEAD"); err != nil {
			return "", err
		}
	} else if _, err := runGitEnv(ctx, gitRoot, env, "read-tree", "--empty"); err != nil {
		return "", err
	}
	if _, err := runGitEnv(ctx, gitRoot, env, "add", "-A", "--", "."); err != nil {
		return "", err
	}
	out, err := runGitEnv(ctx, gitRoot, env, "write-tree")
	if err != nil {
		return "", err
	}
	tree := strings.TrimSpace(string(out))
	if !validRevision(tree) {
		return "", fmt.Errorf("git write-tree returned an invalid object id")
	}
	return tree, nil
}

func runGitEnv(ctx context.Context, cwd string, extraEnv []string, args ...string) ([]byte, error) {
	full := append([]string{"-C", cwd}, args...)
	cmd := exec.CommandContext(ctx, "git", full...)
	cmd.Env = append(os.Environ(), extraEnv...)
	out, err := cmd.Output()
	if err != nil {
		var detail string
		if exitErr, ok := err.(*exec.ExitError); ok {
			detail = strings.TrimSpace(string(exitErr.Stderr))
		}
		if detail != "" {
			return out, fmt.Errorf("git %s: %s", strings.Join(args, " "), detail)
		}
		return out, fmt.Errorf("git %s: %w", strings.Join(args, " "), err)
	}
	return out, nil
}

func gitTreeDiff(ctx context.Context, gitRoot, startTree, endTree string) (gitDiffResp, error) {
	if !validRevision(startTree) || !validRevision(endTree) {
		return gitDiffResp{}, fmt.Errorf("invalid turn tree")
	}
	nameOut, err := runGit(ctx, gitRoot, "diff", "--name-status", "--no-renames", startTree, endTree, "--")
	if err != nil {
		return gitDiffResp{}, err
	}
	files := parseNameStatus(string(nameOut))
	if numOut, numErr := runGit(ctx, gitRoot, "diff", "--numstat", "--no-renames", startTree, endTree, "--"); numErr == nil {
		applyNumstat(files, string(numOut))
	}
	for i := range files {
		diffOut, diffErr := runGit(ctx, gitRoot, "diff", "--no-ext-diff", "--no-color", "--no-renames", startTree, endTree, "--", files[i].Path)
		if diffErr != nil {
			continue
		}
		diff := strings.TrimLeft(string(diffOut), "\n")
		if len(diff) > gitShowDiffMaxBytes {
			diff = diff[:gitShowDiffMaxBytes] + "\n(truncated)"
		}
		files[i].Diff = diff
	}
	summary := gitStatusSummary{FilesChanged: len(files)}
	for _, file := range files {
		summary.Added += file.Added
		summary.Removed += file.Removed
	}
	branch := ""
	if out, branchErr := runGit(ctx, gitRoot, "rev-parse", "--abbrev-ref", "HEAD"); branchErr == nil {
		branch = strings.TrimSpace(string(out))
	}
	return gitDiffResp{
		OK:       true,
		GitRoot:  gitRoot,
		RepoName: filepath.Base(gitRoot),
		Branch:   branch,
		HeadHash: endTree,
		Files:    files,
		Summary:  summary,
	}, nil
}

func (s *Server) handleGitTurns(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	sid, ok := parseSessionID(r.URL.Query().Get("session"))
	if !ok {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	gitRoot, _, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[sid]
	if ses == nil {
		s.sessionsMu.Unlock()
		writeGitError(w, http.StatusBadRequest, "bad_session", "session not found")
		return
	}
	turns := append([]gitTurnSnapshot(nil), ses.gitTurns...)
	s.sessionsMu.Unlock()

	items := make([]gitTurnSummary, 0, len(turns))
	for _, turn := range turns {
		items = append(items, gitTurnSummary{
			Turn:      turn.Turn,
			StartedAt: turn.StartedAt.Format(time.RFC3339),
			EndedAt:   turn.EndedAt.Format(time.RFC3339),
			Files:     turn.Files,
			Added:     turn.Added,
			Removed:   turn.Removed,
		})
	}
	writeJSON(w, gitTurnsResp{OK: true, GitRoot: gitRoot, RepoName: filepath.Base(gitRoot), Turns: items})
}

func (s *Server) handleGitTurnDiff(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	q := r.URL.Query()
	sid, ok := parseSessionID(q.Get("session"))
	if !ok {
		writeGitError(w, http.StatusBadRequest, "bad_request", "session is required")
		return
	}
	turnNo, err := strconv.Atoi(q.Get("turn"))
	if err != nil || turnNo <= 0 {
		writeGitError(w, http.StatusBadRequest, "bad_request", "turn must be a positive integer")
		return
	}
	gitRoot, _, err := s.resolveGitRoot(sid)
	if err != nil {
		writeGitErrorFromResolve(w, sid, err)
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[sid]
	if ses == nil {
		s.sessionsMu.Unlock()
		writeGitError(w, http.StatusBadRequest, "bad_session", "session not found")
		return
	}
	var selected *gitTurnSnapshot
	for i := range ses.gitTurns {
		if ses.gitTurns[i].Turn == turnNo {
			cp := ses.gitTurns[i]
			selected = &cp
			break
		}
	}
	s.sessionsMu.Unlock()
	if selected == nil {
		writeGitError(w, http.StatusNotFound, "not_found", "turn snapshot not found")
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), gitCommandTimeout)
	defer cancel()
	resp, err := gitTreeDiff(ctx, gitRoot, selected.StartTree, selected.EndTree)
	if err != nil {
		s.logger.Warn("git turn diff failed", "session_id", sid, "turn", turnNo, "err", err)
		writeGitError(w, http.StatusInternalServerError, "git_command_failed", sanitizeGitErrMsg(err))
		return
	}
	writeJSON(w, resp)
}
