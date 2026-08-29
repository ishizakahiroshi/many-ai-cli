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
	// agentChatKickDelay は前倒し poll までの待ち。0 にしないのは、端末への描画と
	// トランスクリプトへの書き込みが同時ではないため（実測は同じ秒だが順序の保証は無い）。
	agentChatKickDelay = 80 * time.Millisecond
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
		GrokHome:       ses.GrokHome,
		AgentSessionID: ses.AgentSessionID,
		NativeLogPath:  ses.NativeLogPath,
	}
	s.sessionsMu.Unlock()

	if !isAgentChatProvider(snap.Provider) {
		writeJSON(w, map[string]any{
			"ok":          true,
			"available":   false,
			"total":       0,
			"total_known": true,
			"offset":      0,
			"cursor":      0,
			"next_cursor": 0,
			"has_more":    false,
			"messages":    []agentChatMessage{},
		})
		return
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
	// cursor is a newline-aligned byte offset. The initial request omits it and
	// receives a bounded tail page; subsequent pages can request older bytes by
	// sending next_cursor. offset remains accepted as a compatibility alias.
	cursor := int64(-1)
	for _, key := range []string{"cursor", "offset"} {
		if value := r.URL.Query().Get(key); value != "" {
			cursor, err = strconv.ParseInt(value, 10, 64)
			if err != nil || cursor < -1 {
				writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid cursor")
				return
			}
			break
		}
	}

	path, ok := agentChatTranscriptPathForSnapshot(snap)
	if !ok {
		// The provider may not have created its first transcript file yet. Keep
		// available=true so the browser stays on the structured path and the
		// live tail can discover the file on a later poll.
		writeJSON(w, map[string]any{
			"ok":          true,
			"available":   true,
			"total":       0,
			"total_known": true,
			"offset":      0,
			"cursor":      0,
			"next_cursor": 0,
			"has_more":    false,
			"messages":    []agentChatMessage{},
		})
		return
	}

	state := newAgentChatParseStateWithPage(limit, agentChatBatchBytesMax, -1)
	var messages []agentChatMessage
	pageOffset := cursor
	switch snap.Provider {
	case "claude":
		messages, pageOffset, err = parseClaudeTranscriptTailPage(path, state, cursor)
	case "codex":
		messages, pageOffset, err = parseCodexRolloutTailPage(path, state, cursor)
	}
	nextCursor := pageOffset
	if err != nil {
		s.logger.Warn("agent chat transcript read failed", "session_id", id, "provider", snap.Provider, "err", err)
		writeJSONError(w, http.StatusNotFound, "not_found", "agent transcript not readable")
		return
	}
	if messages == nil {
		messages = []agentChatMessage{}
	}

	hasMore := nextCursor > 0
	writeJSON(w, map[string]any{
		"ok":          true,
		"available":   true,
		"total":       -1,
		"total_known": false,
		"offset":      pageOffset,
		"cursor":      pageOffset,
		"next_cursor": nextCursor,
		"has_more":    hasMore,
		"messages":    messages,
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

// kickAgentChatPollLocked は次の poll を前倒しする。承認マーカーの供給元を
// トランスクリプトへ移した以上、承認パネルの出る速さが 1 秒周期のポーリングに
// 縛られる。終了マーカーが端末へ届いた瞬間（＝AI が回答を書き終えた瞬間）に
// 1 回だけ前倒しして、VT 経路だった頃の体感を保つ。
//
// Stop() が true のときしか組み直さない。すでに発火済みのタイマーを Reset すると
// 同じ generation の poll が二重に走り、両者が同じ agentChatParseState（可変 map）を
// 触ってしまう。取り逃しても次の通常 poll が必ず拾うので、前倒しは best-effort でよい。
func (s *Server) kickAgentChatPollLocked(id int, ses *session) {
	if ses == nil || !ses.agentChatRunning || ses.agentChatTimer == nil {
		return
	}
	if !ses.agentChatTimer.Stop() {
		return
	}
	generation := ses.agentChatGeneration
	ses.agentChatTimer = time.AfterFunc(agentChatKickDelay, func() { s.pollAgentChat(id, generation) })
}

func (s *Server) scheduleAgentChatPollLocked(id int, generation uint64) {
	ses := s.sessions[id]
	if ses == nil || !ses.agentChatRunning || ses.agentChatGeneration != generation {
		return
	}
	ses.agentChatTimer = time.AfterFunc(agentChatPollInterval, func() { s.pollAgentChat(id, generation) })
}

func (s *Server) broadcastAgentChatIfCurrent(id int, generation uint64, message proto.Message) bool {
	// The generation check and UI snapshot are the only critical section. Do not
	// hold sessionsMu or a broadcast mutex while a UI write waits on a socket;
	// a stalled browser must not block session replacement or unrelated Hub work.
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || !ses.agentChatRunning || ses.agentChatGeneration != generation {
		s.sessionsMu.Unlock()
		return false
	}
	ucs := make([]*uiConn, 0, len(s.uis))
	for _, uc := range s.uis {
		if uc.priming || uc.draining {
			uc.queued = append(uc.queued, message)
			continue
		}
		ucs = append(ucs, uc)
	}
	s.sessionsMu.Unlock()

	var dead []*uiConn
	for _, uc := range ucs {
		if !s.agentChatGenerationCurrent(id, generation) {
			return false
		}
		if err := uc.sendWithDeadline(message, time.Now().Add(broadcastWriteTimeout)); err != nil {
			s.logger.Warn("agent chat broadcast: UI send failed", "err", err)
			dead = append(dead, uc)
		}
	}
	for _, uc := range dead {
		s.removeUI(uc.ws)
	}
	return true
}

func (s *Server) agentChatGenerationCurrent(id int, generation uint64) bool {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[id]
	return ses != nil && ses.agentChatRunning && ses.agentChatGeneration == generation
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
	parseState := ses.agentChatParseState
	ses.agentChatTimer = nil
	s.sessionsMu.Unlock()

	snap := s.agentChatSnapshot(id)
	path, pathOK := agentChatTranscriptPathForSnapshot(snap)
	if pathOK && path != previousPath {
		previousOffset = 0
		parseState = nil
	}
	stateWasReset := parseState == nil
	if parseState == nil {
		parseState = newAgentChatParseStateWithPage(agentChatLiveMessageMax, agentChatBatchBytesMax, -1)
	}
	var messages []agentChatMessage
	var codexCompletions []codexTaskCompletion
	newOffset := previousOffset
	var err error
	if pathOK {
		if stateWasReset {
			// Reattach/path replacement gives the new poll a fresh owner. Prime it
			// from a bounded tail page so pending tool IDs are rebuilt without
			// sharing the old poll's mutable maps or reading from byte zero.
			budget := s.agentChatTailPageBudget()
			switch provider {
			case "claude":
				messages, _, err = parseClaudeTranscriptTailPageWithBudget(path, parseState, -1, budget)
			case "codex":
				messages, _, err = parseCodexRolloutTailPageWithBudget(path, parseState, -1, budget)
			}
			if parseState.lastRead.DecodeCommitted {
				// The tail parser owns the snapshot boundary. Do not replace it
				// with a later os.Stat: that could skip an incomplete tail or a
				// record appended while the snapshot was being parsed.
				newOffset = parseState.lastRead.SafeOffset
			} else {
				// A page that did not reach the decode commit boundary must not
				// become a normal forward parser state. Retry the same tail page
				// on the next poll so existing records cannot be skipped.
				messages = nil
				newOffset = previousOffset
				parseState = nil
			}
		} else {
			switch provider {
			case "claude":
				messages, newOffset, err = parseClaudeTranscriptWithState(path, previousOffset, parseState)
			case "codex":
				messages, newOffset, err = parseCodexRolloutWithState(path, previousOffset, parseState)
			}
		}
	}
	if provider == "codex" && pathOK && parseState != nil {
		// A tail prime reconstructs the browser view from already-written rollout
		// records. It is a baseline, not a new completion event. Forward polls
		// alone may publish task_complete records.
		codexCompletions = parseState.takeCodexCompletions()
		if stateWasReset {
			codexCompletions = nil
		}
	}
	if err != nil {
		s.logger.Debug("agent chat tail failed", "session_id", id, "provider", provider, "err", err)
	}
	// 承認マーカーはチャット表示より前に出す。下の配信ループは UI 側の都合で
	// 途中 return しうるので、そこへ承認を巻き込ませない。
	// path は session ではなくこの poll が持っている値を渡す（session 側の
	// agentChatPath はこの関数の末尾で書くので、ここで読むと 1 周ぶん古い）。
	transcriptPath := ""
	if pathOK {
		transcriptPath = path
	}
	s.scanTranscriptApprovalMarkers(id, provider, transcriptPath, messages, stateWasReset, time.Now())

	for _, message := range messages {
		if !s.broadcastAgentChatIfCurrent(id, generation, proto.Message{
			Type:              "agent_chat",
			SessionID:         id,
			Provider:          provider,
			AgentChatMessages: []proto.AgentChatMessage{toProtoAgentChatMessage(message)},
		}) {
			return
		}
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
		cur.agentChatParseState = parseState
	}
	// トランスクリプトが読めなくなったら承認マーカーの供給元を VT へ戻す
	// （approval_marker_transcript.go の「読めなくなったとき」節が理由の正本）。
	transcriptFellBack := false
	transcriptRecovered := false
	knownTranscriptPath := cur.agentChatPath
	if pathOK && err == nil {
		transcriptRecovered = cur.agentChatPath != "" &&
			cur.agentChatMissStreak >= approvalMarkerTranscriptMissLimit
		cur.agentChatMissStreak = 0
	} else if cur.agentChatMissStreak < approvalMarkerTranscriptMissLimit {
		cur.agentChatMissStreak++
		transcriptFellBack = cur.agentChatPath != "" &&
			cur.agentChatMissStreak == approvalMarkerTranscriptMissLimit
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
	} else {
		s.scheduleAgentChatPollLocked(id, generation)
	}
	s.sessionsMu.Unlock()
	// 供給元が入れ替わったことは必ず記録する。無音で沈黙するのが本 bugfix で
	// 消そうとした失敗そのものなので、退避したことまで無音にしない。
	if transcriptFellBack {
		s.logger.Warn("approval marker source fell back to VT mirror",
			"session_id", id, "provider", provider,
			"misses", approvalMarkerTranscriptMissLimit, "path", knownTranscriptPath, "err", err)
	}
	if transcriptRecovered {
		s.logger.Info("approval marker source back on transcript",
			"session_id", id, "provider", provider)
	}
	for _, completion := range codexCompletions {
		s.handleCodexTaskCompletion(id, completion)
	}
}

func (s *Server) agentChatTailPageBudget() agentChatReadBudget {
	now := time.Now()
	var clock func() time.Time
	if s != nil {
		clock = s.agentChatReadClock
	}
	if clock != nil {
		now = clock()
	}
	return agentChatReadBudget{
		MaxBytes:   agentChatPageBytesMax,
		MaxRecords: agentChatPageRecordsMax,
		Deadline:   now.Add(agentChatReadTimeBudget),
		Clock:      clock,
	}
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
			GrokHome:       ses.GrokHome,
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
		MessageID: message.MessageID,
	}
}
