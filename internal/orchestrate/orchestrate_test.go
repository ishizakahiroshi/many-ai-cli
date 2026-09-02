package orchestrate

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestParseRelayProviderModel(t *testing.T) {
	provider, model, err := parseRelayProviderModel(" claude/opus ")
	if err != nil || provider != "claude" || model != "opus" {
		t.Fatalf("parsed provider=%q model=%q err=%v", provider, model, err)
	}
	provider, model, err = parseRelayProviderModel("codex")
	if err != nil || provider != "codex" || model != "" {
		t.Fatalf("provider-only parse = %q/%q err=%v", provider, model, err)
	}
}

func TestRunReportsAvailableSubcommands(t *testing.T) {
	if err := Run(nil); err == nil || !strings.Contains(err.Error(), "spawn|send|relay") {
		t.Fatalf("Run(nil) error = %v, want available subcommands", err)
	}

	err := Run([]string{"relayy"})
	if err == nil || !strings.Contains(err.Error(), "unknown subcommand") || !strings.Contains(err.Error(), "spawn|send|relay") {
		t.Fatalf("Run(relayy) error = %v, want unknown subcommand and available subcommands", err)
	}
}

func TestRunRelayRequiresPlan(t *testing.T) {
	if err := runRelay(nil); err == nil || !strings.Contains(err.Error(), "--plan is required") {
		t.Fatalf("runRelay error = %v, want missing plan", err)
	}
}

// C1 (plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md): 症状Aの調査で
// 「CLI が要求値を捨てている」経路を除外するため、-provider / -cwd が JSON body の
// provider / cwd にそのまま載ることを固定する。
func TestRunSpawnSendsExplicitProviderAndCWD(t *testing.T) {
	var got spawnChildRequest
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.URL.Path != "/api/sessions/9/spawn-child" {
			t.Fatalf("request = %s %s", r.Method, r.URL.Path)
		}
		if r.Header.Get("Authorization") != "Bearer spawn-secret" {
			t.Fatalf("authorization header = %q", r.Header.Get("Authorization"))
		}
		if err := json.NewDecoder(r.Body).Decode(&got); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true,"session_id":12,"board_path":"board.md","cwd":"/tmp/child"}`))
	}))
	defer server.Close()
	port := server.Listener.Addr().String()
	port = port[strings.LastIndex(port, ":")+1:]
	t.Setenv(hubPortEnv, port)
	t.Setenv(sessionIDEnv, "9")
	t.Setenv(hubTokenEnv, "spawn-secret")

	err := runSpawn([]string{
		"--role", "review",
		"--provider", "claude",
		"--cwd", "/tmp/requested",
		"prompt text",
	})
	if err != nil {
		t.Fatalf("runSpawn: %v", err)
	}
	if got.Role != "review" || got.Provider != "claude" || got.CWD != "/tmp/requested" || got.InitialPrompt != "prompt text" {
		t.Fatalf("spawn request = %+v", got)
	}
	if got.SameTree != nil {
		t.Fatalf("same_tree should be omitted without -same-tree, got %+v", got.SameTree)
	}
}

// C3 (plan_spawn-orchestration-backlog-closeout_c2_spawn-args.md): -same-tree は
// tri-state の same_tree へ true を載せる（未指定時は nil のまま省略される）。
func TestRunSpawnSendsSameTree(t *testing.T) {
	var got spawnChildRequest
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if err := json.NewDecoder(r.Body).Decode(&got); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true,"session_id":13,"board_path":"board.md","cwd":"/tmp/requested"}`))
	}))
	defer server.Close()
	port := server.Listener.Addr().String()
	port = port[strings.LastIndex(port, ":")+1:]
	t.Setenv(hubPortEnv, port)
	t.Setenv(sessionIDEnv, "9")
	t.Setenv(hubTokenEnv, "spawn-secret")

	err := runSpawn([]string{
		"--role", "review",
		"--cwd", "/tmp/requested",
		"--same-tree",
		"prompt text",
	})
	if err != nil {
		t.Fatalf("runSpawn: %v", err)
	}
	if got.SameTree == nil || !*got.SameTree {
		t.Fatalf("same_tree = %+v, want true", got.SameTree)
	}
}

func TestRunRelayBuildsSameTreeRequest(t *testing.T) {
	var got relayStartRequest
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost || r.URL.Path != "/api/sessions/9/relay" {
			t.Fatalf("request = %s %s", r.Method, r.URL.Path)
		}
		if r.Header.Get("Authorization") != "Bearer relay-secret" {
			t.Fatalf("authorization header = %q", r.Header.Get("Authorization"))
		}
		if err := json.NewDecoder(r.Body).Decode(&got); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true,"relay":{"orchestration_id":"r9-test","mode":"same-tree","max_rounds":3,"implementation_session_id":12}}`))
	}))
	defer server.Close()
	port := server.Listener.Addr().String()
	port = port[strings.LastIndex(port, ":")+1:]
	t.Setenv(hubPortEnv, port)
	t.Setenv(sessionIDEnv, "9")
	t.Setenv(hubTokenEnv, "relay-secret")

	err := runRelay([]string{
		"--plan", "docs/plan.md",
		"--same-tree",
		"--impl", "claude/opus",
		"--review", "codex/gpt-5",
		"--strong", "claude/opus-strong",
	})
	if err != nil {
		t.Fatalf("runRelay: %v", err)
	}
	if got.PlanPath != "docs/plan.md" || got.Mode != "same-tree" || got.Roles["implementation"].Provider != "claude" || got.Roles["review"].Model != "gpt-5" || got.Roles["implementation-strong"].Model != "opus-strong" {
		t.Fatalf("relay request = %+v", got)
	}
}
