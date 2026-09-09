package wrapper

import (
	"bytes"
	"encoding/json"
	"io"
	"log/slog"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func quietLogger() *slog.Logger {
	return slog.New(slog.NewTextHandler(io.Discard, nil))
}

// exitedPID は実際に子プロセスを起こして終了を待ち、その PID を返す。
// 「絶対に生きていない PID」を定数で決め打ちすると OS ごとの PID 空間に依存するため、
// テストバイナリ自身を 1 件もテストを選ばない引数で起動して使い捨てる。
func exitedPID(t *testing.T) int {
	t.Helper()
	cmd := exec.Command(os.Args[0], "-test.run=^$")
	cmd.Stdout = io.Discard
	cmd.Stderr = io.Discard
	if err := cmd.Start(); err != nil {
		t.Fatalf("start helper process: %v", err)
	}
	pid := cmd.Process.Pid
	_ = cmd.Wait()
	if processAlive(pid) {
		t.Skipf("helper pid %d still reported alive; cannot synthesize a dead pid here", pid)
	}
	return pid
}

func readOpenCodeConfigPermission(t *testing.T, path string) string {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	var parsed struct {
		Permission map[string]string `json:"permission"`
	}
	if err := json.Unmarshal(data, &parsed); err != nil {
		t.Fatalf("unmarshal %s: %v", path, err)
	}
	return parsed.Permission["*"]
}

func TestPrepareOpenCodeConfigCreatesAndRemoves(t *testing.T) {
	cwd := t.TempDir()
	cfgPath := filepath.Join(cwd, "opencode.json")

	cleanup, err := prepareOpenCodeConfig(cwd, "ask", quietLogger())
	if err != nil {
		t.Fatalf("prepareOpenCodeConfig: %v", err)
	}
	if got := readOpenCodeConfigPermission(t, cfgPath); got != "ask" {
		t.Fatalf(`permission["*"] = %q, want "ask"`, got)
	}

	cleanup()
	if _, err := os.Stat(cfgPath); !os.IsNotExist(err) {
		t.Fatalf("opencode.json should be removed after cleanup, stat err = %v", err)
	}
	if _, err := os.Stat(cfgPath + ".many-ai-cli.lock"); !os.IsNotExist(err) {
		t.Fatalf("lock should be removed after cleanup, stat err = %v", err)
	}
}

func TestPrepareOpenCodeConfigRestoresExisting(t *testing.T) {
	cwd := t.TempDir()
	cfgPath := filepath.Join(cwd, "opencode.json")
	original := []byte(`{"permission":{"edit":"deny"}}`)
	if err := os.WriteFile(cfgPath, original, 0o600); err != nil {
		t.Fatalf("seed config: %v", err)
	}

	cleanup, err := prepareOpenCodeConfig(cwd, "allow", quietLogger())
	if err != nil {
		t.Fatalf("prepareOpenCodeConfig: %v", err)
	}
	if got := readOpenCodeConfigPermission(t, cfgPath); got != "allow" {
		t.Fatalf(`permission["*"] = %q, want "allow"`, got)
	}

	cleanup()
	got, err := os.ReadFile(cfgPath)
	if err != nil {
		t.Fatalf("read restored config: %v", err)
	}
	if string(got) != string(original) {
		t.Fatalf("restored config = %s, want %s", got, original)
	}
}

func TestPrepareOpenCodeConfigPreservesConcurrentEdit(t *testing.T) {
	cwd := t.TempDir()
	cfgPath := filepath.Join(cwd, "opencode.json")
	original := []byte(`{"permission":{"edit":"deny"}}`)
	if err := os.WriteFile(cfgPath, original, 0o600); err != nil {
		t.Fatalf("seed config: %v", err)
	}

	cleanup, err := prepareOpenCodeConfig(cwd, "allow", quietLogger())
	if err != nil {
		t.Fatalf("prepareOpenCodeConfig: %v", err)
	}
	edited := []byte(`{"permission":{"edit":"allow"},"model":"local"}`)
	if err := os.WriteFile(cfgPath, edited, 0o600); err != nil {
		t.Fatalf("edit config: %v", err)
	}
	cleanup()

	got, err := os.ReadFile(cfgPath)
	if err != nil {
		t.Fatalf("read edited config: %v", err)
	}
	if !bytes.Equal(got, edited) {
		t.Fatalf("cleanup overwrote concurrent edit: got %s, want %s", got, edited)
	}
}

// TestPrepareOpenCodeConfigRecoversOrphan は「セッションが cleanup を通らずに死に、
// 書き換え後の opencode.json が cwd に置き去りになった」状態からの回収を確かめる。
// 修正前は次のセッションが置き去りをオリジナルとして採用し、cleanup で書き戻すため
// permission の上書きがリポジトリに永久に残った。
func TestPrepareOpenCodeConfigRecoversOrphan(t *testing.T) {
	cwd := t.TempDir()
	cfgPath := filepath.Join(cwd, "opencode.json")
	lockPath := cfgPath + ".many-ai-cli.lock"

	// 1st セッション: 設定を作るところまでは通るが cleanup は呼ばない（kill 相当）。
	if _, err := prepareOpenCodeConfig(cwd, "allow", quietLogger()); err != nil {
		t.Fatalf("first prepareOpenCodeConfig: %v", err)
	}
	// ロックの保持 PID を「終了済みプロセス」に差し替えて死んだセッションを再現する。
	var st openCodeLockState
	lockBytes, err := os.ReadFile(lockPath)
	if err != nil {
		t.Fatalf("read lock: %v", err)
	}
	if err := json.Unmarshal(lockBytes, &st); err != nil {
		t.Fatalf("unmarshal lock: %v", err)
	}
	if st.Rollback == nil {
		t.Fatal("lock should carry rollback state after the config was written")
	}
	st.PID = exitedPID(t)
	rewritten, err := json.Marshal(st)
	if err != nil {
		t.Fatalf("marshal lock: %v", err)
	}
	if err := os.WriteFile(lockPath, rewritten, 0o600); err != nil {
		t.Fatalf("rewrite lock: %v", err)
	}

	// 2nd セッション: 置き去りを回収してから "ask" を書く。
	cleanup, err := prepareOpenCodeConfig(cwd, "ask", quietLogger())
	if err != nil {
		t.Fatalf("second prepareOpenCodeConfig: %v", err)
	}
	if got := readOpenCodeConfigPermission(t, cfgPath); got != "ask" {
		t.Fatalf(`permission["*"] = %q, want "ask"`, got)
	}

	cleanup()
	if _, err := os.Stat(cfgPath); !os.IsNotExist(err) {
		t.Fatalf("orphaned opencode.json should be gone after recovery + cleanup, stat err = %v", err)
	}
}

func TestRecoverOrphanedOpenCodeConfig(t *testing.T) {
	written := []byte(`{"permission":{"*":"allow"}}`)
	original := []byte(`{"permission":{"edit":"deny"}}`)

	t.Run("removes config the dead session created", func(t *testing.T) {
		cwd := t.TempDir()
		cfgPath := filepath.Join(cwd, "opencode.json")
		if err := os.WriteFile(cfgPath, written, 0o600); err != nil {
			t.Fatalf("seed: %v", err)
		}
		st := openCodeLockState{PID: 1, Rollback: &openCodeRollback{ConfigExisted: false, Written: written}}
		recoverOrphanedOpenCodeConfig(cfgPath, st, quietLogger())
		if _, err := os.Stat(cfgPath); !os.IsNotExist(err) {
			t.Fatalf("config should be removed, stat err = %v", err)
		}
	})

	t.Run("restores config the dead session overwrote", func(t *testing.T) {
		cwd := t.TempDir()
		cfgPath := filepath.Join(cwd, "opencode.json")
		if err := os.WriteFile(cfgPath, written, 0o600); err != nil {
			t.Fatalf("seed: %v", err)
		}
		st := openCodeLockState{PID: 1, Rollback: &openCodeRollback{ConfigExisted: true, Original: original, Written: written}}
		recoverOrphanedOpenCodeConfig(cfgPath, st, quietLogger())
		got, err := os.ReadFile(cfgPath)
		if err != nil {
			t.Fatalf("read: %v", err)
		}
		if string(got) != string(original) {
			t.Fatalf("config = %s, want %s", got, original)
		}
	})

	t.Run("keeps a config that changed after the dead session", func(t *testing.T) {
		cwd := t.TempDir()
		cfgPath := filepath.Join(cwd, "opencode.json")
		edited := []byte(`{"permission":{"*":"allow"},"model":"local"}`)
		if err := os.WriteFile(cfgPath, edited, 0o600); err != nil {
			t.Fatalf("seed: %v", err)
		}
		st := openCodeLockState{PID: 1, Rollback: &openCodeRollback{ConfigExisted: false, Written: written}}
		recoverOrphanedOpenCodeConfig(cfgPath, st, quietLogger())
		got, err := os.ReadFile(cfgPath)
		if err != nil {
			t.Fatalf("read: %v", err)
		}
		if string(got) != string(edited) {
			t.Fatalf("config = %s, want it untouched (%s)", got, edited)
		}
	})

	t.Run("no rollback state means no action", func(t *testing.T) {
		cwd := t.TempDir()
		cfgPath := filepath.Join(cwd, "opencode.json")
		if err := os.WriteFile(cfgPath, written, 0o600); err != nil {
			t.Fatalf("seed: %v", err)
		}
		recoverOrphanedOpenCodeConfig(cfgPath, openCodeLockState{PID: 1}, quietLogger())
		if _, err := os.Stat(cfgPath); err != nil {
			t.Fatalf("config should be untouched, stat err = %v", err)
		}
	})
}

func TestReadOpenCodeLockAcceptsLegacyPIDFormat(t *testing.T) {
	cwd := t.TempDir()
	lockPath := filepath.Join(cwd, "opencode.json.many-ai-cli.lock")
	if err := os.WriteFile(lockPath, []byte("4321\n"), 0o600); err != nil {
		t.Fatalf("seed lock: %v", err)
	}
	st, ok := readOpenCodeLock(lockPath)
	if !ok {
		t.Fatal("legacy pid-only lock should parse")
	}
	if st.PID != 4321 {
		t.Fatalf("pid = %d, want 4321", st.PID)
	}
	if st.Rollback != nil {
		t.Fatal("legacy lock carries no rollback state")
	}
}

func TestPrepareOpenCodeConfigSkipsWhenLockedByLiveSession(t *testing.T) {
	cwd := t.TempDir()
	cfgPath := filepath.Join(cwd, "opencode.json")
	lockPath := cfgPath + ".many-ai-cli.lock"
	live, err := json.Marshal(openCodeLockState{PID: os.Getpid()})
	if err != nil {
		t.Fatalf("marshal lock: %v", err)
	}
	if err := os.WriteFile(lockPath, live, 0o600); err != nil {
		t.Fatalf("seed lock: %v", err)
	}

	if _, err := prepareOpenCodeConfig(cwd, "ask", quietLogger()); err == nil {
		t.Fatal("prepareOpenCodeConfig should fail while another live session holds the lock")
	}
	if _, err := os.Stat(cfgPath); !os.IsNotExist(err) {
		t.Fatalf("config must not be written while locked, stat err = %v", err)
	}
}
