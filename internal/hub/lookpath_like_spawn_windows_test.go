//go:build windows

package hub

import (
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/provider"
)

// TestLookPathLikeSpawn_FindsCLIOnlyInRegistryPath は、Hub（またはその親）の
// 起動後に CLI を入れた状態を再現する。実行ファイルはレジストリの Path にだけあり、
// プロセスの PATH には無い。spawn はこの場所を拾うので、導入状況・バージョン確認・
// 更新の判定も同じく「見つかる」を返さなければならない。
func TestLookPathLikeSpawn_FindsCLIOnlyInRegistryPath(t *testing.T) {
	installed := t.TempDir()
	stale := t.TempDir()
	exe := filepath.Join(installed, "fakecli-registry-only.exe")
	if err := os.WriteFile(exe, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", stale)
	withRegistryStub(t, func(name string) string {
		if strings.EqualFold(name, "Path") {
			return installed
		}
		return ""
	})

	// 対照: プロセスの PATH だけを見る exec.LookPath には見えない（直す前の判定）。
	if got, err := exec.LookPath("fakecli-registry-only"); err == nil {
		t.Fatalf("exec.LookPath found %q; the test must put the CLI outside the process PATH", got)
	}

	got, err := lookPathLikeSpawn("fakecli-registry-only")
	if err != nil {
		t.Fatalf("lookPathLikeSpawn() error = %v, want %q", err, exe)
	}
	if !strings.EqualFold(got, exe) {
		t.Fatalf("lookPathLikeSpawn() = %q, want %q", got, exe)
	}

	// 3 経路はいずれも providerCommandLookPath を通る。既定値のまま判定させる。
	launch := &provider.LaunchDefinition{Executable: "fakecli-registry-only"}
	if !providerCommandFound(launch) {
		t.Fatal("providerCommandFound() = false, want true (導入状況が未インストールになる)")
	}
	if path, ok := selectCLIVersionExecutablePath(launch); !ok || !strings.EqualFold(path, exe) {
		t.Fatalf("selectCLIVersionExecutablePath() = %q, %v; want %q, true", path, ok, exe)
	}
	if name, path, ok := selectCLIUpdateExecutable(launch); !ok || name != "fakecli-registry-only" || !strings.EqualFold(path, exe) {
		t.Fatalf("selectCLIUpdateExecutable() = %q, %q, %v; want the registry-only CLI", name, path, ok)
	}
}

// TestModelListCommand_FindsCLIOnlyInRegistryPath は、モデル一覧の取得
// （cursor-agent / grok の runNativeModelListCommand、opencode も同じ seam）も
// レジストリの Path にだけある CLI を見つけて実行できることを確かめる。
func TestModelListCommand_FindsCLIOnlyInRegistryPath(t *testing.T) {
	installed := t.TempDir()
	script := filepath.Join(installed, "fakecli-models.cmd")
	if err := os.WriteFile(script, []byte("@echo fake-model-a\r\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", t.TempDir()+string(os.PathListSeparator)+filepath.Join(os.Getenv("SystemRoot"), "System32"))
	withRegistryStub(t, func(name string) string {
		if strings.EqualFold(name, "Path") {
			return installed
		}
		return ""
	})

	// 対照: プロセスの PATH だけを見る exec.LookPath には見えない（直す前の取得経路）。
	if got, err := exec.LookPath("fakecli-models"); err == nil {
		t.Fatalf("exec.LookPath found %q; the test must put the CLI outside the process PATH", got)
	}

	out, err := runNativeModelListCommand("fakecli-models")
	if err != nil {
		t.Fatalf("runNativeModelListCommand() error = %v, want the registry-only CLI to run", err)
	}
	if !strings.Contains(string(out), "fake-model-a") {
		t.Fatalf("runNativeModelListCommand() output = %q, want it to contain fake-model-a", out)
	}
}

// TestLookPathLikeSpawn_NotFoundIsExecError は、どこにも無い名前で exec.LookPath と
// 同じ形のエラー（ErrNotFound を包んだ *exec.Error）を返すことを確かめる。
func TestLookPathLikeSpawn_NotFoundIsExecError(t *testing.T) {
	t.Setenv("PATH", t.TempDir())
	withRegistryStub(t, func(name string) string { return "" })

	_, err := lookPathLikeSpawn("fakecli-nowhere")
	var execErr *exec.Error
	if !errors.As(err, &execErr) || execErr.Err != exec.ErrNotFound {
		t.Fatalf("lookPathLikeSpawn() error = %v, want *exec.Error wrapping ErrNotFound", err)
	}
}
