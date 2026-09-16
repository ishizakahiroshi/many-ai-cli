package orchestrate

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestParseRelayProviderModel(t *testing.T) {
	provider, model, effort, err := parseRelayProviderModel(" claude/opus ")
	if err != nil || provider != "claude" || model != "opus" || effort != "" {
		t.Fatalf("parsed provider=%q model=%q effort=%q err=%v", provider, model, effort, err)
	}
	provider, model, effort, err = parseRelayProviderModel("codex")
	if err != nil || provider != "codex" || model != "" || effort != "" {
		t.Fatalf("provider-only parse = %q/%q@%q err=%v", provider, model, effort, err)
	}
}

// provider[/model][@effort] の 3 形（@ 無し・model 無し・effort 無し）を固定する
// （子 plan: docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C3）。
// "@" が無い値は effort が導入される前とまったく同じに解釈される。
func TestParseRelayProviderModelWithEffort(t *testing.T) {
	cases := []struct {
		raw, provider, model, effort string
	}{
		{"claude/opus@high", "claude", "opus", "high"},
		{"claude@high", "claude", "", "high"},
		{" codex/gpt-5.5@minimal ", "codex", "gpt-5.5", "minimal"},
		{"claude/opus", "claude", "opus", ""},
		{"claude", "claude", "", ""},
	}
	for _, tc := range cases {
		provider, model, effort, err := parseRelayProviderModel(tc.raw)
		if err != nil {
			t.Fatalf("parseRelayProviderModel(%q) error = %v", tc.raw, err)
		}
		if provider != tc.provider || model != tc.model || effort != tc.effort {
			t.Fatalf("parseRelayProviderModel(%q) = %q/%q@%q, want %q/%q@%q",
				tc.raw, provider, model, effort, tc.provider, tc.model, tc.effort)
		}
	}
	for _, raw := range []string{"", "@high", "claude/opus@", "claude/opus@hi gh"} {
		if _, _, _, err := parseRelayProviderModel(raw); err == nil {
			t.Fatalf("parseRelayProviderModel(%q) = nil error, want an error", raw)
		}
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
	wantPlan, err := filepath.Abs(filepath.Join("docs", "plan.md"))
	if err != nil {
		t.Fatal(err)
	}
	if got.PlanPath != wantPlan || got.Mode != "same-tree" || got.Roles["implementation"].Provider != "claude" || got.Roles["review"].Model != "gpt-5" || got.Roles["implementation-strong"].Model != "opus-strong" {
		t.Fatalf("relay request = %+v", got)
	}
	if !got.AcknowledgeChildFullBypass {
		t.Fatalf("acknowledge_child_full_bypass = false, want true (CLI start is the ack)")
	}
}

func TestRunRelayStatusAndStop(t *testing.T) {
	var methods []string
	var stopped relayControlRequest
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer relay-secret" {
			t.Fatalf("authorization header = %q", r.Header.Get("Authorization"))
		}
		methods = append(methods, r.Method+" "+r.URL.Path)
		w.Header().Set("Content-Type", "application/json")
		switch {
		case r.Method == http.MethodGet && r.URL.Path == "/api/sessions/9/relay":
			_, _ = w.Write([]byte(`{"ok":true,"relays":[{"relay":{"orchestration_id":"r9-test","state":"reviewing","completed_cs":2,"round":1,"max_rounds":3,"branch":"many-ai-cli/relay/r9-test"}}]}`))
		case r.Method == http.MethodPost && r.URL.Path == "/api/sessions/9/relay-stop":
			if err := json.NewDecoder(r.Body).Decode(&stopped); err != nil {
				t.Fatalf("decode stop request: %v", err)
			}
			_, _ = w.Write([]byte(`{"ok":true,"relay":{"orchestration_id":"r9-test","state":"stopped","completed_cs":2,"round":1,"max_rounds":3,"branch":"many-ai-cli/relay/r9-test"}}`))
		default:
			http.NotFound(w, r)
		}
	}))
	defer server.Close()
	port := server.Listener.Addr().String()
	port = port[strings.LastIndex(port, ":")+1:]
	t.Setenv(hubPortEnv, port)
	t.Setenv(sessionIDEnv, "9")
	t.Setenv(hubTokenEnv, "relay-secret")

	if err := runRelay([]string{"status", "--id", "r9-test"}); err != nil {
		t.Fatalf("relay status: %v", err)
	}
	if err := runRelay([]string{"stop", "--id", "r9-test"}); err != nil {
		t.Fatalf("relay stop: %v", err)
	}
	if stopped.OrchestrationID != "r9-test" {
		t.Fatalf("stop request = %+v", stopped)
	}
	want := []string{"GET /api/sessions/9/relay", "POST /api/sessions/9/relay-stop"}
	if len(methods) != len(want) || methods[0] != want[0] || methods[1] != want[1] {
		t.Fatalf("requests = %v, want %v", methods, want)
	}
}

