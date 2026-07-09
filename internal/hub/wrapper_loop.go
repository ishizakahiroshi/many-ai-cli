package hub

// wrapper_loop.go: server.go から分離した「wrapper WS 接続ライフサイクル」の 3 関数群。
//
// C4 追加分割 (plan_audit_score_s_promotion_2026-07-05.md): server.go の関心事別
// 分割の第三弾で最大の塊。以下 3 関数は「wrapper が新規接続 or 再接続で開いた
// WebSocket を受けて session 登録 → メッセージポンプ実行 → 切断時 cleanup」まで
// を扱う一塊で、他の関心事から明確に分離できる。挙動は移動前と完全に同一・
// 全て package-private・呼び出し元は変更なし。

import (
	"encoding/base64"
	"strings"
	"sync"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/proto"
	"many-ai-cli/internal/sessionlog"
	"many-ai-cli/internal/sessionstore"
)

func (s *Server) wrapperLoop(conn *websocket.Conn, reg proto.Message) {
	startedAt := time.Now()
	branch := gitBranch(reg.CWD)
	s.sessionsMu.Lock()
	s.nextID++
	id := s.nextID
	initCols, initRows := s.lastUICols, s.lastUIRows
	initCols, initRows, _ = usableInitPTYSize(initCols, initRows)
	s.sessionsMu.Unlock()

	rawLogPath, jsonlPath := sessionlog.Paths(s.cfg.Hub.LogDir, sessionlog.Metadata{
		SessionID: id,
		Provider:  reg.Provider,
		CWD:       reg.CWD,
		StartedAt: startedAt,
	})
	s.cfgMu.Lock()
	jsonlMaxBytes := int64(s.cfg.Log.SessionMaxSizeMB) * 1024 * 1024
	sessionLogEnabled := s.cfg.Log.SessionEnabled
	s.cfgMu.Unlock()
	s.logger.Info("session_log_gate",
		"stage", "register",
		"session_id", id,
		"provider", reg.Provider,
		"enabled", sessionLogEnabled,
		"raw_log_path", rawLogPath,
		"jsonl_path", jsonlPath,
		"jsonl_max_bytes", jsonlMaxBytes)
	// セッションログが無効（既定）なら .jsonl を作らない（空ファイルも残さない）。
	var history *sessionlog.Writer
	if sessionLogEnabled {
		var histErr error
		history, histErr = sessionlog.NewJSONLWriter(jsonlPath, jsonlMaxBytes)
		if histErr != nil {
			s.logger.Warn("session history create failed", "path", jsonlPath, "err", histErr)
		}
	}

	regRoute := strings.TrimSpace(reg.Route)
	if regRoute == "" {
		regRoute = s.resolveRoute(reg.Provider, reg.Model)
	}
	childMeta := pendingChild{}
	if s.orchestration != nil && reg.Label != "" {
		s.orchestration.mu.Lock()
		if meta, ok := s.orchestration.pending[reg.Label]; ok {
			childMeta = meta
			delete(s.orchestration.pending, reg.Label)
		}
		s.orchestration.mu.Unlock()
	}
	var storeID int64
	if s.sessionStore != nil {
		var storeErr error
		storeID, storeErr = s.sessionStore.StartSession(sessionstore.SessionStart{
			LiveSessionID:   id,
			Provider:        reg.Provider,
			Display:         reg.Display,
			CWD:             reg.CWD,
			Branch:          branch,
			Label:           reg.Label,
			Model:           reg.Model,
			Route:           regRoute,
			Shell:           reg.Shell,
			State:           "standby",
			StartedAt:       startedAt.Format(time.RFC3339),
			LogPath:         rawLogPath,
			JSONLPath:       jsonlPath,
			ParentSessionID: childMeta.ParentSessionID,
			Role:            childMeta.Role,
			Auto:            childMeta.Auto,
			Depth:           childMeta.Depth,
			OrchestrationID: childMeta.OrchestrationID,
			BoardPath:       childMeta.BoardPath,
			WorktreeBranch:  childMeta.WorktreeBranch,
		})
		if storeErr != nil {
			s.logger.Warn("sqlite session start failed", "session_id", id, "err", storeErr)
		}
	}
	s.sessionsMu.Lock()
	ses := &session{
		ID:              id,
		StoreID:         storeID,
		Provider:        reg.Provider,
		Display:         reg.Display,
		CWD:             reg.CWD,
		Branch:          branch,
		Label:           reg.Label,
		Model:           reg.Model,
		Route:           regRoute,
		Shell:           reg.Shell,
		ParentSessionID: childMeta.ParentSessionID,
		Role:            childMeta.Role,
		Auto:            childMeta.Auto,
		Depth:           childMeta.Depth,
		OrchestrationID: childMeta.OrchestrationID,
		BoardPath:       childMeta.BoardPath,
		WorktreeBranch:  childMeta.WorktreeBranch,
		HomeDir:         reg.HomeDir,
		CodexHome:       reg.CodexHome,
		ClaudeDir:       reg.ClaudeDir,
		State:           "standby",
		StartedAt:       startedAt.Format(time.RFC3339),
		branchCheckedAt: startedAt,
		LogPath:         rawLogPath,
		JSONLPath:       jsonlPath,
		History:         history,
	}
	ses.inputMu = new(sync.Mutex) // AUDIT-11: 生成時に必ず allocate（未設定だと Lock で nil panic）
	if childMeta.OrchestrationID != "" {
		// orchestration セッションは登録直後に初期プロンプト注入（conductor は本関数末尾、
		// 子は handleSpawnChild の injectInitialPrompt）が走る。注入完了までユーザー入力を
		// 保留し、起動途中の CLI に入力が捨てられる・注入と混線するのを防ぐ。
		ses.initialInjectPending = true
		ses.initialInjectGateAt = time.Now()
	}
	s.sessions[id] = ses
	wc := newWrapperConn(conn)
	s.wrappers[id] = wc
	s.sessionsMu.Unlock()
	if initCols == 0 || initRows == 0 {
		// UIが未接続の場合はラッパーが報告した呼び出し元端末サイズを優先する
		if cols, rows, ok := usableInitPTYSize(reg.Cols, reg.Rows); ok {
			initCols, initRows = cols, rows
		} else {
			initCols, initRows = defaultInitCols, defaultInitRows
		}
	}
	s.sessionsMu.Lock()
	ses.lastCols, ses.lastRows = initCols, initRows
	ses.vt = newVTBuffer(initCols, initRows)
	activeSessionCount := 0
	for sid, cur := range s.sessions {
		if cur == nil || s.wrappers[sid] == nil {
			continue
		}
		if cur.State == "completed" || cur.State == "error" || cur.State == "disconnected" {
			continue
		}
		activeSessionCount++
	}
	s.sessionsMu.Unlock()
	// startup_latency_probe: register→registered ack の間で走る同期 I/O を計測する
	// （新セッション起動遅延の切り分け用。挙動は変えない）。
	registerT0 := time.Now()
	var approvalDur, usageDur time.Duration
	if s.approvalRulesEnabled() {
		t := time.Now()
		s.injectApprovalRules()
		approvalDur = time.Since(t)
	}
	if s.tokenStatusbarEnabled() {
		t := time.Now()
		s.injectUsageHooks()
		usageDur = time.Since(t)
	}
	totalPreAck := time.Since(registerT0)
	s.logger.Info("startup_latency_probe",
		"session_id", id,
		"provider", reg.Provider,
		"active_sessions", activeSessionCount,
		"inject_approval_ms", approvalDur.Milliseconds(),
		"inject_usage_ms", usageDur.Milliseconds(),
		"pre_ack_total_ms", totalPreAck.Milliseconds())
	_ = wc.send(proto.Message{Type: "registered", SessionID: id, Cols: initCols, Rows: initRows, StartedAt: ses.StartedAt, LogPath: rawLogPath, JSONLPath: jsonlPath, TokenStatusbar: s.tokenStatusbarEnabled(), OrchestrationID: childMeta.OrchestrationID, BoardPath: childMeta.BoardPath})
	s.logger.Info("session registered", "id", id, "provider", reg.Provider, "cwd", reg.CWD, "pid", reg.PID)
	// C2 (plan_orchestration-spawn-ui-exposure.md): conductor セッション（ツールバーの
	// 「オーケストレーション」ボタン経由・Auto=false）にだけ役割マッピングの案内を注入する。
	// spawn-child で自動生成される子（Auto=true）は handleSpawnChild 側で
	// buildChildInitialPrompt を既に注入済みなのでここでは対象外。
	if childMeta.OrchestrationID != "" && !childMeta.Auto {
		roles := s.orchestrationRolesFor(childMeta.OrchestrationID)
		// goroutine で注入する: ここは wrapperMessageLoop 開始前のため、同期で
		// waitForInputReady を呼ぶと出力が観測できず常に maxWait までブロックし、
		// その間 PTY 出力の中継も止まる。
		// safeGo で panic を recover する（素の go だと injectInitialPrompt 中の panic が
		// Hub プロセス全体を落とし、稼働中の全セッションが同時に切断される）。
		prompt := buildConductorInitialPrompt(childMeta.OrchestrationID, roles)
		s.safeGo("inject_initial_prompt_conductor", func() { s.injectInitialPrompt(id, prompt) })
	}
	s.broadcast(proto.Message{Type: "session_update", SessionID: id, Provider: reg.Provider, Display: reg.Display, CWD: reg.CWD, Branch: branch, Label: reg.Label, Model: reg.Model, Route: regRoute, Shell: reg.Shell, State: "standby", StartedAt: ses.StartedAt, LogPath: rawLogPath, JSONLPath: jsonlPath, ParentSessionID: childMeta.ParentSessionID, Role: childMeta.Role, Auto: childMeta.Auto, Depth: childMeta.Depth, OrchestrationID: childMeta.OrchestrationID, BoardPath: childMeta.BoardPath, WorktreeBranch: childMeta.WorktreeBranch})
	s.writeHistory(id, map[string]any{
		"ts":                startedAt.Format(time.RFC3339),
		"type":              "session_start",
		"session_id":        id,
		"provider":          reg.Provider,
		"cwd":               reg.CWD,
		"branch":            branch,
		"label":             reg.Label,
		"model":             reg.Model,
		"shell":             reg.Shell,
		"pid":               reg.PID,
		"parent_session_id": childMeta.ParentSessionID,
		"role":              childMeta.Role,
		"auto":              childMeta.Auto,
		"orchestration_id": childMeta.OrchestrationID,
		"board_path":        childMeta.BoardPath,
	})
	s.wrapperMessageLoop(wc, id)
}

