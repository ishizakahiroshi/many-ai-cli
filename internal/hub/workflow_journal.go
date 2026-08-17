package hub

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

const (
	workflowJournalPollInterval   = time.Second
	workflowJournalSettleDelay    = 10 * time.Second
	workflowJournalIdleStop       = 60 * time.Second
	workflowJournalTimeout        = 5 * time.Minute
	workflowJournalLookback       = 2 * time.Second
	workflowJournalLineMax        = 1 * 1024 * 1024
	workflowJournalReadBuffer     = 64 * 1024
	workflowJournalReadBytesMax   = 4 * 1024 * 1024
	workflowJournalReadRecordsMax = 256
	workflowJournalReadTimeBudget = 100 * time.Millisecond
	workflowJournalFieldMax       = 256
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
	LineBytes   int64
	Parser      workflowJournalRecordParser
}

type workflowJournalGroup struct {
	SessionDir string
	Files      map[string]workflowJournalFileState
	Started    int
	Done       int
}

type workflowJournalReadBudget struct {
	MaxBytes   int
	MaxRecords int
	Deadline   time.Time
	Clock      func() time.Time
}

type workflowJournalReadStats struct {
	BytesRead int
	Records   int
	HitBudget bool
	Elapsed   time.Duration
}

func workflowJournalBudgetNow(budget workflowJournalReadBudget) time.Time {
	if budget.Clock != nil {
		return budget.Clock()
	}
	return time.Now()
}

func workflowJournalBudgetExpired(budget workflowJournalReadBudget) bool {
	return !workflowJournalBudgetNow(budget).Before(budget.Deadline)
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
		state.Parser.quoted = append([]byte(nil), state.Parser.quoted...)
		dst[path] = state
	}
	return dst
}

type workflowJournalParserMode uint8

const (
	workflowJournalParserNeedRoot workflowJournalParserMode = iota
	workflowJournalParserNeedKey
	workflowJournalParserNeedColon
	workflowJournalParserNeedValue
	workflowJournalParserString
	workflowJournalParserPrimitive
	workflowJournalParserComposite
	workflowJournalParserNeedComma
	workflowJournalParserComplete
)

// workflowJournalRecordParser is a bounded JSON object scanner. It extracts
// only the two metadata strings used by the workflow counter and skips all
// other values byte by byte, including a multi-megabyte result string.
type workflowJournalRecordParser struct {
	mode            workflowJournalParserMode
	started         bool
	invalid         bool
	stringIsKey     bool
	stringEscape    bool
	compositeDepth  int
	compositeString bool
	compositeEscape bool
	currentKey      string
	captureField    string
	quoted          []byte
	typeValue       string
	agentID         string
}

func (parser *workflowJournalRecordParser) reset() {
	*parser = workflowJournalRecordParser{
		mode:    workflowJournalParserNeedRoot,
		started: true,
	}
}

func (parser *workflowJournalRecordParser) ensureStarted() {
	if !parser.started {
		parser.reset()
	}
}

func (parser *workflowJournalRecordParser) invalidate() {
	parser.invalid = true
	parser.quoted = nil
}

func (parser *workflowJournalRecordParser) beginString(isKey bool) {
	parser.mode = workflowJournalParserString
	parser.stringIsKey = isKey
	parser.stringEscape = false
	parser.captureField = ""
	if isKey {
		parser.quoted = parser.quoted[:0]
	} else if parser.currentKey == "type" || parser.currentKey == "agentId" {
		parser.captureField = parser.currentKey
		parser.quoted = parser.quoted[:0]
	} else {
		parser.quoted = nil
	}
}

func (parser *workflowJournalRecordParser) captureByte(b byte) {
	if !parser.stringIsKey && parser.captureField == "" {
		return
	}
	if len(parser.quoted) < workflowJournalFieldMax {
		parser.quoted = append(parser.quoted, b)
	}
}

func decodeWorkflowJournalQuoted(raw []byte) (string, bool) {
	if len(raw) > workflowJournalFieldMax {
		return "", false
	}
	encoded := make([]byte, 0, len(raw)+2)
	encoded = append(encoded, '"')
	encoded = append(encoded, raw...)
	encoded = append(encoded, '"')
	var value string
	if err := json.Unmarshal(encoded, &value); err != nil {
		return "", false
	}
	return value, true
}

