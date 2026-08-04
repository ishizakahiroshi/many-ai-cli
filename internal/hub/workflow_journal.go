package hub

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

const (
	workflowJournalPollInterval = time.Second
	workflowJournalSettleDelay  = 10 * time.Second
	workflowJournalIdleStop     = 60 * time.Second
	workflowJournalTimeout      = 5 * time.Minute
	workflowJournalLookback     = 2 * time.Second
)

// workflowJournalEvent is intentionally the only JSON decoding shape used for
// journal lines. In particular it must never acquire a Result field: result
// bodies are neither retained, logged, forwarded, nor persisted by the Hub.
type workflowJournalEvent struct {
	Type    string `json:"type"`
	AgentID string `json:"agentId"`
}

// workflowJournalFileState lives only in a live session. Agent IDs are kept as
// in-memory sets solely to make append/restart processing idempotent.
type workflowJournalFileState struct {
	Offset      int64
	Started     map[string]struct{}
	Results     map[string]struct{}
	Frozen      bool
	ModTime     time.Time
	LastEventAt time.Time
}

type workflowJournalGroup struct {
	SessionDir string
	Files      map[string]workflowJournalFileState
	Started    int
	Done       int
}

func cloneWorkflowJournalSet(src map[string]struct{}) map[string]struct{} {
	if src == nil {
		return nil
	}
	dst := make(map[string]struct{}, len(src))
	for key := range src {
		dst[key] = struct{}{}
	}
	return dst
}

func cloneWorkflowJournalFiles(src map[string]workflowJournalFileState) map[string]workflowJournalFileState {
	if src == nil {
		return nil
	}
	dst := make(map[string]workflowJournalFileState, len(src))
	for path, state := range src {
		state.Started = cloneWorkflowJournalSet(state.Started)
		state.Results = cloneWorkflowJournalSet(state.Results)
		dst[path] = state
	}
	return dst
}

// tailWorkflowJournal consumes only newline-terminated bytes. ReadBytes has no
// Scanner-style 64 KiB token limit, so large result bodies remain supported.
// A truncate freezes the file at its last trusted counts instead of replaying.
func tailWorkflowJournal(path string, prior workflowJournalFileState) (workflowJournalFileState, error) {
	info, err := os.Stat(path)
	if err != nil {
		return prior, err
	}
	prior.ModTime = info.ModTime()
	if prior.Frozen {
		return prior, nil
	}
	if info.Size() < prior.Offset {
		prior.Frozen = true
		return prior, nil
	}
	if prior.Started == nil {
		prior.Started = make(map[string]struct{})
	}
	if prior.Results == nil {
		prior.Results = make(map[string]struct{})
	}
	if info.Size() == prior.Offset {
		return prior, nil
	}

	f, err := os.Open(path) // #nosec G304 -- path is derived from the local Claude transcript root.
	if err != nil {
		return prior, err
	}
	defer f.Close()
	if _, err := f.Seek(prior.Offset, io.SeekStart); err != nil {
		return prior, err
	}

	reader := bufio.NewReader(f)
	for {
		line, readErr := reader.ReadBytes('\n')
		if len(line) == 0 && readErr != nil {
			if errors.Is(readErr, io.EOF) {
				return prior, nil
			}
			return prior, readErr
		}
		if len(line) == 0 || line[len(line)-1] != '\n' {
			// Incomplete tail: leave Offset unchanged for these bytes so the next
			// poll rereads the complete JSON object.
			return prior, nil
		}
		prior.Offset += int64(len(line))
		line = line[:len(line)-1]
		if len(line) > 0 && line[len(line)-1] == '\r' {
			line = line[:len(line)-1]
		}
		var event workflowJournalEvent
		if err := json.Unmarshal(line, &event); err == nil && event.AgentID != "" {
			changed := false
			switch event.Type {
			case "started":
				if _, exists := prior.Started[event.AgentID]; !exists {
					prior.Started[event.AgentID] = struct{}{}
					changed = true
				}
			case "result":
				if _, exists := prior.Results[event.AgentID]; !exists {
					prior.Results[event.AgentID] = struct{}{}
					changed = true
				}
			}
			if changed {
				prior.LastEventAt = info.ModTime()
			}
		}
		if readErr != nil && !errors.Is(readErr, io.EOF) {
			return prior, readErr
		}
	}
}

func workflowJournalCounts(files map[string]workflowJournalFileState) (started, done int, lastEvent, lastMTime time.Time) {
	for _, state := range files {
		started += len(state.Started)
		done += len(state.Results)
		if state.LastEventAt.After(lastEvent) {
			lastEvent = state.LastEventAt
		}
		if state.ModTime.After(lastMTime) {
			lastMTime = state.ModTime
		}
	}
	return
}

