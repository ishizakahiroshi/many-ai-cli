//go:build windows

package hub

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// withRegistryStub は readUserEnvFromRegistry をテスト中だけ差し替える。
// 同時に readMachineEnvFromRegistry と providerFallbackDirs もテスト中だけ
// no-op に固定し、テスト実行マシン側の HKLM / 実在ディレクトリの差で挙動が
// ブレないようにする。HKLM / fallback を検証したい個別テストは
// withMachineRegistryStub / withProviderFallbackStub で上書きする。
func withRegistryStub(t *testing.T, stub func(name string) string) {
	t.Helper()
	prevUser := readUserEnvFromRegistry
	prevMachine := readMachineEnvFromRegistry
	prevFb := providerFallbackDirs
	readUserEnvFromRegistry = stub
	readMachineEnvFromRegistry = func(name string) string { return "" }
	providerFallbackDirs = func(cache map[string]string) []string { return nil }
	t.Cleanup(func() {
		readUserEnvFromRegistry = prevUser
		readMachineEnvFromRegistry = prevMachine
		providerFallbackDirs = prevFb
	})
}

// withMachineRegistryStub は readMachineEnvFromRegistry のみテスト中だけ差し替える。
func withMachineRegistryStub(t *testing.T, stub func(name string) string) {
	t.Helper()
	prev := readMachineEnvFromRegistry
	readMachineEnvFromRegistry = stub
	t.Cleanup(func() { readMachineEnvFromRegistry = prev })
}

