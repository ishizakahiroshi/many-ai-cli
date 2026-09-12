package wrapper

import (
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
)

func TestNativePermissionModeArgs(t *testing.T) {
	tests := []struct {
		name           string
		permissionMode string
		want           []string
	}{
		{name: "empty keeps approvals", permissionMode: "", want: nil},
		{name: "default keeps approvals", permissionMode: "default", want: nil},
		{name: "auto is passed through", permissionMode: "auto", want: []string{"--permission-mode", "auto"}},
		{name: "bypass is passed through", permissionMode: "bypassPermissions", want: []string{"--permission-mode", "bypassPermissions"}},
		{name: "plan is passed through", permissionMode: "plan", want: []string{"--permission-mode", "plan"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := nativePermissionModeArgs(tt.permissionMode); !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("nativePermissionModeArgs(%q) = %v, want %v", tt.permissionMode, got, tt.want)
			}
		})
	}
}

func TestCopilotPermissionArgs(t *testing.T) {
	tests := []struct {
		name           string
		permissionMode string
		want           []string
	}{
		{name: "default keeps approvals", permissionMode: "default", want: nil},
		{name: "auto enables autopilot", permissionMode: "auto", want: []string{"--autopilot"}},
		{name: "bypass enables allow-all", permissionMode: "bypassPermissions", want: []string{"--allow-all"}},
		{name: "plan is not a copilot permission flag", permissionMode: "plan", want: nil},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := copilotPermissionArgs(tt.permissionMode); !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("copilotPermissionArgs(%q) = %v, want %v", tt.permissionMode, got, tt.want)
			}
		})
	}
}

func TestCursorAgentPermissionArgs(t *testing.T) {
	tests := []struct {
		name           string
		permissionMode string
		want           []string
	}{
		{name: "default keeps approvals", permissionMode: "default", want: nil},
		{name: "auto enables auto-review", permissionMode: "auto", want: []string{"--auto-review"}},
		{name: "bypass enables force", permissionMode: "bypassPermissions", want: []string{"--force"}},
		{name: "plan is not a cursor permission flag", permissionMode: "plan", want: nil},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := cursorAgentPermissionArgs(tt.permissionMode); !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("cursorAgentPermissionArgs(%q) = %v, want %v", tt.permissionMode, got, tt.want)
			}
		})
	}
}

func TestCommandCodePermissionArgs(t *testing.T) {
	tests := []struct {
		name           string
		permissionMode string
		want           []string
	}{
		{name: "empty keeps approvals", permissionMode: "", want: nil},
		{name: "default keeps approvals", permissionMode: "default", want: nil},
		{name: "plan enables permission-mode plan", permissionMode: "plan", want: []string{"--permission-mode", "plan"}},
		{name: "acceptEdits enables auto-accept", permissionMode: "acceptEdits", want: []string{"--auto-accept"}},
		{name: "auto enables auto-accept", permissionMode: "auto", want: []string{"--auto-accept"}},
		{name: "bypass enables yolo", permissionMode: "bypassPermissions", want: []string{"--yolo"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := commandCodePermissionArgs(tt.permissionMode); !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("commandCodePermissionArgs(%q) = %v, want %v", tt.permissionMode, got, tt.want)
			}
		})
	}
}

// 段 2（範囲を限った無人）で各 provider が実際に受け取る引数を固定する。
// ここが変わると、無人の子が触れる範囲が黙って変わる
// （子 plan: docs/local/plan_derived-session-launch_c2_permission-tiers.md 内部 C2）。
func TestBoundedPermissionArgsPerProvider(t *testing.T) {
	tools := []string{"Read", "Bash(git log *)"}

	// claude: --permission-mode dontAsk + --allowedTools <v1> <v2>
	claude := append(nativePermissionModeArgs("dontAsk"), allowedToolArgs("claude", tools)...)
	want := []string{"--permission-mode", "dontAsk", "--allowedTools", "Read", "Bash(git log *)"}
	if !reflect.DeepEqual(claude, want) {
		t.Fatalf("claude 段 2 = %q, want %q", claude, want)
	}

	// copilot: --no-ask-user + --allow-tool=<v> を値の数だけ
	copilot := append(copilotPermissionArgs(config.PermissionModeBounded), allowedToolArgs("copilot", tools)...)
	wantCopilot := []string{"--no-ask-user", "--allow-tool=Read", "--allow-tool=Bash(git log *)"}
	if !reflect.DeepEqual(copilot, wantCopilot) {
		t.Fatalf("copilot 段 2 = %q, want %q", copilot, wantCopilot)
	}

	// opencode: --auto（範囲は opencode.json 側）。段 3 も同じフラグで、
	// 差は permission 規則だけ。
	for _, mode := range []string{config.PermissionModeBounded, "bypassPermissions"} {
		if got := openCodePermissionArgs(mode); !reflect.DeepEqual(got, []string{"--auto"}) {
			t.Fatalf("opencode %q = %q, want [--auto]", mode, got)
		}
	}
}

