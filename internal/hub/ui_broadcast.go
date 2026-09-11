package hub

// ui_broadcast.go: server.go から分離した「UI WebSocket 接続の add/remove/ping/
// snapshot/broadcast」の関数群。
//
// C4 追加分割 (plan_audit_score_s_promotion_2026-07-05.md): server.go の関心事別
// 分割の第七弾。以下 5 関数は UI 側 WS ライフサイクルと全 UI への 1:N ブロードキャスト
// を扱う一塊で、他の関心事から明確に分離できる。挙動は移動前と完全に同一・
// 全て package-private・呼び出し元は変更なし。

import (
	"bytes"
	"context"
	"encoding/json"
	"sort"
	"time"

	"golang.org/x/net/websocket"
	"many-ai-cli/internal/proto"
)

// addUIWithHistoryAtEpoch atomically registers c in the broadcast set and captures a
// snapshot of every session's ptyBuf at the same instant, then returns those
// snapshots as ready-to-send messages. The connection remains in priming mode
// until handleWS has sent the snapshot, history, and replay completion frames;
// broadcasts produced in that interval are queued on the connection so they
// cannot overtake the initial state.
// activeSessionID は UI が現在表示中のセッション ID。このセッションは全量 replay し、
// 他は replayTailForNonActive バイトの tail のみ送信する（UI 接続時のメモリ・帯域削減）。
// authEpoch は handshake 時点の認証世代。登録までの間に全アクセス失効が挟まって
// 世代が進んでいたら、この接続は登録せず nil を返す（失効直後の取りこぼし防止）。
func (s *Server) addUIWithHistoryAtEpoch(c *websocket.Conn, activeSessionID int, authEpoch uint64) (*uiConn, []proto.Message) {
	var items []proto.Message
	s.sessionsMu.Lock()
	if s.authEpoch != authEpoch {
		s.sessionsMu.Unlock()
		return nil, nil
	}
	uc := newUIConn(c)
	uc.activeSessionID = activeSessionID
	uc.authEpoch = authEpoch
	uc.priming = true
	s.uis[c] = uc
	s.stopIdleTimerLocked()
	for id, ses := range s.sessions {
		if ses.UsageProbe {
			continue
		}
		if len(ses.ptyBuf) == 0 {
			continue
		}
		raw := ses.ptyBuf
		if id != activeSessionID && len(raw) > replayTailForNonActive {
			tail := raw[len(raw)-replayTailForNonActive:]
			marker := []byte(chatHistoryUserTurnMarker)
			if !bytes.Contains(tail, marker) {
				// 64KB 末尾にマーカーがない場合、最後のマーカー位置まで遡って含める
				if lastIdx := bytes.LastIndex(raw, marker); lastIdx >= 0 {
					raw = raw[lastIdx:]
				} else {
					raw = tail
				}
			} else {
				raw = tail
			}
		}
		buf := make([]byte, len(raw))
		copy(buf, raw)
		if ses.altScreen {
			// 代替画面バッファ中は、窓（maxPTYBuf / replayTailForNonActive）を切り出した
			// "後" に前置する。切り出し前に足すと窓計算に混ざる。ブラウザの xterm は
			// 再接続のたびに通常画面から始まるため、これを送らないと「CLI は代替画面に
			// いるのに UI は通常画面だと思っている」食い違いが起き、上へスクロールできなく
			// なる（docs/local/bugfix_alt-screen-mode-lost-on-ui-replay_2026-09-12.md）。
			// ESC[?1049l は前置しない: 通常画面が既定なので、代替画面でないときは
			// 何も足さないのが正しい状態。
			prefixed := make([]byte, 0, len(altScreenEnterSeq)+len(buf))
			prefixed = append(prefixed, altScreenEnterSeq...)
			prefixed = append(prefixed, buf...)
			buf = prefixed
		}
		replayEpoch := ensureReplayEpochLocked(ses)
		replayEpoch++
		if replayEpoch == 0 {
			replayEpoch = 1
		}
		ses.replayEpoch = replayEpoch
		items = append(items, proto.Message{
			Type:                "pty_data",
			SessionID:           id,
			Data:                buf,
			Replay:              true,
			ReplayEpoch:         replayEpoch,
			ApprovalSourceEpoch: ensureApprovalSourceEpochLocked(ses),
		})
		items = append(items, proto.Message{
			Type:                   "reattach_replay_done",
			SessionID:              id,
			Replay:                 true,
			ReplayEpoch:            replayEpoch,
			ApprovalSourceEpoch:    ensureApprovalSourceEpochLocked(ses),
			ApprovalConsumed:       ses.approvalConsumedCandidateKey != "",
			ApprovalCandidateKey:   ses.approvalConsumedCandidateKey,
			ApprovalCandidateShape: ses.approvalConsumedCandidateShape,
			ApprovalConsumedEpoch:  ses.approvalConsumedEpoch,
		})
		// lastCols/lastRows はリセットしない。ここで 0 にすると attach 直後の
		// fit → pty_resize が handleResize の skip 判定を必ず通過し、サイズ未変更でも
		// PTY へ resize（SIGWINCH 相当）が届いて TUI が全画面再描画する。replay 済みの
		// 旧フレーム（フッター等）はスクロールバックに残るため二重描画になる。
		// lastCols は「PTY に最後に送った実サイズ」なので保持したままで正しく、
		// 新 UI のサイズが本当に異なる場合のみ resize が通る。
	}
	count := len(s.uis)
	s.sessionsMu.Unlock()
	s.logger.Info("UI connected", "ui_count", count, "active_session", activeSessionID)
	return uc, items
}