func TestPostChildAPISpawnTimeoutPendingMessage(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		// Simulate a long wait that exceeds client timeout.
		select {
		case <-r.Context().Done():
		case <-time.After(500 * time.Millisecond):
		}
	}))
	defer server.Close()

	port := server.Listener.Addr().String()
	port = port[strings.LastIndex(port, ":")+1:]
	t.Setenv(hubPortEnv, port)
	t.Setenv(sessionIDEnv, "9")
	t.Setenv(hubTokenEnv, "spawn-secret")
	t.Setenv(spawnTimeoutEnv, "50ms")

	err := runSpawn([]string{
		"--role", "implementation",
		"prompt text",
	})
	if err == nil {
		t.Fatal("expected timeout error, got nil")
	}
	msg := err.Error()
	if !strings.Contains(msg, "spawn confirmation pending in Hub") {
		t.Errorf("error message missing pending notice: %s", msg)
	}
	if !strings.Contains(msg, "DO NOT retry spawn") {
		t.Errorf("error message missing DO NOT retry directive: %s", msg)
	}
	if !strings.Contains(msg, "delivered via orchestration notification") {
		t.Errorf("error message missing notification delivery guidance: %s", msg)
	}
}

// 起動要求の共通 3 項目が `orchestrate spawn` の body に載ることと、付けなかった
// ときの body が従来と完全に一致すること（子 plan:
// docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C3）。
func TestRunSpawnSendsLaunchOptions(t *testing.T) {
	var raw map[string]any
	var got spawnChildRequest
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Fatalf("read request: %v", err)
		}
		if err := json.Unmarshal(body, &raw); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		if err := json.Unmarshal(body, &got); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true,"session_id":31,"board_path":"board.md","cwd":"/tmp/child"}`))
	}))
	defer server.Close()
	port := server.Listener.Addr().String()
	port = port[strings.LastIndex(port, ":")+1:]
	t.Setenv(hubPortEnv, port)
	t.Setenv(sessionIDEnv, "9")
	t.Setenv(hubTokenEnv, "spawn-secret")

	if err := runSpawn([]string{
		"--role", "review",
		"--provider", "claude",
		"--effort", "high",
		"--execution-mode", "interactive",
		"--permission", "attended",
		"prompt text",
	}); err != nil {
		t.Fatalf("runSpawn: %v", err)
	}
	if got.Effort != "high" || got.ExecutionMode != "interactive" || got.PermissionPreset != "attended" {
		t.Fatalf("spawn request = %+v", got)
	}

	// 3 項目を付けない呼び出しでは、JSON にキー自体が現れない。
	raw = nil
	if err := runSpawn([]string{"--role", "review", "prompt text"}); err != nil {
		t.Fatalf("runSpawn without options: %v", err)
	}
	for _, key := range []string{"effort", "execution_mode", "permission_preset"} {
		if _, present := raw[key]; present {
			t.Fatalf("body carries %q even though the flag was not passed: %v", key, raw)
		}
	}
}

// relay の役割は 1 役割 1 引数のまま provider/model@effort を受け、実行モードは
// 役割共通の 1 フラグから各役割へ写る。付けなければ body は従来と完全に一致する。
func TestRunRelaySendsRoleEffortAndExecutionMode(t *testing.T) {
	var raw map[string]any
	var got relayStartRequest
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Fatalf("read request: %v", err)
		}
		if err := json.Unmarshal(body, &raw); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		if err := json.Unmarshal(body, &got); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true,"relay":{"orchestration_id":"r9-test","mode":"worktree","max_rounds":3,"implementation_session_id":12}}`))
	}))
	defer server.Close()
	port := server.Listener.Addr().String()
	port = port[strings.LastIndex(port, ":")+1:]
	t.Setenv(hubPortEnv, port)
	t.Setenv(sessionIDEnv, "9")
	t.Setenv(hubTokenEnv, "relay-secret")

	if err := runRelay([]string{
		"--plan", "docs/plan.md",
		"--impl", "claude/opus@high",
		"--review", "codex/gpt-5",
		"--execution-mode", "interactive",
	}); err != nil {
		t.Fatalf("runRelay: %v", err)
	}
	impl := got.Roles["implementation"]
	if impl.Provider != "claude" || impl.Model != "opus" || impl.Effort != "high" || impl.ExecutionMode != "interactive" {
		t.Fatalf("implementation role = %+v", impl)
	}
	review := got.Roles["review"]
	if review.Effort != "" || review.ExecutionMode != "interactive" {
		t.Fatalf("review role = %+v (effort must stay empty, execution mode is role-common)", review)
	}

	// 何も付けない relay の役割 JSON には effort も execution_mode も現れない。
	raw = nil
	if err := runRelay([]string{
		"--plan", "docs/plan.md",
		"--impl", "claude/opus",
		"--review", "codex/gpt-5",
	}); err != nil {
		t.Fatalf("runRelay without options: %v", err)
	}
	roles, _ := raw["roles"].(map[string]any)
	if len(roles) == 0 {
		t.Fatalf("relay body carries no roles: %v", raw)
	}
	for role, value := range roles {
		fields, _ := value.(map[string]any)
		for _, key := range []string{"effort", "execution_mode"} {
			if _, present := fields[key]; present {
				t.Fatalf("role %s carries %q even though nothing was passed: %v", role, key, fields)
			}
		}
	}
}

