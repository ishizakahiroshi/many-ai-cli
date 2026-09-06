package hub

import (
	"fmt"
	"strings"
	"time"
)

const maxBoardEvents = 256

type boardEvent struct {
	ID        uint64
	SessionID int
	Text      string
	Remaining string
}

// notifyBoardEvent records an actionable orchestration event before placing it
// on the bounded in-memory delivery queue. If the queue is full, the complete
// event remains in board.md and a later queue entry points the conductor to it.
func (s *Server) notifyBoardEvent(boardID string, sessionID int, text string) {
	if boardID == "" || sessionID <= 0 || s.relayOwns(boardID) {
		return
	}
	text = sanitizeInjectText(text)

	s.orchestration.mu.Lock()
	board := s.orchestration.boards[boardID]
	boardPath := ""
	validConductor := false
	if board != nil {
		boardPath = board.Path
		validConductor = boardConductorSessionIDLocked(board) == sessionID
	}
	s.orchestration.mu.Unlock()
	if boardPath == "" {
		s.logger.Warn("orchestration event has no board evidence target", "board_id", boardID, "session_id", sessionID)
		return
	}

	// The board is the restart-safe evidence. Delivery state itself deliberately
	// remains in memory so a stale event cannot be injected after a Hub restart.
	if err := s.appendBoardSection(boardPath, "hub", "event pending: "+strings.TrimSpace(text)); err != nil {
		s.logger.Error("orchestration event board record failed", "board_id", boardID, "session_id", sessionID, "err", err)
	}
	if !validConductor {
		s.logger.Warn("orchestration event retained on board; conductor ownership invalid",
			"board_id", boardID, "session_id", sessionID, "board_path", boardPath)
		return
	}

	s.orchestration.mu.Lock()
	board = s.orchestration.boards[boardID]
	if board == nil || boardConductorSessionIDLocked(board) != sessionID {
		s.orchestration.mu.Unlock()
		s.logger.Warn("orchestration event retained on board; conductor ownership changed",
			"board_id", boardID, "session_id", sessionID, "board_path", boardPath)
		return
	}
	if len(board.PendingEvents) < maxBoardEvents {
		board.NextEventID++
		board.PendingEvents = append(board.PendingEvents, boardEvent{
			ID:        board.NextEventID,
			SessionID: sessionID,
			Text:      text,
		})
	} else {
		if board.EventOverflow == nil {
			board.EventOverflow = map[int]uint64{}
		}
		board.EventOverflow[sessionID]++
		s.logger.Warn("orchestration event queue full; full event retained on board",
			"board_id", boardID, "session_id", sessionID, "board_path", boardPath)
	}
	s.orchestration.mu.Unlock()
	s.setBoardNotifyPending(sessionID, true)
}

// orchestrationEventBlockedLocked is evaluated while sessionsMu is held at
// the actual input boundary. Events wait through every state where an injected
// Enter could answer a prompt, enter a modal, or land in the wrong thread.
func orchestrationEventBlockedLocked(ses *session, now time.Time) string {
	if ses == nil {
		return "session unavailable"
	}
	if ses.initialInjectPending || sessionInjectGated(ses, now) {
		return "initial prompt pending"
	}
	if ses.Activity.AwaitingUser || ses.Activity.AwaitingApproval || ses.approvalVisible {
		return "awaiting user"
	}
	if !ses.Activity.IsIdle() {
		return "workflow active"
	}
	if ses.Provider != "codex" {
		return ""
	}
	screen := ""
	if ses.vt != nil {
		screen = collapseWhitespace(strings.Join(ses.vt.Lines(), ""))
	}
	// Observed on 2026-09-05: Codex renders this while /btw owns the composer.
	if strings.Contains(screen, "Sidefrommainthread") {
		return "codex side thread"
	}
	if containsAnySignal(screen, providerBlockingSignals["codex"]) {
		return "codex modal"
	}
	if !containsAnySignal(screen, providerComposerSignals["codex"]) {
		return "codex composer not ready"
	}
	return ""
}

