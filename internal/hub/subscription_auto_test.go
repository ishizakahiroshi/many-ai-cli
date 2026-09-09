package hub

import (
	"go/ast"
	"go/parser"
	gotoken "go/token"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

func autoTestServer(t *testing.T) *Server {
	t.Helper()
	s, _ := subsTestServer(t)
	off := false
	s.cfg.Subscriptions = config.SubscriptionProfiles{
		"claude": {
			{ID: "a", Name: "A"},
			{ID: "b", Name: "B"},
			{ID: "off", Name: "Off", Enabled: &off},
			{ID: "../broken"},
		},
	}
	return s
}

// TestAutoSubscriptionRoundRobin は auto が有効 profile を順番に使い、
// 同じ profile へ偏らないことを確認する。
func TestAutoSubscriptionRoundRobin(t *testing.T) {
	s := autoTestServer(t)
	seen := make([]string, 0, 4)
	for i := 0; i < 4; i++ {
		_, resolved, err := s.subscriptionLaunch("claude", "auto")
		if err != nil {
			t.Fatalf("auto launch #%d: %v", i, err)
		}
		seen = append(seen, resolved.ID)
	}
	if strings.Join(seen, ",") != "a,b,a,b" {
		t.Fatalf("auto picks = %v, want a,b,a,b", seen)
	}
}

// TestAutoSubscriptionRecordsConcreteProfile は auto で起動したセッションに
// 「auto」ではなく実際に選ばれた profile ID が残ることを確認する。
func TestAutoSubscriptionRecordsConcreteProfile(t *testing.T) {
	s := autoTestServer(t)
	env, resolved, err := s.subscriptionLaunch("claude", "auto")
	if err != nil {
		t.Fatal(err)
	}
	if resolved.ID == "auto" || resolved.ID == "" {
		t.Fatalf("resolved id = %q, want a concrete profile id", resolved.ID)
	}
	joined := strings.Join(env, "\n")
	if !strings.Contains(joined, subscription.SessionEnvVar+"="+resolved.ID) {
		t.Fatalf("env %v does not carry the concrete profile id", env)
	}
	if strings.Contains(joined, subscription.SessionEnvVar+"=auto") {
		t.Fatalf("env %v records the keyword instead of the chosen profile", env)
	}
}

// TestAutoSubscriptionSkipsDisabledAndBroken は無効化・設定不正の profile が
// auto の候補に入らないことを確認する。
func TestAutoSubscriptionSkipsDisabledAndBroken(t *testing.T) {
	s := autoTestServer(t)
	for i := 0; i < 6; i++ {
		_, resolved, err := s.subscriptionLaunch("claude", "auto")
		if err != nil {
			t.Fatal(err)
		}
		if resolved.ID == "off" || strings.Contains(resolved.ID, "..") {
			t.Fatalf("auto picked an ineligible profile: %q", resolved.ID)
		}
	}
}

// TestAutoSubscriptionWithoutCandidatesFails は候補 0 件のときに既定ログインへ
// 黙って落ちないことを確認する。auto を選んだ利用者は「登録した契約のどれかで
// 動く」ことを期待しているので、0 件は設定の問題として見せる。
func TestAutoSubscriptionWithoutCandidatesFails(t *testing.T) {
	s, _ := subsTestServer(t)
	if _, _, err := s.subscriptionLaunch("claude", "auto"); err == nil {
		t.Fatal("auto with no registered profile must fail")
	}
	off := false
	s.cfg.Subscriptions = config.SubscriptionProfiles{"claude": {{ID: "only", Enabled: &off}}}
	if _, _, err := s.subscriptionLaunch("claude", "auto"); err == nil {
		t.Fatal("auto with only disabled profiles must fail")
	}
}

// TestAutoIsReservedAndCannotBeRegistered は "auto" という名前の profile を
// 作れないことを確認する。作れてしまうと、その profile を明示指定できなくなる。
func TestAutoIsReservedAndCannotBeRegistered(t *testing.T) {
	s, _ := subsTestServer(t)
	w := httptest.NewRecorder()
	s.handleSubscriptions(w, subsRequest(t, http.MethodPost, "/api/subscriptions", map[string]any{
		"provider": "claude", "id": "auto", "name": "Auto",
	}))
	if w.Code != http.StatusBadRequest {
		t.Fatalf("code = %d, want 400: %s", w.Code, w.Body.String())
	}
	if len(s.cfg.Subscriptions["claude"]) != 0 {
		t.Fatalf("a reserved id was registered: %#v", s.cfg.Subscriptions)
	}
}

// TestLiveSessionAuthIsNeverSwapped は「実行中セッションの認証を途中で差し替えない」
// という親 plan の禁止事項を、コード上の不変条件として固定する。
//
// 振る舞いテストでは「差し替える経路が無いこと」を示せない（無いものは呼べない）。
// そこで代入箇所そのものを検査し、register / reattach 以外から書かれていないことを見る。
func TestLiveSessionAuthIsNeverSwapped(t *testing.T) {
	entries, err := os.ReadDir(".")
	if err != nil {
		t.Fatal(err)
	}
	const field = "SubscriptionProfileID"
	allowed := map[string]bool{"wrapper_loop.go": true}
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || filepath.Ext(name) != ".go" || strings.HasSuffix(name, "_test.go") {
			continue
		}
		body, err := os.ReadFile(name)
		if err != nil {
			t.Fatal(err)
		}
		for _, line := range strings.Split(string(body), "\n") {
			trimmed := strings.TrimSpace(line)
			// 構造体リテラルのフィールド指定（`Field: value,`）は生成時なので対象外。
			if !strings.Contains(trimmed, field+" =") && !strings.Contains(trimmed, field+"=") {
				continue
			}
			if allowed[name] {
				continue
			}
			t.Errorf("%s assigns %s outside session creation: %s", name, field, trimmed)
		}
	}
}

