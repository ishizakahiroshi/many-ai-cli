package hub

import (
	"net/http"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"many-ai-cli/internal/proto"
)

const (
	agentChatDefaultLimit = 50
	agentChatMaxLimit     = 200
	agentChatPollInterval = time.Second
	agentChatIdleStop     = time.Minute
)

func isAgentChatProvider(provider string) bool {
	return provider == "claude" || provider == "codex"
}

// handleAgentChat reads provider-owned structured transcripts without writing
// their contents to the many-ai-cli session store.
func (s *Server) handleAgentChat(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	id, err := strconv.Atoi(r.URL.Query().Get("session_id"))
	if err != nil || id <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session_id")
		return
	}

	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		writeJSONError(w, http.StatusNotFound, "not_found", "session not found")
		return
	}
	snap := agentLogSession{
		Provider:       ses.Provider,
		CWD:            ses.CWD,
		StartedAt:      ses.StartedAt,
		HomeDir:        ses.HomeDir,
		CodexHome:      ses.CodexHome,
		ClaudeDir:      ses.ClaudeDir,
		AgentSessionID: ses.AgentSessionID,
		NativeLogPath:  ses.NativeLogPath,
	}
	s.sessionsMu.Unlock()

	if !isAgentChatProvider(snap.Provider) {
		writeJSON(w, map[string]any{
			"ok":        true,
			"available": false,
			"total":     0,
			"offset":    0,
			"messages":  []agentChatMessage{},
		})
		return
	}

	path, ok := agentChatTranscriptPathForSnapshot(snap)
	if !ok {
		// The provider may not have created its first transcript file yet. Keep
		// available=true so the browser stays on the structured path and the
		// live tail can discover the file on a later poll.
		writeJSON(w, map[string]any{
			"ok":        true,
			"available": true,
			"total":     0,
			"offset":    0,
			"messages":  []agentChatMessage{},
		})
		return
	}

	var messages []agentChatMessage
	switch snap.Provider {
	case "claude":
		messages, _, err = parseClaudeTranscript(path, 0)
	case "codex":
		messages, _, err = parseCodexRollout(path, 0)
	}
	if err != nil {
		s.logger.Warn("agent chat transcript read failed", "session_id", id, "provider", snap.Provider, "err", err)
		writeJSONError(w, http.StatusNotFound, "not_found", "agent transcript not readable")
		return
	}
	if messages == nil {
		messages = []agentChatMessage{}
	}

	limit := agentChatDefaultLimit
	if value := r.URL.Query().Get("limit"); value != "" {
		limit, err = strconv.Atoi(value)
		if err != nil || limit <= 0 {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid limit")
			return
		}
		limit = min(limit, agentChatMaxLimit)
	}
	offset := -1
	if value := r.URL.Query().Get("offset"); value != "" {
		offset, err = strconv.Atoi(value)
		if err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid offset")
			return
		}
	}
	total := len(messages)
	if offset < 0 {
		offset = max(total-limit, 0)
	}
	if offset > total {
		offset = total
	}
	end := min(offset+limit, total)
	writeJSON(w, map[string]any{
		"ok":        true,
		"available": true,
		"total":     total,
		"offset":    offset,
		"messages":  messages[offset:end],
	})
}

func agentChatTranscriptPathForSnapshot(snap agentLogSession) (string, bool) {
	switch snap.Provider {
	case "claude":
		root := strings.TrimSpace(snap.ClaudeDir)
		if root == "" {
			if strings.TrimSpace(snap.HomeDir) == "" {
				return "", false
			}
			root = filepath.Join(snap.HomeDir, ".claude")
		}
		if snap.AgentSessionID != "" {
			return claudeTranscriptPath(root, snap.CWD, snap.AgentSessionID)
		}
		startedAt, err := time.Parse(time.RFC3339, snap.StartedAt)
		if err != nil || root == "" || snap.CWD == "" {
			return "", false
		}
		return findClaudeTranscript(root, snap.CWD, startedAt)
	case "codex":
		root := strings.TrimSpace(snap.CodexHome)
		if root == "" {
			if strings.TrimSpace(snap.HomeDir) == "" {
				return "", false
			}
			root = filepath.Join(snap.HomeDir, ".codex")
		}
		if snap.CWD == "" || root == "" {
			return "", false
		}
		if startedAt, err := time.Parse(time.RFC3339, snap.StartedAt); err == nil {
			if path, ok := findCodexRolloutLog(root, snap.CWD, startedAt); ok {
				return path, true
			}
		}
		if snap.NativeLogPath != "" && isExistingFile(snap.NativeLogPath) {
			return snap.NativeLogPath, true
		}
	}
	return "", false
}