func (s *Server) currentAuthEpoch() uint64 {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	return s.authEpoch
}

func (s *Server) currentUIForAuth(c *websocket.Conn, authEpoch uint64) (*uiConn, bool) {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	uc := s.uis[c]
	return uc, uc != nil && s.authEpoch == authEpoch && uc.authEpoch == authEpoch
}

func (s *Server) isCurrentUI(c *websocket.Conn, authEpoch uint64) bool {
	_, ok := s.currentUIForAuth(c, authEpoch)
	return ok
}

// invalidateAllUI advances the authentication boundary and removes every UI
// connection from the live broadcast set. The wrapper set is deliberately not
// touched: revoking browser access must not interrupt running provider CLIs.
func (s *Server) invalidateAllUI() {
	idleMin := s.idleTimeoutMin()
	s.sessionsMu.Lock()
	s.authEpoch++
	connections := make([]*uiConn, 0, len(s.uis))
	for c, uc := range s.uis {
		delete(s.uis, c)
		uc.priming = false
		uc.draining = false
		uc.queued = nil
		s.releaseResizeOwnershipLocked(c)
		connections = append(connections, uc)
	}
	count := len(s.uis)
	if count == 0 {
		s.startIdleTimerLocked(idleMin)
	}
	s.sessionsMu.Unlock()

	for _, uc := range connections {
		uc.invalidate()
	}
	if len(connections) > 0 {
		s.logger.Info("UI connections invalidated", "ui_count", len(connections), "ui_count_remaining", count)
	}
}

func (s *Server) removeUI(c *websocket.Conn) {
	idleMin := s.idleTimeoutMin()
	s.sessionsMu.Lock()
	uc, ok := s.uis[c]
	if !ok {
		s.sessionsMu.Unlock()
		return
	}
	delete(s.uis, c)
	uc.priming = false
	uc.draining = false
	uc.queued = nil
	s.releaseResizeOwnershipLocked(c)
	count := len(s.uis)
	if count == 0 {
		s.startIdleTimerLocked(idleMin)
	}
	s.sessionsMu.Unlock()
	// Ensure the underlying TCP connection is closed so that any goroutine
	// blocked on Receive (e.g. uiLoop) unblocks and exits.
	uc.invalidate()
	s.logger.Info("UI disconnected", "ui_count", count)
}

func (s *Server) releaseResizeOwnershipLocked(c *websocket.Conn) {
	for _, ses := range s.sessions {
		if ses.controllingUI == c {
			ses.controllingUI = nil
		}
	}
}

// pingLoop は uiPingInterval ごとに UI WebSocket へ JSON ping を送り続ける。
// keepalive として機能し、dead connection を検出したら s.uis から除去して終了する。
func (s *Server) pingLoop(ctx context.Context, uc *uiConn) {
	t := time.NewTicker(uiPingInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			if err := uc.sendWithDeadline(map[string]string{"type": "ping"}, time.Now().Add(broadcastWriteTimeout)); err != nil {
				s.logger.Warn("ping failed, removing dead UI connection", "err", err)
				s.removeUI(uc.ws)
				return
			}
		}
	}
}

