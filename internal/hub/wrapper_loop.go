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

func (s *Server) startSessionLog(logDir string, meta sessionlog.Metadata, appendMode bool) (string, string, *sessionlog.Writer) {
	rawLogPath, jsonlPath := sessionlog.Paths(logDir, meta)
	s.cfgMu.Lock()
	jsonlMaxBytes := int64(s.cfg.Log.SessionMaxSizeMB) * 1024 * 1024
	sessionLogEnabled := s.cfg.Log.SessionEnabled
	s.cfgMu.Unlock()
	stage := "register"
	if appendMode {
		stage = "reattach"
	}
	s.logger.Info("session_log_gate",
		"stage", stage,
		"session_id", meta.SessionID,
		"provider", meta.Provider,
		"enabled", sessionLogEnabled,
		"raw_log_path", rawLogPath,
		"jsonl_path", jsonlPath,
		"jsonl_max_bytes", jsonlMaxBytes)
	if !sessionLogEnabled {
		return rawLogPath, jsonlPath, nil
	}
	var history *sessionlog.Writer
	var err error
	if appendMode {
		history, err = sessionlog.NewJSONLWriterAppend(jsonlPath, jsonlMaxBytes)
	} else {
		history, err = sessionlog.NewJSONLWriter(jsonlPath, jsonlMaxBytes)
	}
	if err != nil {
		action := "create"
		if appendMode {
			action = "append"
		}
		s.logger.Warn("session history "+action+" failed", "path", jsonlPath, "err", err)
	}
	return rawLogPath, jsonlPath, history
}

func (s *Server) attachStore(start sessionstore.SessionStart, cardMeta sessionstore.SessionCardMeta) (int64, sessionstore.SessionCardMeta) {
	if s.sessionStore == nil {
		return 0, cardMeta
	}
	storeID, err := s.sessionStore.StartSession(start)
	if err != nil {
		s.logger.Warn("sqlite session start failed", "session_id", start.LiveSessionID, "err", err)
		return 0, cardMeta
	}
	if storeID == 0 {
		return 0, cardMeta
	}
	saved, err := s.sessionStore.SessionCardMetaByLiveSession(start.LiveSessionID)
	if err != nil {
		s.logger.Warn("session card meta restore failed", "session_id", start.LiveSessionID, "err", err)
		return storeID, cardMeta
	}
	return storeID, saved
}