func workflowJournalProjectDir(cwd, homeDir, claudeDir string) string {
	root := strings.TrimSpace(claudeDir)
	if root == "" {
		home := strings.TrimSpace(homeDir)
		if home == "" {
			home, _ = os.UserHomeDir()
		}
		if home != "" {
			root = filepath.Join(home, ".claude")
		}
	}
	if strings.TrimSpace(cwd) == "" || strings.TrimSpace(root) == "" {
		return ""
	}
	return filepath.Join(root, "projects", claudeProjectDirName(cwd))
}

func scanWorkflowJournalSession(sessionDir string, detectedAt time.Time, existing map[string]workflowJournalFileState) (map[string]workflowJournalFileState, error) {
	workflowDir := filepath.Join(sessionDir, "subagents", "workflows")
	entries, err := os.ReadDir(workflowDir)
	if err != nil {
		return nil, err
	}
	files := cloneWorkflowJournalFiles(existing)
	if files == nil {
		files = make(map[string]workflowJournalFileState)
	}
	for _, entry := range entries {
		if !entry.IsDir() || !strings.HasPrefix(entry.Name(), "wf_") {
			continue
		}
		path := filepath.Join(workflowDir, entry.Name(), "journal.jsonl")
		info, err := os.Stat(path)
		if err != nil || info.IsDir() {
			continue
		}
		state, known := files[path]
		if !known && info.ModTime().Before(detectedAt.Add(-workflowJournalLookback)) {
			continue
		}
		state, err = tailWorkflowJournal(path, state)
		if err != nil {
			continue
		}
		// A run already complete before VT first detected the current workflow is
		// historical residue. Do not let it contribute to the new run.
		if !known && len(state.Started) > 0 && len(state.Results) == len(state.Started) && info.ModTime().Before(detectedAt) {
			continue
		}
		if len(state.Started) > 0 || known {
			files[path] = state
		}
	}
	return files, nil
}

func discoverWorkflowJournalGroups(projectDir string, detectedAt time.Time) ([]workflowJournalGroup, error) {
	entries, err := os.ReadDir(projectDir)
	if err != nil {
		return nil, err
	}
	groups := make([]workflowJournalGroup, 0, 2)
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		sessionDir := filepath.Join(projectDir, entry.Name())
		files, err := scanWorkflowJournalSession(sessionDir, detectedAt, nil)
		if err != nil || len(files) == 0 {
			continue
		}
		started, done, _, _ := workflowJournalCounts(files)
		if started == 0 {
			continue
		}
		groups = append(groups, workflowJournalGroup{SessionDir: sessionDir, Files: files, Started: started, Done: done})
	}
	return groups, nil
}

// selectWorkflowJournalGroup applies the ambiguity-safe association rule. A
// single transcript session is safe to use; with multiple concurrent sessions,
// exactly one must satisfy result <= VT done <= started.
func selectWorkflowJournalGroup(groups []workflowJournalGroup, vtDone int) (workflowJournalGroup, bool) {
	if len(groups) == 1 {
		return groups[0], true
	}
	match := -1
	for i := range groups {
		if groups[i].Done <= vtDone && vtDone <= groups[i].Started {
			if match >= 0 {
				return workflowJournalGroup{}, false
			}
			match = i
		}
	}
	if match < 0 {
		return workflowJournalGroup{}, false
	}
	return groups[match], true
}

func (s *Server) workflowJournalEnabled() bool {
	if s == nil || s.cfg == nil {
		return false
	}
	return s.snapshotCfg().Workflow.JournalEnabled
}

func (s *Server) workflowCompletionPushEnabled() bool {
	if s == nil || s.cfg == nil {
		return false
	}
	return s.snapshotCfg().UserPrefs.WorkflowCompletionNotify.Enabled
}

// notifyWorkflowCompletionPush deliberately sends only aggregate counts. The
// session display name is supplied by notifyApprovalPush; no journal content or
// agent ID crosses the process boundary.
func (s *Server) notifyWorkflowCompletionPush(id int, progress *proto.WorkflowProgress) {
	if progress == nil || !progress.Settled || !s.workflowCompletionPushEnabled() {
		return
	}
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	provider := "claude"
	if ses != nil && ses.Provider != "" {
		provider = ses.Provider
	}
	s.sessionsMu.Unlock()
	// Push 本文はフロント i18n を通らず OS 通知に素通しされる。既存 Push の既定
	// 文言（push.go "Approval is waiting." 等）に合わせ英語固定にする。
	body := "Workflow completed"
	if progress.Total > 0 {
		body = fmt.Sprintf("Workflow completed (%d/%d agents)", progress.Done, progress.Total)
	} else if progress.Done > 0 {
		body = fmt.Sprintf("Workflow completed (%d agents)", progress.Done)
	}
	idempotencyKey := workflowProgressSignature(progress)
	if len(idempotencyKey) > 16 {
		idempotencyKey = idempotencyKey[:16]
	}
	s.notifyApprovalPush(id, fmt.Sprintf("workflow-%d-%s", id, idempotencyKey), provider, body, "")
}

