package hub

import (
	"time"

	"many-ai-cli/internal/proto"
)

// reattachPreservedState contains logical session state that belongs to the
// conversation rather than to one wrapper WebSocket. It deliberately excludes
// timers, goroutine ownership, websocket pointers, and parser pointers; those
// are stopped/reset on the old session and restarted for the replacement.
type reattachPreservedState struct {
	ParentSessionID    int
	Role               string
	Auto               bool
	Depth              int
	OrchestrationID    string
	BoardPath          string
	WorktreeBranch     string
	NormalWorktree     normalWorktree
	WorktreeCleanup    string
	BoardNotifyPending bool

	Activity         SessionActivity
	LastOutputAt     string
	lastOutputAt     time.Time
	TranscriptGrewAt string
	StartedAt        string
	FirstMessage     string
	LastMessage      string
	EndReason        string

	transcriptPath        string
	transcriptResolvedAt  time.Time
	transcriptStatAt      time.Time
	transcriptSize        int64
	initialInjectPending  bool
	initialInjectGateAt   time.Time
	gitChecked            bool
	gitFiles              int
	gitAdded              int
	gitDeleted            int
	lastDoneNotifyAt      time.Time
	doneSummaryMarkerSeen bool

	vtCounts                  workflowCounts
	journalCounts             workflowCounts
	workflowVTProgress        *proto.WorkflowProgress
	workflowVTSignature       string
	workflowLastScanAt        time.Time
	workflowScanDue           time.Time
	workflowMissingScans      int
	workflowFrozenScans       int
	workflowElapsedBase       int
	workflowElapsedObservedAt time.Time
	workflowVTHasSignal       bool

	workflowJournalFiles              map[string]workflowJournalFileState
	workflowJournalSessionDir         string
	workflowJournalDetectedAt         time.Time
	workflowJournalLastEventAt        time.Time
	workflowJournalLastMTime          time.Time
	workflowJournalDue                time.Time
	workflowJournalRunning            bool
	workflowJournalPendingAssociation bool
	workflowJournalDormant            bool
	workflowJournalDormantVTSignature string
	workflowJournalSettledVTSignature string
	workflowCompletionNotified        bool
	workflowCompletionSignature       string

	taskDetailWfDir           string
	taskDetailSessionUUID     string
	taskDetailTaskID          string
	taskDetailResolveAttempts int
	taskDetailUnavailable     bool
	taskDetailFileState       workflowTaskDetailFileState
	taskDetailProgress        *proto.WorkflowProgress

	gitTurnStartTree string
	gitTurnStartedAt time.Time
	gitTurns         []gitTurnSnapshot

	commitMsgAwait        bool
	commitMsgDeadline     time.Time
	commitMsgLang         string
	commitMsgBuf          string
	commitMsgProgressed   bool
	doneMsgBuf            string
	initialModelScanBytes int
	initialModelScanDone  bool
}

