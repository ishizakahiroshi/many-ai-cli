//go:build windows

package hub

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// withRegistryStub は readUserEnvFromRegistry をテスト中だけ差し替える。
func withRegistryStub(t *testing.T, stub func(name string) string) {
	t.Helper()
	prev := readUserEnvFromRegistry
	readUserEnvFromRegistry = stub
	t.Cleanup(func() { readUserEnvFromRegistry = prev })
}

func TestExpandPathEntries_PnpmHomeFromRegistry(t *testing.T) {
	t.Setenv("PNPM_HOME", "")
	withRegistryStub(t, func(name string) string {
		if strings.EqualFold(name, "PNPM_HOME") {
			return `C:\Users\test\AppData\Local\pnpm`
		}
		return ""
	})
	in := []string{`%PNPM_HOME%\bin`, `C:\Windows\System32`}
	out := expandPathEntries(in)
	want := []string{`C:\Users\test\AppData\Local\pnpm\bin`, `C:\Windows\System32`}
	if !equalSlice(out, want) {
		t.Fatalf("expandPathEntries() = %v, want %v", out, want)
	}
}

func TestExpandPathEntries_PnpmHomeFallbackLocalAppData(t *testing.T) {
	t.Setenv("PNPM_HOME", "")
	withRegistryStub(t, func(name string) string { return "" })
	// LOCALAPPDATA を一時ディレクトリに向け、pnpm サブディレクトリを実在させる
	tmp := t.TempDir()
	pnpmDir := filepath.Join(tmp, "pnpm")
	if err := os.MkdirAll(pnpmDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	t.Setenv("LOCALAPPDATA", tmp)

	in := []string{`%PNPM_HOME%\bin`}
	out := expandPathEntries(in)
	want := []string{filepath.Join(pnpmDir, "bin")}
	if !equalSlice(out, want) {
		t.Fatalf("expandPathEntries() = %v, want %v", out, want)
	}
}

func TestExpandPathEntries_PnpmHomeFallbackMissingDirectory(t *testing.T) {
	t.Setenv("PNPM_HOME", "")
	withRegistryStub(t, func(name string) string { return "" })
	tmp := t.TempDir() // pnpm ディレクトリは作らない
	t.Setenv("LOCALAPPDATA", tmp)

	in := []string{`%PNPM_HOME%\bin`}
	out := expandPathEntries(in)
	if !equalSlice(out, in) {
		t.Fatalf("expandPathEntries() = %v, want preserved %v", out, in)
	}
}

func TestExpandPathEntries_AlreadyExpandedEntryPreserved(t *testing.T) {
	withRegistryStub(t, func(name string) string {
		if strings.EqualFold(name, "Path") {
			return ""
		}
		t.Fatalf("registry lookup should not be called for %s", name)
		return ""
	})
	in := []string{`C:\Users\test\AppData\Local\pnpm\bin`, `C:\Windows`}
	out := expandPathEntries(in)
	if !equalSlice(out, in) {
		t.Fatalf("expandPathEntries() = %v, want %v", out, in)
	}
}

func TestExpandPathEntries_UnknownVarPreserved(t *testing.T) {
	t.Setenv("FOO_NOT_SET", "")
	withRegistryStub(t, func(name string) string { return "" })

	in := []string{`%FOO_NOT_SET%\bar`, `C:\ok`}
	out := expandPathEntries(in)
	if !equalSlice(out, in) {
		t.Fatalf("expandPathEntries() = %v, want preserved %v", out, in)
	}
}

func TestExpandPathEntries_EnvBeatsRegistry(t *testing.T) {
	t.Setenv("PNPM_HOME", `D:\custom\pnpm`)
	withRegistryStub(t, func(name string) string {
		if strings.EqualFold(name, "Path") {
			return ""
		}
		t.Fatalf("registry lookup should not be called when env is set: %s", name)
		return ""
	})

	in := []string{`%PNPM_HOME%\bin`}
	out := expandPathEntries(in)
	want := []string{`D:\custom\pnpm\bin`}
	if !equalSlice(out, want) {
		t.Fatalf("expandPathEntries() = %v, want %v", out, want)
	}
}

// TestExpandPathEntries_RecoverDroppedRegistryEntry は、Windows が
// プロセス起動時に未解決の `%VAR%` エントリを literal として保持せず
// 完全に脱落させたケースを再現する。プロセス PATH を見ているだけでは
// `%VAR%` が見つからないため、registry 生値からの救済経路がないと
// pnpm bin が永遠に見えない。
func TestExpandPathEntries_RecoverDroppedRegistryEntry(t *testing.T) {
	t.Setenv("PNPM_HOME", "")
	withRegistryStub(t, func(name string) string {
		switch strings.ToUpper(name) {
		case "PATH":
			return `%PNPM_HOME%\bin;C:\WINDOWS\system32`
		case "PNPM_HOME":
			return `C:\Users\test\AppData\Local\pnpm`
		}
		return ""
	})
	// プロセス PATH には pnpm bin がそもそも乗っていない (Windows に脱落させられた状態)。
	in := []string{`C:\WINDOWS\system32`, `C:\Program Files\nodejs\`}
	out := expandPathEntries(in)
	want := []string{
		`C:\WINDOWS\system32`,
		`C:\Program Files\nodejs\`,
		`C:\Users\test\AppData\Local\pnpm\bin`,
	}
	if !equalSlice(out, want) {
		t.Fatalf("expandPathEntries() = %v, want %v", out, want)
	}
}

// TestExpandPathEntries_RecoveredEntryNotDuplicated は、registry 救済が
// 既に PATH 内に存在するエントリ (大文字小文字違い含む) を二重追加しない
// ことを確認する。
func TestExpandPathEntries_RecoveredEntryNotDuplicated(t *testing.T) {
	t.Setenv("PNPM_HOME", "")
	withRegistryStub(t, func(name string) string {
		switch strings.ToUpper(name) {
		case "PATH":
			return `%PNPM_HOME%\bin`
		case "PNPM_HOME":
			return `C:\Users\test\AppData\Local\pnpm`
		}
		return ""
	})
	in := []string{`C:\users\test\appdata\local\pnpm\bin`} // 既に展開済みエントリが PATH にある
	out := expandPathEntries(in)
	if !equalSlice(out, in) {
		t.Fatalf("expandPathEntries() = %v, want %v (no duplicate append)", out, in)
	}
}

func equalSlice(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}
