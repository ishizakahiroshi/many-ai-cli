package hub

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// TestDownloadFileRejectsNonHTTPS は downloadFile が非 https URL を
// ネットワークに触れる前に拒否することを確認する（最小ハードニング, finding #15）。
func TestDownloadFileRejectsNonHTTPS(t *testing.T) {
	dest := filepath.Join(t.TempDir(), "model.bin")
	for _, url := range []string{
		"http://example.com/ggml-small.bin",
		"ftp://example.com/ggml-small.bin",
		"file:///etc/passwd",
	} {
		err := downloadFile(context.Background(), url, dest, "", nil)
		if err == nil {
			t.Fatalf("downloadFile(%q) = nil, want non-https rejection error", url)
		}
		if !strings.Contains(err.Error(), "non-https") {
			t.Fatalf("downloadFile(%q) err = %v, want non-https message", url, err)
		}
	}
}

// TestWhisperDownloadWatchdogTrips は進捗が途絶えたとき onStall が一度だけ
// 発火し stalled() が true を返すことを確認する（恒久ハング防止, finding #6）。
func TestWhisperDownloadWatchdogTrips(t *testing.T) {
	var mu sync.Mutex
	calls := 0
	done := make(chan struct{})
	w := whisperNewDownloadWatchdog(15*time.Millisecond, func() {
		mu.Lock()
		calls++
		if calls == 1 {
			close(done)
		}
		mu.Unlock()
	})
	defer w.stop()

	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("watchdog did not trip within timeout")
	}
	if !w.stalled() {
		t.Fatal("stalled() = false after onStall fired, want true")
	}
	// 発火後 stop しても onStall は重複しない。
	w.stop()
	time.Sleep(30 * time.Millisecond)
	mu.Lock()
	got := calls
	mu.Unlock()
	if got != 1 {
		t.Fatalf("onStall called %d times, want exactly 1", got)
	}
}

// TestWhisperDownloadWatchdogTickResets は tick() が来続ける限り stall しないこと、
// tick を止めると最終的に stall することを確認する。
func TestWhisperDownloadWatchdogTickResets(t *testing.T) {
	var mu sync.Mutex
	fired := false
	w := whisperNewDownloadWatchdog(40*time.Millisecond, func() {
		mu.Lock()
		fired = true
		mu.Unlock()
	})
	defer w.stop()

	// 進捗を刻み続ける間は stall させない。
	for i := 0; i < 6; i++ {
		time.Sleep(10 * time.Millisecond)
		w.tick()
	}
	mu.Lock()
	earlyFired := fired
	mu.Unlock()
	if earlyFired {
		t.Fatal("watchdog tripped while ticking, want no stall")
	}
	if w.stalled() {
		t.Fatal("stalled() = true while ticking, want false")
	}

	// tick を止めれば timeout 後に stall する。
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		mu.Lock()
		f := fired
		mu.Unlock()
		if f {
			break
		}
		time.Sleep(10 * time.Millisecond)
	}
	if !w.stalled() {
		t.Fatal("watchdog never tripped after ticks stopped, want stall")
	}
}

// TestWhisperDownloadClientNoGlobalTimeout は専用クライアントが全体 Timeout を
// 持たない（488MB の正常な低速転送を切らない）ことを確認する（finding #6 注記）。
func TestWhisperDownloadClientNoGlobalTimeout(t *testing.T) {
	if whisperDownloadClient.Timeout != 0 {
		t.Fatalf("whisperDownloadClient.Timeout = %v, want 0 (no global timeout)", whisperDownloadClient.Timeout)
	}
}

// TestDownloadFileHashMatch はハッシュが一致するとき downloadFile が成功し、
// 宛先ファイルが作成されることを確認する（finding #15 回帰テスト）。
func TestDownloadFileHashMatch(t *testing.T) {
	content := []byte("hello whisper model")
	h := sha256.Sum256(content)
	correctHex := hex.EncodeToString(h[:])

	srv := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write(content)
	}))
	defer srv.Close()

	origClient := whisperDownloadClient
	whisperDownloadClient = srv.Client()
	defer func() { whisperDownloadClient = origClient }()

	dest := filepath.Join(t.TempDir(), "model.bin")
	err := downloadFile(context.Background(), srv.URL, dest, correctHex, nil)
	if err != nil {
		t.Fatalf("downloadFile with correct sha256 returned error: %v", err)
	}
	if _, err := os.Stat(dest); err != nil {
		t.Fatalf("dest file not created after successful download: %v", err)
	}
	// tmp ファイルが残っていないことも確認。
	if _, err := os.Stat(dest + ".download"); err == nil {
		t.Fatal("tmp .download file leaked after successful download")
	}
}

// TestDownloadFileHashMismatch はハッシュが不一致のとき downloadFile が失敗し、
// tmp ファイルが残らないことを確認する（finding #15 回帰テスト）。
func TestDownloadFileHashMismatch(t *testing.T) {
	content := []byte("hello whisper model")
	wrongHex := strings.Repeat("0", 64)

	srv := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write(content)
	}))
	defer srv.Close()

	origClient := whisperDownloadClient
	whisperDownloadClient = srv.Client()
	defer func() { whisperDownloadClient = origClient }()

	dest := filepath.Join(t.TempDir(), "model.bin")
	err := downloadFile(context.Background(), srv.URL, dest, wrongHex, nil)
	if err == nil {
		t.Fatal("downloadFile with wrong sha256 returned nil, want error")
	}
	if !strings.Contains(err.Error(), "sha256 mismatch") {
		t.Fatalf("error %q does not contain 'sha256 mismatch'", err.Error())
	}
	// 宛先ファイルは作られていない。
	if _, err := os.Stat(dest); err == nil {
		t.Fatal("dest file exists after hash mismatch, want no file")
	}
	// tmp ファイルも残っていない。
	if _, err := os.Stat(dest + ".download"); err == nil {
		t.Fatal("tmp .download file leaked after hash mismatch")
	}
}

// TestWhisperModelOptionsSHA256Set は whisperModelOptions の全エントリに
// SHA256 が設定されていることを確認する（finding #15: manifest 完全性）。
func TestWhisperModelOptionsSHA256Set(t *testing.T) {
	for _, opt := range whisperModelOptions {
		if opt.SHA256 == "" {
			t.Errorf("whisperModelOptions[%q].SHA256 is empty; add the HF LFS oid before shipping", opt.ID)
		}
		if len(opt.SHA256) != 64 {
			t.Errorf("whisperModelOptions[%q].SHA256 = %q, want 64-char hex string", opt.ID, opt.SHA256)
		}
	}
}