func (parser *workflowJournalRecordParser) finishString() {
	value, ok := decodeWorkflowJournalQuoted(parser.quoted)
	if !ok {
		parser.invalidate()
		return
	}
	if parser.stringIsKey {
		parser.currentKey = value
		parser.mode = workflowJournalParserNeedColon
	} else {
		switch parser.captureField {
		case "type":
			parser.typeValue = value
		case "agentId":
			parser.agentID = value
		}
		parser.mode = workflowJournalParserNeedComma
	}
	parser.captureField = ""
	parser.quoted = nil
}

func (parser *workflowJournalRecordParser) feed(b byte) {
	parser.ensureStarted()
	if parser.invalid || parser.mode == workflowJournalParserComplete {
		return
	}
	if parser.mode == workflowJournalParserString {
		if parser.stringEscape {
			parser.captureByte(b)
			parser.stringEscape = false
			return
		}
		if b == '\\' {
			parser.captureByte(b)
			parser.stringEscape = true
			return
		}
		if b == '"' {
			parser.finishString()
			return
		}
		parser.captureByte(b)
		return
	}
	if parser.mode == workflowJournalParserComposite {
		if parser.compositeString {
			if parser.compositeEscape {
				parser.compositeEscape = false
			} else if b == '\\' {
				parser.compositeEscape = true
			} else if b == '"' {
				parser.compositeString = false
			}
			return
		}
		switch b {
		case '"':
			parser.compositeString = true
		case '{', '[':
			parser.compositeDepth++
		case '}', ']':
			parser.compositeDepth--
			if parser.compositeDepth <= 0 {
				parser.compositeDepth = 0
				parser.mode = workflowJournalParserNeedComma
			}
		}
		return
	}

	switch parser.mode {
	case workflowJournalParserNeedRoot:
		switch b {
		case ' ', '\t', '\r':
			return
		case '{':
			parser.mode = workflowJournalParserNeedKey
		default:
			parser.invalidate()
		}
	case workflowJournalParserNeedKey:
		switch b {
		case ' ', '\t', '\r', '\n':
			return
		case '"':
			parser.beginString(true)
		case '}':
			parser.mode = workflowJournalParserComplete
		default:
			parser.invalidate()
		}
	case workflowJournalParserNeedColon:
		if b == ' ' || b == '\t' || b == '\r' {
			return
		}
		if b == ':' {
			parser.mode = workflowJournalParserNeedValue
		} else {
			parser.invalidate()
		}
	case workflowJournalParserNeedValue:
		switch b {
		case ' ', '\t', '\r':
			return
		case '"':
			parser.beginString(false)
		case '{', '[':
			parser.mode = workflowJournalParserComposite
			parser.compositeDepth = 1
			parser.compositeString = false
			parser.compositeEscape = false
		case 't', 'f', 'n', '-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9':
			parser.mode = workflowJournalParserPrimitive
		default:
			parser.invalidate()
		}
	case workflowJournalParserPrimitive:
		switch b {
		case ',':
			parser.mode = workflowJournalParserNeedKey
		case '}':
			parser.mode = workflowJournalParserComplete
		case ' ', '\t', '\r':
			parser.mode = workflowJournalParserNeedComma
		}
	case workflowJournalParserNeedComma:
		switch b {
		case ' ', '\t', '\r':
			return
		case ',':
			parser.mode = workflowJournalParserNeedKey
		case '}':
			parser.mode = workflowJournalParserComplete
		default:
			parser.invalidate()
		}
	case workflowJournalParserComplete:
		if b != ' ' && b != '\t' && b != '\r' {
			parser.invalidate()
		}
	}
}

func (parser *workflowJournalRecordParser) event() (workflowJournalEvent, bool) {
	if parser == nil || parser.invalid || parser.mode != workflowJournalParserComplete || parser.typeValue == "" || parser.agentID == "" {
		return workflowJournalEvent{}, false
	}
	return workflowJournalEvent{Type: parser.typeValue, AgentID: parser.agentID}, true
}

// tailWorkflowJournal streams newline-delimited records through a bounded
// metadata scanner. Offset advances through partial lines and Parser retains
// only the small JSON state needed to finish that line in a later poll. A
// truncate freezes the file at its last trusted counts instead of replaying.
func tailWorkflowJournal(path string, prior workflowJournalFileState) (workflowJournalFileState, error) {
	now := time.Now()
	next, _, err := tailWorkflowJournalWithBudget(path, prior, workflowJournalReadBudget{
		MaxBytes:   workflowJournalReadBytesMax,
		MaxRecords: workflowJournalReadRecordsMax,
		Deadline:   now.Add(workflowJournalReadTimeBudget),
	})
	return next, err
}