// 内部マーカー "bounded" を実 CLI へ出さない。claude / grok の --permission-mode に
// その値は無いので、渡せば起動に失敗する。段 2 を持たない provider も何も足さない。
func TestBoundedMarkerNeverReachesTheCLI(t *testing.T) {
	if got := nativePermissionModeArgs(config.PermissionModeBounded); got != nil {
		t.Fatalf("claude/grok へ内部マーカーが漏れている: %q", got)
	}
	if got := cursorAgentPermissionArgs(config.PermissionModeBounded); got != nil {
		t.Fatalf("cursor-agent = %q, want nil", got)
	}
	if got := commandCodePermissionArgs(config.PermissionModeBounded); got != nil {
		t.Fatalf("command-code = %q, want nil", got)
	}
}

// --allowed-tools を渡さない起動は argv が従来と完全に一致する（親 plan 不変条件 1）。
// 段 2 の写像が無い provider も同じ。
func TestAllowedToolArgsAddNothingWithoutValues(t *testing.T) {
	if got := parseAllowedToolValues(""); got != nil {
		t.Fatalf("parseAllowedToolValues(\"\") = %q, want nil", got)
	}
	for _, provider := range []string{"claude", "codex", "copilot", "cursor-agent", "opencode", "grok", "command-code", "shell", "my-custom-cli"} {
		if got := allowedToolArgs(provider, nil); got != nil {
			t.Fatalf("%s: 値が無いのに引数が増えた: %q", provider, got)
		}
	}
	for _, provider := range []string{"codex", "cursor-agent", "opencode", "grok", "command-code", "shell", "my-custom-cli"} {
		if got := allowedToolArgs(provider, []string{"Read"}); got != nil {
			t.Fatalf("%s: 写像が無いのに引数が増えた: %q", provider, got)
		}
	}
}

// wrap は Hub を経由せず手で叩ける。コマンドラインへ出せない形の値はここでも落とす。
func TestParseAllowedToolValuesFilters(t *testing.T) {
	got := parseAllowedToolValues("Read, Bash(git log *) ,--oops,rm -rf / ; echo x,")
	want := []string{"Read", "Bash(git log *)"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("parseAllowedToolValues = %q, want %q", got, want)
	}
}

// 段 2 の opencode.json。permission は全体 allow のまま、取り返しのつかない bash だけ
// deny になる。opencode は「最後に一致した規則が勝つ」ので、"*" が deny 群より前に
// 並んでいることまで確認する（並びが逆だと段 2 が段 3 と同じになる）。
func TestPrepareOpenCodeConfigBoundedWritesBashDenyRules(t *testing.T) {
	cwd := t.TempDir()
	cfgPath := filepath.Join(cwd, OpenCodeConfigFileName)
	cleanup, err := prepareOpenCodeConfig(cwd, "allow", openCodeBoundedBashDeny, quietLogger())
	if err != nil {
		t.Fatalf("prepareOpenCodeConfig: %v", err)
	}
	raw, err := os.ReadFile(cfgPath)
	if err != nil {
		t.Fatalf("read %s: %v", cfgPath, err)
	}
	var parsed struct {
		Permission struct {
			All  string            `json:"*"`
			Bash map[string]string `json:"bash"`
		} `json:"permission"`
	}
	if err := json.Unmarshal(raw, &parsed); err != nil {
		t.Fatalf("unmarshal %s: %v", cfgPath, err)
	}
	if parsed.Permission.All != "allow" {
		t.Fatalf("permission[*] = %q, want allow", parsed.Permission.All)
	}
	if parsed.Permission.Bash["*"] != "allow" {
		t.Fatalf("permission.bash[*] = %q, want allow", parsed.Permission.Bash["*"])
	}
	for _, pattern := range openCodeBoundedBashDeny {
		if parsed.Permission.Bash[pattern] != "deny" {
			t.Fatalf("permission.bash[%q] = %q, want deny", pattern, parsed.Permission.Bash[pattern])
		}
	}
	body := string(raw)
	starAt := strings.Index(body, `"*": "allow"`)
	for _, pattern := range openCodeBoundedBashDeny {
		denyAt := strings.Index(body, `"`+pattern+`"`)
		if denyAt < 0 || denyAt < starAt {
			t.Fatalf("deny 規則 %q が \"*\" より前にある（最後に一致した規則が勝つので効かない）:\n%s", pattern, body)
		}
	}
	cleanup()
	if _, statErr := os.Stat(cfgPath); !os.IsNotExist(statErr) {
		t.Fatalf("cleanup で opencode.json が消えていない: %v", statErr)
	}
}

// bashDeny を渡さない呼び出し（通常セッションと段 3）は、この引数が無かった頃と
// 同じ 1 キーだけの permission を書く。
func TestPrepareOpenCodeConfigWithoutBashDenyIsUnchanged(t *testing.T) {
	cwd := t.TempDir()
	cleanup, err := prepareOpenCodeConfig(cwd, "allow", nil, quietLogger())
	if err != nil {
		t.Fatalf("prepareOpenCodeConfig: %v", err)
	}
	defer cleanup()
	raw, err := os.ReadFile(filepath.Join(cwd, OpenCodeConfigFileName))
	if err != nil {
		t.Fatalf("read opencode.json: %v", err)
	}
	var parsed struct {
		Permission map[string]any `json:"permission"`
	}
	if err := json.Unmarshal(raw, &parsed); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if len(parsed.Permission) != 1 || parsed.Permission["*"] != "allow" {
		t.Fatalf("permission = %v, want only {\"*\": \"allow\"}", parsed.Permission)
	}
}
