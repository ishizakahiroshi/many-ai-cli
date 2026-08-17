package subscription

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

// withRegistry replaces the adapter registry for one test and restores it after.
// Tests need this to prove the "no adapter registered" path keeps working.
func withRegistry(t *testing.T, adapters ...Adapter) {
	t.Helper()
	registryMu.Lock()
	saved := registry
	registry = map[string]Adapter{}
	for _, a := range adapters {
		registry[a.Provider()] = a
	}
	registryMu.Unlock()
	t.Cleanup(func() {
		registryMu.Lock()
		registry = saved
		registryMu.Unlock()
	})
}

type fakeAdapter struct {
	provider  string
	envVar    string
	status    Status
	statusErr error
}

func (f fakeAdapter) Provider() string { return f.provider }
func (f fakeAdapter) EnvVar() string   { return f.envVar }
func (f fakeAdapter) LaunchEnv(dir string) []string {
	if dir == "" {
		return nil
	}
	return []string{f.envVar + "=" + dir}
}
func (f fakeAdapter) LoginArgs() []string { return []string{"login"} }
func (f fakeAdapter) Status(context.Context, string) (Status, error) {
	return f.status, f.statusErr
}

func fakeCfg() *config.Config {
	off := false
	return &config.Config{Subscriptions: config.SubscriptionProfiles{
		"claude": {
			{ID: "main", Name: "Main"},
			{ID: "sub", Name: "Sub"},
			{ID: "old", Name: "Old", Enabled: &off},
		},
	}}
}

// TestResolveEmptyIDIsDefaultLogin は「profile 未指定 = 従来どおり」を保証する。
// ここが nil を返さないと、既存利用者の起動経路に env が足される。
func TestResolveEmptyIDIsDefaultLogin(t *testing.T) {
	withRegistry(t, fakeAdapter{provider: "claude", envVar: "FAKE_DIR"})
	for _, id := range []string{"", "   "} {
		resolved, err := Resolve(fakeCfg(), t.TempDir(), "claude", id)
		if err != nil {
			t.Fatalf("Resolve(%q) = %v, want nil error", id, err)
		}
		if resolved != nil {
			t.Fatalf("Resolve(%q) = %#v, want nil", id, resolved)
		}
	}
}

func TestResolveProducesProviderEnv(t *testing.T) {
	withRegistry(t, fakeAdapter{provider: "claude", envVar: "FAKE_DIR"})
	base := t.TempDir()
	resolved, err := Resolve(fakeCfg(), base, "claude", "MAIN")
	if err != nil {
		t.Fatalf("Resolve: %v", err)
	}
	if resolved.ID != "main" {
		t.Fatalf("ID = %q, want normalized main", resolved.ID)
	}
	wantDir := filepath.Join(base, config.SubscriptionsDirName, "claude", "main")
	if resolved.ProfileDir != wantDir {
		t.Fatalf("ProfileDir = %q, want %q", resolved.ProfileDir, wantDir)
	}
	if len(resolved.Env) != 1 || resolved.Env[0] != "FAKE_DIR="+wantDir {
		t.Fatalf("Env = %v, want one FAKE_DIR entry", resolved.Env)
	}
}

// TestResolveTwoProfilesDoNotShareADirectory は A/B の env が互いに混ざらないことを
// 確認する。この機能で最も重要な性質。
func TestResolveTwoProfilesDoNotShareADirectory(t *testing.T) {
	withRegistry(t, fakeAdapter{provider: "claude", envVar: "FAKE_DIR"})
	base := t.TempDir()
	cfg := fakeCfg()
	a, err := Resolve(cfg, base, "claude", "main")
	if err != nil {
		t.Fatal(err)
	}
	b, err := Resolve(cfg, base, "claude", "sub")
	if err != nil {
		t.Fatal(err)
	}
	if a.ProfileDir == b.ProfileDir {
		t.Fatalf("both profiles resolved to %q", a.ProfileDir)
	}
	// ディレクトリの末尾要素が profile ID そのもので、互いに入れ替わっていないこと。
	// （親パスに "subscriptions" が含まれるので、部分文字列一致では判定できない）
	if filepath.Base(a.ProfileDir) != a.ID || filepath.Base(b.ProfileDir) != b.ID {
		t.Fatalf("profile dirs do not end with their own id: %q / %q", a.ProfileDir, b.ProfileDir)
	}
	if a.Env[0] == b.Env[0] {
		t.Fatalf("profile env leaked across profiles: %v / %v", a.Env, b.Env)
	}
}