func (s *Server) wrapperLoop(conn *websocket.Conn, reg proto.Message) {
	startedAt := time.Now()
	branch := gitBranch(reg.CWD)
	s.sessionsMu.Lock()
	s.nextID++
	id := s.nextID
	initCols, initRows := s.lastUICols, s.lastUIRows
	initCols, initRows, _ = usableInitPTYSize(initCols, initRows)
	s.sessionsMu.Unlock()

	s.cfgMu.Lock()
	logDir := s.cfg.Hub.LogDir
	s.cfgMu.Unlock()
	rawLogPath, jsonlPath, history := s.startSessionLog(logDir, sessionlog.Metadata{
		SessionID: id,
		Provider:  reg.Provider,
		CWD:       reg.CWD,
		StartedAt: startedAt,
	}, false)

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
	cardMeta := sessionstore.SessionCardMeta{Label: reg.Label}
	storeID, cardMeta := s.attachStore(sessionstore.SessionStart{
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
	}, cardMeta)
	s.sessionsMu.Lock()
	ses := &session{
		ID:              id,
		StoreID:         storeID,
		Provider:        reg.Provider,
		Display:         reg.Display,
		CWD:             reg.CWD,
		Branch:          branch,
		Label:           cardMeta.Label,
		Pinned:          cardMeta.Pinned,
		Color:           cardMeta.Color,
		Note:            cardMeta.Note,
		AutoTitle:       cardMeta.AutoTitle,
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
		NormalWorktree:  childMeta.NormalWorktree,
		WorktreeCleanup: childMeta.WorktreeCleanup,
		HomeDir:         reg.HomeDir,
		CodexHome:       reg.CodexHome,
		ClaudeDir:       reg.ClaudeDir,
		AgentSessionID:  reg.AgentSessionID,
		Activity:        SessionActivity{OutputIdle: true},
		State:           "standby",
		StartedAt:       startedAt.Format(time.RFC3339),
		branchCheckedAt: startedAt,
		LogPath:         rawLogPath,
		JSONLPath:       jsonlPath,
		History:         history,
		inflightInput:   map[int64]inflightInput{},
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
	wc.pid = reg.PID
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
	// inject 中に UI の ×（session_dismiss）が走ると sessions/wrappers から消える。
	// その後も session_update を送ると UI に幽霊カードが復活し、以降の dismiss が
	// Hub !exists で session_removed を返せない経路と結合して消えない（観察: #2/#13/#14）。
	if !s.wrapperStillRegistered(id, wc) {
		s.logger.Info("session dismissed during register; skip announce",
			"session_id", id, "provider", reg.Provider)
		return
	}
	// diag 継続: docs/local/archive/v0.5.x/bugfix_statusline-settings-skip_2026-07-10.md
	// Hub が register ack に載せる TokenStatusbar 実測値。wrapper の
	// statusline_gate_wrapper と突き合わせ「send=true → reg=false」の JSON 劣化を
	// 確定する。2026-07-19 時点 hub.log は send=true のみ（false 0）だが wrapper 側
	// 未突合のため残置（原因確定後に外す）。
	tokenStatusbarForAck := s.tokenStatusbarEnabled()
	s.logger.Info("statusline_gate_hub", "session_id", id, "provider", reg.Provider, "token_statusbar_send", tokenStatusbarForAck)
	_ = wc.send(proto.Message{Type: "registered", SessionID: id, Cols: initCols, Rows: initRows, StartedAt: ses.StartedAt, LogPath: rawLogPath, JSONLPath: jsonlPath, TokenStatusbar: tokenStatusbarForAck, OrchestrationID: childMeta.OrchestrationID, Auto: childMeta.Auto, BoardPath: childMeta.BoardPath})
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
	// announce 直前に再確認（inject 後〜ここまでの狭い窓での dismiss も拾う）。
	if !s.wrapperStillRegistered(id, wc) {
		s.logger.Info("session dismissed before announce; skip session_update",
			"session_id", id, "provider", reg.Provider)
		return
	}
	announce := sessionUpdateMessage(ses)
	announce.Shell = reg.Shell
	announce.LogPath = rawLogPath
	announce.JSONLPath = jsonlPath
	s.broadcast(announce)
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
		"orchestration_id":  childMeta.OrchestrationID,
		"board_path":        childMeta.BoardPath,
	})
	s.startAgentChatTail(id)
	s.wrapperMessageLoop(wc, id)
}

