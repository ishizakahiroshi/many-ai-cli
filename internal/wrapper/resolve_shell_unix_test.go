//go:build !windows

package wrapper

import (
	"os"
	"testing"
)

// TestResolveDefaultShell_ShellEnv は SHELL 環境変数が実在するパスを指すとき
// resolveDefaultShell がそのパスを返すことを確認する。
func TestResolveDefaultShell_ShellEnv(t *testing.T) {
	// /bin/sh は Unix 上でほぼ確実に存在するので SHELL に設定して確認する。
	const knownShell = "/bin/sh"
	if _, err := os.Stat(knownShell); err != nil {
		t.Skipf("/bin/sh not found on this system: %v", err)
	}
	t.Setenv("SHELL", knownShell)
	got := resolveDefaultShell()
	if got != knownShell {
		t.Errorf("resolveDefaultShell() = %q, want %q", got, knownShell)
	}
}

// TestResolveDefaultShell_NonexistentShellEnv は SHELL 環境変数が存在しないパスを指すとき
// resolveDefaultShell が bash/sh にフォールバックすることを確認する。
func TestResolveDefaultShell_NonexistentShellEnv(t *testing.T) {
	t.Setenv("SHELL", "/nonexistent/shell/that/does/not/exist")
	got := resolveDefaultShell()
	// bash か sh か /bin/sh のいずれかが返るはず。
	if got == "" {
		t.Error("resolveDefaultShell() returned empty string with nonexistent SHELL")
	}
	if got == "/nonexistent/shell/that/does/not/exist" {
		t.Error("resolveDefaultShell() should not return nonexistent SHELL path")
	}
}

// TestResolveDefaultShell_NoShellEnv は SHELL 環境変数が未設定のとき
// resolveDefaultShell が空でない文字列を返すことを確認する。
func TestResolveDefaultShell_NoShellEnv(t *testing.T) {
	t.Setenv("SHELL", "")
	got := resolveDefaultShell()
	if got == "" {
		t.Error("resolveDefaultShell() returned empty string with no SHELL env")
	}
}

// TestResolveCmdShellProvider は provider="shell" のとき resolveCmd が
// resolveDefaultShell の結果と同じコマンドを返すことを確認する。
func TestResolveCmdShellProvider(t *testing.T) {
	want := resolveDefaultShell()
	got, gotArgs := resolveCmd("shell", []string{})
	if got != want {
		t.Errorf("resolveCmd(shell) = %q, want %q", got, want)
	}
	if len(gotArgs) != 0 {
		t.Errorf("resolveCmd(shell) args = %v, want empty", gotArgs)
	}
}

// TestResolveCmdShellProviderPassthroughArgs は provider="shell" のとき
// resolveCmd が追加の args をそのまま返すことを確認する。
func TestResolveCmdShellProviderPassthroughArgs(t *testing.T) {
	args := []string{"-c", "echo hello"}
	_, gotArgs := resolveCmd("shell", args)
	if len(gotArgs) != len(args) {
		t.Errorf("resolveCmd(shell) args len = %d, want %d", len(gotArgs), len(args))
	}
	for i, a := range args {
		if gotArgs[i] != a {
			t.Errorf("resolveCmd(shell) args[%d] = %q, want %q", i, gotArgs[i], a)
		}
	}
}