func TestResolveRejectsMissingDisabledAndUnsupported(t *testing.T) {
	withRegistry(t, fakeAdapter{provider: "claude", envVar: "FAKE_DIR"})
	base := t.TempDir()
	cfg := fakeCfg()

	if _, err := Resolve(cfg, base, "claude", "nope"); !errors.Is(err, ErrProfileNotFound) {
		t.Fatalf("missing profile error = %v, want ErrProfileNotFound", err)
	}
	if _, err := Resolve(cfg, base, "claude", "old"); !errors.Is(err, ErrProfileDisabled) {
		t.Fatalf("disabled profile error = %v, want ErrProfileDisabled", err)
	}
	if _, err := Resolve(cfg, base, "codex", "main"); !errors.Is(err, ErrProviderUnsupported) {
		t.Fatalf("unregistered provider error = %v, want ErrProviderUnsupported", err)
	}
	if _, err := Resolve(nil, base, "claude", "main"); err == nil {
		t.Fatal("a nil config must not resolve a profile")
	}
}

// TestResolveWithNoAdaptersRegistered は adapter を 1 つも登録していない状態でも
// 「profile 未指定なら従来どおり」が壊れないことを確認する。
func TestResolveWithNoAdaptersRegistered(t *testing.T) {
	withRegistry(t)
	if got := SupportedProviders(); len(got) != 0 {
		t.Fatalf("SupportedProviders() = %v, want empty", got)
	}
	resolved, err := Resolve(fakeCfg(), t.TempDir(), "claude", "")
	if err != nil || resolved != nil {
		t.Fatalf("Resolve with no adapters = (%#v, %v), want (nil, nil)", resolved, err)
	}
	if _, err := Resolve(fakeCfg(), t.TempDir(), "claude", "main"); !errors.Is(err, ErrProviderUnsupported) {
		t.Fatalf("error = %v, want ErrProviderUnsupported", err)
	}
}

func TestListMarksUnsupportedAndBrokenEntries(t *testing.T) {
	withRegistry(t, fakeAdapter{provider: "claude", envVar: "FAKE_DIR"})
	cfg := &config.Config{Subscriptions: config.SubscriptionProfiles{
		"claude":  {{ID: "main"}, {ID: "main"}, {ID: "../escape"}},
		"someday": {{ID: "x"}},
	}}
	entries := List(cfg, t.TempDir())

	byProvider := map[string]ProviderEntry{}
	for _, e := range entries {
		byProvider[e.Provider] = e
	}
	claude, ok := byProvider["claude"]
	if !ok || !claude.Supported {
		t.Fatalf("claude entry = %#v, want supported", claude)
	}
	if claude.EnvVar != "FAKE_DIR" {
		t.Fatalf("EnvVar = %q, want FAKE_DIR", claude.EnvVar)
	}
	if len(claude.Profiles) != 3 {
		t.Fatalf("claude profiles = %d, want 3", len(claude.Profiles))
	}
	if claude.Profiles[0].Issue != "" {
		t.Fatalf("first profile should be usable, got issue %q", claude.Profiles[0].Issue)
	}
	if claude.Profiles[1].Issue == "" {
		t.Fatal("duplicate profile id must be reported as an issue")
	}
	if claude.Profiles[2].Issue == "" {
		t.Fatal("invalid profile id must be reported as an issue")
	}

	someday, ok := byProvider["someday"]
	if !ok {
		t.Fatal("a configured but unregistered provider must still be listed")
	}
	if someday.Supported {
		t.Fatal("an unregistered provider must be marked unsupported")
	}
}

func TestListWithNilConfigDoesNotPanic(t *testing.T) {
	withRegistry(t, fakeAdapter{provider: "claude", envVar: "FAKE_DIR"})
	entries := List(nil, t.TempDir())
	if len(entries) != 1 || entries[0].Provider != "claude" {
		t.Fatalf("List(nil) = %#v", entries)
	}
	if entries[0].Profiles == nil {
		t.Fatal("Profiles must be an empty slice, not nil, so the JSON stays an array")
	}
}

