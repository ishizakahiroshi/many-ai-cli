package wrapper

import (
	"reflect"
	"testing"
)

// TestEffortArgsForProvider は `wrap` の --effort が provider ごとにどの引数へ
// 写るかを固定する（子 plan:
// docs/local/plan_derived-session-launch_c1_request-schema.md 内部 C2）。
// 写像は internal/config/effort.go の表が持つので、ここで固定するのは
// 「wrapper がその表どおりに引数を組み、写像が無い provider では何も足さない」
// ことだけ。
func TestEffortArgsForProvider(t *testing.T) {
	cases := []struct {
		name            string
		provider        string
		effort          string
		isCustom        bool
		want            []string
		wantSkipReason  bool
		wantEmptyResult bool
	}{
		{name: "claude", provider: "claude", effort: "high", want: []string{"--effort", "high"}},
		{name: "codex", provider: "codex", effort: "high", want: []string{"-c", `model_reasoning_effort="high"`}},
		{name: "opencode", provider: "opencode", effort: "high", want: []string{"--variant", "high"}},
		// 写像が無い provider。引数は足さず、理由が返る（呼び出し側が 1 行ログに残す）。
		{name: "copilot", provider: "copilot", effort: "high", wantSkipReason: true},
		{name: "grok", provider: "grok", effort: "high", wantSkipReason: true},
		{name: "cursor-agent", provider: "cursor-agent", effort: "high", wantSkipReason: true},
		// custom provider は --model と同じ理由で built-in フラグを一切受け取らない。
		{name: "custom provider", provider: "my-cli", effort: "high", isCustom: true, wantSkipReason: true},
		// 表にある provider でも、表に無い値は引数にしない。
		{name: "unknown level", provider: "claude", effort: "turbo", wantSkipReason: true},
		// 指定なしは skip ではない（ログも出ない）。
		{name: "empty", provider: "claude", effort: "", wantEmptyResult: true},
		{name: "empty on unmapped provider", provider: "copilot", effort: "", wantEmptyResult: true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			args, skip := effortArgsForProvider(tc.provider, tc.effort, tc.isCustom)
			switch {
			case tc.wantSkipReason:
				if skip == "" {
					t.Fatalf("effortArgsForProvider(%q, %q, %v) = %v, want a skip reason", tc.provider, tc.effort, tc.isCustom, args)
				}
				if len(args) != 0 {
					t.Fatalf("skipped effort still produced args: %v", args)
				}
			case tc.wantEmptyResult:
				if skip != "" || len(args) != 0 {
					t.Fatalf("effortArgsForProvider(%q, \"\", %v) = (%v, %q), want no args and no skip reason", tc.provider, tc.isCustom, args, skip)
				}
			default:
				if skip != "" {
					t.Fatalf("effortArgsForProvider(%q, %q) skipped unexpectedly: %s", tc.provider, tc.effort, skip)
				}
				if !reflect.DeepEqual(args, tc.want) {
					t.Fatalf("effortArgsForProvider(%q, %q) = %v, want %v", tc.provider, tc.effort, args, tc.want)
				}
			}
		})
	}
}

// Run は --model を足した直後に effort を足す。並び（--model が先、effort が後、
// 権限フラグはさらに後）が変わると、引数を目で追う調査が読みにくくなるので固定する。
func TestEffortArgsFollowModelFlag(t *testing.T) {
	var extra []string
	if shouldAppendModelFlag("claude-opus", false) {
		extra = append(extra, "--model", "claude-opus")
	}
	args, skip := effortArgsForProvider("claude", "high", false)
	if skip != "" {
		t.Fatalf("unexpected skip: %s", skip)
	}
	extra = append(extra, args...)
	extra = append(extra, nativePermissionModeArgs("bypassPermissions")...)
	want := []string{"--model", "claude-opus", "--effort", "high", "--permission-mode", "bypassPermissions"}
	if !reflect.DeepEqual(extra, want) {
		t.Fatalf("assembled args = %v, want %v", extra, want)
	}
}

// effort を指定しない起動では、組み上がる引数が effort を知らなかった頃と
// 完全に一致する（不変条件 1）。
func TestNoEffortLeavesArgsUnchanged(t *testing.T) {
	for _, provider := range []string{"claude", "codex", "opencode", "copilot", "grok", "cursor-agent"} {
		args, skip := effortArgsForProvider(provider, "", false)
		if len(args) != 0 || skip != "" {
			t.Fatalf("provider %s: effortArgsForProvider(provider, \"\") = (%v, %q), want nothing", provider, args, skip)
		}
	}
}
