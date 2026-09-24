package hub

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/headless"
)

// 不変条件 1 の直接の固定: headless でない起動は、この実行モードが存在しなかった
// 頃とまったく同じ argv を組み、ファイルも 1 つも書かない。
func TestHeadlessWrapArgsAddsNothingToAnInteractiveLaunch(t *testing.T) {
	s := newTestServer()
	home := t.TempDir()
	t.Setenv("USERPROFILE", home)
	t.Setenv("HOME", home)
	for _, mode := range []string{"", config.ExecutionModeInteractive, config.ExecutionModeAuto} {
		args, path, err := s.headlessWrapArgs(mode, "do the thing")
		if err != nil {
			t.Fatalf("execution_mode %q: %v", mode, err)
		}
		if len(args) != 0 || path != "" {
			t.Fatalf("execution_mode %q: args = %v path = %q, want nothing added", mode, args, path)
		}
	}
	if entries, err := os.ReadDir(filepath.Join(home, ".many-ai-cli", "tmp")); err == nil && len(entries) > 0 {
		t.Fatalf("an interactive launch wrote %d prompt files", len(entries))
	}
}

// headless の起動は --headless と、プロンプトを持つファイルの場所だけを足す。
// **プロンプト本文は argv に出さない**（プロセス一覧から読めてしまう）。
func TestHeadlessWrapArgsWritesThePromptToAFile(t *testing.T) {
	s := newTestServer()
	home := t.TempDir()
	t.Setenv("USERPROFILE", home)
	t.Setenv("HOME", home)
	prompt := "Role: review\nRead the board before acting."
	args, path, err := s.headlessWrapArgs(config.ExecutionModeHeadless, prompt)
	if err != nil {
		t.Fatalf("headlessWrapArgs: %v", err)
	}
	if len(args) != 3 || args[0] != "--headless" || args[1] != "--prompt-file" {
		t.Fatalf("args = %v, want [--headless --prompt-file <path>]", args)
	}
	if args[2] != path {
		t.Fatalf("args = %v, path = %q, want the same path", args, path)
	}
	for _, arg := range args {
		if strings.Contains(arg, "Read the board") {
			t.Fatalf("the prompt itself must not reach argv: %v", args)
		}
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read prompt file: %v", err)
	}
	if string(data) != prompt {
		t.Errorf("prompt file = %q, want the prompt verbatim", data)
	}
	if dir := filepath.Dir(path); filepath.Base(dir) != "tmp" || filepath.Base(filepath.Dir(dir)) != ".many-ai-cli" {
		t.Errorf("prompt file lives at %q, want it under ~/.many-ai-cli/tmp", path)
	}

	// 文面が無い headless 起動は --headless だけ（空のプロンプトでファイルを作らない）。
	args, path, err = s.headlessWrapArgs(config.ExecutionModeHeadless, "   ")
	if err != nil {
		t.Fatalf("headlessWrapArgs (empty prompt): %v", err)
	}
	if len(args) != 1 || args[0] != "--headless" || path != "" {
		t.Fatalf("args = %v path = %q, want just --headless", args, path)
	}
}

// 置き去りの回収は「次に起動したとき」に効く形にする（graceful な終了に依存しない）。
func TestSweepStaleHeadlessPrompts(t *testing.T) {
	dir := t.TempDir()
	now := time.Now()
	write := func(name string, age time.Duration) string {
		path := filepath.Join(dir, name)
		if err := os.WriteFile(path, []byte("x"), 0o600); err != nil {
			t.Fatalf("write %s: %v", name, err)
		}
		if err := os.Chtimes(path, now.Add(-age), now.Add(-age)); err != nil {
			t.Fatalf("chtimes %s: %v", name, err)
		}
		return path
	}
	stale := write("prompt-aaaa.md", 3*time.Hour)
	fresh := write("prompt-bbbb.md", time.Minute)
	other := write("notes.md", 3*time.Hour)

	sweepStaleHeadlessPrompts(dir, now)

	if _, err := os.Stat(stale); !os.IsNotExist(err) {
		t.Errorf("the abandoned prompt file survived (stat err = %v)", err)
	}
	for _, keep := range []string{fresh, other} {
		if _, err := os.Stat(keep); err != nil {
			t.Errorf("%s was removed: %v", filepath.Base(keep), err)
		}
	}
}