func snapshotReattachStateLocked(ses *session) reattachPreservedState {
	if ses == nil {
		return reattachPreservedState{}
	}
	fileState := ses.taskDetailFileState
	fileState.Entries = append([]workflowTaskOutputEntry(nil), ses.taskDetailFileState.Entries...)
	return reattachPreservedState{
		ParentSessionID:    ses.ParentSessionID,
		Role:               ses.Role,
		Auto:               ses.Auto,
		Depth:              ses.Depth,
		OrchestrationID:    ses.OrchestrationID,
		BoardPath:          ses.BoardPath,
		WorktreeBranch:     ses.WorktreeBranch,
		NormalWorktree:     ses.NormalWorktree,
		WorktreeCleanup:    ses.WorktreeCleanup,
		BoardNotifyPending: ses.BoardNotifyPending,
		Activity:           ses.Activity,
		LastOutputAt:       ses.LastOutputAt,
		lastOutputAt:       ses.lastOutputAt,
		TranscriptGrewAt:   ses.TranscriptGrewAt,
		StartedAt:          ses.StartedAt,
		FirstMessage:       ses.FirstMessage,
		LastMessage:        ses.LastMessage,
		EndReason:          ses.EndReason,

		transcriptPath:        ses.transcriptPath,
		transcriptResolvedAt:  ses.transcriptResolvedAt,
		transcriptStatAt:      ses.transcriptStatAt,
		transcriptSize:        ses.transcriptSize,
		initialInjectPending:  ses.initialInjectPending,
		initialInjectGateAt:   ses.initialInjectGateAt,
		gitChecked:            ses.gitChecked,
		gitFiles:              ses.gitFiles,
		gitAdded:              ses.gitAdded,
		gitDeleted:            ses.gitDeleted,
		lastDoneNotifyAt:      ses.lastDoneNotifyAt,
		doneSummaryMarkerSeen: ses.doneSummaryMarkerSeen,

		vtCounts:                  ses.vtCounts,
		journalCounts:             ses.journalCounts,
		workflowVTProgress:        cloneWorkflowProgress(ses.workflowVTProgress),
		workflowVTSignature:       ses.workflowVTSignature,
		workflowLastScanAt:        ses.workflowLastScanAt,
		workflowScanDue:           ses.workflowScanDue,
		workflowMissingScans:      ses.workflowMissingScans,
		workflowFrozenScans:       ses.workflowFrozenScans,
		workflowElapsedBase:       ses.workflowElapsedBase,
		workflowElapsedObservedAt: ses.workflowElapsedObservedAt,
		workflowVTHasSignal:       ses.workflowVTHasSignal,

		workflowJournalFiles:              cloneWorkflowJournalFiles(ses.workflowJournalFiles),
		workflowJournalSessionDir:         ses.workflowJournalSessionDir,
		workflowJournalDetectedAt:         ses.workflowJournalDetectedAt,
		workflowJournalLastEventAt:        ses.workflowJournalLastEventAt,
		workflowJournalLastMTime:          ses.workflowJournalLastMTime,
		workflowJournalDue:                ses.workflowJournalDue,
		workflowJournalRunning:            ses.workflowJournalRunning,
		workflowJournalPendingAssociation: ses.workflowJournalPendingAssociation,
		workflowJournalDormant:            ses.workflowJournalDormant,
		workflowJournalDormantVTSignature: ses.workflowJournalDormantVTSignature,
		workflowJournalSettledVTSignature: ses.workflowJournalSettledVTSignature,
		workflowCompletionNotified:        ses.workflowCompletionNotified,
		workflowCompletionSignature:       ses.workflowCompletionSignature,

		taskDetailWfDir:           ses.taskDetailWfDir,
		taskDetailSessionUUID:     ses.taskDetailSessionUUID,
		taskDetailTaskID:          ses.taskDetailTaskID,
		taskDetailResolveAttempts: ses.taskDetailResolveAttempts,
		taskDetailUnavailable:     ses.taskDetailUnavailable,
		taskDetailFileState:       fileState,
		taskDetailProgress:        cloneWorkflowProgress(ses.taskDetailProgress),

		gitTurnStartTree: ses.gitTurnStartTree,
		gitTurnStartedAt: ses.gitTurnStartedAt,
		gitTurns:         append([]gitTurnSnapshot(nil), ses.gitTurns...),

		commitMsgAwait:        ses.commitMsgAwait,
		commitMsgDeadline:     ses.commitMsgDeadline,
		commitMsgLang:         ses.commitMsgLang,
		commitMsgBuf:          ses.commitMsgBuf.String(),
		commitMsgProgressed:   ses.commitMsgProgressed,
		doneMsgBuf:            ses.doneMsgBuf.String(),
		initialModelScanBytes: ses.initialModelScanBytes,
		initialModelScanDone:  ses.initialModelScanDone,
	}
}