func (s *Server) sendSnapshot(uc *uiConn) {
	s.sessionsMu.Lock()
	list := make([]*session, 0, len(s.sessions))
	sessionIDs := make([]int, 0, len(s.sessions))
	providerByID := make(map[int]string, len(s.sessions))
	for _, ses := range s.sessions {
		if ses.UsageProbe {
			continue
		}
		list = append(list, ses)
		sessionIDs = append(sessionIDs, ses.ID)
		providerByID[ses.ID] = ses.Provider
	}
	// Go の map イテレーションは順序が不定なため、ID 昇順へ決定的にソートしてから送る。
	// UI 側はまだ sessionOrder に載っていないセッションをこの配列順で並べる（state.ts
	// orderSessions の末尾フォールバック）。ソートしないと再接続のたびにカードの並びが
	// バラバラに変わり、固定したはずの順序が崩れて見える。
	sort.Slice(list, func(i, j int) bool { return list[i].ID < list[j].ID })
	// json.Marshal は sessionsMu 保持下で行う。list は *session ポインタを保持し、
	// markRunning / evaluateIdle / applyDetectedModel 等が sessionsMu 下で同じフィールド
	// （State / Model / Branch 等）を書き換えるため、ロック外で Marshal すると read/write
	// data race になる（-race ビルドで検出可能）。
	b, _ := json.Marshal(list)
	s.sessionsMu.Unlock()
	// hub_instance: Hub 再起動を UI が検出するための起動毎 ID。
	// UI 側は前回値と異なる場合に live session ID キーのローカル状態
	// （チャット・ターミナルバッファ等）を破棄してから snapshot を適用する。
	deadline := time.Now().Add(broadcastWriteTimeout)
	if err := uc.sendWithDeadline(map[string]any{"type": "snapshot", "sessions": json.RawMessage(b), "hub_instance": s.instanceID}, deadline); err != nil {
		s.logger.Warn("sendSnapshot: UI send failed, removing dead connection", "err", err)
		s.removeUI(uc.ws)
		return
	}

	// C3: UI 接続時に既存セッションの usageStat をまとめて送る。
	// usageStatsMu 下では wire message の値だけをコピーし、ネットワーク送信は
	// ロック外で行う。遅いUIがusage受信を止めても、usage更新や他のsnapshotを
	// 待たせない。
	for _, usage := range snapshotUsageStatMessages(sessionIDs, providerByID) {
		if err := uc.sendWithDeadline(usage, time.Now().Add(broadcastWriteTimeout)); err != nil {
			s.logger.Warn("sendSnapshot: usage_stat send failed, removing dead connection", "err", err)
			s.removeUI(uc.ws)
			return
		}
	}

	// C2 (plan_spawn-orchestration-backlog-closeout_c4_spawn-confirm-ui.md):
	// resend every spawn confirmation the Hub is still holding, so a browser
	// that just connected (or reconnected after a reload) does not lose track
	// of a confirmation it never got to decide.
	for _, m := range s.pendingSpawnConfirmationMessages() {
		if err := uc.sendWithDeadline(m, time.Now().Add(broadcastWriteTimeout)); err != nil {
			s.logger.Warn("sendSnapshot: spawn_confirmation_requested resend failed, removing dead connection", "err", err)
			s.removeUI(uc.ws)
			return
		}
	}
}