// wrapper の案内のファイル（launch-<pid>-….md）は、書いた wrapper が終わっている
// ときだけ回収する。動いている wrapper の CLI は信頼の確認で待っていて、まだ
// 読んでいないかもしれない（古さだけでは判断できない）。
func TestSweepStaleHeadlessPromptsKeepsALiveWrappersLaunchFile(t *testing.T) {
	dir := t.TempDir()
	now := time.Now()
	write := func(name string) string {
		path := filepath.Join(dir, name)
		if err := os.WriteFile(path, []byte("synthetic instruction"), 0o600); err != nil {
			t.Fatalf("write %s: %v", name, err)
		}
		old := now.Add(-3 * time.Hour)
		if err := os.Chtimes(path, old, old); err != nil {
			t.Fatalf("chtimes %s: %v", name, err)
		}
		return path
	}
	live := write(fmt.Sprintf("launch-%d-aaaa.md", os.Getpid()))
	// 実在しないはずの PID（Windows の PID は 4 の倍数・Unix の上限は 2^22 程度）。
	gone := write("launch-2147483001-bbbb.md")

	sweepStaleHeadlessPrompts(dir, now)

	if _, err := os.Stat(live); err != nil {
		t.Errorf("a running wrapper's launch file was removed: %v", err)
	}
	if _, err := os.Stat(gone); !os.IsNotExist(err) {
		t.Errorf("a dead wrapper's old launch file survived (stat err = %v)", err)
	}
}

// 子への指示は 1 つの文面で、渡し方だけが 2 通り。session ID の置き換えを済ませた
// headless の文面は、対話の子が受け取る文面と 1 バイトも違わない。
func TestHeadlessChildPromptMatchesTheInteractiveOne(t *testing.T) {
	board := filepath.Join(t.TempDir(), "board.md")
	interactive := buildChildInitialPrompt("review the diff", board, "review", "orch/review", 42)
	expanded := headless.ExpandPrompt(buildHeadlessChildInitialPrompt("review the diff", board, "review", "orch/review"), 42)
	if expanded != interactive {
		t.Fatalf("headless prompt differs from the interactive one:\n--- headless ---\n%s\n--- interactive ---\n%s", expanded, interactive)
	}
	// 展開前は自分の ID を持たない（Hub は register 前に ID を知らない）。
	raw := buildHeadlessChildInitialPrompt("review the diff", board, "review", "")
	if !strings.Contains(raw, headless.SessionIDPlaceholder) {
		t.Errorf("headless prompt has no session id placeholder:\n%s", raw)
	}
	if strings.Contains(raw, "child-0.md") {
		t.Errorf("headless prompt names a made-up session id:\n%s", raw)
	}
}

// 配送経路はどの子でも必ず 1 つだけ。2 つ通ると同じ指示が 2 回入り、どれも通らないと
// 子は何も指示されないまま起動する。
func TestChildLaunchPromptPicksExactlyOneRoute(t *testing.T) {
	board := filepath.Join(t.TempDir(), "board.md")
	for _, mode := range []string{"", config.ExecutionModeInteractive} {
		prompt, inject := childLaunchPrompt(mode, false, "hi", board, "review", "")
		if prompt != "" || !inject {
			t.Fatalf("execution_mode %q, typed: prompt = %q inject = %v, want 注入だけ", mode, prompt, inject)
		}
		prompt, inject = childLaunchPrompt(mode, true, "hi", board, "review", "")
		if inject || !strings.Contains(prompt, headless.SessionIDPlaceholder) {
			t.Fatalf("execution_mode %q, launch arg: prompt = %q inject = %v, want 起動時に渡すだけ（ID は置き場所）", mode, prompt, inject)
		}
	}
	prompt, inject := childLaunchPrompt(config.ExecutionModeHeadless, false, "hi", board, "review", "")
	if inject {
		t.Fatal("headless の子へ注入してはいけない（入力欄が無い）")
	}
	if !strings.Contains(prompt, "Role: review") || !strings.Contains(prompt, "hi") {
		t.Fatalf("headless launch prompt = %q, want the child's own instructions", prompt)
	}
}