func (s *Server) reattachLoop(conn *websocket.Conn, req proto.Message) {
	if req.SessionID <= 0 {
		_ = websocket.JSON.Send(conn, proto.Message{Type: "reattach_reject", Reason: "invalid session_id"})
		return
	}
	startedAt := time.Now()
	startedAtText := req.StartedAt
	if startedAtText != "" {
		if parsed, err := time.Parse(time.RFC3339, startedAtText); err == nil {
			startedAt = parsed
		} else {
			startedAtText = startedAt.Format(time.RFC3339)
		}
	} else {
		startedAtText = startedAt.Format(time.RFC3339)
	}
	branch := gitBranch(req.CWD)

	replay, err := base64.StdEncoding.DecodeString(req.ReplayB64)
	if err != nil {
		_ = websocket.JSON.Send(conn, proto.Message{Type: "reattach_reject", SessionID: req.SessionID, Reason: "invalid replay_b64"})
		return
	}
	if len(replay) > maxPTYBuf {
		replay = replay[len(replay)-maxPTYBuf:]
	}

	// LogPath / JSONLPath は wrapper からの提案値を信用せず、常に Hub 側の
	// LogDir + Metadata から決定的に再構築する（リモート token 保持者による
	// 任意ファイル append プリミティブを閉じるため）。req.LogPath / req.JSONLPath
	// は無視する。06-15 監査の F2 と同様の方針（attacker-controlled パスを read/write
	// プリミティブにしない）。
	s.cfgMu.Lock()
	logDir := s.cfg.Hub.LogDir
	jsonlMaxBytesReattach := int64(s.cfg.Log.SessionMaxSizeMB) * 1024 * 1024
	sessionLogEnabled := s.cfg.Log.SessionEnabled
	s.cfgMu.Unlock()
	rawLogPath, jsonlPath := sessionlog.Paths(logDir, sessionlog.Metadata{
		SessionID: req.SessionID,
		Provider:  req.Provider,
		CWD:       req.CWD,
		StartedAt: startedAt,
	})
	s.logger.Info("session_log_gate",
		"stage", "reattach",
		"requested_session_id", req.SessionID,
		"provider", req.Provider,
		"enabled", sessionLogEnabled,
		"raw_log_path", rawLogPath,
		"jsonl_path", jsonlPath,
		"jsonl_max_bytes", jsonlMaxBytesReattach)
	// セッションログが無効（既定）なら .jsonl を作らない。
	var history *sessionlog.Writer
	if sessionLogEnabled {
		var histErr error
		history, histErr = sessionlog.NewJSONLWriterAppend(jsonlPath, jsonlMaxBytesReattach)
		if histErr != nil {
			s.logger.Warn("session history append failed", "path", jsonlPath, "err", histErr)
		}
	}

	reqRoute := strings.TrimSpace(req.Route)
	if reqRoute == "" {
		reqRoute = s.resolveRoute(req.Provider, req.Model)
	}
	// renumber 判定（既存 wrapper と衝突する場合は新 ID を割り当てる）を SQLite
	// アクセスより前に行い、StartSession を 1 回だけ呼ぶ（重複 INSERT による orphan
	// 行の発生を防ぐ）。
	s.sessionsMu.Lock()
	acceptedID := req.SessionID
	if s.wrappers[acceptedID] != nil {
		s.nextID++
		acceptedID = s.nextID
	}
	if s.nextID < acceptedID {
		s.nextID = acceptedID
	}
	s.sessionsMu.Unlock()
	if acceptedID != req.SessionID {
		// renumber が確定したら JSONLPath も新 ID で再計算する（旧 ID のパスは
		// 既に他 wrapper が使っている可能性がある）。
		rawLogPath, jsonlPath = sessionlog.Paths(logDir, sessionlog.Metadata{
			SessionID: acceptedID,
			Provider:  req.Provider,
			CWD:       req.CWD,
			StartedAt: startedAt,
		})
		s.logger.Info("session_log_gate",
			"stage", "reattach_renumbered",
			"requested_session_id", req.SessionID,
			"session_id", acceptedID,
			"provider", req.Provider,
			"enabled", sessionLogEnabled,
			"raw_log_path", rawLogPath,
			"jsonl_path", jsonlPath,
			"jsonl_max_bytes", jsonlMaxBytesReattach)
		if sessionLogEnabled {
			if history != nil {
				_ = history.Close()
			}
			var histErr error
			history, histErr = sessionlog.NewJSONLWriterAppend(jsonlPath, jsonlMaxBytesReattach)
			if histErr != nil {
				s.logger.Warn("session history append failed (renumbered)", "path", jsonlPath, "err", histErr)
				history = nil
			}
		}
	}
	var storeID int64
	if s.sessionStore != nil {
		var storeErr error
		storeID, storeErr = s.sessionStore.StartSession(sessionstore.SessionStart{
			LiveSessionID: acceptedID,
			Provider:      req.Provider,
			Display:       req.Display,
			CWD:           req.CWD,
			Branch:        branch,
			Label:         req.Label,
			Model:         req.Model,
			Route:         reqRoute,
			Shell:         req.Shell,
			State:         "running",
			StartedAt:     startedAtText,
			LogPath:       rawLogPath,
			JSONLPath:     jsonlPath,
		})
		if storeErr != nil {
			s.logger.Warn("sqlite session reattach failed", "session_id", acceptedID, "err", storeErr)
		}
	}
	s.sessionsMu.Lock()
	var oldHistory *sessionlog.Writer
	if cur := s.sessions[acceptedID]; cur != nil {
		oldHistory = cur.History
	}
	now := time.Now()
	lastOutputAt := ""
	var lastOutputAtTime time.Time
	if len(replay) > 0 {
		lastOutputAtTime = now
		lastOutputAt = now.Format(time.RFC3339)
	}
	s.sessions[acceptedID] = &session{
		ID:              acceptedID,
		StoreID:         storeID,
		Provider:        req.Provider,
		Display:         req.Display,
		CWD:             req.CWD,
		Branch:          branch,
		Label:           req.Label,
		Model:           req.Model,
		Route:           reqRoute,
		Shell:           req.Shell,
		HomeDir:         req.HomeDir,
		CodexHome:       req.CodexHome,
		ClaudeDir:       req.ClaudeDir,
		State:           "running",
		LastOutputAt:    lastOutputAt,
		StartedAt:       startedAtText,
		lastOutputAt:    lastOutputAtTime,
		branchCheckedAt: now,
		ptyBuf:          replay,
		vt:              newVTBuffer(req.Cols, req.Rows),
		lastCols:        req.Cols,
		lastRows:        req.Rows,
		LogPath:         rawLogPath,
		JSONLPath:       jsonlPath,
		History:         history,
	}
	s.sessions[acceptedID].inputMu = new(sync.Mutex) // AUDIT-11: 生成時に必ず allocate（未設定だと Lock で nil panic）
	if s.sessions[acceptedID].vt != nil && len(replay) > 0 {
		s.sessions[acceptedID].vt.Write(replay)
	}
	wc := newWrapperConn(conn)
	s.wrappers[acceptedID] = wc
	if s.nextID < acceptedID {
		s.nextID = acceptedID
	}
	s.sessionsMu.Unlock()
	// wrapper が一時切断中に届かなかった保留入力を、再接続したこの wrapper へ順番に再送する。
	// 他のバックグラウンド goroutine と同様 safeGo で起動し、panic で Hub 全体を巻き込まないようにする。
	s.safeGo("flush_pending_input", func() { s.flushPendingInput(acceptedID) })
	if oldHistory != nil {
		_ = oldHistory.Close()
	}
	if s.approvalRulesEnabled() {
		s.injectApprovalRules()
	}
	if s.tokenStatusbarEnabled() {
		s.injectUsageHooks()
	}
	_ = wc.send(proto.Message{Type: "reattach_ack", SessionID: acceptedID})
	s.broadcast(proto.Message{Type: "session_update", SessionID: acceptedID, Provider: req.Provider, Display: req.Display, CWD: req.CWD, Branch: branch, Label: req.Label, Model: req.Model, Route: reqRoute, Shell: req.Shell, State: "running", LastOutputAt: lastOutputAt, StartedAt: startedAtText, LogPath: rawLogPath, JSONLPath: jsonlPath})
	s.writeHistory(acceptedID, map[string]any{
		"ts":             now.Format(time.RFC3339),
		"type":           "session_reattach",
		"session_id":     acceptedID,
		"old_session_id": req.SessionID,
		"provider":       req.Provider,
		"cwd":            req.CWD,
		"branch":         branch,
		"label":          req.Label,
		"model":          req.Model,
		"shell":          req.Shell,
		"pid":            req.PID,
		"renumbered":     acceptedID != req.SessionID,
	})
	s.wrapperMessageLoop(wc, acceptedID)
}