func (s *Server) flushBoardEvents(now time.Time) {
	if !s.orchestration.eventFlushMu.TryLock() {
		return
	}
	defer s.orchestration.eventFlushMu.Unlock()

	s.orchestration.mu.Lock()
	boardIDs := make([]string, 0, len(s.orchestration.boards))
	for boardID, board := range s.orchestration.boards {
		if len(board.PendingEvents) > 0 || len(board.EventOverflow) > 0 {
			boardIDs = append(boardIDs, boardID)
		}
	}
	s.orchestration.mu.Unlock()

	for _, boardID := range boardIDs {
		if s.relayOwns(boardID) {
			continue
		}

		s.orchestration.mu.Lock()
		board := s.orchestration.boards[boardID]
		if board == nil {
			s.orchestration.mu.Unlock()
			continue
		}
		// Once space opens, surface overflow as an ordered delivery reference.
		for sessionID, count := range board.EventOverflow {
			if len(board.PendingEvents) >= maxBoardEvents {
				break
			}
			board.NextEventID++
			board.PendingEvents = append(board.PendingEvents, boardEvent{
				ID:        board.NextEventID,
				SessionID: sessionID,
				Text: fmt.Sprintf("\n[orchestration] %d additional events were retained on the board; read the event pending sections: %s\n",
					count, board.Path),
			})
			delete(board.EventOverflow, sessionID)
		}
		if len(board.PendingEvents) == 0 {
			s.orchestration.mu.Unlock()
			continue
		}
		event := board.PendingEvents[0]
		s.orchestration.mu.Unlock()

		s.sessionsMu.Lock()
		ses := s.sessions[event.SessionID]
		s.sessionsMu.Unlock()
		if ses == nil {
			continue
		}

		// inputMu keeps the readiness check and the PTY write ordered with user
		// input. Events never enter pendingInput, whose reconnect flush has no
		// awareness of approvals, modals, or Codex side threads.
		ses.inputMu.Lock()
		s.sessionsMu.Lock()
		current := s.sessions[event.SessionID]
		wrapper := s.wrappers[event.SessionID]
		blocked := current != ses || ses.ParentSessionID != 0 || ses.OrchestrationID != boardID ||
			orchestrationEventBlockedLocked(ses, now) != "" || wrapper == nil ||
			len(s.pendingInput[event.SessionID]) != 0 || len(ses.resendInput) != 0
		s.sessionsMu.Unlock()
		if blocked {
			ses.inputMu.Unlock()
			s.setBoardNotifyPending(event.SessionID, true)
			continue
		}

		combined := event.Remaining
		if combined == "" {
			combined = bracketedPasteStart + strings.TrimRight(event.Text, "\r\n") + bracketedPasteEnd + "\r"
		}
		remaining := s.trySendInputToWrapper(event.SessionID, wrapper, combined)
		ses.inputMu.Unlock()

		s.orchestration.mu.Lock()
		board = s.orchestration.boards[boardID]
		if board == nil || len(board.PendingEvents) == 0 || board.PendingEvents[0].ID != event.ID {
			s.orchestration.mu.Unlock()
			continue
		}
		if remaining == "" {
			board.PendingEvents = board.PendingEvents[1:]
			// An actionable event includes the board/progress reference, making a
			// queued progress-only Enter redundant.
			delete(board.PendingNotices, event.SessionID)
		} else {
			board.PendingEvents[0].Remaining = remaining
		}
		pending := false
		for _, queued := range board.PendingEvents {
			if queued.SessionID == event.SessionID {
				pending = true
				break
			}
		}
		if board.EventOverflow[event.SessionID] > 0 || board.PendingNotices[event.SessionID] != "" {
			pending = true
		}
		s.orchestration.mu.Unlock()
		s.setBoardNotifyPending(event.SessionID, pending)
	}
}
