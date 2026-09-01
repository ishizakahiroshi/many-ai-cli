package hub

// project_id_latch_test.go: project_id の「1 度きりの取得」が壊れないことを固定する。
//
// 守る不変条件は 2 つ。どちらも破ると、そのセッションだけサイドバーの箱のキーが
// cwd 末尾へ落ちたまま二度と直らない。
//
//  1. 確定しなかった取得（タイムアウト等）は記録しない
//  2. 実行中の cwd に来た再取得依頼を捨てない
//
// 由来: docs/local/bugfix_sidebar-box-splits-on-empty-project-id_2026-09-01.md

import (
	"sync"
	"testing"
	"time"
)

// stubProjectRootLookup は projectRootLookup を差し替え、テスト終了時に戻す。
func stubProjectRootLookup(t *testing.T, fn func(cwd string) (string, bool)) {
	t.Helper()
	orig := projectRootLookup
	projectRootLookup = fn
	t.Cleanup(func() { projectRootLookup = orig })
}

func sessionProjectState(s *Server, id int) (string, bool) {
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[id]
	if ses == nil {
		return "", false
	}
	return ses.ProjectID, ses.projectChecked
}

// TestRefreshBranchKeepsRetryingUnresolvedProjectRoot は、取得に失敗した cwd を
// latch せず次の機会に取り直すことを固定する。ここが崩れると、git が 1 回でも
// branchLookupTimeout(250ms) を超えたセッションが恒久的に別の箱へ落ちる。
func TestRefreshBranchKeepsRetryingUnresolvedProjectRoot(t *testing.T) {
	s := newTestServer()
	cwd := t.TempDir()
	ses := registerTestSession(s, 1, "claude")
	ses.CWD = cwd

	stubProjectRootLookup(t, func(string) (string, bool) { return "", false })
	s.refreshBranchForCWD(cwd, []int{1})

	if id, checked := sessionProjectState(s, 1); checked || id != "" {
		t.Fatalf("未確定の取得を記録した: ProjectID=%q projectChecked=%v", id, checked)
	}

	stubProjectRootLookup(t, func(string) (string, bool) { return cwd, true })
	s.refreshBranchForCWD(cwd, []int{1})

	if id, checked := sessionProjectState(s, 1); !checked || id != cwd {
		t.Fatalf("取り直しが記録されない: ProjectID=%q projectChecked=%v, want %q true", id, checked, cwd)
	}
}

// TestRefreshBranchLatchesResolvedEmptyProjectRoot は、git 管理外だと確定した空は
// ちゃんと記録することを固定する。ここまで retry にすると、git 管理外の cwd で
// 毎周期 git を起動し続ける。
func TestRefreshBranchLatchesResolvedEmptyProjectRoot(t *testing.T) {
	s := newTestServer()
	cwd := t.TempDir()
	ses := registerTestSession(s, 1, "claude")
	ses.CWD = cwd

	calls := 0
	stubProjectRootLookup(t, func(string) (string, bool) { calls++; return "", true })

	s.refreshBranchForCWD(cwd, []int{1})
	s.refreshBranchForCWD(cwd, []int{1})

	if _, checked := sessionProjectState(s, 1); !checked {
		t.Fatal("確定した空を記録していない")
	}
	if calls != 1 {
		t.Fatalf("確定後も取り直している: calls = %d, want 1", calls)
	}
}

// TestQueueBranchRefreshesQueuesRequestForBusyCWD は、実行中の cwd に来た依頼を
// 捨てずに待ち行列へ入れることを固定する。呼び出し元（idle_state）は依頼を出した
// 時点で branchCheckedAt を進めているので、ここで捨てると自動では戻ってこない。
func TestQueueBranchRefreshesQueuesRequestForBusyCWD(t *testing.T) {
	s := newTestServer()
	cwd := t.TempDir()
	s.branchRefreshInFlight = map[string]struct{}{cwd: {}}

	s.queueBranchRefreshes([]branchRefreshRequest{{id: 5, cwd: cwd}})
	s.queueBranchRefreshes([]branchRefreshRequest{{id: 5, cwd: cwd}, {id: 6, cwd: cwd}})

	s.branchRefreshMu.Lock()
	pending := append([]int(nil), s.branchRefreshPending[cwd]...)
	s.branchRefreshMu.Unlock()

	if len(pending) != 2 || pending[0] != 5 || pending[1] != 6 {
		t.Fatalf("待ち行列 = %v, want [5 6]（重複なしで保持）", pending)
	}
}

// TestBranchRefreshDrainsPendingRequests は、実行中に積まれた依頼が実行後に
// ちゃんと回ることを固定する。待ち行列へ入れるだけで流さないと、捨てるのと同じ。
func TestBranchRefreshDrainsPendingRequests(t *testing.T) {
	s := newTestServer()
	cwd := t.TempDir()
	first := registerTestSession(s, 1, "claude")
	first.CWD = cwd
	second := registerTestSession(s, 2, "claude")
	second.CWD = cwd

	entered := make(chan struct{})
	gate := make(chan struct{})
	var once sync.Once
	stubProjectRootLookup(t, func(string) (string, bool) {
		once.Do(func() {
			close(entered)
			<-gate
		})
		return cwd, true
	})

	// #1 の再取得を走らせ、lookup の中で止める。
	s.queueBranchRefreshes([]branchRefreshRequest{{id: 1, cwd: cwd}})
	select {
	case <-entered:
	case <-time.After(5 * time.Second):
		t.Fatal("1 本目の再取得が始まらない")
	}

	// 実行中の cwd へ #2 の依頼が来る。
	s.queueBranchRefreshes([]branchRefreshRequest{{id: 2, cwd: cwd}})
	close(gate)

	deadline := time.Now().Add(5 * time.Second)
	for {
		if _, checked := sessionProjectState(s, 2); checked {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("実行中に積まれた依頼が流れず、#2 の project_id が未解決のまま残った")
		}
		time.Sleep(10 * time.Millisecond)
	}

	if id, _ := sessionProjectState(s, 2); id != cwd {
		t.Fatalf("#2 の ProjectID = %q, want %q", id, cwd)
	}
}