// stopReattachAsyncStateLocked invalidates work owned by the old session. Any
// worker that is already outside sessionsMu will fail its expected-pointer or
// generation check instead of committing into the replacement session.
func stopReattachAsyncStateLocked(ses *session) {
	if ses == nil {
		return
	}
	if ses.workflowScanTimer != nil {
		ses.workflowScanTimer.Stop()
		ses.workflowScanTimer = nil
	}
	ses.workflowScanGeneration++
	if ses.workflowJournalTimer != nil {
		ses.workflowJournalTimer.Stop()
		ses.workflowJournalTimer = nil
	}
	ses.workflowJournalGeneration++
	if ses.taskDetailTimer != nil {
		ses.taskDetailTimer.Stop()
		ses.taskDetailTimer = nil
	}
	ses.taskDetailGeneration++
}

func applyReattachPreservedStateLocked(dst *session, state reattachPreservedState) {
	if dst == nil {
		return
	}
	dst.ParentSessionID = state.ParentSessionID
	dst.Role = state.Role
	dst.Auto = state.Auto
	dst.Depth = state.Depth
	dst.OrchestrationID = state.OrchestrationID
	dst.BoardPath = state.BoardPath
	dst.WorktreeBranch = state.WorktreeBranch
	dst.NormalWorktree = state.NormalWorktree
	dst.WorktreeCleanup = state.WorktreeCleanup
	dst.BoardNotifyPending = state.BoardNotifyPending

	// Connection state is reset to running by the reattach constructor; the
	// orthogonal activity and user-visible history continue across the socket.
	dst.Activity = state.Activity
	dst.LastOutputAt = state.LastOutputAt
	dst.lastOutputAt = state.lastOutputAt
	dst.TranscriptGrewAt = state.TranscriptGrewAt
	if state.StartedAt != "" {
		dst.StartedAt = state.StartedAt
	}
	dst.FirstMessage = state.FirstMessage
	dst.LastMessage = state.LastMessage
	dst.EndReason = state.EndReason

	dst.transcriptPath = state.transcriptPath
	dst.transcriptResolvedAt = state.transcriptResolvedAt
	dst.transcriptStatAt = state.transcriptStatAt
	dst.transcriptSize = state.transcriptSize
	dst.initialInjectPending = state.initialInjectPending
	dst.initialInjectGateAt = state.initialInjectGateAt
	dst.gitChecked = state.gitChecked
	dst.gitFiles = state.gitFiles
	dst.gitAdded = state.gitAdded
	dst.gitDeleted = state.gitDeleted
	dst.lastDoneNotifyAt = state.lastDoneNotifyAt
	dst.doneSummaryMarkerSeen = state.doneSummaryMarkerSeen

	dst.vtCounts = state.vtCounts
	dst.journalCounts = state.journalCounts
	dst.workflowVTProgress = cloneWorkflowProgress(state.workflowVTProgress)
	dst.workflowVTSignature = state.workflowVTSignature
	// The logical workflow frame is preserved, but wire delivery starts a new
	// restoration epoch so connected UIs receive it again after reattach.
	dst.workflowBroadcastSignature = ""
	dst.workflowLastBroadcastAt = time.Time{}
	dst.workflowLastScanAt = state.workflowLastScanAt
	dst.workflowScanDue = state.workflowScanDue
	dst.workflowMissingScans = state.workflowMissingScans
	dst.workflowFrozenScans = state.workflowFrozenScans
	dst.workflowElapsedBase = state.workflowElapsedBase
	dst.workflowElapsedObservedAt = state.workflowElapsedObservedAt
	dst.workflowVTHasSignal = state.workflowVTHasSignal

	dst.workflowJournalFiles = cloneWorkflowJournalFiles(state.workflowJournalFiles)
	dst.workflowJournalSessionDir = state.workflowJournalSessionDir
	dst.workflowJournalDetectedAt = state.workflowJournalDetectedAt
	dst.workflowJournalLastEventAt = state.workflowJournalLastEventAt
	dst.workflowJournalLastMTime = state.workflowJournalLastMTime
	dst.workflowJournalDue = state.workflowJournalDue
	dst.workflowJournalRunning = state.workflowJournalRunning
	dst.workflowJournalPendingAssociation = state.workflowJournalPendingAssociation
	dst.workflowJournalDormant = state.workflowJournalDormant
	dst.workflowJournalDormantVTSignature = state.workflowJournalDormantVTSignature
	dst.workflowJournalSettledVTSignature = state.workflowJournalSettledVTSignature
	dst.workflowCompletionNotified = state.workflowCompletionNotified
	dst.workflowCompletionSignature = state.workflowCompletionSignature

	dst.taskDetailWfDir = state.taskDetailWfDir
	dst.taskDetailSessionUUID = state.taskDetailSessionUUID
	dst.taskDetailTaskID = state.taskDetailTaskID
	dst.taskDetailResolveAttempts = state.taskDetailResolveAttempts
	dst.taskDetailUnavailable = state.taskDetailUnavailable
	dst.taskDetailFileState = state.taskDetailFileState
	dst.taskDetailFileState.Entries = append([]workflowTaskOutputEntry(nil), state.taskDetailFileState.Entries...)
	dst.taskDetailProgress = cloneWorkflowProgress(state.taskDetailProgress)

	dst.gitTurnStartTree = state.gitTurnStartTree
	dst.gitTurnStartedAt = state.gitTurnStartedAt
	dst.gitTurns = append([]gitTurnSnapshot(nil), state.gitTurns...)
	// An in-flight git worker belongs to the old session pointer. Reset its
	// completion channel and let the worker observe the pointer mismatch.
	dst.gitTurnCaptureInFlight = false
	dst.gitTurnCaptureDone = nil
	dst.gitTurnCaptureWaiters = 0

	dst.commitMsgAwait = state.commitMsgAwait
	dst.commitMsgDeadline = state.commitMsgDeadline
	dst.commitMsgLang = state.commitMsgLang
	dst.commitMsgBuf.Reset()
	dst.commitMsgBuf.WriteString(state.commitMsgBuf)
	dst.commitMsgProgressed = state.commitMsgProgressed
	dst.doneMsgBuf.Reset()
	dst.doneMsgBuf.WriteString(state.doneMsgBuf)
	dst.initialModelScanBytes = state.initialModelScanBytes
	dst.initialModelScanDone = state.initialModelScanDone
}

