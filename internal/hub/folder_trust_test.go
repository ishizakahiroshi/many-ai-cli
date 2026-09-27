package hub

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"

	"many-ai-cli/internal/clitrust"
	"many-ai-cli/internal/subscription"
)

// 子 plan: docs/local/plan_child-launch-prompt-and-trust_c3_spawn-confirm-trust.md 内部 C1。
// 承認画面の「このフォルダを信頼済みとして登録する」が、承認の決定からだけ入り、
// 起動の直前に子の env と作業フォルダで 1 度だけ書かれることを固定する。
// 実際の CLI の設定ファイルには触らない（folderTrustGrant を必ず差し替える）。

// AI（conductor）の起動要求の本文に grant_folder_trust を書いても入らない。
func TestSpawnChildRequestBodyCannotSetGrantFolderTrust(t *testing.T) {
	r := httptest.NewRequest(http.MethodPost, "/api/sessions/1/spawn-child",
		strings.NewReader(`{"role":"tester","provider":"claude","grant_folder_trust":true}`))
	var body spawnChildRequest
	if !decodeJSON(httptest.NewRecorder(), r, &body) {
		t.Fatal("decodeJSON failed")
	}
	if body.GrantFolderTrust {
		t.Fatal("GrantFolderTrust was set from the AI's request body")
	}
}

// 決定の true だけが要求へ写る。nil（欄なし）/ false は false。要求の側に
// true が紛れ込んでいても、決定が無ければ false に戻る。
func TestApplySpawnConfirmationDecisionGrantFolderTrust(t *testing.T) {
	yes, no := true, false
	for _, tc := range []struct {
		name     string
		decision *bool
		want     bool
	}{
		{"absent", nil, false},
		{"false", &no, false},
		{"true", &yes, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			requested := spawnChildRequest{Role: "tester", Provider: "claude", GrantFolderTrust: true}
			got := applySpawnConfirmationDecision(requested, spawnConfirmationDecision{GrantFolderTrust: tc.decision})
			if got.GrantFolderTrust != tc.want {
				t.Errorf("GrantFolderTrust = %v, want %v", got.GrantFolderTrust, tc.want)
			}
		})
	}
}

// 承認画面へ送るメッセージと、その再送の両方に trust_grant_providers が入る。
func TestSpawnConfirmationMessagesCarryTrustGrantProviders(t *testing.T) {
	s := newTestServer()
	parent := registerTestSession(s, 1, "claude")
	pending, err := s.registerSpawnConfirmation(parent, "", spawnChildRequest{Role: "tester", Provider: "claude"})
	if err != nil {
		t.Fatalf("registerSpawnConfirmation: %v", err.detail)
	}
	want := []string{"claude", "codex"}
	if got := s.spawnConfirmationRequestedMessage(pending).TrustGrantProviders; !reflect.DeepEqual(got, want) {
		t.Errorf("requested message: TrustGrantProviders = %v, want %v", got, want)
	}
	resent := s.pendingSpawnConfirmationMessages()
	if len(resent) != 1 {
		t.Fatalf("resend: %d messages, want 1", len(resent))
	}
	if got := resent[0].TrustGrantProviders; !reflect.DeepEqual(got, want) {
		t.Errorf("resend: TrustGrantProviders = %v, want %v", got, want)
	}
}

type folderTrustCall struct {
	target clitrust.Target
}

type folderTrustStub struct {
	mu     sync.Mutex
	calls  []folderTrustCall
	result clitrust.Result
	err    error
}

func (f *folderTrustStub) grant(t clitrust.Target) (clitrust.Result, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls = append(f.calls, folderTrustCall{target: t})
	return f.result, f.err
}

func (f *folderTrustStub) snapshot() []folderTrustCall {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]folderTrustCall(nil), f.calls...)
}

