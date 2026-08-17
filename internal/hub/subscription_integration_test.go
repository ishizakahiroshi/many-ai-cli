package hub

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	"sync"
	"testing"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/subscription"
)

// TestSubscriptionFakeProviderHelper は「公式 CLI の代わりに起動される偽プロセス」。
// 実際の claude / codex を呼ばずに、子プロセスが受け取った env を親へ報告する。
// 通常のテスト実行では即 return し、親テストから env 付きで起動されたときだけ働く。
func TestSubscriptionFakeProviderHelper(t *testing.T) {
	if os.Getenv("MANY_AI_CLI_FAKE_PROVIDER") != "1" {
		return
	}
	for _, key := range []string{subscription.ClaudeConfigDirEnv, subscription.SessionEnvVar, "MANY_AI_CLI"} {
		fmt.Printf("%s=%s\n", key, os.Getenv(key))
	}
	// テストフレームワークの PASS 出力より前に抜ける（親は行単位で読む）。
	os.Exit(0)
}

// spawnEnvFor は handleSpawn と同じ手順で子プロセス env を組み立てる。
// 実プロセス起動の代わりに、この env を偽 CLI へ渡して観測する。
func spawnEnvFor(t *testing.T, s *Server, provider, profileID string) []string {
	t.Helper()
	subEnv, _, err := s.subscriptionLaunch(provider, profileID)
	if err != nil {
		t.Fatalf("subscriptionLaunch(%q): %v", profileID, err)
	}
	env := append(sanitizeEnv(os.Environ()), "MANY_AI_CLI=1", "MANY_AI_CLI_HUB_PORT=47777")
	if len(subEnv) > 0 {
		env = mergeEnvOverrides(env, subEnv)
	}
	return env
}

// runFakeProvider は偽 CLI を起動し、そのプロセスが観測した env を返す。
func runFakeProvider(t *testing.T, env []string) map[string]string {
	t.Helper()
	cmd := exec.Command(os.Args[0], "-test.run=^TestSubscriptionFakeProviderHelper$")
	cmd.Env = append(append([]string(nil), env...), "MANY_AI_CLI_FAKE_PROVIDER=1")
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("fake provider failed: %v (output %q)", err, string(out))
	}
	got := map[string]string{}
	for _, line := range strings.Split(strings.ReplaceAll(string(out), "\r\n", "\n"), "\n") {
		if line == "" {
			continue
		}
		key, value, ok := strings.Cut(line, "=")
		if !ok {
			continue
		}
		got[key] = value
	}
	return got
}

// TestFakeProviderReceivesProfileEnv は「起動した」だけでなく、子プロセスが
// 実際に受け取った env を確認する。
func TestFakeProviderReceivesProfileEnv(t *testing.T) {
	s, _ := subsTestServer(t)
	s.cfg.Subscriptions = config.SubscriptionProfiles{
		"claude": {{ID: "main", Name: "Main"}, {ID: "sub", Name: "Sub"}},
	}
	_, a, err := s.subscriptionLaunch("claude", "main")
	if err != nil {
		t.Fatal(err)
	}
	got := runFakeProvider(t, spawnEnvFor(t, s, "claude", "main"))
	if got[subscription.ClaudeConfigDirEnv] != a.ProfileDir {
		t.Fatalf("child saw %s=%q, want %q", subscription.ClaudeConfigDirEnv, got[subscription.ClaudeConfigDirEnv], a.ProfileDir)
	}
	if got[subscription.SessionEnvVar] != "main" {
		t.Fatalf("child saw %s=%q, want main", subscription.SessionEnvVar, got[subscription.SessionEnvVar])
	}
}

// TestFakeProviderWithoutProfileKeepsInheritedEnv は profile を指定しない起動で
// CLAUDE_CONFIG_DIR が親から継承されたまま（＝従来どおり）であることを確認する。
func TestFakeProviderWithoutProfileKeepsInheritedEnv(t *testing.T) {
	s, _ := subsTestServer(t)
	t.Setenv(subscription.ClaudeConfigDirEnv, "inherited-value")
	got := runFakeProvider(t, spawnEnvFor(t, s, "claude", ""))
	if got[subscription.ClaudeConfigDirEnv] != "inherited-value" {
		t.Fatalf("child saw %s=%q, want the inherited value", subscription.ClaudeConfigDirEnv, got[subscription.ClaudeConfigDirEnv])
	}
	if got[subscription.SessionEnvVar] != "" {
		t.Fatalf("child saw a subscription id (%q) for a default-login spawn", got[subscription.SessionEnvVar])
	}
}

// TestFakeProviderConcurrentSpawnDoesNotLeakEnv は A/B を同時起動しても
// 互いの profile ディレクトリが混ざらないことを、実プロセスで確認する。
// この機能で最も重要な性質のうち、実 CLI を使わずに検証できる部分。
func TestFakeProviderConcurrentSpawnDoesNotLeakEnv(t *testing.T) {
	s, _ := subsTestServer(t)
	s.cfg.Subscriptions = config.SubscriptionProfiles{
		"claude": {{ID: "main", Name: "Main"}, {ID: "sub", Name: "Sub"}},
	}
	_, a, err := s.subscriptionLaunch("claude", "main")
	if err != nil {
		t.Fatal(err)
	}
	_, b, err := s.subscriptionLaunch("claude", "sub")
	if err != nil {
		t.Fatal(err)
	}
	envA := spawnEnvFor(t, s, "claude", "main")
	envB := spawnEnvFor(t, s, "claude", "sub")

	var wg sync.WaitGroup
	results := make([]map[string]string, 2)
	for i, env := range [][]string{envA, envB} {
		wg.Add(1)
		go func(idx int, e []string) {
			defer wg.Done()
			results[idx] = runFakeProvider(t, e)
		}(i, env)
	}
	wg.Wait()

	if results[0][subscription.ClaudeConfigDirEnv] != a.ProfileDir {
		t.Fatalf("session A saw %q, want %q", results[0][subscription.ClaudeConfigDirEnv], a.ProfileDir)
	}
	if results[1][subscription.ClaudeConfigDirEnv] != b.ProfileDir {
		t.Fatalf("session B saw %q, want %q", results[1][subscription.ClaudeConfigDirEnv], b.ProfileDir)
	}
	if results[0][subscription.SessionEnvVar] != "main" || results[1][subscription.SessionEnvVar] != "sub" {
		t.Fatalf("subscription ids crossed over: %q / %q",
			results[0][subscription.SessionEnvVar], results[1][subscription.SessionEnvVar])
	}
}