// usageIdentifierFragments は「残量を見て選んでいる」ことを示す識別子の断片。
//
// pickAutoSubscription の中にこれらが現れたら、auto 選択が残量に依存し始めた
// ということである。subscription.go の pickAutoSubscription 冒頭に置いた設計
// 不変条件（正本）の、機械的な裏づけ。
var usageIdentifierFragments = []string{
	"usage",
	"quota",
	"remaining",
	"percent",
	"resets",
	"credit",
	"limit",
	"exhaust",
}

// TestAutoSubscriptionNeverConsultsUsage は auto 選択が残量を参照していない
// ことをソース走査で固定する。
//
// 残量が取れなかった頃は「見ていない」が自明だったが、subscription_usage.go が
// 入って Claude / Codex / Grok の残量が同じパッケージの中に存在するようになった。
// 値が手元にある以上、「上限に当たった profile を飛ばす」「残量の多い方を選ぶ」は
// 数行で書けてしまい、しかも一見すると親切な改善に見える。
//
// それは各ベンダーが広く禁じている「レート制限・保護措置の回避」の自動化であり、
// 本ツールが越えないと決めた線である。契約を切り替えるのは利用者の操作であって、
// Hub の判断ではない。上の RoundRobin / SkipsDisabled 群が「順送りである」ことを
// 押さえるのに対し、こちらは「順送り以外の根拠を持ち込んでいない」ことを押さえる。
//
// このテストが落ちたときの正しい対処は、断片一覧から語を削って通すことではない。
// 入れようとしている変更が「Hub が残量を見て乗り換える」に当たらないかを先に
// 確かめること。当たるなら入れない。
func TestAutoSubscriptionNeverConsultsUsage(t *testing.T) {
	fset := gotoken.NewFileSet()
	file, err := parser.ParseFile(fset, "subscription.go", nil, 0)
	if err != nil {
		t.Fatalf("subscription.go を解析できなかった: %v", err)
	}

	var body *ast.BlockStmt
	ast.Inspect(file, func(n ast.Node) bool {
		fn, ok := n.(*ast.FuncDecl)
		if !ok || fn.Name == nil || fn.Name.Name != "pickAutoSubscription" {
			return true
		}
		body = fn.Body
		return false
	})
	if body == nil {
		t.Fatal("pickAutoSubscription を見つけられなかった。走査条件が実装とずれている")
	}

	ast.Inspect(body, func(n ast.Node) bool {
		ident, ok := n.(*ast.Ident)
		if !ok {
			return true
		}
		lower := strings.ToLower(ident.Name)
		for _, fragment := range usageIdentifierFragments {
			if strings.Contains(lower, fragment) {
				t.Errorf("pickAutoSubscription が %q を参照している（%s）。auto 選択は残量を見ない",
					ident.Name, fset.Position(ident.Pos()))
			}
		}
		return true
	})
}