func (s *Server) wrapperMessageLoop(wc *wrapperConn, id int) {
	for {
		var m proto.Message
		if err := websocket.JSON.Receive(wc.ws, &m); err != nil {
			s.logger.Debug("wrapper WS closed", "session_id", id, "err", err)
			break
		}
		m.SessionID = id
		switch m.Type {
		case "pty_data":
			now := time.Now()
			maskedRaw := sessionlog.MaskSecrets(string(m.Data))
			cleanText := sessionlog.StripANSI(maskedRaw)
			s.writeHistory(id, map[string]any{
				"ts":         now.Format(time.RFC3339),
				"type":       "pty_output",
				"session_id": id,
				"data_b64":   sessionlog.EncodeBase64([]byte(maskedRaw)),
				"text":       cleanText,
			})
			var provider string
			var vtLines []string
			var marker *approvalMarkerBlock
			var initialModelLines []string
			var initialModelCWD string
			scanNativeApproval := false
			hadNativeApprovalSig := false
			chunkHasApprovalTrigger := ptyChunkContainsAny(m.Data, nativeApprovalTriggerTokens)
			s.sessionsMu.Lock()
			if ses := s.sessions[id]; ses != nil {
				ses.ptyBuf = appendPTYReplay(ses.ptyBuf, m.Data)
				if ses.vt == nil {
					ses.vt = newVTBuffer(ses.lastCols, ses.lastRows)
				}
				ses.vt.Write(m.Data)
				provider = ses.Provider
				if chunkHasApprovalTrigger && now.Before(ses.vtResizeDebounceUntil) {
					ses.nativeApprovalScanQueued = true
				}
				shouldCheckApproval := isAIProvider(provider) &&
					now.After(ses.vtResizeDebounceUntil) &&
					(chunkHasApprovalTrigger || ses.nativeApprovalSig != "" || ses.nativeApprovalScanQueued)
				if shouldCheckApproval {
					hadNativeApprovalSig = ses.nativeApprovalSig != ""
					vtLines = ses.vt.TailLines(vtTailLinesForApproval)
					tailSig := nativeApprovalTailSignature(vtLines)
					if tailSig != ses.nativeApprovalTailSig {
						ses.nativeApprovalTailSig = tailSig
						scanNativeApproval = true
					}
					ses.nativeApprovalScanQueued = false
				}
				if isAIProvider(provider) {
					marker = extractApprovalMarkerBlock(ses.vt.TailLines(vtTailLinesForApproval))
				}
				// 起動バナーからの初期モデル検出（--model 指定なしのセッション向け）。
				// Model が埋まる・上限バイト超過のどちらかで打ち切る。
				if !ses.initialModelScanDone && ses.Model == "" && initialModelScanProviders[provider] {
					ses.initialModelScanBytes += len(m.Data)
					if ses.initialModelScanBytes > initialModelScanMaxBytes {
						ses.initialModelScanDone = true
					} else {
						initialModelCWD = ses.CWD
						initialModelLines = ses.vt.Lines()
					}
				}
			}
			s.sessionsMu.Unlock()
			s.maybeBroadcastApprovalMarker(id, marker, now)
			s.broadcast(m)
			if scanNativeApproval {
				approval := detectNativeApproval(provider, vtLines)
				if approval == nil && hadNativeApprovalSig && shouldSuppressNativeApprovalClearMiss(provider, vtLines) {
					s.resetNativeApprovalClearMisses(id)
				} else {
					s.handleNativeApprovalDetection(id, approval)
				}
			}
			s.markRunning(id)
			s.detectModelChange(id, m.Data, cleanText)
			if initialModelLines != nil {
				s.detectInitialModel(id, provider, initialModelCWD, initialModelLines)
			}
			// [MANY-AI-CLI-DONE] は wrapper 側 PTY 読み取り 4096 バイトの
			// チャンク境界をまたぐことがあるため、m.Data 単体を走査すると
			// OPEN と CLOSE が別チャンクへ落ちた瞬間に取りこぼす。git_commit_ai.go
			// の commitMsgBuf と同型の累積スキャンバッファでチャンク境界を吸収する。
			var doneSnap []byte
			s.sessionsMu.Lock()
			if ses := s.sessions[id]; ses != nil {
				ses.doneMsgBuf.WriteString(cleanText)
				if ses.doneMsgBuf.Len() > doneMsgScanBufMax {
					trimmed := ses.doneMsgBuf.String()
					trimmed = trimmed[len(trimmed)-doneMsgScanBufMax:]
					ses.doneMsgBuf.Reset()
					ses.doneMsgBuf.WriteString(trimmed)
				}
				buf := ses.doneMsgBuf.String()
				if strings.Contains(buf, string(doneSummaryMarkerOpen)) && strings.Contains(buf, string(doneSummaryMarkerClose)) {
					// 二重通知防止のため走査前にバッファをリセットする。
					ses.doneMsgBuf.Reset()
					doneSnap = []byte(buf)
				}
			}
			s.sessionsMu.Unlock()
			if doneSnap != nil {
				s.handleDoneSummaryMarker(id, doneSnap)
			}
			s.handleCommitMsgChunk(id, cleanText)
		case "session_end":
			histEvent := map[string]any{
				"ts":         time.Now().Format(time.RFC3339),
				"type":       "session_end",
				"session_id": id,
				"state":      m.State,
				"exit_code":  m.ExitCode,
			}
			if m.Reason != "" {
				histEvent["reason"] = m.Reason
			}
			s.writeHistory(id, histEvent)
			if m.State == "completed" || m.State == "error" {
				s.sessionsMu.Lock()
				if cur := s.sessions[id]; cur != nil {
					cur.State = m.State
					if m.Reason != "" {
						cur.EndReason = m.Reason
					}
				}
				s.sessionsMu.Unlock()
			}
		}
	}

	// wrapper 切断
	s.sessionsMu.Lock()
	if s.wrappers[id] == wc {
		delete(s.wrappers, id)
	}
	var historyToClose *sessionlog.Writer
	var jsonlPathForTranscript string
	var endedProvider, endedCWD string
	// done/timeout も終端として保持する（オーケストレーション完了状態を disconnected で潰さない）。
	if cur := s.sessions[id]; cur != nil && !isTerminalSessionState(cur.State) {
		cur.State = "disconnected"
	}
	endState := "disconnected"
	endReason := ""
	if cur := s.sessions[id]; cur != nil {
		endState = cur.State
		historyToClose = cur.History
		cur.History = nil
		jsonlPathForTranscript = cur.JSONLPath
		endReason = cur.EndReason
		endedProvider = cur.Provider
		endedCWD = cur.CWD
	}
	s.sessionsMu.Unlock()
	if endState == "disconnected" {
		if historyToClose != nil {
			ev := map[string]any{
				"ts":         time.Now().Format(time.RFC3339),
				"type":       "session_end",
				"session_id": id,
				"state":      endState,
				"exit_code":  0,
			}
			if endReason != "" {
				ev["reason"] = endReason
			}
			_ = historyToClose.Event(ev)
		}
	}
	if historyToClose != nil {
		_ = historyToClose.Close()
	}
	if s.sessionStore != nil {
		s.sessionStore.EndSession(id, endState, endReason, time.Now())
	}
	s.removeInactiveApprovalRules(providerApprovalRuleTargets(endedProvider, endedCWD))
	s.removeInactiveUsageHooks(endedProvider, endedCWD)
	s.finalizeTranscript(id, jsonlPathForTranscript)
	// usage 集計マップから当該セッションのエントリを掃除する。dismiss 経路だけでなく
	// completed/error/disconnected の wrapper 切断時にも削除しないと、長期稼働 Hub
	// (VPS / docker) で線形にメモリが累積する。
	DeleteSessionUsageStat(id)
	s.broadcast(proto.Message{Type: "session_end", SessionID: id, State: endState, Reason: endReason})
}
