package hub

// branch_refresh.go: server.go から分離した「git branch 情報の非同期再取得」の
// 2 関数群。
//
// C4 追加分割 (plan_audit_score_s_promotion_2026-07-05.md): server.go の関心事別
// 分割の第五弾。以下 2 関数は cwd 単位で並列制限付きセマフォを使った git branch /
// change stats の再取得と、結果のセッションへの反映を扱う一塊で、他の関心事から
// 明確に分離できる。挙動は移動前と完全に同一・全て package-private・呼び出し元は
// 変更なし。

import (
	"strings"

	"many-ai-cli/internal/proto"
)

func (s *Server) queueBranchRefreshes(checks []branchRefreshRequest) {
	if len(checks) == 0 {
		return
	}
	byCWD := make(map[string][]int, len(checks))
	for _, check := range checks {
		cwd := strings.TrimSpace(check.cwd)
		if cwd == "" {
			continue
		}
		byCWD[cwd] = append(byCWD[cwd], check.id)
	}
	if len(byCWD) == 0 {
		return
	}
	s.branchRefreshMu.Lock()
	if s.branchRefreshSem == nil {
		s.branchRefreshSem = make(chan struct{}, branchRefreshWorkers)
	}
	if s.branchRefreshInFlight == nil {
		s.branchRefreshInFlight = make(map[string]struct{})
	}
	for cwd, ids := range byCWD {
		if _, ok := s.branchRefreshInFlight[cwd]; ok {
			continue
		}
		s.branchRefreshInFlight[cwd] = struct{}{}
		cwd := cwd
		ids := append([]int(nil), ids...)
		sem := s.branchRefreshSem
		s.safeGo("branch refresh", func() {
			sem <- struct{}{}
			defer func() {
				<-sem
				s.branchRefreshMu.Lock()
				delete(s.branchRefreshInFlight, cwd)
				s.branchRefreshMu.Unlock()
			}()
			s.refreshBranchForCWD(cwd, ids)
		})
	}
	s.branchRefreshMu.Unlock()
}

func (s *Server) refreshBranchForCWD(cwd string, ids []int) {
	branch := gitBranch(cwd)
	gitFiles, gitAdded, gitDeleted := gitChangeStats(cwd)
	// project_id は cwd が変わらない限り不変なので、この cwd のセッションが全員
	// 解決済みなら git を叩かない。branch と違って毎回取り直す値ではない。
	projectRoot, projectNeeded := "", false
	s.sessionsMu.Lock()
	for _, id := range ids {
		if ses := s.sessions[id]; ses != nil && ses.CWD == cwd && !ses.projectChecked {
			projectNeeded = true
			break
		}
	}
	s.sessionsMu.Unlock()
	if projectNeeded {
		projectRoot = gitProjectRoot(cwd)
	}

	msgs := make([]proto.Message, 0, len(ids))
	s.sessionsMu.Lock()
	for _, id := range ids {
		ses := s.sessions[id]
		if ses == nil || ses.CWD != cwd {
			continue
		}
		branchChanged := ses.Branch != branch
		gitChanged := !ses.gitChecked || ses.gitFiles != gitFiles || ses.gitAdded != gitAdded || ses.gitDeleted != gitDeleted
		projectChanged := projectNeeded && !ses.projectChecked
		if projectChanged {
			ses.ProjectID = projectRoot
			ses.projectChecked = true
		}
		if !branchChanged && !gitChanged && !projectChanged {
			continue
		}
		ses.Branch = branch
		ses.gitChecked = true
		ses.gitFiles = gitFiles
		ses.gitAdded = gitAdded
		ses.gitDeleted = gitDeleted
		msgs = append(msgs, proto.Message{
			Type:         "session_update",
			SessionID:    id,
			Provider:     ses.Provider,
			Display:      ses.Display,
			CWD:          ses.CWD,
			Branch:       ses.Branch,
			ProjectID:    ses.ProjectID,
			Label:        ses.Label,
			Model:        ses.Model,
			Route:        ses.Route,
			State:        ses.State,
			LastOutputAt: ses.LastOutputAt,
			StartedAt:    ses.StartedAt,
			FirstMessage: ses.FirstMessage,
			LastMessage:  ses.LastMessage,
			GitChecked:   true,
			GitFiles:     gitFiles,
			GitAdded:     gitAdded,
			GitDeleted:   gitDeleted,
		})
	}
	s.sessionsMu.Unlock()
	for _, msg := range msgs {
		s.broadcast(msg)
	}
}