func TestEnsureProfileDirCreatesPrivateDirectory(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "subscriptions", "claude", "main")
	if err := EnsureProfileDir(dir); err != nil {
		t.Fatalf("EnsureProfileDir: %v", err)
	}
	info, err := os.Stat(dir)
	if err != nil {
		t.Fatal(err)
	}
	if !info.IsDir() {
		t.Fatal("EnsureProfileDir did not create a directory")
	}
	if runtime.GOOS != "windows" {
		if perm := info.Mode().Perm(); perm != 0o700 {
			t.Fatalf("profile dir mode = %o, want 0700", perm)
		}
	}
	// 冪等であること（既存ディレクトリで失敗しない）。
	if err := EnsureProfileDir(dir); err != nil {
		t.Fatalf("second EnsureProfileDir: %v", err)
	}
	if err := EnsureProfileDir(""); err == nil {
		t.Fatal("an empty dir must be rejected")
	}
}

func TestMergeEnvReplacesInsteadOfAppending(t *testing.T) {
	base := []string{"A=1", "B=2"}
	got := mergeEnv(base, []string{"B=9", "C=3"})
	want := map[string]string{"A": "1", "B": "9", "C": "3"}
	if len(got) != 3 {
		t.Fatalf("mergeEnv = %v, want 3 entries", got)
	}
	for _, kv := range got {
		parts := strings.SplitN(kv, "=", 2)
		if want[parts[0]] != parts[1] {
			t.Fatalf("mergeEnv produced %q, want %s=%s", kv, parts[0], want[parts[0]])
		}
	}
	if out := mergeEnv(base, nil); len(out) != 2 {
		t.Fatalf("mergeEnv with no overlay changed the base: %v", out)
	}
}

// TestClaudeAdapterContract は Claude adapter の外形を固定する。CLI 側の仕様が
// 変わったときにここが落ちれば、原因が「env 変数名」なのかが即分かる。
func TestClaudeAdapterContract(t *testing.T) {
	a := claudeAdapter{}
	if a.Provider() != "claude" {
		t.Fatalf("Provider() = %q", a.Provider())
	}
	if a.EnvVar() != "CLAUDE_CONFIG_DIR" {
		t.Fatalf("EnvVar() = %q, want CLAUDE_CONFIG_DIR", a.EnvVar())
	}
	if got := a.LaunchEnv(""); got != nil {
		t.Fatalf("LaunchEnv(\"\") = %v, want nil so the child env is untouched", got)
	}
	if got := a.LaunchEnv("/tmp/p"); len(got) != 1 || got[0] != "CLAUDE_CONFIG_DIR=/tmp/p" {
		t.Fatalf("LaunchEnv = %v", got)
	}
	if got := a.LoginArgs(); len(got) != 2 || got[0] != "auth" || got[1] != "login" {
		t.Fatalf("LoginArgs() = %v, want [auth login]", got)
	}
}

// TestClaudeStatusParsesOnlySafeFields は `claude auth status` の JSON から
// アカウント識別情報（email / orgId）を持ち出さないことを確認する。
func TestClaudeStatusParsesOnlySafeFields(t *testing.T) {
	savedLook, savedRun := lookPath, commandOutput
	t.Cleanup(func() { lookPath, commandOutput = savedLook, savedRun })
	lookPath = func(string) (string, error) { return "claude", nil }

	commandOutput = func(context.Context, string, []string, []string) (string, int, error) {
		return `{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"someone@example.com","orgId":"org-1","subscriptionType":"max"}`, 0, nil
	}
	status, err := claudeAdapter{}.Status(context.Background(), "/tmp/p")
	if err != nil {
		t.Fatal(err)
	}
	if !status.LoggedIn || status.Plan != "max" || status.Method != "claude.ai" {
		t.Fatalf("status = %#v", status)
	}
	blob := status.Plan + status.Method
	if strings.Contains(blob, "@") || strings.Contains(blob, "org-") {
		t.Fatalf("status leaked account identity: %#v", status)
	}

	commandOutput = func(context.Context, string, []string, []string) (string, int, error) {
		return `{"loggedIn":false,"authMethod":"none","apiProvider":"firstParty"}`, 1, nil
	}
	status, err = claudeAdapter{}.Status(context.Background(), "/tmp/p")
	if err != nil {
		t.Fatal(err)
	}
	if status.LoggedIn {
		t.Fatalf("status = %#v, want signed out", status)
	}

	// 出力が JSON でないときは、生出力をエラーに載せない。
	commandOutput = func(context.Context, string, []string, []string) (string, int, error) {
		return "Signed in as someone@example.com", 0, nil
	}
	if _, err := (claudeAdapter{}).Status(context.Background(), "/tmp/p"); err == nil {
		t.Fatal("unparseable output must be an error")
	} else if strings.Contains(err.Error(), "@") {
		t.Fatalf("error message leaked the raw output: %v", err)
	}
}

