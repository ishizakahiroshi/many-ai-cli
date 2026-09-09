package hub

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"time"
)

const maxGitTurnSnapshots = 100

const gitTurnIndexTempPrefix = "many-ai-cli-git-turn-"

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
		ses.doneSummaryMarkerSeen = false
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
	tree, err := s.writeGitTurnWorktreeTree(ctx, sessionID, ses, gitRoot)

	s.sessionsMu.Lock()
	live := s.sessions[sessionID]
	if gitTurnSessionMatches(live, ses) && live.gitTurnCaptureDone == captureDone {
		if err != nil {
			s.logger.Debug("git turn start snapshot skipped", "session_id", sessionID, "err", err)
		} else {
			live.gitTurnStartTree = tree
			live.gitTurnStartedAt = time.Now()
		}
		live.gitTurnCaptureInFlight = false
		live.gitTurnCaptureDone = nil
	}
	close(captureDone)
	s.sessionsMu.Unlock()
}

// captureGitTurnEnd closes the pending turn at the existing DONE boundary.
// It marks the capture as pending synchronously, then runs Git I/O in a
// goroutine so PTY processing is not blocked.
func (s *Server) captureGitTurnEnd(sessionID int, endedAtText string) {
	s.captureGitTurnEndWithCallback(sessionID, endedAtText, nil)
}

// captureGitTurnEndWithCallback closes the pending turn at the existing DONE
// boundary and optionally hands the immutable result to a caller after the
// Git I/O has completed. The callback runs outside sessionsMu, but the next
// turn remains blocked until it returns.
func (s *Server) captureGitTurnEndWithCallback(sessionID int, endedAtText string, callback func(gitTurnSnapshot)) {
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

	go s.captureGitTurnEndWorker(sessionID, endedAtText, ses, captureDone, startTree, startedAt, callback)
}

