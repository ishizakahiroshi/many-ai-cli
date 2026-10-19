package securefile

import (
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestWriteAtomicCreatesFileWithMode(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "sub", "config.yaml")

	if err := WriteAtomic(path, []byte("token: abc\n"), 0o600); err != nil {
		t.Fatalf("WriteAtomic: %v", err)
	}

	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read back: %v", err)
	}
	if string(got) != "token: abc\n" {
		t.Errorf("content = %q, want %q", got, "token: abc\n")
	}

	// Windows は Unix パーミッションを再現しないため mode 検証は Unix のみ。
	if runtime.GOOS != "windows" {
		fi, err := os.Stat(path)
		if err != nil {
			t.Fatalf("stat: %v", err)
		}
		if perm := fi.Mode().Perm(); perm != 0o600 {
			t.Errorf("perm = %04o, want 0600", perm)
		}
	}
}

// rename ベースなので、書き込み後に temp ファイルが残らないこと。
// 残っていると秘密が緩い権限で放置される恐れがある。
func TestWriteAtomicLeavesNoTempFile(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "state.json")

	if err := WriteAtomic(path, []byte("{}"), 0o600); err != nil {
		t.Fatalf("WriteAtomic: %v", err)
	}

	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatalf("readdir: %v", err)
	}
	for _, e := range entries {
		if e.Name() == "state.json" {
			continue
		}
		if strings.Contains(e.Name(), ".tmp-") {
			t.Errorf("temp file left behind: %s", e.Name())
		}
	}
	if len(entries) != 1 {
		t.Errorf("dir has %d entries, want 1", len(entries))
	}
}

// 既存ファイルは丸ごと置換される（追記や部分上書きにならない）。
func TestWriteAtomicReplacesExistingFile(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yaml")

	if err := os.WriteFile(path, []byte("old content that is long\n"), 0o600); err != nil {
		t.Fatalf("seed: %v", err)
	}
	if err := WriteAtomic(path, []byte("new\n"), 0o600); err != nil {
		t.Fatalf("WriteAtomic: %v", err)
	}

	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read back: %v", err)
	}
	if string(got) != "new\n" {
		t.Errorf("content = %q, want %q (old bytes must not survive)", got, "new\n")
	}
}

// 親ディレクトリを作れない場合（同名の通常ファイルが存在）は MkdirAll 段で失敗する。
func TestWriteAtomicMkdirFailure(t *testing.T) {
	dir := t.TempDir()
	blocker := filepath.Join(dir, "blocker")
	if err := os.WriteFile(blocker, []byte("x"), 0o600); err != nil {
		t.Fatalf("seed: %v", err)
	}

	err := WriteAtomic(filepath.Join(blocker, "child.yaml"), []byte("data"), 0o600)
	if err == nil {
		t.Fatal("WriteAtomic succeeded, want error")
	}
	if !strings.Contains(err.Error(), "securefile.WriteAtomic mkdir") {
		t.Errorf("err = %v, want wrapped mkdir error", err)
	}
}

func TestWriteAtomicEmptyData(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "empty.txt")

	if err := WriteAtomic(path, nil, 0o600); err != nil {
		t.Fatalf("WriteAtomic: %v", err)
	}
	fi, err := os.Stat(path)
	if err != nil {
		t.Fatalf("stat: %v", err)
	}
	if fi.Size() != 0 {
		t.Errorf("size = %d, want 0", fi.Size())
	}
}
