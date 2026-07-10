package hub

// idle_state.go: server.go から分離した「セッションのアイドル状態機械 + Hub 全体
// アイドルタイマー」の関数群。
//
// C4 追加分割 (plan_audit_score_s_promotion_2026-07-05.md): server.go の関心事別
// 分割の第六弾。以下 5 関数は「PTY 出力受信で running へ格上げ」「一定間隔で
// running→waiting/standby へ格下げ判定」「UI 切断時の Hub 全体アイドル停止
// タイマー」を扱う一塊で、他の関心事から明確に分離できる。挙動は移動前と完全
// に同一・全て package-private・呼び出し元は変更なし。

import (
	"context"
	"fmt"
	"time"

	"many-ai-cli/internal/proto"
)

func (s *Server) markRunning(id int) {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return
	}
	now := time.Now()
	ses.lastOutputAt = now
	ses.LastOutputAt = now.Format(time.RFC3339)
	if ses.approvalVisible {
		s.sessionsMu.Unlock()
		return
	}
	// 終端状態は PTY 残余チャンクで running に戻さない（オーケストレーション DONE/timeout 含む）。
	// lastOutputAt は上で既に更新済み。State だけ触らない。
	if isTerminalSessionState(ses.State) {
		s.sessionsMu.Unlock()
		return
	}
	changed := ses.State != "running"
	if changed {
		ses.State = "running"
	}
	provider, display, cwd, branch, label, model, route, lastOutputAt := ses.Provider, ses.Display, ses.CWD, ses.Branch, ses.Label, ses.Model, ses.Route, ses.LastOutputAt
	s.sessionsMu.Unlock()
	if changed {
		s.broadcast(proto.Message{Type: "session_update", SessionID: id, Provider: provider, Display: display, CWD: cwd, Branch: branch, Label: label, Model: model, Route: route, State: "running", LastOutputAt: lastOutputAt})
	}
	if s.sessionStore != nil {
		s.sessionStore.UpdateSessionState(id, "running", lastOutputAt)
	}
}

// stateTicker は idleAfter 経過後の running → waiting 遷移を担う。
func (s *Server) stateTicker(ctx context.Context) {
	defer func() {
		if r := recover(); r != nil {
			s.logger.Error("stateTicker panic recovered", "recover", fmt.Sprintf("%v", r))
		}
	}()
	t := time.NewTicker(tickerInterval)
	defer t.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-t.C:
			s.evaluateIdle()
		}
	}
}

func (s *Server) evaluateIdle() {
	now := time.Now()
	s.sessionsMu.Lock()
	type change struct {
		id           int
		provider     string
		display      string
		cwd          string
		branch       string
		label        string
		model        string
		route        string
		state        string
		lastOutputAt string
		approvalWait bool
		fallbackDone bool
	}
	var changes []change
	var branchChecks []branchRefreshRequest
	for id, ses := range s.sessions {
		if now.Sub(ses.branchCheckedAt) >= branchRefreshAfter {
			ses.branchCheckedAt = now
			branchChecks = append(branchChecks, branchRefreshRequest{id: id, cwd: ses.CWD})
		}
		// approvalVisible リース切れの自動クリア:
		// UI からの false ヒントが失われても（リロード desync・複数クライアント等）
		// 再主張が止まれば waiting 固着から自動回復する。
		// go_vt detector がまだ native prompt を見ている間は UI 不在でも維持する。
		if ses.approvalVisible && ses.nativeApprovalSig == "" && now.Sub(ses.approvalVisibleAt) >= approvalVisibleLease {
			ses.approvalVisible = false
			ses.approvalVisibleAt = time.Time{}
		}
		var newState string
		switch ses.State {
		case "running":
			if ses.approvalVisible {
				// 承認UI表示中はアイドルタイマーを待たず即 waiting に遷移
				newState = "waiting"
			} else if !ses.lastOutputAt.IsZero() && now.Sub(ses.lastOutputAt) >= idleAfter {
				newState = ses.idleStateName()
			}
		case "waiting", "standby":
			// approvalVisible のフリップに追従（UI hint 反映）
			newState = ses.idleStateName()
		}
		if newState != "" && newState != ses.State {
			ses.State = newState
			changes = append(changes, change{id: id, provider: ses.Provider, display: ses.Display, cwd: ses.CWD, branch: ses.Branch, label: ses.Label, model: ses.Model, route: ses.Route, state: newState, lastOutputAt: ses.LastOutputAt, approvalWait: newState == "waiting" && ses.approvalVisible, fallbackDone: newState == "standby"})
		}
	}
	s.sessionsMu.Unlock()
	for _, c := range changes {
		s.broadcast(proto.Message{Type: "session_update", SessionID: c.id, Provider: c.provider, Display: c.display, CWD: c.cwd, Branch: c.branch, Label: c.label, Model: c.model, Route: c.route, State: c.state, LastOutputAt: c.lastOutputAt})
		if s.sessionStore != nil {
			s.sessionStore.UpdateSessionState(c.id, c.state, c.lastOutputAt)
		}
		if c.approvalWait {
			approvalID := fmt.Sprintf("ui-%d-%s", c.id, c.lastOutputAt)
			s.notifyApprovalPush(c.id, approvalID, c.provider, "", "")
			s.notifyApprovalOutbound(c.id, approvalID, c.provider, "", "")
		}
		if c.fallbackDone {
			s.maybeCreateFallbackDoneSummary(c.id)
		}
	}
	s.queueBranchRefreshes(branchChecks)
}

// startIdleTimerLocked starts the idle-timeout timer. Caller must hold
// sessionsMu. idleMin is the configured timeout, snapshotted via idleTimeoutMin
// before taking sessionsMu so cfgMu is never held under sessionsMu.
func (s *Server) startIdleTimerLocked(idleMin int) {
	if idleMin <= 0 || s.idleTimer != nil {
		return
	}
	s.idleGen++
	gen := s.idleGen
	d := time.Duration(idleMin) * time.Minute
	s.idleTimer = time.AfterFunc(d, func() {
		s.sessionsMu.Lock()
		if s.idleGen != gen {
			// A newer timer was started (UI reconnected) or the timer was
			// stopped; skip the kill to avoid evicting a just-reconnected UI.
			s.sessionsMu.Unlock()
			return
		}
		s.idleTimer = nil
		s.sessionsMu.Unlock()
		s.logger.Info("idle timeout reached, killing all wrappers", "minutes", idleMin)
		s.killAllWrappers()
	})
}

func (s *Server) stopIdleTimerLocked() {
	if s.idleTimer == nil {
		return
	}
	s.idleGen++ // invalidate any in-flight AfterFunc callback
	s.idleTimer.Stop()
	s.idleTimer = nil
}
