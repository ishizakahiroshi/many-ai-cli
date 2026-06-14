package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// auditNoLeftoverTempFiles は dir 内に config-*.yaml.tmp が残っていないことを検証する。
// 原子書き込み（temp + Rename）が完了すると temp は Rename で消費されるため、
// 失敗していなければ .yaml.tmp は一切残らない。
func auditNoLeftoverTempFiles(t *testing.T, dir string) {
	t.Helper()
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	for _, e := range entries {
		if strings.HasSuffix(e.Name(), ".yaml.tmp") {
			t.Errorf("leftover temp file from atomic write: %s", e.Name())
		}
	}
}

// TestWriteConfigAtomicHelper は writeConfigAtomic が path を新規生成し、
// 内容一致・0o600・temp 残りなしで原子的に書くことを直接検証する。
func TestWriteConfigAtomicHelper(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yaml")
	want := []byte("token: atomic-helper\n")

	if err := writeConfigAtomic(dir, path, want); err != nil {
		t.Fatalf("writeConfigAtomic: %v", err)
	}

	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read back: %v", err)
	}
	if string(got) != string(want) {
		t.Errorf("content mismatch: got %q, want %q", got, want)
	}
	auditNoLeftoverTempFiles(t, dir)

	// 既存ファイルへの上書きも原子的に成功すること（Rename で差し替え）。
	want2 := []byte("token: atomic-helper-2\n")
	if err := writeConfigAtomic(dir, path, want2); err != nil {
		t.Fatalf("writeConfigAtomic overwrite: %v", err)
	}
	got2, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read back overwrite: %v", err)
	}
	if string(got2) != string(want2) {
		t.Errorf("overwrite content mismatch: got %q, want %q", got2, want2)
	}
	auditNoLeftoverTempFiles(t, dir)
}

// TestLoadOrCreateNewUsesAtomicWrite は新規作成パス（config.yaml 不在）が
// writeConfigAtomic 経由になり temp ファイルを残さないことを検証する（finding #34）。
func TestLoadOrCreateNewUsesAtomicWrite(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	if _, err := LoadOrCreate(); err != nil {
		t.Fatalf("LoadOrCreate: %v", err)
	}

	dir := filepath.Join(home, ".many-ai-cli")
	if _, err := os.Stat(filepath.Join(dir, "config.yaml")); err != nil {
		t.Fatalf("config.yaml not created: %v", err)
	}
	auditNoLeftoverTempFiles(t, dir)
}

// TestLoadOrCreateCorruptUsesAtomicWrite は破損リカバリパスが
// バックアップ書き込み・再生成書き込みともに writeConfigAtomic 経由になり
// temp ファイルを残さないことを検証する（finding #34）。
func TestLoadOrCreateCorruptUsesAtomicWrite(t *testing.T) {
	home := t.TempDir()
	dir := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "config.yaml")
	if err := os.WriteFile(path, []byte("\t\tinvalid yaml content"), 0o600); err != nil {
		t.Fatal(err)
	}

	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	if _, err := LoadOrCreate(); err != nil {
		t.Fatalf("LoadOrCreate on corrupt config: %v", err)
	}

	// .bak と再生成された config.yaml が揃っていること
	if _, err := os.Stat(path + ".bak"); err != nil {
		t.Errorf(".bak not created: %v", err)
	}
	if _, err := os.Stat(path); err != nil {
		t.Errorf("config.yaml not regenerated: %v", err)
	}
	auditNoLeftoverTempFiles(t, dir)
}