// 機械検査（親 plan の C6 で CLAUDE.md の設計原則の表に載せる）: config の
// launch_prompt.go の表で true の provider の対話の子は、打ち込み（注入）を通らない。
// 起動（dispatchSpawn）と再起動（respawnTimedOutChild）はどちらもこの 2 つの関数だけで
// 渡し方を決めるので、ここで全 provider を固定すれば両方が固定される。表に無い
// provider は今までどおり注入。
func TestLaunchArgProvidersAreNeverTyped(t *testing.T) {
	s := newTestServer()
	s.launchArgUsable = func(string) (bool, string) { return true, "" }
	board := filepath.Join(t.TempDir(), "board.md")
	for _, provider := range orchestrationProviders {
		for _, mode := range []string{"", config.ExecutionModeInteractive} {
			viaArg := s.childPromptViaLaunchArg(provider, mode)
			_, inject := childLaunchPrompt(mode, viaArg, "hi", board, "review", "")
			if want := !config.LaunchPromptViaArg(provider); inject != want {
				t.Errorf("provider %q mode %q: inject = %v, want %v", provider, mode, inject, want)
			}
		}
	}
	for _, provider := range []string{"claude", "codex"} {
		if !config.LaunchPromptViaArg(provider) {
			t.Fatalf("fixture: %s is expected in the launch-prompt table", provider)
		}
	}
}

// 起動引数で指示を渡す対話の子は、wrapper へ --prompt-file だけを足す（本文は argv に
// 出さない）。指示の無い対話の起動は今までどおり何も足さない。
func TestLaunchPromptWrapArgsHandsAnInteractiveInstructionOverAsAFile(t *testing.T) {
	s := newTestServer()
	home := t.TempDir()
	t.Setenv("USERPROFILE", home)
	t.Setenv("HOME", home)
	prompt := "Role: review\nSession ID: " + headless.SessionIDPlaceholder
	args, path, err := s.launchPromptWrapArgs("", prompt)
	if err != nil {
		t.Fatalf("launchPromptWrapArgs: %v", err)
	}
	if len(args) != 2 || args[0] != "--prompt-file" || args[1] != path {
		t.Fatalf("args = %v path = %q, want [--prompt-file <path>]", args, path)
	}
	if data, err := os.ReadFile(path); err != nil || string(data) != prompt {
		t.Fatalf("prompt file = %q (err %v), want the instruction verbatim", data, err)
	}
	if args, path, err := s.launchPromptWrapArgs(config.ExecutionModeInteractive, "  "); err != nil || len(args) != 0 || path != "" {
		t.Fatalf("empty interactive instruction: args = %v path = %q err = %v, want nothing", args, path, err)
	}
	if args, _, err := s.launchPromptWrapArgs(config.ExecutionModeHeadless, prompt); err != nil || len(args) != 3 || args[0] != "--headless" {
		t.Fatalf("headless: args = %v err = %v, want headlessWrapArgs unchanged", args, err)
	}
}

// 起動時に指示を受け取った子には、登録時の入力保留を掛けない。注入される子と
// conductor・指示付きの /api/spawn には今までどおり掛ける。
func TestInitialInjectGateSkipsChildrenThatGotTheirPromptAtLaunch(t *testing.T) {
	for _, tc := range []struct {
		name string
		meta pendingChild
		want bool
	}{
		{"typed child", pendingChild{OrchestrationID: "o1", Auto: true}, true},
		{"launch-arg child", pendingChild{OrchestrationID: "o1", Auto: true, PromptAtLaunch: true}, false},
		{"conductor", pendingChild{OrchestrationID: "o1"}, true},
		{"prompted /api/spawn", pendingChild{InitialPrompt: "hi"}, true},
		{"plain session", pendingChild{}, false},
	} {
		if got := initialInjectGateNeeded(tc.meta); got != tc.want {
			t.Errorf("%s: gate = %v, want %v", tc.name, got, tc.want)
		}
	}
}

// 表で true でも、この端末で渡せない（cmd.exe を通り、shim のパスに空白がある等）なら
// 打ち込みへ戻す。headless は表と関係なく headless のまま。
func TestChildPromptViaLaunchArgFallsBackWhenTheMachineCannot(t *testing.T) {
	s := newTestServer()
	s.launchArgUsable = func(string) (bool, string) { return false, "synthetic: shim path contains whitespace" }
	if s.childPromptViaLaunchArg("claude", "") {
		t.Fatal("an unusable launch argument was chosen")
	}
	s.launchArgUsable = func(string) (bool, string) { return true, "" }
	if s.childPromptViaLaunchArg("claude", config.ExecutionModeHeadless) {
		t.Fatal("a headless child was put on the interactive launch-argument route")
	}
}