func (s *Server) captureGitTurnEndWorker(sessionID int, endedAtText string, ses *session, captureDone chan struct{}, startTree string, startedAt time.Time, callback func(gitTurnSnapshot)) {
	gitRoot, _, err := s.resolveGitRoot(sessionID)
	if err != nil {
		s.finishGitTurnCaptureFailure(sessionID, ses, captureDone, true)
		return
	}
	snapshotCtx, snapshotCancel := context.WithTimeout(context.Background(), gitCommandTimeout)
	endTree, err := s.writeGitTurnWorktreeTree(snapshotCtx, sessionID, ses, gitRoot)
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
	live := s.sessions[sessionID]
	// Reattach replaces the session pointer. Commit onto the live session when
	// it inherited this turn's index and still owns this capture channel.
	if !gitTurnSessionMatches(live, ses) || live.gitTurnCaptureDone != captureDone {
		close(captureDone)
		s.sessionsMu.Unlock()
		return
	}
	turnNo := 1
	if n := len(live.gitTurns); n > 0 {
		turnNo = live.gitTurns[n-1].Turn + 1
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
	live.gitTurns = append(live.gitTurns, turn)
	if len(live.gitTurns) > maxGitTurnSnapshots {
		live.gitTurns = append([]gitTurnSnapshot(nil), live.gitTurns[len(live.gitTurns)-maxGitTurnSnapshots:]...)
	}
	live.gitTurnStartTree = ""
	live.gitTurnStartedAt = time.Time{}
	s.sessionsMu.Unlock()

	// handoff は sessionsMu を離してから書く（Git I/O をロック内に持ち込まない）。
	// log.session_enabled とは無関係の独立ゲート（handoff.go 参照）。この 100 件
	// 上限・Hub 再起動で消える live.gitTurns とは別に、jsonl 側は保持期間まで残る。
	s.recordHandoffGitTurn(sessionID, gitRoot, turn, diff.Files)
	// 案 3（turn-summary、既定 off）: 有効な利用者だけ、このターンの終わりへ
	// 要約プロンプトを注入する。handoff.go 側の 2 段ゲート（記録 on かつ
	// intent_mode=turn-summary）を通らなければ即 return する軽量呼び出し。
	s.maybeInjectHandoffTurnSummary(sessionID, turn.Turn)

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
	if callback != nil {
		callback(turn)
	}
	// Keep the next confirmed input behind the callback. The callback may publish
	// a fallback summary, and publishDoneSummary must not mistake the next
	// turn's newly-created baseline for the turn that just ended.
	//
	// captureGitTurnStart only waits on gitTurnCaptureDone while
	// gitTurnCaptureInFlight is set, so both have to stay in place until the
	// callback returns. Clearing them before the call left the channel as the
	// only barrier, and nothing was reading it: the next baseline walked
	// straight past a callback that had not published its summary yet. That
	// race stayed invisible on Windows, where spawning Git for the new baseline
	// is slow enough to hide it, and only surfaced on the Linux and macOS
	// runners.
	s.sessionsMu.Lock()
	live = s.sessions[sessionID]
	if gitTurnSessionMatches(live, ses) && live.gitTurnCaptureDone == captureDone {
		live.gitTurnCaptureInFlight = false
		live.gitTurnCaptureDone = nil
	}
	close(captureDone)
	s.sessionsMu.Unlock()
}

func (s *Server) finishGitTurnCaptureFailure(sessionID int, expected *session, captureDone chan struct{}, clearStart bool) {
	s.sessionsMu.Lock()
	ses := s.sessions[sessionID]
	if gitTurnSessionMatches(ses, expected) && ses.gitTurnCaptureDone == captureDone {
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

// gitTurnSessionMatches reports whether current is the session a Git-turn
// worker started with, or the reattach replacement that inherited its index.
func gitTurnSessionMatches(current, expected *session) bool {
	if current == nil || expected == nil {
		return false
	}
	if current == expected {
		return true
	}
	return current.gitTurnIndexDir != "" && current.gitTurnIndexDir == expected.gitTurnIndexDir
}

// writeGitTurnWorktreeTree materializes a tree through the index retained by a
// live session. Keeping that index preserves Git's stat cache across turn
// boundaries, so unchanged files do not have to be re-read and hashed for
// every user message.
func (s *Server) writeGitTurnWorktreeTree(ctx context.Context, sessionID int, expected *session, gitRoot string) (string, error) {
	s.sessionsMu.Lock()
	current := s.sessions[sessionID]
	if !gitTurnSessionMatches(current, expected) {
		s.sessionsMu.Unlock()
		return "", fmt.Errorf("git turn session changed before index setup")
	}
	indexDir := current.gitTurnIndexDir
	s.sessionsMu.Unlock()

	if indexDir == "" {
		created, err := os.MkdirTemp("", gitTurnIndexTempPrefix)
		if err != nil {
			return "", fmt.Errorf("create reusable git index directory: %w", err)
		}
		s.sessionsMu.Lock()
		current = s.sessions[sessionID]
		if !gitTurnSessionMatches(current, expected) {
			s.sessionsMu.Unlock()
			removeGitTurnIndexDir(created)
			return "", fmt.Errorf("git turn session changed during index setup")
		}
		if current.gitTurnIndexDir == "" {
			current.gitTurnIndexDir = created
			indexDir = created
			created = ""
		} else {
			indexDir = current.gitTurnIndexDir
		}
		sessionsRoot := current.gitTurnIndexRoot
		previousHead := current.gitTurnIndexHead
		ready := current.gitTurnIndexReady
		s.sessionsMu.Unlock()
		if created != "" {
			removeGitTurnIndexDir(created)
		}
		return s.finishGitTurnWorktreeTree(ctx, sessionID, expected, gitRoot, indexDir, sessionsRoot, previousHead, ready)
	}

	s.sessionsMu.Lock()
	current = s.sessions[sessionID]
	if !gitTurnSessionMatches(current, expected) {
		s.sessionsMu.Unlock()
		return "", fmt.Errorf("git turn session changed before index setup")
	}
	sessionsRoot := current.gitTurnIndexRoot
	previousHead := current.gitTurnIndexHead
	ready := current.gitTurnIndexReady
	s.sessionsMu.Unlock()
	return s.finishGitTurnWorktreeTree(ctx, sessionID, expected, gitRoot, indexDir, sessionsRoot, previousHead, ready)
}

func (s *Server) finishGitTurnWorktreeTree(ctx context.Context, sessionID int, expected *session, gitRoot, indexDir, previousRoot, previousHead string, ready bool) (string, error) {
	indexPath := filepath.Join(indexDir, "index")
	if previousRoot != gitRoot {
		ready = false
	}
	if _, err := os.Stat(indexPath); err != nil {
		ready = false
	}
	tree, headTree, err := writeGitWorktreeTreeAtIndex(ctx, gitRoot, indexPath, ready, previousHead)

	s.sessionsMu.Lock()
	current := s.sessions[sessionID]
	cleanupIndex := false
	// Reattach replaces the session pointer but preserves the index directory.
	// In that case the completed Git command still owns the same logical cache.
	if current == expected || (current != nil && current.gitTurnIndexDir == indexDir) {
		if err != nil {
			current.gitTurnIndexReady = false
		} else {
			current.gitTurnIndexRoot = gitRoot
			current.gitTurnIndexHead = headTree
			current.gitTurnIndexReady = true
		}
	} else {
		// Dismiss may race with a Git command. Its immediate cleanup can fail on
		// Windows while git.exe still has the index open, so retry after the
		// command has released the file.
		cleanupIndex = true
	}
	s.sessionsMu.Unlock()
	if cleanupIndex {
		removeGitTurnIndexDir(indexDir)
	}
	return tree, err
}

// writeGitWorktreeTree is the cold, one-shot variant used by tests and callers
// that do not own a live session cache.
func writeGitWorktreeTree(ctx context.Context, gitRoot string) (string, error) {
	tmpDir, err := os.MkdirTemp("", gitTurnIndexTempPrefix)
	if err != nil {
		return "", fmt.Errorf("create temporary git index directory: %w", err)
	}
	defer removeGitTurnIndexDir(tmpDir)
	indexPath := filepath.Join(tmpDir, "index")
	tree, _, err := writeGitWorktreeTreeAtIndex(ctx, gitRoot, indexPath, false, "")
	return tree, err
}

func writeGitWorktreeTreeAtIndex(ctx context.Context, gitRoot, indexPath string, ready bool, previousHead string) (string, string, error) {
	env := []string{"GIT_INDEX_FILE=" + indexPath}

	currentHead := ""
	if out, err := runGitEnv(ctx, gitRoot, env, "rev-parse", "--verify", "HEAD^{tree}"); err == nil {
		currentHead = strings.TrimSpace(string(out))
	}
	if gitTurnIndexNeedsReset(ready, previousHead, currentHead) {
		if currentHead != "" {
			if _, err := runGitEnv(ctx, gitRoot, env, "read-tree", "HEAD"); err != nil {
				return "", "", err
			}
		} else if _, err := runGitEnv(ctx, gitRoot, env, "read-tree", "--empty"); err != nil {
			return "", "", err
		}
	}
	if _, err := runGitEnv(ctx, gitRoot, env, "add", "-A", "--", "."); err != nil {
		return "", "", err
	}
	out, err := runGitEnv(ctx, gitRoot, env, "write-tree")
	if err != nil {
		return "", "", err
	}
	tree := strings.TrimSpace(string(out))
	if !validRevision(tree) {
		return "", "", fmt.Errorf("git write-tree returned an invalid object id")
	}
	return tree, currentHead, nil
}

func gitTurnIndexNeedsReset(ready bool, previousHead, currentHead string) bool {
	return !ready || previousHead != currentHead
}

func removeGitTurnIndexDir(dir string) {
	clean := filepath.Clean(dir)
	parent := filepath.Dir(clean)
	tempRoot := filepath.Clean(os.TempDir())
	insideTempRoot := parent == tempRoot
	if runtime.GOOS == "windows" {
		insideTempRoot = strings.EqualFold(parent, tempRoot)
	}
	if !insideTempRoot || !strings.HasPrefix(filepath.Base(clean), gitTurnIndexTempPrefix) {
		return
	}
	_ = os.RemoveAll(clean)
}

func (s *Server) cleanupGitTurnIndexes() {
	s.sessionsMu.Lock()
	dirs := make([]string, 0, len(s.sessions))
	for _, ses := range s.sessions {
		if ses != nil && ses.gitTurnIndexDir != "" {
			dirs = append(dirs, ses.gitTurnIndexDir)
			ses.gitTurnIndexDir = ""
			ses.gitTurnIndexRoot = ""
			ses.gitTurnIndexHead = ""
			ses.gitTurnIndexReady = false
		}
	}
	s.sessionsMu.Unlock()
	for _, dir := range dirs {
		removeGitTurnIndexDir(dir)
	}
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