// 結果ごとに board へ 1 行が残る。失敗・既存の untrusted では、子の画面で
// 確認が出うることまで書く（board は conductor も読む）。
func TestGrantChildFolderTrustRecordsEachOutcome(t *testing.T) {
	for _, tc := range []struct {
		name   string
		result clitrust.Result
		err    error
		want   []string
	}{
		{"registered", clitrust.Result{Written: true, Key: "D:/work/a", ConfigPath: `C:\cfg\.claude.json`}, nil,
			[]string{"result=registered", "key=D:/work/a", `file=C:\cfg\.claude.json`}},
		{"already trusted", clitrust.Result{Existing: "trusted", Key: "D:/work/a"}, nil,
			[]string{"result=already_trusted"}},
		{"untrusted kept", clitrust.Result{Existing: "untrusted", Key: "D:/work/a"}, nil,
			[]string{"result=left_as_is", "existing=untrusted", "trust prompt"}},
		{"failed", clitrust.Result{ConfigPath: `C:\cfg\.claude.json`}, errors.New("lock busy"), []string{"result=failed", "reason=lock busy", "trust prompt"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s := newTestServer()
			stub := &folderTrustStub{result: tc.result, err: tc.err}
			s.folderTrustGrant = stub.grant
			boardPath := filepath.Join(t.TempDir(), "board.md")

			env := []string{"CLAUDE_CONFIG_DIR=" + t.TempDir()}
			s.grantChildFolderTrust("tester", "claude", env, `D:\work\a`, boardPath)

			calls := stub.snapshot()
			if len(calls) != 1 {
				t.Fatalf("grant called %d times, want 1", len(calls))
			}
			if got := calls[0].target; got.Provider != "claude" || got.Dir != `D:\work\a` || !reflect.DeepEqual(got.Env, env) {
				t.Errorf("grant target = %+v", got)
			}
			board, err := os.ReadFile(boardPath)
			if err != nil {
				t.Fatal(err)
			}
			for _, want := range append([]string{"folder trust: role=tester provider=claude"}, tc.want...) {
				if !strings.Contains(string(board), want) {
					t.Errorf("board does not contain %q:\n%s", want, board)
				}
			}
		})
	}
}

// folderTrustSpawnServer は、承認された子の起動を wrapper のプロセスを作る直前まで
// 本物の経路で通すための Server を作る。wrapper の実行ファイルは存在しないパスに
// 差し替えるので cmd.Start は必ず失敗し、テストから子のプロセスが立つことは無い。
// Hub 自身の env の CLAUDE_CONFIG_DIR は一時フォルダにし、既定の契約（subEnv が
// 空）でも子がそれを受け取ることを見られるようにする。
func folderTrustSpawnServer(t *testing.T) (*Server, *session, *folderTrustStub, string) {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	hubClaudeDir := t.TempDir()
	t.Setenv(subscription.ClaudeConfigDirEnv, hubClaudeDir)
	t.Setenv(subscription.CodexHomeEnv, "")

	s := newTestServer()
	s.cfg.Orchestration.MaxDepth = 1
	s.cfg.Hub.LogDir = t.TempDir()
	missing := filepath.Join(t.TempDir(), "missing-many-ai-cli-wrapper.exe")
	s.wrapExecutable = func() (string, error) { return missing, nil }
	stub := &folderTrustStub{result: clitrust.Result{Written: true, Key: "k", ConfigPath: "p"}}
	s.folderTrustGrant = stub.grant

	parent := registerTestSession(s, 1, "claude")
	parent.CWD = t.TempDir() // git ではないので worktree は作られず、この cwd がそのまま子の cwd
	return s, parent, stub, hubClaudeDir
}

func boardText(t *testing.T, path string) string {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(data)
}

// 承認の決定に grant_folder_trust: true があると、起動の直前に Grant が 1 回だけ、
// 確定した作業フォルダと子の env で呼ばれる。
func TestApprovedSpawnGrantsFolderTrustOnceRightBeforeStart(t *testing.T) {
	s, parent, stub, hubClaudeDir := folderTrustSpawnServer(t)
	yes := true
	pending, regErr := s.registerSpawnConfirmation(parent, "", spawnChildRequest{Role: "tester", Provider: "claude", RiskConfirmed: true})
	if regErr != nil {
		t.Fatalf("registerSpawnConfirmation: %v", regErr.detail)
	}
	launch := applySpawnConfirmationDecision(pending.Body, spawnConfirmationDecision{GrantFolderTrust: &yes})
	s.resolveSpawnConfirmationDecision(pending, true, launch)

	outcome := <-pending.Outcome
	if outcome.Err == nil || !strings.Contains(outcome.Err.detail, "missing-many-ai-cli-wrapper") {
		t.Fatalf("spawn outcome = %+v, want the start failure of the stubbed wrapper path (the launch must reach cmd.Start)", outcome)
	}
	calls := stub.snapshot()
	if len(calls) != 1 {
		t.Fatalf("grant called %d times, want exactly 1", len(calls))
	}
	target := calls[0].target
	if target.Provider != "claude" || target.Dir != parent.CWD {
		t.Errorf("grant target = provider %q dir %q, want claude / %q", target.Provider, target.Dir, parent.CWD)
	}
	// 既定の契約（subEnv が空）でも、子は Hub 自身の CLAUDE_CONFIG_DIR を受け取る。
	// 信頼はその子が読むファイルへ書かれなければならない。
	if got := subscription.ClaudeStateFileFromEnv(target.Env); got != filepath.Join(hubClaudeDir, ".claude.json") {
		t.Errorf("grant env resolves to %q, want the Hub-inherited %q", got, filepath.Join(hubClaudeDir, ".claude.json"))
	}
}

