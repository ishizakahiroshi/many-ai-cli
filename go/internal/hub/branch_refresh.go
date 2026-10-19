package hub

// branch_refresh.go: server.go から分離した「git branch 情報の非同期再取得」の
// 2 関数群。
//
// C4 追加分割 (plan_audit_score_s_promotion_2026-07-05.md): server.go の関心事別
// 分割の第五弾。以下 2 関数は cwd 単位で並列制限付きセマフォを使った git branch /
// change stats の再取得と、結果のセッションへの反映を扱う一塊で、他の関心事から
// 明確に分離できる。
//
// project_id だけは他の値と性質が違う（2026-09-01 追記）。branch や変更数は毎回
// 取り直すので 1 回の失敗は次の周期で消えるが、project_id は 1 度きりの取得なので
// 失敗をそのまま記録すると恒久的な誤りになる。取りこぼしを作らないために、この
// ファイルは 2 つの規則を守る。
//
//  1. 取得が確定しなかった cwd では projectChecked を立てない（次の周期で取り直す）
//  2. 実行中の cwd に来た再取得依頼は捨てず、実行中の処理が終わったらもう一度回す
//
// 由来: docs/local/bugfix_sidebar-box-splits-on-empty-project-id_2026-09-01.md

import (
	"strings"

	"many-ai-cli/internal/proto"
)

// projectRootLookup は gitProjectRoot への差し替え口。テストが「取れなかった」を
// 再現するためだけに存在する（本番では常に gitProjectRoot）。
var projectRootLookup = gitProjectRoot

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
	defer s.branchRefreshMu.Unlock()
	if s.branchRefreshInFlight == nil {
		s.branchRefreshInFlight = make(map[string]struct{})
	}
	if s.branchRefreshPending == nil {
		s.branchRefreshPending = make(map[string][]int)
	}
	for cwd, ids := range byCWD {
		if _, ok := s.branchRefreshInFlight[cwd]; ok {
			// 実行中の処理は自分が抱えた id しか見ないので、ここで捨てると
			// このセッションだけ project_id が未解決のまま取り残される。
			// 呼び出し元（idle_state）は branchCheckedAt を既に進めているため、
			// 捨てた依頼は自動では戻ってこない。
			s.branchRefreshPending[cwd] = mergeSessionIDs(s.branchRefreshPending[cwd], ids)
			continue
		}
		s.startBranchRefreshLocked(cwd, ids)
	}
}

// startBranchRefreshLocked は 1 つの cwd の再取得を開始する。branchRefreshMu を
// 保持したまま呼ぶこと。
func (s *Server) startBranchRefreshLocked(cwd string, ids []int) {
	if len(ids) == 0 {
		return
	}
	s.branchRefreshInFlight[cwd] = struct{}{}
	if s.branchRefreshSem == nil {
		s.branchRefreshSem = make(chan struct{}, branchRefreshWorkers)
	}
	sem := s.branchRefreshSem
	ids = append([]int(nil), ids...)
	s.safeGo("branch refresh", func() {
		sem <- struct{}{}
		defer func() {
			<-sem
			s.branchRefreshMu.Lock()
			delete(s.branchRefreshInFlight, cwd)
			if next := s.branchRefreshPending[cwd]; len(next) > 0 {
				delete(s.branchRefreshPending, cwd)
				s.startBranchRefreshLocked(cwd, next)
			}
			s.branchRefreshMu.Unlock()
		}()
		s.refreshBranchForCWD(cwd, ids)
	})
}

// mergeSessionIDs は dst に src を重複なく足す。待ち行列が同じ id で膨らまないようにする。
func mergeSessionIDs(dst, src []int) []int {
	seen := make(map[int]struct{}, len(dst)+len(src))
	for _, id := range dst {
		seen[id] = struct{}{}
	}
	for _, id := range src {
		if _, ok := seen[id]; ok {
			continue
		}
		seen[id] = struct{}{}
		dst = append(dst, id)
	}
	return dst
}

func (s *Server) refreshBranchForCWD(cwd string, ids []int) {
	branch := gitBranch(cwd)
	gitFiles, gitAdded, gitDeleted := gitChangeStats(cwd)
	// project_id は cwd が変わらない限り不変なので、この cwd のセッションが全員
	// 解決済みなら git を叩かない。branch と違って毎回取り直す値ではない。
	projectRoot, projectResolved, projectNeeded := "", false, false
	s.sessionsMu.Lock()
	for _, id := range ids {
		if ses := s.sessions[id]; ses != nil && ses.CWD == cwd && !ses.projectChecked {
			projectNeeded = true
			break
		}
	}
	s.sessionsMu.Unlock()
	if projectNeeded {
		projectRoot, projectResolved = projectRootLookup(cwd)
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
		// 確定しなかった取得は記録しない。空を記録すると、そのセッションだけ
		// サイドバーの箱のキーが cwd 末尾へ落ちたまま二度と直らない。
		projectChanged := projectNeeded && projectResolved && !ses.projectChecked
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