func tailWorkflowJournalWithBudget(path string, prior workflowJournalFileState, budget workflowJournalReadBudget) (workflowJournalFileState, workflowJournalReadStats, error) {
	started := time.Now()
	stats := workflowJournalReadStats{}
	defer func() { stats.Elapsed = time.Since(started) }()
	if budget.MaxBytes <= 0 {
		budget.MaxBytes = workflowJournalReadBytesMax
	}
	if budget.MaxRecords <= 0 {
		budget.MaxRecords = workflowJournalReadRecordsMax
	}
	if budget.Deadline.IsZero() {
		budget.Deadline = workflowJournalBudgetNow(budget).Add(workflowJournalReadTimeBudget)
	}
	info, err := os.Stat(path)
	if err != nil {
		return prior, stats, err
	}
	prior.ModTime = info.ModTime()
	if prior.Frozen {
		return prior, stats, nil
	}
	if info.Size() < prior.Offset {
		prior.Frozen = true
		return prior, stats, nil
	}
	if prior.Started == nil {
		prior.Started = make(map[string]struct{})
	}
	if prior.Results == nil {
		prior.Results = make(map[string]struct{})
	}
	if info.Size() == prior.Offset {
		return prior, stats, nil
	}

	f, err := os.Open(path) // #nosec G304 -- path is derived from the local Claude transcript root.
	if err != nil {
		return prior, stats, err
	}
	defer f.Close()
	if _, err := f.Seek(prior.Offset, io.SeekStart); err != nil {
		return prior, stats, err
	}

	prior.Parser.ensureStarted()
	buffer := make([]byte, workflowJournalReadBuffer)
readLoop:
	for stats.Records < budget.MaxRecords && stats.BytesRead < budget.MaxBytes {
		if workflowJournalBudgetExpired(budget) {
			stats.HitBudget = true
			break
		}
		remaining := budget.MaxBytes - stats.BytesRead
		readBuf := buffer
		if remaining < len(readBuf) {
			readBuf = readBuf[:remaining]
		}
		n, readErr := f.Read(readBuf)
		stats.BytesRead += n
		for _, b := range readBuf[:n] {
			if workflowJournalBudgetExpired(budget) {
				stats.HitBudget = true
				break readLoop
			}
			prior.Offset++
			prior.LineBytes++
			if b != '\n' {
				prior.Parser.feed(b)
				continue
			}

			event, ok := prior.Parser.event()
			if ok {
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
			prior.LineBytes = 0
			prior.Parser.reset()
			stats.Records++
			if stats.Records >= budget.MaxRecords {
				stats.HitBudget = true
				break readLoop
			}
		}
		if stats.HitBudget || stats.BytesRead >= budget.MaxBytes || workflowJournalBudgetExpired(budget) {
			stats.HitBudget = true
			break
		}
		if readErr == io.EOF {
			break
		}
		if readErr != nil {
			return prior, stats, readErr
		}
		if n == 0 {
			break
		}
	}
	return prior, stats, nil
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
		// The old run's task detail (if any) no longer applies; a new
		// wf_<runid> directory for the next run will re-trigger resolution.
		s.stopWorkflowTaskDetailLocked(ses)
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
	// Read before sessionsMu so this stays consistent with the existing
	// journalEnabled-before-lock pattern in applyWorkflowVTScan.
	taskDetailEnabled := s.workflowTaskDetailEnabled()
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses != expected || ses.workflowJournalGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	if associated {
		prevFiles := ses.workflowJournalFiles
		ses.workflowJournalFiles = files
		ses.workflowJournalSessionDir = sessionDir
		ses.workflowJournalPendingAssociation = false
		// internal C1 trigger point: a wf_<runid> directory the watcher was
		// not already tracking just got linked. Kick off (or leave alone, if
		// already resolving/resolved/unavailable) the one-shot taskId
		// resolution chain for it (docs/local/plan_workflow-progress-agent-transcript-detail_c2_hub-implementation.md 内部C1).
		if taskDetailEnabled {
			if wfDir, ok := newWorkflowJournalRunDir(files, prevFiles); ok {
				s.startWorkflowTaskDetailLocked(id, ses, now, sessionDir, wfDir)
			}
		}
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
	s.stopWorkflowTaskDetailLocked(ses)
	out, shouldBroadcast, shouldNotify := s.workflowProgressForBroadcastLocked(ses, now)
	s.sessionsMu.Unlock()
	if shouldBroadcast {
		s.broadcast(proto.Message{Type: "workflow_progress", SessionID: id, WorkflowProgress: out})
	}
	if shouldNotify {
		s.notifyWorkflowCompletionPush(id, out)
	}
}