// withProviderFallbackStub は providerFallbackDirs のみテスト中だけ差し替える。
func withProviderFallbackStub(t *testing.T, stub func(cache map[string]string) []string) {
	t.Helper()
	prev := providerFallbackDirs
	providerFallbackDirs = stub
	t.Cleanup(func() { providerFallbackDirs = prev })
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

// TestExpandPathEntries_RecoverPlainRegistryEntry は、HKCU\Environment\Path に
// `%VAR%` を含まない通常の絶対パスが書かれているのに、プロセス起動時の env から
// 何らかの理由 (空エントリ打ち切り等) で脱落したケースを再現する。新実装では
// `%` の有無に関わらずレジストリ Path を読み直して救済する。
func TestExpandPathEntries_RecoverPlainRegistryEntry(t *testing.T) {
	withRegistryStub(t, func(name string) string {
		if strings.EqualFold(name, "Path") {
			return `C:\Users\test\AppData\Roaming\npm;C:\WINDOWS\system32`
		}
		return ""
	})
	// プロセス PATH には npm bin が乗っていない（脱落させられた状態）。
	in := []string{`C:\WINDOWS\system32`}
	out := expandPathEntries(in)
	want := []string{
		`C:\WINDOWS\system32`,
		`C:\Users\test\AppData\Roaming\npm`,
	}
	if !equalSlice(out, want) {
		t.Fatalf("expandPathEntries() = %v, want %v", out, want)
	}
}

// TestExpandPathEntries_RecoverMachinePath は HKLM (Machine) Path 由来のエントリも
// 救済されることを確認する。Hub の親シェルが limited な env だった場合の救済経路。
func TestExpandPathEntries_RecoverMachinePath(t *testing.T) {
	withRegistryStub(t, func(name string) string { return "" })
	withMachineRegistryStub(t, func(name string) string {
		if strings.EqualFold(name, "Path") {
			return `C:\Program Files\nodejs\;C:\WINDOWS\system32`
		}
		return ""
	})
	in := []string{`C:\WINDOWS\system32`}
	out := expandPathEntries(in)
	want := []string{
		`C:\WINDOWS\system32`,
		`C:\Program Files\nodejs\`,
	}
	if !equalSlice(out, want) {
		t.Fatalf("expandPathEntries() = %v, want %v", out, want)
	}
}

// TestExpandPathEntries_MachinePathDeduplicated は HKLM Path のエントリが
// すでにプロセス PATH や HKCU Path に乗っている場合に重複追加されないことを
// 確認する。pathKey は大文字小文字・末尾区切り文字を無視する。
func TestExpandPathEntries_MachinePathDeduplicated(t *testing.T) {
	withRegistryStub(t, func(name string) string {
		if strings.EqualFold(name, "Path") {
			return `C:\Users\test\AppData\Roaming\npm`
		}
		return ""
	})
	withMachineRegistryStub(t, func(name string) string {
		if strings.EqualFold(name, "Path") {
			// 大文字小文字違い + 末尾バックスラッシュ違いを混ぜる
			return `c:\users\test\appdata\roaming\npm\;C:\Program Files\nodejs\`
		}
		return ""
	})
	in := []string{`C:\WINDOWS\system32`}
	out := expandPathEntries(in)
	want := []string{
		`C:\WINDOWS\system32`,
		`C:\Users\test\AppData\Roaming\npm`,
		`C:\Program Files\nodejs\`,
	}
	if !equalSlice(out, want) {
		t.Fatalf("expandPathEntries() = %v, want %v (no duplicates)", out, want)
	}
}

// TestExpandPathEntries_ProviderFallbackAppended は、provider の典型インストール先
// (実在しかつ PATH に無いもの) が最終 fallback として append されることを確認する。
func TestExpandPathEntries_ProviderFallbackAppended(t *testing.T) {
	withRegistryStub(t, func(name string) string { return "" })
	withProviderFallbackStub(t, func(cache map[string]string) []string {
		return []string{
			`C:\Users\test\AppData\Local\pnpm`,
			`C:\Users\test\AppData\Roaming\npm`,
		}
	})
	in := []string{`C:\WINDOWS\system32`}
	out := expandPathEntries(in)
	want := []string{
		`C:\WINDOWS\system32`,
		`C:\Users\test\AppData\Local\pnpm`,
		`C:\Users\test\AppData\Roaming\npm`,
	}
	if !equalSlice(out, want) {
		t.Fatalf("expandPathEntries() = %v, want %v", out, want)
	}
}

// TestExpandPathEntries_ProviderFallbackSkippedWhenAlreadyPresent は、
// fallback ディレクトリがすでに PATH (プロセス側 or レジストリ側) に乗っている場合
// 重複追加されないことを確認する。
func TestExpandPathEntries_ProviderFallbackSkippedWhenAlreadyPresent(t *testing.T) {
	withRegistryStub(t, func(name string) string { return "" })
	withProviderFallbackStub(t, func(cache map[string]string) []string {
		return []string{
			`C:\Users\test\AppData\Local\pnpm`,
		}
	})
	in := []string{`c:\users\test\appdata\local\pnpm\`} // 末尾区切り＋大小違いでも seen に一致
	out := expandPathEntries(in)
	if !equalSlice(out, in) {
		t.Fatalf("expandPathEntries() = %v, want preserved %v (fallback should not duplicate)", out, in)
	}
}

// TestDefaultProviderFallbackDirs_ExistenceCheck は defaultProviderFallbackDirs が
// 実在するディレクトリのみを返すことを確認する。pnpm 候補を実在に、npm 候補を非実在に
// セットして、pnpm のみが返ることを確認する。
func TestDefaultProviderFallbackDirs_ExistenceCheck(t *testing.T) {
	tmp := t.TempDir()
	t.Setenv("LOCALAPPDATA", tmp)
	t.Setenv("APPDATA", filepath.Join(tmp, "nonexistent_appdata"))
	t.Setenv("USERPROFILE", filepath.Join(tmp, "nonexistent_userprofile"))

	pnpmDir := filepath.Join(tmp, "pnpm")
	if err := os.MkdirAll(pnpmDir, 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	cache := map[string]string{}
	out := defaultProviderFallbackDirs(cache)
	want := []string{pnpmDir}
	if !equalSlice(out, want) {
		t.Fatalf("defaultProviderFallbackDirs() = %v, want %v", out, want)
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