// reattachIdentityMatches は「今 reattach を要求してきた wrapper が、その ID に
// 載っている既存接続と同じプロセスか」を判定する。
//
// wrapper は PTY 出力の送信キューが溢れたとき、途中のバイト列を捨てると端末画面が
// 復元不能に壊れるため、自分から WS を切って張り直す（internal/wrapper/wrapper.go
// の ptyOutputWriter.enqueue）。この張り直しは Hub 側の切断検知より先に着くことが
// あり、その瞬間 s.wrappers[id] にはまだ古い接続が載っている。これを「別 wrapper と
// の ID 衝突」と扱って新 ID を振ると、同一プロセスに 2 つ目のセッション番号が生まれ、
// UI ではカードが 2 枚に割れる。古い側は承認待ちの表示を抱えたまま宛先を失い、
// 新しい側は Hub の受信累計 0 から始まってスクロールバックを失う
// （2026-08-05 に #15 → #17 で発生。同一 pid=6224）。
//
// PID は OS が再利用するため単独では同一性の根拠にせず、provider と cwd も一致を
// 要求する。いずれかが欠ける古い wrapper からの reattach は従来どおり renumber へ
// 倒す（誤って他人のセッションを乗っ取るより、番号が変わる方が安全側）。
func reattachIdentityMatches(prev *wrapperConn, ses *session, req proto.Message) bool {
	if prev == nil || ses == nil {
		return false
	}
	if req.PID <= 0 || prev.pid <= 0 || prev.pid != req.PID {
		return false
	}
	return ses.Provider == req.Provider && ses.CWD == req.CWD
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
	s.cfgMu.Unlock()
	rawLogPath, jsonlPath, history := s.startSessionLog(logDir, sessionlog.Metadata{
		SessionID: req.SessionID,
		Provider:  req.Provider,
		CWD:       req.CWD,
		StartedAt: startedAt,
	}, true)

	reqRoute := strings.TrimSpace(req.Route)
	if reqRoute == "" {
		reqRoute = s.resolveRoute(req.Provider, req.Model)
	}
	// renumber 判定（既存 wrapper と衝突する場合は新 ID を割り当てる）を SQLite
	// アクセスより前に行い、StartSession を 1 回だけ呼ぶ（重複 INSERT による orphan
	// 行の発生を防ぐ）。
	s.sessionsMu.Lock()
	acceptedID := req.SessionID
	var staleConn *wrapperConn
	if prev := s.wrappers[acceptedID]; prev != nil {
		if reattachIdentityMatches(prev, s.sessions[acceptedID], req) {
			// 同一 wrapper の張り直し。ID は維持し、古い接続だけ後で閉じる。
			staleConn = prev
		} else {
			s.nextID++
			acceptedID = s.nextID
		}
	}
	if s.nextID < acceptedID {
		s.nextID = acceptedID
	}
	s.sessionsMu.Unlock()
	if acceptedID != req.SessionID {
		// renumber が確定したら JSONLPath も新 ID で再計算する（旧 ID のパスは
		// 既に他 wrapper が使っている可能性がある）。
		if history != nil {
			_ = history.Close()
		}
		rawLogPath, jsonlPath, history = s.startSessionLog(logDir, sessionlog.Metadata{
			SessionID: acceptedID,
			Provider:  req.Provider,
			CWD:       req.CWD,
			StartedAt: startedAt,
		}, true)
	}
	cardMeta := sessionstore.SessionCardMeta{Label: req.Label}
	storeID, cardMeta := s.attachStore(sessionstore.SessionStart{
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
	}, cardMeta)
	s.sessionsMu.Lock()
	var oldHistory *sessionlog.Writer
	// 切断前のターミナル文脈（ptyBuf / VT ミラー / 受信累計）を引き継ぐ。
	// 以前は replay(64KB) で丸ごと置き換えていたため、再接続のたびに Hub 側の
	// スクロールバックが 64KB へ切り詰められ、ブラウザ再読込で履歴が失われていた。
	var prevPTYBuf []byte
	var prevVT *vtBuffer
	var prevCols, prevRows int
	var prevSeen int64
	var prevInputSeq int64
	var prevInflightInput map[int64]inflightInput
	var prevResendInput []pendingFrame
	var prevInputAckCapable bool
	var prevAgentChatPath string
	var prevAgentChatOffset int64
	var requeuedInputCount int
	var requeuedInputMinSeq int64
	var requeuedInputMaxSeq int64
	prevExists := false
	if cur := s.sessions[acceptedID]; cur != nil {
		prevAgentChatPath = cur.agentChatPath
		prevAgentChatOffset = cur.agentChatOffset
		s.stopAgentChatTailLocked(cur)
		oldHistory = cur.History
		prevPTYBuf = cur.ptyBuf
		prevVT = cur.vt
		prevCols, prevRows = cur.lastCols, cur.lastRows
		prevSeen = cur.ptyBytesSeen
		prevInputSeq = cur.inputSeq
		prevInflightInput = cur.inflightInput
		if staleConn != nil {
			requeuedInputCount, requeuedInputMinSeq, requeuedInputMaxSeq = s.deferInflightForResendLocked(acceptedID, staleConn)
		}
		// 再送キューと ack 対応フラグは deferInflightForResendLocked の後に読む
		// （直前の切断分がここで resendInput へ積まれるため）。
		prevResendInput = cur.resendInput
		prevInputAckCapable = cur.inputAckCapable
		prevExists = true
	}
	// gap = 切断中に Hub が受け取れなかったぶん。既存 UI へはこれだけを流す。
	gap := replay
	ptyBuf := replay
	ptyBytesSeen := int64(len(replay))
	var vt *vtBuffer
	if prevExists {
		gap = reattachReplayGap(replay, req.PTYBytes, prevSeen)
		ptyBuf = appendPTYReplay(prevPTYBuf, gap)
		ptyBytesSeen = prevSeen + int64(len(gap))
		if req.PTYBytes > 0 {
			// replay 窓を超える穴があった場合でも、以降の差分計算が
			// ずれ続けないよう累計は wrapper 側の実数に合わせ直す。
			ptyBytesSeen = req.PTYBytes
		}
		if prevVT != nil && prevCols == req.Cols && prevRows == req.Rows {
			// PTY サイズが同じなら既存ミラーを温存し、差分だけを書き足す。
			// 作り直すと scrollback（承認マーカー抽出が使う）が 64KB 相当に痩せる。
			vt = prevVT
			if len(gap) > 0 {
				vt.Write(gap)
			}
		}
	}
	if vt == nil {
		// 新規（Hub 再起動後の cold reattach）／PTY サイズ変更時は従来どおり
		// replay からミラーを作り直す。旧サイズのまま書くと折り返しがずれる。
		vt = newVTBuffer(req.Cols, req.Rows)
		if len(replay) > 0 {
			vt.Write(replay)
		}
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
		Label:           cardMeta.Label,
		Pinned:          cardMeta.Pinned,
		Color:           cardMeta.Color,
		Note:            cardMeta.Note,
		AutoTitle:       cardMeta.AutoTitle,
		Model:           req.Model,
		Route:           reqRoute,
		Shell:           req.Shell,
		HomeDir:         req.HomeDir,
		CodexHome:       req.CodexHome,
		ClaudeDir:       req.ClaudeDir,
		AgentSessionID:  req.AgentSessionID,
		Activity:        SessionActivity{OutputIdle: len(replay) == 0, WorkflowActive: len(replay) > 0},
		State:           "running",
		LastOutputAt:    lastOutputAt,
		StartedAt:       startedAtText,
		lastOutputAt:    lastOutputAtTime,
		branchCheckedAt: now,
		ptyBuf:          ptyBuf,
		ptyBytesSeen:    ptyBytesSeen,
		vt:              vt,
		lastCols:        req.Cols,
		lastRows:        req.Rows,
		LogPath:         rawLogPath,
		JSONLPath:       jsonlPath,
		History:         history,
		agentChatPath:   prevAgentChatPath,
		agentChatOffset: prevAgentChatOffset,
		inputSeq:        prevInputSeq,
		inflightInput:   prevInflightInput,
		resendInput:     prevResendInput,
		inputAckCapable: prevInputAckCapable,
	}
	s.sessions[acceptedID].inputMu = new(sync.Mutex) // AUDIT-11: 生成時に必ず allocate（未設定だと Lock で nil panic）
	wc := newWrapperConn(conn)
	wc.pid = req.PID
	s.wrappers[acceptedID] = wc
	if s.nextID < acceptedID {
		s.nextID = acceptedID
	}
	s.sessionsMu.Unlock()
	if requeuedInputCount > 0 {
		s.logger.Info("requeued in-flight pty_input",
			"session_id", acceptedID,
			"count", requeuedInputCount,
			"seq_start", requeuedInputMinSeq,
			"seq_end", requeuedInputMaxSeq,
			"cause", "reattach")
	}
	if staleConn != nil {
		// 新しい接続を wrappers へ載せた後に閉じる。逆順だと、閉じたことで走る
		// 旧 wrapperMessageLoop の後始末が「まだ自分が現役」と見えてしまう。
		staleConn.close()
	}
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
	// inject 中に dismiss されたら reattach 完了を UI に告知しない（wrapperLoop と同趣旨）。
	if !s.wrapperStillRegistered(acceptedID, wc) {
		s.logger.Info("session dismissed during reattach inject; skip announce",
			"session_id", acceptedID, "provider", req.Provider)
		return
	}
	_ = wc.send(proto.Message{Type: "reattach_ack", SessionID: acceptedID})
	s.sessionsMu.Lock()
	announce := sessionUpdateMessage(s.sessions[acceptedID])
	s.sessionsMu.Unlock()
	announce.Shell = req.Shell
	announce.LogPath = rawLogPath
	announce.JSONLPath = jsonlPath
	s.broadcast(announce)
	// 切断中に取りこぼしたぶんを、すでに開いている UI へ流す。
	// これを送らないとブラウザの xterm.js にはそのバイト列が永久に届かず、
	// 絶対座標で部分再描画する TUI（Codex 等）は画面が古いまま復帰できない。
	// wrapperMessageLoop はまだ開始していないので、ライブ配信と前後しない。
	if len(gap) > 0 {
		s.broadcast(proto.Message{Type: "pty_data", SessionID: acceptedID, Data: append([]byte(nil), gap...)})
	}
	s.logger.Info("reattach replay gap",
		"session_id", acceptedID,
		"had_session", prevExists,
		"replay_bytes", len(replay),
		"gap_bytes", len(gap),
		"wrapper_pty_bytes", req.PTYBytes,
		"hub_pty_bytes", prevSeen)
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
	s.startAgentChatTail(acceptedID)
	s.wrapperMessageLoop(wc, acceptedID)
}

