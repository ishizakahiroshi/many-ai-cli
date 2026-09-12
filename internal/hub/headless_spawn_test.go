package hub

import (
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

// 配送経路はどの子でも必ず片方だけ。両方通ると同じ指示が 2 回入り、どちらも通らないと
// 子は何も指示されないまま起動する。
func TestChildLaunchPromptPicksExactlyOneRoute(t *testing.T) {
	board := filepath.Join(t.TempDir(), "board.md")
	for _, mode := range []string{"", config.ExecutionModeInteractive} {
		prompt, inject := childLaunchPrompt(spawnChildRequest{Role: "review", InitialPrompt: "hi", ExecutionMode: mode}, board, "")
		if prompt != "" || !inject {
			t.Fatalf("execution_mode %q: prompt = %q inject = %v, want 注入だけ", mode, prompt, inject)
		}
	}
	prompt, inject := childLaunchPrompt(spawnChildRequest{Role: "review", InitialPrompt: "hi", ExecutionMode: config.ExecutionModeHeadless}, board, "")
	if inject {
		t.Fatal("headless の子へ注入してはいけない（入力欄が無い）")
	}
	if !strings.Contains(prompt, "Role: review") || !strings.Contains(prompt, "hi") {
		t.Fatalf("headless launch prompt = %q, want the child's own instructions", prompt)
	}
}