func (s *Server) startWorkflowJournalLocked(id int, ses *session, now time.Time) {
	if ses == nil || ses.Provider != "claude" {
		return
	}
	// The VT heartbeat replays the same retained frame while a session is
	// output-idle. Do not let that heartbeat wake a watcher intentionally put
	// into the 60-second dormant state or replace its one-shot timeout timer.
	// Real PTY output clears OutputIdle before the VT scan; a new run also has a
	// different signature, so both cases are allowed to resume immediately.
	if ses.workflowJournalDormant && ses.Activity.OutputIdle &&
		ses.workflowJournalDormantVTSignature == ses.workflowVTSignature {
		return
	}
	if ses.journalCounts.Settled {
		if ses.workflowJournalSettledVTSignature == ses.workflowVTSignature {
			return
		}
		// A different VT frame after a settled run starts a new in-memory run.
		ses.workflowJournalFiles = nil
		ses.workflowJournalSessionDir = ""
		ses.journalCounts = workflowCounts{}
		ses.workflowJournalLastEventAt = time.Time{}
		ses.workflowJournalLastMTime = time.Time{}
		ses.workflowCompletionNotified = false
		ses.workflowCompletionSignature = ""
	}
	if ses.workflowJournalDetectedAt.IsZero() || ses.workflowJournalFiles == nil && ses.workflowJournalSessionDir == "" && !ses.workflowJournalRunning {
		ses.workflowJournalDetectedAt = now
		ses.workflowJournalPendingAssociation = true
	}
	wasRunning := ses.workflowJournalRunning
	ses.workflowJournalRunning = true
	ses.workflowJournalDormant = false
	ses.workflowJournalDormantVTSignature = ""
	if !wasRunning {
		s.scheduleWorkflowJournalLocked(id, ses, now)
	}
}

func (s *Server) scheduleWorkflowJournalLocked(id int, ses *session, due time.Time) {
	if ses.workflowJournalTimer != nil && !due.Before(ses.workflowJournalDue) {
		return
	}
	if ses.workflowJournalTimer != nil {
		ses.workflowJournalTimer.Stop()
	}
	ses.workflowJournalGeneration++
	generation := ses.workflowJournalGeneration
	ses.workflowJournalDue = due
	delay := time.Until(due)
	if delay < 0 {
		delay = 0
	}
	expected := ses
	ses.workflowJournalTimer = time.AfterFunc(delay, func() { s.runWorkflowJournalPoll(id, expected, generation) })
}

func (s *Server) runWorkflowJournalPoll(id int, expected *session, generation uint64) {
	if !s.workflowJournalEnabled() {
		s.disableWorkflowJournal(id, expected)
		return
	}
	now := time.Now()
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses != expected || ses.Provider != "claude" || ses.workflowJournalGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	ses.workflowJournalTimer = nil
	projectDir := workflowJournalProjectDir(ses.CWD, ses.HomeDir, ses.ClaudeDir)
	detectedAt := ses.workflowJournalDetectedAt
	linkedDir := ses.workflowJournalSessionDir
	existing := cloneWorkflowJournalFiles(ses.workflowJournalFiles)
	vtDone := ses.vtCounts.Done
	s.sessionsMu.Unlock()

	var files map[string]workflowJournalFileState
	var selectedDir string
	associated := false
	if linkedDir != "" {
		if scanned, err := scanWorkflowJournalSession(linkedDir, detectedAt, existing); err == nil {
			files, selectedDir, associated = scanned, linkedDir, true
		}
	} else if projectDir != "" {
		if groups, err := discoverWorkflowJournalGroups(projectDir, detectedAt); err == nil {
			if group, ok := selectWorkflowJournalGroup(groups, vtDone); ok {
				files, selectedDir, associated = group.Files, group.SessionDir, true
			}
		}
	}
	s.applyWorkflowJournalPoll(id, expected, generation, files, selectedDir, associated, now)
}

func workflowJournalVTIncomplete(ses *session) bool {
	if ses == nil || !ses.workflowVTHasSignal {
		return false
	}
	// VT 側が settle 済み（done==total / 凍結判定 / セッション終端）なら、その
	// stale フレームの未完カウントで journal settle をブロックしない（F1 派生）。
	if ses.vtCounts.Settled {
		return false
	}
	return ses.vtCounts.WaitingDynamic > 0 || ses.vtCounts.Running > 0 || ses.vtCounts.Pending > 0 ||
		(ses.vtCounts.Total > 0 && ses.vtCounts.Done < ses.vtCounts.Total)
}