// grant_folder_trust が無い / false / 対象外の provider では呼ばれない。拒否でも呼ばれない。
func TestSpawnWithoutGrantDecisionNeverWritesTrust(t *testing.T) {
	no := false
	for _, tc := range []struct {
		name     string
		provider string
		decision *bool
		approved bool
	}{
		{"absent", "claude", nil, true},
		{"false", "claude", &no, true},
		{"unsupported provider", "copilot", nil, true},
		{"refused", "claude", nil, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			s, parent, stub, _ := folderTrustSpawnServer(t)
			decision := tc.decision
			if tc.name == "unsupported provider" {
				yes := true
				decision = &yes
			}
			pending, regErr := s.registerSpawnConfirmation(parent, "", spawnChildRequest{Role: "tester", Provider: tc.provider, RiskConfirmed: true})
			if regErr != nil {
				t.Fatalf("registerSpawnConfirmation: %v", regErr.detail)
			}
			launch := applySpawnConfirmationDecision(pending.Body, spawnConfirmationDecision{GrantFolderTrust: decision})
			s.resolveSpawnConfirmationDecision(pending, tc.approved, launch)
			outcome := <-pending.Outcome
			// 承認した場合は、起動が cmd.Start まで進んだうえで呼ばれていないことを見る
			// （手前で失敗して呼ばれないだけなら、このテストは何も確かめていない）。
			if tc.approved && (outcome.Err == nil || !strings.Contains(outcome.Err.detail, "missing-many-ai-cli-wrapper")) {
				t.Fatalf("spawn outcome = %+v, want the launch to reach cmd.Start", outcome)
			}
			if calls := stub.snapshot(); len(calls) != 0 {
				t.Fatalf("grant called %d times, want 0", len(calls))
			}
		})
	}
}

// Grant がエラーを返しても起動は続き（cmd.Start まで進む）、board に失敗の 1 行が残る。
func TestFolderTrustFailureDoesNotStopTheLaunch(t *testing.T) {
	s, parent, stub, _ := folderTrustSpawnServer(t)
	stub.err = errors.New("synthetic lock timeout")
	stub.result = clitrust.Result{ConfigPath: "synthetic.json"}
	yes := true
	pending, regErr := s.registerSpawnConfirmation(parent, "", spawnChildRequest{Role: "tester", Provider: "claude", RiskConfirmed: true})
	if regErr != nil {
		t.Fatalf("registerSpawnConfirmation: %v", regErr.detail)
	}
	launch := applySpawnConfirmationDecision(pending.Body, spawnConfirmationDecision{GrantFolderTrust: &yes})
	s.resolveSpawnConfirmationDecision(pending, true, launch)

	outcome := <-pending.Outcome
	if outcome.Err == nil || !strings.Contains(outcome.Err.detail, "missing-many-ai-cli-wrapper") {
		t.Fatalf("spawn outcome = %+v, want the launch to continue to cmd.Start despite the trust failure", outcome)
	}
	if strings.Contains(outcome.Err.detail, "synthetic lock timeout") {
		t.Fatalf("the trust failure leaked into the spawn error: %s", outcome.Err.detail)
	}
	boardPath := boardPathForParent(t, s, parent.ID)
	if board := boardText(t, boardPath); !strings.Contains(board, "result=failed") || !strings.Contains(board, "synthetic lock timeout") {
		t.Errorf("board does not record the failure:\n%s", board)
	}
}

func boardPathForParent(t *testing.T, s *Server, parentID int) string {
	t.Helper()
	s.sessionsMu.Lock()
	defer s.sessionsMu.Unlock()
	ses := s.sessions[parentID]
	if ses == nil || ses.BoardPath == "" {
		t.Fatal("parent has no board path")
	}
	return ses.BoardPath
}
