package hub

import (
	"os"
	"path/filepath"
	"strconv"
	"testing"
)

func TestParseHubPIDRejectsInvalidValues(t *testing.T) {
	for _, input := range []string{"", "not-a-pid", "0", "-12", "123abc"} {
		if pid, err := parseHubPID([]byte(input)); err == nil {
			t.Fatalf("parseHubPID(%q) = %d, nil error; want error", input, pid)
		}
	}
}

func TestParseHubPIDAcceptsTrimmedPositivePID(t *testing.T) {
	pid, err := parseHubPID([]byte(" 12345\n"))
	if err != nil {
		t.Fatalf("parseHubPID returned error: %v", err)
	}
	if pid != 12345 {
		t.Fatalf("parseHubPID = %d, want 12345", pid)
	}
}

func TestStopWithPIDPathInvalidPIDRemovesFile(t *testing.T) {
	pidPath := filepath.Join(t.TempDir(), "any-ai-cli.pid")
	if err := os.WriteFile(pidPath, []byte("not-a-pid"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := stopWithPIDPath(pidPath); err == nil {
		t.Fatal("stopWithPIDPath returned nil error for invalid pid")
	}
	if _, err := os.Stat(pidPath); !os.IsNotExist(err) {
		t.Fatalf("pid file still exists after invalid stop: %v", err)
	}
}

// killStalePid は自分自身の PID を kill してはならない。コンテナでは Hub が
// 毎回同じ PID で起動するため、前回 boot の PID ファイルが自分の PID と一致
// しうる（一致時に kill すると起動直後の自殺ループになる）。
func TestKillStalePid_SelfPIDGuard(t *testing.T) {
	pidPath := filepath.Join(t.TempDir(), "any-ai-cli.pid")
	if err := os.WriteFile(pidPath, []byte(strconv.Itoa(os.Getpid())), 0o600); err != nil {
		t.Fatal(err)
	}
	// 自分の PID が書かれていても生きて戻ってくること（kill されたらテストごと死ぬ）
	killStalePid(pidPath)
	if _, err := os.Stat(pidPath); !os.IsNotExist(err) {
		t.Errorf("pid file should be removed, stat err=%v", err)
	}
}

// 不正な内容でもファイルは必ず除去される（除去が kill より先なら自殺時も残らない）。
func TestKillStalePid_RemovesInvalidFile(t *testing.T) {
	pidPath := filepath.Join(t.TempDir(), "any-ai-cli.pid")
	if err := os.WriteFile(pidPath, []byte("garbage"), 0o600); err != nil {
		t.Fatal(err)
	}
	killStalePid(pidPath)
	if _, err := os.Stat(pidPath); !os.IsNotExist(err) {
		t.Errorf("pid file should be removed, stat err=%v", err)
	}
}
