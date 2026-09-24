package wrapper

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"many-ai-cli/internal/headless"
)

// 子 plan: docs/local/plan_child-launch-prompt-and-trust_c4_launch-arg-prompt.md 内部 C2。
// 起動経路の判定がこの端末に入っている CLI に左右されないよう、テストは custom の
// 実行ファイル（テストバイナリ自身 = 直に起動できる .exe / 一時フォルダの偽 .cmd =
// cmd.exe を通る shim）で経路を固定する。指示の本文はすべて合成。

func directLaunchArgv(t *testing.T) []string {
	t.Helper()
	exe, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	return []string{exe}
}

// Windows で .exe へ解けない npm shim を真似た .cmd。起動は cmd.exe /c を通る。
func shellLaunchArgv(t *testing.T, dir string) []string {
	t.Helper()
	if runtime.GOOS != "windows" {
		t.Skip("cmd.exe shims exist only on Windows")
	}
	shim := filepath.Join(dir, "fake-cli.cmd")
	if err := os.WriteFile(shim, []byte("@echo off\r\nnode \"%~dp0\\cli.js\" %*\r\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	return []string{shim}
}

func writePromptFile(t *testing.T, body string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "prompt-synthetic.md")
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}

// 段 2 の --allowedTools（値を複数取る）と --settings があっても、指示は先頭に
// 置かれ、--allowedTools の値にならない。
func TestWithLaunchPromptPutsTheInstructionFirst(t *testing.T) {
	providerArgs := append([]string{"--permission-mode", "dontAsk"}, allowedToolArgs("claude", []string{"Read", "Bash(go test:*)"})...)
	providerArgs = append(providerArgs, "--settings", filepath.Join(t.TempDir(), "settings.json"))
	prompt := "synthetic instruction\nsecond line"

	got := withLaunchPrompt(providerArgs, prompt)
	if got[0] != prompt {
		t.Fatalf("first argument = %q, want the instruction", got[0])
	}
	if len(got) != len(providerArgs)+1 {
		t.Fatalf("got %d arguments, want %d", len(got), len(providerArgs)+1)
	}
	for i, arg := range got[1:] {
		if arg != providerArgs[i] {
			t.Fatalf("argument %d = %q, want %q (the rest must be unchanged)", i+1, arg, providerArgs[i])
		}
	}
	// --allowedTools の値の並び（次のフラグまで）に指示が入っていない。
	for i, arg := range got {
		if arg != "--allowedTools" {
			continue
		}
		for _, value := range got[i+1:] {
			if strings.HasPrefix(value, "--") {
				break
			}
			if value == prompt {
				t.Fatal("the instruction was placed among --allowedTools values")
			}
		}
	}
}

// 指示の中のセッション ID の置き場所が、登録で受け取った ID に置き換わる。
// --prompt-file のファイルは読んだあと無い。
func TestPrepareLaunchPromptExpandsTheSessionIDAndConsumesTheFile(t *testing.T) {
	promptPath := writePromptFile(t, "report to session "+headless.SessionIDPlaceholder+" when done")
	lp, cleanup, err := prepareLaunchPrompt(promptPath, 4242, "synthetic", directLaunchArgv(t), t.TempDir(), 1)
	if err != nil {
		t.Fatalf("prepareLaunchPrompt: %v", err)
	}
	if cleanup != nil {
		defer cleanup()
	}
	if lp.Pointer {
		t.Fatal("a short instruction on a direct launch became a pointer")
	}
	if lp.Arg != "report to session 4242 when done" {
		t.Errorf("Arg = %q", lp.Arg)
	}
	if _, err := os.Stat(promptPath); !os.IsNotExist(err) {
		t.Errorf("--prompt-file still exists after reading (stat err=%v)", err)
	}
}

// 直に起動できる CLI には、改行を含む指示がそのまま 1 つの引数で渡る。
func TestPrepareLaunchPromptPassesMultiLineInstructionsDirectly(t *testing.T) {
	body := "line one\nline two\n"
	lp, _, err := prepareLaunchPrompt(writePromptFile(t, body), 1, "synthetic", directLaunchArgv(t), t.TempDir(), 1)
	if err != nil {
		t.Fatalf("prepareLaunchPrompt: %v", err)
	}
	if lp.Pointer || lp.Arg != body {
		t.Errorf("got %+v, want the instruction itself", lp)
	}
}

func assertPointer(t *testing.T, lp preparedLaunchPrompt, dir, body string) {
	t.Helper()
	if !lp.Pointer {
		t.Fatalf("got %+v, want a pointer", lp)
	}
	if strings.ContainsAny(lp.Arg, "\r\n") {
		t.Errorf("pointer argument is not one line: %q", lp.Arg)
	}
	if !strings.Contains(lp.Arg, lp.Path) {
		t.Errorf("pointer argument %q does not name the file %q", lp.Arg, lp.Path)
	}
	if filepath.Dir(lp.Path) != dir || !strings.HasPrefix(filepath.Base(lp.Path), launchPromptFilePrefix+"77-") {
		t.Errorf("pointer file %q is not launch-<pid>-… in %q", lp.Path, dir)
	}
	data, err := os.ReadFile(lp.Path)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != body {
		t.Errorf("pointer file holds %d bytes, want the %d-byte instruction", len(data), len(body))
	}
}

// 長すぎる指示は、案内の 1 行（改行なし）とファイルになる。後始末でファイルが消える。
func TestPrepareLaunchPromptUsesAPointerForALongInstruction(t *testing.T) {
	dir := t.TempDir()
	body := strings.Repeat("a", launchPromptMaxArgCost) + "\nsecond line"
	lp, cleanup, err := prepareLaunchPrompt(writePromptFile(t, body), 1, "synthetic", directLaunchArgv(t), dir, 77)
	if err != nil {
		t.Fatalf("prepareLaunchPrompt: %v", err)
	}
	assertPointer(t, lp, dir, body)
	if cleanup == nil {
		t.Fatal("no cleanup for the pointer file")
	}
	cleanup()
	if _, err := os.Stat(lp.Path); !os.IsNotExist(err) {
		t.Errorf("pointer file survived cleanup (stat err=%v)", err)
	}
}

// cmd.exe を通る起動では、短い指示でも案内の 1 行になる（cmd.exe は改行で切る）。
func TestPrepareLaunchPromptUsesAPointerThroughCmdExe(t *testing.T) {
	dir := t.TempDir()
	argv := shellLaunchArgv(t, t.TempDir())
	body := "line one\nline two"
	lp, cleanup, err := prepareLaunchPrompt(writePromptFile(t, body), 1, "synthetic", argv, dir, 77)
	if err != nil {
		t.Fatalf("prepareLaunchPrompt: %v", err)
	}
	if cleanup != nil {
		defer cleanup()
	}
	assertPointer(t, lp, dir, body)
}

// - で始まる指示は、フラグと取り違えられないよう先頭に空白を足して渡す。
func TestPrepareLaunchPromptKeepsALeadingDashFromBeingAFlag(t *testing.T) {
	lp, _, err := prepareLaunchPrompt(writePromptFile(t, "--help me fix the tests"), 1, "synthetic", directLaunchArgv(t), t.TempDir(), 1)
	if err != nil {
		t.Fatalf("prepareLaunchPrompt: %v", err)
	}
	if lp.Arg != " --help me fix the tests" {
		t.Errorf("Arg = %q, want a leading space", lp.Arg)
	}
}

// 空の指示は何も渡さない（引数を増やさない）。
func TestPrepareLaunchPromptPassesNothingForAnEmptyInstruction(t *testing.T) {
	lp, _, err := prepareLaunchPrompt(writePromptFile(t, "  \n"), 1, "synthetic", directLaunchArgv(t), t.TempDir(), 1)
	if err != nil {
		t.Fatalf("prepareLaunchPrompt: %v", err)
	}
	if lp.Arg != "" {
		t.Errorf("Arg = %q, want empty", lp.Arg)
	}
}

// cmd.exe を通る起動で、shim のパスに空白があるか、一時フォルダのパスに cmd.exe が
// 解釈する文字があると使えない（Hub はこの答えを見て打ち込みへ戻す）。
func TestLaunchPromptArgUsableRefusesUnsafeShellLaunches(t *testing.T) {
	if ok, reason := launchPromptArgUsable("synthetic", directLaunchArgv(t), `C:\odd%dir`); !ok {
		t.Errorf("direct launch refused: %s", reason)
	}
	spaced := filepath.Join(t.TempDir(), "has space")
	if err := os.MkdirAll(spaced, 0o700); err != nil {
		t.Fatal(err)
	}
	if ok, _ := launchPromptArgUsable("synthetic", shellLaunchArgv(t, spaced), t.TempDir()); ok {
		t.Error("a cmd.exe launch through a shim path with whitespace was accepted")
	}
	plain := shellLaunchArgv(t, t.TempDir())
	if ok, reason := launchPromptArgUsable("synthetic", plain, t.TempDir()); !ok {
		t.Errorf("a plain cmd.exe launch was refused: %s", reason)
	}
	if ok, _ := launchPromptArgUsable("synthetic", plain, filepath.Join(t.TempDir(), "100%")); ok {
		t.Error("a cmd.exe launch with % in the temp path was accepted")
	}
}
