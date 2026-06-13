package hub

import (
	"os"
	"path/filepath"
	"testing"

	"many-ai-cli/internal/config"
)

// setTempHome は HOME / USERPROFILE を一時ディレクトリへ向け、
// hub-runtime.json の読み書きをテスト内に隔離する。
func setTempHome(t *testing.T) string {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	return home
}

func TestHubRuntimeRoundtrip(t *testing.T) {
	setTempHome(t)

	if err := writeHubRuntime(47778); err != nil {
		t.Fatalf("writeHubRuntime: %v", err)
	}
	rt, err := readHubRuntime()
	if err != nil {
		t.Fatalf("readHubRuntime: %v", err)
	}
	if rt == nil {
		t.Fatal("readHubRuntime returned nil after write")
	}
	if rt.Port != 47778 {
		t.Errorf("Port = %d, want 47778", rt.Port)
	}
	if rt.PID != os.Getpid() {
		t.Errorf("PID = %d, want %d", rt.PID, os.Getpid())
	}
	if rt.StartedAt.IsZero() {
		t.Error("StartedAt should be set")
	}
}

func TestHubRuntimeReadMissing(t *testing.T) {
	setTempHome(t)

	rt, err := readHubRuntime()
	if err != nil {
		t.Fatalf("readHubRuntime: %v", err)
	}
	if rt != nil {
		t.Errorf("missing file should yield nil, got %+v", rt)
	}
}

func TestHubRuntimeReadCorrupt(t *testing.T) {
	home := setTempHome(t)

	dir := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, hubRuntimeFile), []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}

	rt, err := readHubRuntime()
	if err != nil {
		t.Fatalf("readHubRuntime: %v", err)
	}
	if rt != nil {
		t.Errorf("corrupt file should yield nil, got %+v", rt)
	}
}

func TestRemoveHubRuntimeIfPID(t *testing.T) {
	setTempHome(t)

	if err := writeHubRuntime(47778); err != nil {
		t.Fatalf("writeHubRuntime: %v", err)
	}

	// 別 PID の指定では消えない（新しい Hub の記録を古い Hub が消さない）。
	removeHubRuntimeIfPID(os.Getpid() + 1)
	if rt, _ := readHubRuntime(); rt == nil {
		t.Fatal("file removed by foreign PID")
	}

	// 自 PID なら消える。
	removeHubRuntimeIfPID(os.Getpid())
	if rt, _ := readHubRuntime(); rt != nil {
		t.Fatal("file should be removed for own PID")
	}
}

func testConfig(port int) *config.Config {
	cfg := &config.Config{Token: "test-token"}
	cfg.Hub.Port = port
	return cfg
}

func TestRunningHubPortConfiguredPort(t *testing.T) {
	setTempHome(t)
	cfg := testConfig(47777)

	port, ok := runningHubPortWith(cfg,
		func(int) bool { t.Fatal("alive should not be called"); return false },
		func(p int) bool { return p == 47777 })
	if !ok || port != 47777 {
		t.Fatalf("got (%d, %v), want (47777, true)", port, ok)
	}
}

func TestRunningHubPortFromRuntimeFile(t *testing.T) {
	setTempHome(t)
	cfg := testConfig(47777)
	if err := writeHubRuntime(47778); err != nil {
		t.Fatal(err)
	}

	port, ok := runningHubPortWith(cfg,
		func(int) bool { return true },
		func(p int) bool { return p == 47778 }) // 設定ポートは不応答、退避先のみ応答
	if !ok || port != 47778 {
		t.Fatalf("got (%d, %v), want (47778, true)", port, ok)
	}
}

func TestRunningHubPortDeadPIDPrunes(t *testing.T) {
	setTempHome(t)
	cfg := testConfig(47777)
	if err := writeHubRuntime(47778); err != nil {
		t.Fatal(err)
	}

	_, ok := runningHubPortWith(cfg,
		func(int) bool { return false }, // PID 死亡
		func(p int) bool { return p == 47778 })
	if ok {
		t.Fatal("dead PID entry should not be reported as running")
	}
	if rt, _ := readHubRuntime(); rt != nil {
		t.Fatal("dead PID entry should be pruned")
	}
}

func TestRunningHubPortProbeFailKeepsFile(t *testing.T) {
	setTempHome(t)
	cfg := testConfig(47777)
	if err := writeHubRuntime(47778); err != nil {
		t.Fatal(err)
	}

	_, ok := runningHubPortWith(cfg,
		func(int) bool { return true },  // PID は生存
		func(int) bool { return false }) // 一時的に無応答
	if ok {
		t.Fatal("unreachable Hub should not be reported as running")
	}
	if rt, _ := readHubRuntime(); rt == nil {
		t.Fatal("live-PID entry should be kept on transient probe failure")
	}
}

func TestRunningHubPortSamePortNotReprobed(t *testing.T) {
	setTempHome(t)
	cfg := testConfig(47777)
	if err := writeHubRuntime(47777); err != nil { // 退避なし＝設定ポートと同じ
		t.Fatal(err)
	}

	probes := 0
	_, ok := runningHubPortWith(cfg,
		func(int) bool { return true },
		func(int) bool { probes++; return false })
	if ok {
		t.Fatal("should not be running")
	}
	if probes != 1 {
		t.Fatalf("configured port should be probed exactly once, got %d", probes)
	}
}

func TestRunningHubPortNoFile(t *testing.T) {
	setTempHome(t)
	cfg := testConfig(47777)

	_, ok := runningHubPortWith(cfg,
		func(int) bool { return true },
		func(int) bool { return false })
	if ok {
		t.Fatal("no file and no probe response should mean not running")
	}
}
