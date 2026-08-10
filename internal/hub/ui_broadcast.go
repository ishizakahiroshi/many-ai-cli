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

// addUIWithHistory atomically registers c in the broadcast set and captures a
// snapshot of every session's ptyBuf at the same instant, then returns those
// snapshots as ready-to-send messages. Callers must send the returned messages
// to c after this call; any PTY data arriving after the lock is released is
// delivered via broadcast, so the snapshot and live stream do not overlap.
// activeSessionID は UI が現在表示中のセッション ID。このセッションは全量 replay し、
// 他は replayTailForNonActive バイトの tail のみ送信する（UI 接続時のメモリ・帯域削減）。
func (s *Server) addUIWithHistory(c *websocket.Conn, activeSessionID int) (*uiConn, []proto.Message) {
	var items []proto.Message
	s.sessionsMu.Lock()
	uc := newUIConn(c)
	uc.activeSessionID = activeSessionID
	s.uis[c] = uc
	s.stopIdleTimerLocked()
	for id, ses := range s.sessions {
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

func (s *Server) removeUI(c *websocket.Conn) {
	idleMin := s.idleTimeoutMin()
	s.sessionsMu.Lock()
	uc, ok := s.uis[c]
	if !ok {
		s.sessionsMu.Unlock()
		return
	}
	delete(s.uis, c)
	s.releaseResizeOwnershipLocked(c)
	count := len(s.uis)
	if count == 0 {
		s.startIdleTimerLocked(idleMin)
	}
	s.sessionsMu.Unlock()
	// Ensure the underlying TCP connection is closed so that any goroutine
	// blocked on Receive (e.g. uiLoop) unblocks and exits.
	uc.close()
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
	// これにより再接続時・リロード時にステータスバーが即座に復元される。
	// ロック順序（usage_stat.go の不変条件）: usageStatsMu 保持中に sessionsMu を取得しない。
	// provider は上の sessionsMu 区間で確定済みの providerByID から引く（ネスト取得を避ける）。
	usageStatsMu.Lock()
	for _, id := range sessionIDs {
		if stat, ok := usageStats[id]; ok {
			if err := uc.sendWithDeadline(proto.Message{
				Type:             "usage_stat",
				SessionID:        id,
				Provider:         providerByID[id],
				CostUSD:          stat.CostUSD,
				CostKnown:        stat.CostKnown,
				TokensIn:         stat.TokensIn,
				TokensOut:        stat.TokensOut,
				TokensCache:      stat.TokensCache,
				TokensTotal:      stat.TokensTotal,
				CtxWindow:        stat.CtxWindow,
				CtxUsedPct:       stat.CtxUsedPct,
				RateLimit5hPct:   stat.RateLimit5hPct,
				RateLimit5hReset: stat.RateLimit5hReset,
				RateLimit7dPct:   stat.RateLimit7dPct,
				RateLimit7dReset: stat.RateLimit7dReset,
				LinesAdded:       stat.LinesAdded,
				LinesRemoved:     stat.LinesRemoved,
				EffortLevel:      stat.EffortLevel,
				Thinking:         stat.Thinking,
				Exceeds200k:      stat.Exceeds200k,
				DurationMs:       stat.DurationMs,
				APIDurationMs:    stat.APIDurationMs,
				Version:          stat.Version,
				OutputStyle:      stat.OutputStyle,
				VimMode:          stat.VimMode,
				AgentName:        stat.AgentName,
				RepoHost:         stat.RepoHost,
				RepoOwner:        stat.RepoOwner,
				RepoName:         stat.RepoName,
				RemainingPct:     stat.RemainingPct,
				ReasoningOut:     stat.ReasoningOut,
				UsageModel:       stat.UsageModel,
				UsageStartedAt:   stat.StartedAt,
			}, deadline); err != nil {
				usageStatsMu.Unlock()
				s.logger.Warn("sendSnapshot: usage_stat send failed, removing dead connection", "err", err)
				s.removeUI(uc.ws)
				return
			}
		}
	}
	usageStatsMu.Unlock()
}

func (s *Server) broadcast(m any) {
	s.sessionsMu.Lock()
	ucs := make([]*uiConn, 0, len(s.uis))
	for _, uc := range s.uis {
		ucs = append(ucs, uc)
	}
	s.sessionsMu.Unlock()
	deadline := time.Now().Add(broadcastWriteTimeout)
	for _, uc := range ucs {
		if err := uc.sendWithDeadline(m, deadline); err != nil {
			s.logger.Warn("broadcast: UI send failed, removing dead connection", "err", err)
			s.removeUI(uc.ws)
		}
	}
}