// 権限の段は役割共通の 1 フラグで、全役割の割り当てへ同じ値が写る（子 plan:
// docs/local/plan_derived-session-launch_c2_permission-tiers.md 内部 C6 の設計 7）。
// 受理値の判断は Hub の既存検証が持つので、CLI 側では写ることだけを固定する。
func TestRunRelaySendsRolePermissionPreset(t *testing.T) {
	var raw map[string]any
	var got relayStartRequest
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(r.Body)
		if err != nil {
			t.Fatalf("read request: %v", err)
		}
		if err := json.Unmarshal(body, &raw); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		if err := json.Unmarshal(body, &got); err != nil {
			t.Fatalf("decode request: %v", err)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true,"relay":{"orchestration_id":"r9-test","mode":"worktree","max_rounds":3,"implementation_session_id":12}}`))
	}))
	defer server.Close()
	port := server.Listener.Addr().String()
	port = port[strings.LastIndex(port, ":")+1:]
	t.Setenv(hubPortEnv, port)
	t.Setenv(sessionIDEnv, "9")
	t.Setenv(hubTokenEnv, "relay-secret")

	if err := runRelay([]string{
		"--plan", "docs/plan.md",
		"--impl", "claude/opus",
		"--review", "codex/gpt-5",
		"--strong", "claude/opus",
		"--permission", "bounded",
	}); err != nil {
		t.Fatalf("runRelay: %v", err)
	}
	for _, role := range []string{"implementation", "review", "implementation-strong"} {
		if assignment := got.Roles[role]; assignment.PermissionPreset != "bounded" {
			t.Fatalf("role %s = %+v, want permission_preset bounded on every role", role, assignment)
		}
	}

	// 付けない relay の役割 JSON には permission_preset のキー自体が現れない。
	raw = nil
	if err := runRelay([]string{
		"--plan", "docs/plan.md",
		"--impl", "claude/opus",
		"--review", "codex/gpt-5",
	}); err != nil {
		t.Fatalf("runRelay without --permission: %v", err)
	}
	roles, _ := raw["roles"].(map[string]any)
	if len(roles) == 0 {
		t.Fatalf("relay body carries no roles: %v", raw)
	}
	for role, value := range roles {
		fields, _ := value.(map[string]any)
		if _, present := fields["permission_preset"]; present {
			t.Fatalf("role %s carries permission_preset even though --permission was not passed: %v", role, fields)
		}
	}
}