func snapshotUsageStatMessages(sessionIDs []int, providerByID map[int]string) []proto.Message {
	usageStatsMu.Lock()
	defer usageStatsMu.Unlock()

	items := make([]proto.Message, 0, len(sessionIDs))
	for _, id := range sessionIDs {
		stat, ok := usageStats[id]
		if !ok || stat == nil {
			continue
		}
		items = append(items, proto.Message{
			Type:                        "usage_stat",
			SessionID:                   id,
			Provider:                    providerByID[id],
			CostUSD:                     stat.CostUSD,
			CostKnown:                   stat.CostKnown,
			TokensIn:                    stat.TokensIn,
			TokensOut:                   stat.TokensOut,
			TokensCache:                 stat.TokensCache,
			TokensTotal:                 stat.TokensTotal,
			CtxWindow:                   stat.CtxWindow,
			CtxUsedPct:                  stat.CtxUsedPct,
			RateLimit5hPct:              stat.RateLimit5hPct,
			RateLimit5hReset:            stat.RateLimit5hReset,
			RateLimit7dPct:              stat.RateLimit7dPct,
			RateLimit7dReset:            stat.RateLimit7dReset,
			ClaudeRateLimitsPresent:     stat.ClaudeRateLimitsPresent,
			ClaudeFiveHourFieldPresent:  stat.ClaudeFiveHourFieldPresent,
			ClaudeFiveHourPresent:       stat.ClaudeFiveHourPresent,
			ClaudeSevenDayFieldPresent:  stat.ClaudeSevenDayFieldPresent,
			ClaudeSevenDayPresent:       stat.ClaudeSevenDayPresent,
			CodexRateLimitsPresent:      stat.CodexRateLimitsPresent,
			CodexPrimaryPresent:         stat.CodexPrimaryPresent,
			CodexPrimaryUsedPct:         stat.CodexPrimaryUsedPct,
			CodexPrimaryWindowMinutes:   stat.CodexPrimaryWindowMinutes,
			CodexPrimaryReset:           stat.CodexPrimaryReset,
			CodexSecondaryUsedPct:       stat.CodexSecondaryUsedPct,
			CodexSecondaryPresent:       stat.CodexSecondaryPresent,
			CodexSecondaryWindowMinutes: stat.CodexSecondaryWindowMinutes,
			CodexSecondaryReset:         stat.CodexSecondaryReset,
			CodexCreditsPresent:         stat.CodexCreditsPresent,
			CodexHasCredits:             stat.CodexHasCredits,
			CodexCreditsUnlimited:       stat.CodexCreditsUnlimited,
			CodexCreditsBalance:         stat.CodexCreditsBalance,
			CodexPlanType:               stat.CodexPlanType,
			UsageObservedAt:             stat.UsageObservedAt,
			LinesAdded:                  stat.LinesAdded,
			LinesRemoved:                stat.LinesRemoved,
			EffortLevel:                 stat.EffortLevel,
			Thinking:                    stat.Thinking,
			Exceeds200k:                 stat.Exceeds200k,
			DurationMs:                  stat.DurationMs,
			APIDurationMs:               stat.APIDurationMs,
			Version:                     stat.Version,
			OutputStyle:                 stat.OutputStyle,
			VimMode:                     stat.VimMode,
			AgentName:                   stat.AgentName,
			RepoHost:                    stat.RepoHost,
			RepoOwner:                   stat.RepoOwner,
			RepoName:                    stat.RepoName,
			RemainingPct:                stat.RemainingPct,
			ReasoningOut:                stat.ReasoningOut,
			UsageModel:                  stat.UsageModel,
			UsageStartedAt:              stat.StartedAt,
		})
	}
	return items
}

// finishUIPriming switches a UI from snapshot/replay mode to live mode while
// preserving the exact order of events produced during that transition. The
// draining flag is held under sessionsMu while a copied queue is sent, so a
// broadcast concurrent with the send is appended behind the current batch and
// flushed by the next iteration.
func (s *Server) finishUIPriming(uc *uiConn) bool {
	if uc == nil {
		return false
	}
	for {
		s.sessionsMu.Lock()
		if current := s.uis[uc.ws]; current != uc {
			uc.priming = false
			uc.draining = false
			uc.queued = nil
			s.sessionsMu.Unlock()
			return false
		}
		uc.priming = false
		uc.draining = true
		items := append([]any(nil), uc.queued...)
		uc.queued = nil
		if len(items) == 0 {
			uc.draining = false
			s.sessionsMu.Unlock()
			return true
		}
		s.sessionsMu.Unlock()

		for _, item := range items {
			if err := uc.sendWithDeadline(item, time.Now().Add(broadcastWriteTimeout)); err != nil {
				s.logger.Warn("finishUIPriming: UI send failed, removing dead connection", "err", err)
				s.removeUI(uc.ws)
				return false
			}
		}
	}
}

func (s *Server) broadcast(m any) {
	if msg, ok := m.(proto.Message); ok && msg.SessionID > 0 {
		s.sessionsMu.Lock()
		probe := false
		if ses := s.sessions[msg.SessionID]; ses != nil {
			probe = ses.UsageProbe
		}
		s.sessionsMu.Unlock()
		if probe {
			return
		}
	}
	s.sessionsMu.Lock()
	ucs := make([]*uiConn, 0, len(s.uis))
	for _, uc := range s.uis {
		if uc.priming || uc.draining {
			uc.queued = append(uc.queued, m)
			continue
		}
		ucs = append(ucs, uc)
	}
	s.sessionsMu.Unlock()
	for _, uc := range ucs {
		// Per-UI deadline: a single shared absolute deadline lets one slow UI
		// (which can block up to broadcastWriteTimeout) leave the remaining
		// healthy UIs with an already-expired budget, so they time out and get
		// dropped through no fault of their own.
		if err := uc.sendWithDeadline(m, time.Now().Add(broadcastWriteTimeout)); err != nil {
			s.logger.Warn("broadcast: UI send failed, removing dead connection", "err", err)
			s.removeUI(uc.ws)
		}
	}
}