func (s *Server) startAgentChatTail(id int) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || !isAgentChatProvider(ses.Provider) || ses.agentChatRunning {
		s.sessionsMu.Unlock()
		return
	}
	ses.agentChatRunning = true
	ses.agentChatGeneration++
	generation := ses.agentChatGeneration
	ses.agentChatTimer = time.AfterFunc(0, func() { s.pollAgentChat(id, generation) })
	s.sessionsMu.Unlock()
}

func (s *Server) stopAgentChatTailLocked(ses *session) {
	if ses == nil {
		return
	}
	if ses.agentChatTimer != nil {
		ses.agentChatTimer.Stop()
		ses.agentChatTimer = nil
	}
	ses.agentChatRunning = false
	ses.agentChatGeneration++
}

func (s *Server) stopAgentChatTail(id int) {
	s.sessionsMu.Lock()
	s.stopAgentChatTailLocked(s.sessions[id])
	s.sessionsMu.Unlock()
}

func (s *Server) scheduleAgentChatPollLocked(id int, generation uint64) {
	ses := s.sessions[id]
	if ses == nil || !ses.agentChatRunning || ses.agentChatGeneration != generation {
		return
	}
	ses.agentChatTimer = time.AfterFunc(agentChatPollInterval, func() { s.pollAgentChat(id, generation) })
}

func (s *Server) pollAgentChat(id int, generation uint64) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || !ses.agentChatRunning || ses.agentChatGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	provider := ses.Provider
	previousPath := ses.agentChatPath
	previousOffset := ses.agentChatOffset
	ses.agentChatTimer = nil
	s.sessionsMu.Unlock()

	snap := s.agentChatSnapshot(id)
	path, pathOK := agentChatTranscriptPathForSnapshot(snap)
	if pathOK && path != previousPath {
		previousOffset = 0
	}
	var messages []agentChatMessage
	newOffset := previousOffset
	var err error
	if pathOK {
		switch provider {
		case "claude":
			messages, newOffset, err = parseClaudeTranscript(path, previousOffset)
		case "codex":
			messages, newOffset, err = parseCodexRollout(path, previousOffset)
		}
	}
	if err != nil {
		s.logger.Debug("agent chat tail failed", "session_id", id, "provider", provider, "err", err)
	}

	for _, message := range messages {
		s.broadcast(proto.Message{
			Type:              "agent_chat",
			SessionID:         id,
			Provider:          provider,
			AgentChatMessages: []proto.AgentChatMessage{toProtoAgentChatMessage(message)},
		})
	}

	now := time.Now()
	s.sessionsMu.Lock()
	cur := s.sessions[id]
	if cur == nil || !cur.agentChatRunning || cur.agentChatGeneration != generation {
		s.sessionsMu.Unlock()
		return
	}
	if pathOK {
		cur.agentChatPath = path
		cur.agentChatOffset = newOffset
	}
	if len(messages) > 0 {
		cur.agentChatLastAt = now
	}
	idleBase := cur.lastOutputAt
	if idleBase.IsZero() {
		idleBase, _ = time.Parse(time.RFC3339, cur.StartedAt)
	}
	shouldStop := isTerminalSessionState(cur.State) || (!idleBase.IsZero() && now.Sub(idleBase) >= agentChatIdleStop)
	if shouldStop {
		s.stopAgentChatTailLocked(cur)
		s.sessionsMu.Unlock()
		return
	}
	s.scheduleAgentChatPollLocked(id, generation)
	s.sessionsMu.Unlock()
}

func (s *Server) agentChatSnapshot(id int) agentLogSession {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	if ses := s.sessions[id]; ses != nil {
		return agentLogSession{
			Provider:       ses.Provider,
			CWD:            ses.CWD,
			StartedAt:      ses.StartedAt,
			HomeDir:        ses.HomeDir,
			CodexHome:      ses.CodexHome,
			ClaudeDir:      ses.ClaudeDir,
			AgentSessionID: ses.AgentSessionID,
			NativeLogPath:  ses.NativeLogPath,
		}
	}
	return agentLogSession{}
}

func toProtoAgentChatMessage(message agentChatMessage) proto.AgentChatMessage {
	tools := make([]proto.AgentChatTool, len(message.Tools))
	for i, tool := range message.Tools {
		tools[i] = proto.AgentChatTool{ID: tool.ID, Name: tool.Name, Input: tool.Input, Result: tool.Result}
	}
	return proto.AgentChatMessage{
		Role: message.Role, Kind: message.Kind, Text: message.Text,
		Thinking: append([]string(nil), message.Thinking...), Tools: tools, TS: message.TS,
	}
}
