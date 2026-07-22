package hub

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"testing"
	"time"
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
	pidPath := filepath.Join(t.TempDir(), "many-ai-cli.pid")
	if err := os.WriteFile(pidPath, []byte("not-a-pid"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := stopWithPIDPath(pidPath, nil, 0, ""); err == nil {
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
	pidPath := filepath.Join(t.TempDir(), "many-ai-cli.pid")
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
	pidPath := filepath.Join(t.TempDir(), "many-ai-cli.pid")
	if err := os.WriteFile(pidPath, []byte("garbage"), 0o600); err != nil {
		t.Fatal(err)
	}
	killStalePid(pidPath)
	if _, err := os.Stat(pidPath); !os.IsNotExist(err) {
		t.Errorf("pid file should be removed, stat err=%v", err)
	}
}

// gracefulStop はポートが不明（<=0）なら HTTP を叩かず即 false を返し、
// stopWithPIDPath 側の force-kill フォールバックに委ねる
//（plan_hub-lifecycle-logging.md C2）。
func TestGracefulStop_NoPortFallsBackImmediately(t *testing.T) {
	if gracefulStop(12345, 0, "tok", nil) {
		t.Fatal("gracefulStop with port<=0 should return false")
	}
}

// /api/shutdown 相当が 200 以外を返したら gracefulStop は false を返す
//（force kill へのフォールバック対象）。
func TestGracefulStop_NonOKStatusFallsBack(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
	}))
	defer srv.Close()

	u, err := url.Parse(srv.URL)
	if err != nil {
		t.Fatalf("parse test server URL: %v", err)
	}
	port, err := strconv.Atoi(u.Port())
	if err != nil {
		t.Fatalf("parse test server port: %v", err)
	}

	if gracefulStop(os.Getpid(), port, "tok", nil) {
		t.Fatal("gracefulStop should return false on non-200 response")
	}
}

// waitForPIDExit は生存し続けるプロセス（自分自身）に対しては、タイムアウトまで
// ポーリングして false を返す。
func TestWaitForPIDExit_TimesOutWhenProcessStillAlive(t *testing.T) {
	if waitForPIDExit(os.Getpid(), 150*time.Millisecond, 20*time.Millisecond) {
		t.Fatal("waitForPIDExit should be false for a still-alive process")
	}
}

// waitForPIDExit は既に終了した PID に対しては即 true を返す。
func TestWaitForPIDExit_TrueForAlreadyExitedProcess(t *testing.T) {
	cmd := exec.Command(os.Args[0], "-test.run=TestHelperProcessNoop")
	cmd.Env = append(os.Environ(), "GO_WANT_HELPER_PROCESS=1")
	if err := cmd.Start(); err != nil {
		t.Fatalf("start helper process: %v", err)
	}
	pid := cmd.Process.Pid
	if err := cmd.Wait(); err != nil {
		t.Fatalf("wait helper process: %v", err)
	}
	if !waitForPIDExit(pid, gracefulShutdownPoll, 10*time.Millisecond) {
		t.Fatal("waitForPIDExit should be true for an already-exited pid")
	}
}

// TestHelperProcessNoop is not a real test — it is exec'd by
// TestWaitForPIDExit_TrueForAlreadyExitedProcess as a disposable child
// process that exits immediately (standard os/exec test helper pattern).
func TestHelperProcessNoop(t *testing.T) {
	if os.Getenv("GO_WANT_HELPER_PROCESS") != "1" {
		return
	}
	os.Exit(0)
}