// wrapperStillRegistered は inject 等の遅い処理の後で、当該 wrapper がまだ
// sessions/wrappers に載っているか（dismiss されていないか）を返す。
func (s *Server) wrapperStillRegistered(id int, wc *wrapperConn) bool {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	return s.sessions[id] != nil && s.wrappers[id] == wc
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
				ses.ptyBytesSeen += int64(len(m.Data))
				if ses.vt == nil {
					ses.vt = newVTBuffer(ses.lastCols, ses.lastRows)
				}
				ses.vt.Write(m.Data)
				provider = ses.Provider
				// Workflow VT は Claude の表示形式だけを較正済み。全 provider へ
				// 広げると shell/codex の通常出力を workflow と誤認するため厳密に限定する。
				if provider == "claude" {
					s.queueWorkflowVTScanLocked(id, ses, now)
				}
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
				// リサイズ直後の VT ミラーは reflow 途中で、TUI のコンポーザ枠が本文へ重なった
				// 「マーカーの対も番号構造も正常だが本文だけ壊れた」ブロックを返す。これを配信すると
				// 本文が変わって sha256 sig も変わるため Hub 側 dedupe を素通りし、Web 側の
				// approvalConsumedSig / _blockSig も一致しなくなって、回答済みの承認バーが復活する。
				// 実測（2026-08-01）: 一括送信 → action-bar 消滅 → rows 15→33 の resize →
				// 約 40ms 後にマーカー再配信、選択肢ラベル末尾が罫線に置き換わっていた。
				// ネイティブ承認スキャン（上の shouldCheckApproval）と同じく、resize debounce の間は
				// ミラーを読まない。SIGWINCH 後の再描画で必ず次チャンクが来るので取りこぼしにならない。
				if isAIProvider(provider) && now.After(ses.vtResizeDebounceUntil) {
					// マーカー抽出は scrollback 込み（画面高超えブロック / Grok 対応）。
					// ネイティブ承認は上の TailLines（現在画面のみ）のまま — 解決済みプロンプトの再検出を避ける。
					marker = extractApprovalMarkerBlock(ses.vt.TailLinesWithScrollback(vtTailLinesForMarker))
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
		case "pty_input_ack":
			s.handlePTYInputAck(wc, id, m.InputSeq)
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
			s.stopAgentChatTail(id)
			if m.State == "completed" || m.State == "error" {
				s.sessionsMu.Lock()
				if cur := s.sessions[id]; cur != nil {
					cur.State = m.State
					if m.Reason != "" {
						cur.EndReason = m.Reason
					}
				}
				s.sessionsMu.Unlock()
				// A child may terminate before it emits the voluntary DONE marker.
				// EOF is nevertheless conclusive: notify the conductor through the
				// same board path and stop waiting for that child.
				s.completeOrchestrationChildOnSessionEnd(id, m.State)
				// 実行中 workflow の heartbeat/journal タイマーを終端させる。
				// 放置すると stale VT が settle を永久ブロックする（F1）。
				s.finalizeWorkflowOnSessionEnd(id)
			}
		}
	}

	// wrapper 切断
	s.sessionsMu.Lock()
	if cur := s.wrappers[id]; cur != nil && cur != wc {
		// この ID は既に別の接続へ差し替わっている（reattachIdentityMatches が同一
		// wrapper と判定して番号を引き継いだ場合）。後始末を続けると現役セッションを
		// disconnected に落とし、UI へ session_end を流し、保留入力・承認ルール・
		// ログライターまで新しい接続の足元から消してしまう。差し替え済みなら降りる。
		// なお wrappers[id] が nil のケース（UI の × による dismiss）は従来どおり
		// 後始末を続ける必要がある（ここで降りるとログが閉じられず漏れる）。
		requeuedCount, requeuedMinSeq, requeuedMaxSeq := s.deferInflightForResendLocked(id, wc)
		s.sessionsMu.Unlock()
		if requeuedCount > 0 {
			s.logger.Info("requeued in-flight pty_input",
				"session_id", id,
				"count", requeuedCount,
				"seq_start", requeuedMinSeq,
				"seq_end", requeuedMaxSeq,
				"cause", "stale_wrapper_disconnect")
		}
		s.logger.Debug("stale wrapper loop ended; session already reattached", "session_id", id)
		return
	}
	delete(s.wrappers, id)
	requeuedCount, requeuedMinSeq, requeuedMaxSeq := s.deferInflightForResendLocked(id, wc)
	var historyToClose *sessionlog.Writer
	var jsonlPathForTranscript string
	var endedProvider, endedCWD string
	// done/timeout も終端として保持する（オーケストレーション完了状態を disconnected で潰さない）。
	if cur := s.sessions[id]; cur != nil && !isTerminalSessionState(cur.State) {
		s.stopAgentChatTailLocked(cur)
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
	// 旧 wrapper（ack 未対応）では従来どおり未送信の保留入力を捨てる。
	// ack 対応 wrapper の場合は in-flight と既存 pending を次の reattach へ残す。
	// 判定に wc 単位のフラグだけを使うと、reattach 直後に 1 件も ack を受けないまま
	// 再び切れた接続が旧 wrapper と誤判定され、保留入力が捨てられる。
	// セッション単位の inputAckCapable も併せて見る。
	// 滞留症状の観測 (input_trace.go / 2026-08-04) として、旧経路で捨てた事実は
	// ログへ残し、「送ったのに何も起きない」の切り分けに使う。
	ackCapable := wc.inputAckSeen.Load()
	if cur := s.sessions[id]; cur != nil && cur.inputAckCapable {
		ackCapable = true
	}
	if n := len(s.pendingInput[id]); n > 0 && !ackCapable {
		s.logger.Warn("pending_input_dropped", "session_id", id, "count", n, "cause", "wrapper_disconnect")
		delete(s.pendingInput, id)
	}
	s.sessionsMu.Unlock()
	if requeuedCount > 0 {
		s.logger.Info("requeued in-flight pty_input",
			"session_id", id,
			"count", requeuedCount,
			"seq_start", requeuedMinSeq,
			"seq_end", requeuedMaxSeq,
			"cause", "wrapper_disconnect")
	}
	// session_end を経ない切断（プロセス kill / Hub 側 WS 断）でも workflow の
	// 追跡タイマーを必ず終端させる（session_end 経由と重複しても冪等）。
	s.finalizeWorkflowOnSessionEnd(id)
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