func (s *Server) applyWorkflowJournalPoll(id int, expected *session, generation uint64, files map[string]workflowJournalFileState, sessionDir string, associated bool, now time.Time) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses != expected || ses.workflowJournalGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	if associated {
		ses.workflowJournalFiles = files
		ses.workflowJournalSessionDir = sessionDir
		ses.workflowJournalPendingAssociation = false
		started, done, lastEvent, lastMTime := workflowJournalCounts(files)
		ses.workflowJournalLastEventAt = lastEvent
		ses.workflowJournalLastMTime = lastMTime
		ses.journalCounts = workflowCounts{Detected: true, Started: started, Done: done, Total: started, ObservedAt: now}
		complete := started > 0 && done == started
		quietSince := lastMTime
		if complete && !workflowJournalVTIncomplete(ses) && !quietSince.IsZero() && now.Sub(quietSince) >= workflowJournalSettleDelay {
			ses.journalCounts.Settled = true
			ses.journalCounts.SettledBy = "journal"
			ses.workflowJournalSettledVTSignature = ses.workflowVTSignature
		} else if started > done && (!ses.workflowVTHasSignal || ses.vtCounts.Settled) && !lastMTime.IsZero() && now.Sub(lastMTime) >= workflowJournalTimeout {
			ses.journalCounts.Settled = true
			ses.journalCounts.SettledBy = "timeout"
			ses.workflowJournalSettledVTSignature = ses.workflowVTSignature
		}
	} else {
		// One complete discovery pass without an unambiguous candidate releases
		// VT settle. Discovery continues while active, but journal data is not used.
		ses.workflowJournalPendingAssociation = false
	}

	out, shouldBroadcast, shouldNotify := s.workflowProgressForBroadcastLocked(ses, now)
	settled := out != nil && out.Settled
	if settled {
		ses.workflowJournalRunning = false
		ses.workflowJournalDormant = false
		ses.workflowJournalDormantVTSignature = ""
	} else {
		lastActivity := ses.workflowJournalLastMTime
		if lastActivity.IsZero() {
			lastActivity = ses.workflowJournalDetectedAt
		}
		if ses.Activity.OutputIdle && !lastActivity.IsZero() && now.Sub(lastActivity) >= workflowJournalIdleStop {
			ses.workflowJournalRunning = false
			ses.workflowJournalDormant = true
			ses.workflowJournalDormantVTSignature = ses.workflowVTSignature
			if ses.journalCounts.Detected && ses.journalCounts.Started > ses.journalCounts.Done {
				due := lastActivity.Add(workflowJournalTimeout)
				// `now` へ clamp すると、期日超過なのに settle 条件が満たせない間
				// AfterFunc(0) が自己再スケジュールする無スロットルの busy loop に
				// なる（敵対レビュー 2026-08-05 F2）。最低でも 1 poll 間隔は空ける。
				if minDue := now.Add(workflowJournalPollInterval); due.Before(minDue) {
					due = minDue
				}
				s.scheduleWorkflowJournalLocked(id, ses, due)
			}
		} else {
			ses.workflowJournalRunning = true
			ses.workflowJournalDormant = false
			ses.workflowJournalDormantVTSignature = ""
			s.scheduleWorkflowJournalLocked(id, ses, now.Add(workflowJournalPollInterval))
		}
	}
	s.sessionsMu.Unlock()
	if shouldBroadcast {
		s.broadcast(proto.Message{Type: "workflow_progress", SessionID: id, WorkflowProgress: out})
	}
	if shouldNotify {
		s.notifyWorkflowCompletionPush(id, out)
	}
}

func (s *Server) disableWorkflowJournal(id int, expected *session) {
	now := time.Now()
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses != expected {
		s.sessionsMu.Unlock()
		return
	}
	if ses.workflowJournalTimer != nil {
		ses.workflowJournalTimer.Stop()
		ses.workflowJournalTimer = nil
	}
	ses.workflowJournalRunning = false
	ses.workflowJournalDormant = false
	ses.workflowJournalDormantVTSignature = ""
	ses.workflowJournalPendingAssociation = false
	ses.journalCounts = workflowCounts{}
	out, shouldBroadcast, shouldNotify := s.workflowProgressForBroadcastLocked(ses, now)
	s.sessionsMu.Unlock()
	if shouldBroadcast {
		s.broadcast(proto.Message{Type: "workflow_progress", SessionID: id, WorkflowProgress: out})
	}
	if shouldNotify {
		s.notifyWorkflowCompletionPush(id, out)
	}
}