func TestClaudeStatusPassesProfileEnv(t *testing.T) {
	savedLook, savedRun := lookPath, commandOutput
	t.Cleanup(func() { lookPath, commandOutput = savedLook, savedRun })
	lookPath = func(string) (string, error) { return "claude", nil }

	var gotEnv []string
	commandOutput = func(_ context.Context, _ string, _ []string, env []string) (string, int, error) {
		gotEnv = env
		return `{"loggedIn":false}`, 1, nil
	}
	if _, err := (claudeAdapter{}).Status(context.Background(), filepath.Join("x", "profile-a")); err != nil {
		t.Fatal(err)
	}
	want := "CLAUDE_CONFIG_DIR=" + filepath.Join("x", "profile-a")
	found := false
	for _, kv := range gotEnv {
		if kv == want {
			found = true
		}
	}
	if !found {
		t.Fatalf("status command env did not contain %q", want)
	}
}

// TestCodexAdapterContract は Codex adapter の外形を固定する。
func TestCodexAdapterContract(t *testing.T) {
	a := codexAdapter{}
	if a.Provider() != "codex" {
		t.Fatalf("Provider() = %q", a.Provider())
	}
	if a.EnvVar() != "CODEX_HOME" {
		t.Fatalf("EnvVar() = %q, want CODEX_HOME", a.EnvVar())
	}
	if got := a.LaunchEnv(""); got != nil {
		t.Fatalf("LaunchEnv(\"\") = %v, want nil so the child env is untouched", got)
	}
	if got := a.LaunchEnv("/tmp/p"); len(got) != 1 || got[0] != "CODEX_HOME=/tmp/p" {
		t.Fatalf("LaunchEnv = %v", got)
	}
	if got := a.LoginArgs(); len(got) != 1 || got[0] != "login" {
		t.Fatalf("LoginArgs() = %v, want [login]", got)
	}
}

// TestParseCodexLoginStatus は実機で観測した出力形をそのまま固定する。
// 「認証ファイルを読まずに公式 CLI の答えだけを使う」判断の中身。
func TestParseCodexLoginStatus(t *testing.T) {
	cases := []struct {
		name     string
		out      string
		exitCode int
		want     Status
	}{
		{"chatgpt", "Logged in using ChatGPT\n", 0, Status{LoggedIn: true, Method: "chatgpt"}},
		{"api key", "Logged in using an API key\n", 0, Status{LoggedIn: true, Method: "api-key"}},
		{"signed out", "Not logged in\n", 1, Status{LoggedIn: false}},
		{"unexpected non-zero", "", 1, Status{LoggedIn: false}},
		{"unknown method", "Logged in\n", 0, Status{LoggedIn: true}},
	}
	for _, tc := range cases {
		got := parseCodexLoginStatus(tc.out, tc.exitCode)
		if got != tc.want {
			t.Errorf("%s: parseCodexLoginStatus = %#v, want %#v", tc.name, got, tc.want)
		}
	}
}

// TestCodexStatusDoesNotLeakAccountLine は、公式 CLI がアカウント行を出しても
// Status がそれを持ち出さないことを確認する。
func TestCodexStatusDoesNotLeakAccountLine(t *testing.T) {
	savedLook, savedRun := lookPath, commandOutput
	t.Cleanup(func() { lookPath, commandOutput = savedLook, savedRun })
	lookPath = func(string) (string, error) { return "codex", nil }
	commandOutput = func(context.Context, string, []string, []string) (string, int, error) {
		return "Logged in using ChatGPT (someone@example.com)\n", 0, nil
	}
	status, err := (codexAdapter{}).Status(context.Background(), "/tmp/p")
	if err != nil {
		t.Fatal(err)
	}
	if !status.LoggedIn || status.Method != "chatgpt" {
		t.Fatalf("status = %#v", status)
	}
	if strings.Contains(status.Method+status.Plan, "@") {
		t.Fatalf("status leaked the account line: %#v", status)
	}
}

