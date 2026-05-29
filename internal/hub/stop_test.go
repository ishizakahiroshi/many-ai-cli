package hub

import (
	"os"
	"path/filepath"
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