// restartReattachAsyncStateLocked restarts only the timer chains whose logical
// state says they were active. Parser pointers and old generation numbers are
// not reused.
func (s *Server) restartReattachAsyncStateLocked(id int, ses *session, now time.Time) {
	if ses == nil || ses.Provider != "claude" {
		return
	}
	if ses.workflowVTProgress != nil && !ses.workflowVTProgress.Settled {
		s.queueWorkflowVTScanLocked(id, ses, now)
	}
	if ses.workflowJournalRunning {
		due := ses.workflowJournalDue
		if due.IsZero() || due.Before(now) {
			due = now
		}
		s.scheduleWorkflowJournalLocked(id, ses, due)
	}
	if ses.taskDetailUnavailable || ses.journalCounts.Settled {
		return
	}
	if ses.taskDetailTaskID != "" {
		ses.taskDetailGeneration++
		generation := ses.taskDetailGeneration
		expected := ses
		ses.taskDetailTimer = time.AfterFunc(0, func() { s.runWorkflowTaskDetailPoll(id, expected, generation) })
		return
	}
	if ses.taskDetailWfDir != "" && ses.workflowJournalSessionDir != "" {
		ses.taskDetailGeneration++
		generation := ses.taskDetailGeneration
		expected := ses
		ses.taskDetailTimer = time.AfterFunc(0, func() { s.runWorkflowTaskDetailResolve(id, expected, generation) })
	}
}