func TestGrokAdapterContract(t *testing.T) {
	a := grokAdapter{}
	if a.Provider() != "grok" || a.EnvVar() != "GROK_HOME" {
		t.Fatalf("adapter = %q / %q", a.Provider(), a.EnvVar())
	}
	if got := a.LaunchEnv(""); got != nil {
		t.Fatalf("LaunchEnv(\"\") = %v, want nil", got)
	}
	if got := a.LoginArgs(); len(got) != 1 || got[0] != "login" {
		t.Fatalf("LoginArgs() = %v", got)
	}
}

// TestParseGrokModelsStatus は実機で観測した 2 通りの出力を固定する。
// grok は status サブコマンドを持たず終了コードも常に 0 なので、判定は本文だけが頼り。
func TestParseGrokModelsStatus(t *testing.T) {
	cases := []struct {
		name string
		out  string
		want Status
	}{
		{"signed in", "You are logged in with grok.com.\n\nDefault model: grok-4.6\n", Status{LoggedIn: true, Method: "grok.com"}},
		{"signed out", "You are not authenticated.\n\nDefault model: grok-4.6\n", Status{LoggedIn: false}},
		{"unknown wording", "Default model: grok-4.6\n", Status{LoggedIn: false}},
	}
	for _, tc := range cases {
		if got := parseGrokModelsStatus(tc.out); got != tc.want {
			t.Errorf("%s: parseGrokModelsStatus = %#v, want %#v", tc.name, got, tc.want)
		}
	}
}

func TestOpenCodeAdapterContract(t *testing.T) {
	a := openCodeAdapter{}
	if a.Provider() != "opencode" || a.EnvVar() != "XDG_DATA_HOME" {
		t.Fatalf("adapter = %q / %q", a.Provider(), a.EnvVar())
	}
	if got := a.LaunchEnv(""); got != nil {
		t.Fatalf("LaunchEnv(\"\") = %v, want nil", got)
	}
	if got := a.LaunchEnv("/tmp/p"); len(got) != 1 || got[0] != "XDG_DATA_HOME=/tmp/p" {
		t.Fatalf("LaunchEnv = %v", got)
	}
	if got := a.LoginArgs(); len(got) != 2 || got[0] != "providers" || got[1] != "login" {
		t.Fatalf("LoginArgs() = %v", got)
	}
}

func TestParseOpenCodeProvidersList(t *testing.T) {
	cases := []struct {
		name string
		out  string
		want Status
	}{
		{
			"go subscription",
			"┌  Credentials ~\\.local\\share\\opencode\\auth.json\n│\n●  OpenAI oauth\n●  OpenCode Go api\n└  3 credentials\n",
			Status{LoggedIn: true, Plan: "go"},
		},
		{
			"other credentials only",
			"┌  Credentials path\n●  Google api\n└  1 credential\n",
			Status{LoggedIn: true},
		},
		{"empty profile", "┌  Credentials path\n└  0 credentials\n", Status{LoggedIn: false}},
		{"unexpected output", "something went wrong\n", Status{LoggedIn: false}},
	}
	for _, tc := range cases {
		if got := parseOpenCodeProvidersList(tc.out); got != tc.want {
			t.Errorf("%s: parseOpenCodeProvidersList = %#v, want %#v", tc.name, got, tc.want)
		}
	}
}

// TestUnsupportedProvidersStayUnregistered は、調査で profile 分離が安全に
// できないと判定した provider を実装していないことを固定する（C7 の結論）。
// 実装したくなったときにここが落ちるので、判断を読み直す入口になる。
func TestUnsupportedProvidersStayUnregistered(t *testing.T) {
	for _, provider := range []string{"copilot", "cursor-agent"} {
		if _, ok := AdapterFor(provider); ok {
			t.Fatalf("%s has an adapter; see plan_multi-subscription-pool_c7_other-providers.md "+
				"for why it was recorded as unsupported before adding one", provider)
		}
	}
	for _, provider := range []string{"claude", "codex", "grok", "opencode"} {
		if _, ok := AdapterFor(provider); !ok {
			t.Fatalf("%s should have an adapter", provider)
		}
	}
}

func TestRunVendorCLIReportsMissingBinary(t *testing.T) {
	savedLook := lookPath
	t.Cleanup(func() { lookPath = savedLook })
	lookPath = func(string) (string, error) { return "", errors.New("not found") }
	if _, _, err := runVendorCLI(context.Background(), "nope", nil, nil); !errors.Is(err, ErrCLINotFound) {
		t.Fatalf("err = %v, want ErrCLINotFound", err)
	}
}
