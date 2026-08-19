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
	// 終端状態は PTY 残余チャンクで running に戻さない（オーケストレーション DONE/timeout 含む）。
	// lastOutputAt は上で既に更新済み。State だけ触らない。
	if isTerminalSessionState(ses.State) {
		s.sessionsMu.Unlock()
		return
	}
	if ses.State != "running" {
		// standby/waiting から running へ入る = 新しいターンの開始。
		// 前ターン終了後の待ち時間を transcript 停滞として持ち越さない（transcript_stall.go）。
		resetTranscriptTrackingLocked(ses, now)
	}
	before := ses.Activity
	ses.Activity.OutputIdle = false
	ses.Activity.WorkflowActive = !ses.approvalVisible
	ses.Activity.AwaitingApproval = ses.approvalVisible
	ses.Activity.AwaitingUser = ses.approvalVisible
	ses.Activity.Normalize()
	ses.State = ses.Activity.DisplayState()
	changed := before != ses.Activity || ses.State != "running"
	provider, display, cwd, branch, label, model, route, state, activity, lastOutputAt := ses.Provider, ses.Display, ses.CWD, ses.Branch, ses.Label, ses.Model, ses.Route, ses.State, ses.Activity, ses.LastOutputAt
	transcriptGrewAt := ses.TranscriptGrewAt
	s.sessionsMu.Unlock()
	if changed {
		s.broadcast(proto.Message{Type: "session_update", SessionID: id, Provider: provider, Display: display, CWD: cwd, Branch: branch, Label: label, Model: model, Route: route, State: state, OutputIdle: activity.OutputIdle, WorkflowActive: activity.WorkflowActive, AwaitingUser: activity.AwaitingUser, AwaitingApproval: activity.AwaitingApproval, Activity: activityMessage(activity), LastOutputAt: lastOutputAt, TranscriptGrewAt: transcriptGrewAt})
	}
	if s.sessionStore != nil {
		s.sessionStore.UpdateSessionState(id, state, lastOutputAt)
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
		state            string
		activity         SessionActivity
		lastOutputAt     string
		transcriptGrewAt string
		approvalWait     bool
		fallbackDone     bool
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
		if isTerminalSessionState(ses.State) {
			continue
		}
		before := ses.Activity
		// Sessions that have not emitted PTY output yet are output-idle too.
		// This also preserves compatibility for restored/legacy sessions that
		// predate the activity fields and therefore have a zero timestamp.
		if ses.lastOutputAt.IsZero() || now.Sub(ses.lastOutputAt) >= idleAfter {
			ses.Activity.OutputIdle = true
		}
		ses.Activity.AwaitingApproval = ses.approvalVisible
		ses.Activity.AwaitingUser = ses.approvalVisible
		ses.Activity.WorkflowActive = !ses.Activity.OutputIdle && !ses.Activity.AwaitingUser
		ses.Activity.Normalize()
		newState := ses.Activity.DisplayState()
		if before != ses.Activity || newState != ses.State {
			ses.State = newState
			changes = append(changes, change{id: id, provider: ses.Provider, display: ses.Display, cwd: ses.CWD, branch: ses.Branch, label: ses.Label, model: ses.Model, route: ses.Route, state: newState, activity: ses.Activity, lastOutputAt: ses.LastOutputAt, transcriptGrewAt: ses.TranscriptGrewAt, approvalWait: newState == "waiting" && ses.Activity.AwaitingApproval, fallbackDone: newState == "standby"})
		}
	}
	// State を確定させた後に集める（running になったばかりのセッションも拾うため）。
	transcriptChecks := s.collectTranscriptChecksLocked(now)
	s.sessionsMu.Unlock()
	for _, c := range changes {
		s.broadcast(proto.Message{Type: "session_update", SessionID: c.id, Provider: c.provider, Display: c.display, CWD: c.cwd, Branch: c.branch, Label: c.label, Model: c.model, Route: c.route, State: c.state, OutputIdle: c.activity.OutputIdle, WorkflowActive: c.activity.WorkflowActive, AwaitingUser: c.activity.AwaitingUser, AwaitingApproval: c.activity.AwaitingApproval, Activity: activityMessage(c.activity), LastOutputAt: c.lastOutputAt, TranscriptGrewAt: c.transcriptGrewAt})
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
	s.queueTranscriptChecks(transcriptChecks)
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
		s.killAllWrappers("idle_timeout")
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
