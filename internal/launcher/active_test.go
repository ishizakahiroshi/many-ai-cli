package launcher

import (
	"os"
	"testing"
	"time"
)

// alwaysAlive / alwaysDead / alwaysOK / alwaysFail are guard stubs for
// collectActive.
func alwaysAlive(int) bool   { return true }
func alwaysDead(int) bool    { return false }
func alwaysOK(string) bool   { return true }
func alwaysFail(string) bool { return false }

// --- Register / Unregister round-trip ---

func TestRegisterActiveConnectionRoundTrip(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	if err := RegisterActiveConnection("wsl-ubuntu", "http://127.0.0.1:47777/?token=aaa"); err != nil {
		t.Fatalf("RegisterActiveConnection: %v", err)
	}

	got, err := collectActive(alwaysAlive, alwaysOK)
	if err != nil {
		t.Fatalf("collectActive: %v", err)
	}
	if len(got) != 1 {
		t.Fatalf("len: got %d, want 1", len(got))
	}
	c := got[0]
	if c.Profile != "wsl-ubuntu" {
		t.Errorf("Profile: got %q", c.Profile)
	}
	if c.PID != os.Getpid() {
		t.Errorf("PID: got %d, want %d", c.PID, os.Getpid())
	}
	if c.HubURL != "http://127.0.0.1:47777/?token=aaa" {
		t.Errorf("HubURL: got %q", c.HubURL)
	}
	if c.StartedAt.IsZero() {
		t.Error("StartedAt is zero")
	}
}

func TestRegisterActiveConnectionReplacesSameProfile(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	if err := RegisterActiveConnection("vps", "http://127.0.0.1:47777/?token=old"); err != nil {
		t.Fatal(err)
	}
	if err := RegisterActiveConnection("vps", "http://127.0.0.1:47877/?token=new"); err != nil {
		t.Fatal(err)
	}

	got, err := collectActive(alwaysAlive, alwaysOK)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 {
		t.Fatalf("len: got %d, want 1 (same profile+pid must be replaced)", len(got))
	}
	if got[0].HubURL != "http://127.0.0.1:47877/?token=new" {
		t.Errorf("HubURL: got %q, want the newer URL", got[0].HubURL)
	}
}

func TestUnregisterActiveConnection(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	if err := RegisterActiveConnection("a", "http://127.0.0.1:1/?token=x"); err != nil {
		t.Fatal(err)
	}
	if err := RegisterActiveConnection("b", "http://127.0.0.1:2/?token=y"); err != nil {
		t.Fatal(err)
	}
	if err := UnregisterActiveConnection("a"); err != nil {
		t.Fatalf("UnregisterActiveConnection: %v", err)
	}

	got, err := collectActive(alwaysAlive, alwaysOK)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || got[0].Profile != "b" {
		t.Fatalf("got %+v, want only profile b", got)
	}
}

func TestUnregisterAllForPID(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	if err := RegisterActiveConnection("a", "http://127.0.0.1:1/?token=x"); err != nil {
		t.Fatal(err)
	}
	if err := RegisterActiveConnection("b", "http://127.0.0.1:2/?token=y"); err != nil {
		t.Fatal(err)
	}
	// 他プロセスのエントリは消えないこと。
	other := ActiveConnection{Profile: "other", PID: os.Getpid() + 100000, HubURL: "http://127.0.0.1:3/?token=z", StartedAt: time.Now()}
	d, err := loadActiveFile()
	if err != nil {
		t.Fatal(err)
	}
	d.Connections = append(d.Connections, other)
	if err := saveActiveFile(d); err != nil {
		t.Fatal(err)
	}

	if err := UnregisterAllForPID(); err != nil {
		t.Fatalf("UnregisterAllForPID: %v", err)
	}
	got, err := collectActive(alwaysAlive, alwaysOK)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || got[0].Profile != "other" {
		t.Fatalf("got %+v, want only the other process's entry", got)
	}
}

// --- Pruning (double guard) ---

func TestCollectActivePrunesDeadPID(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	if err := RegisterActiveConnection("dead", "http://127.0.0.1:1/?token=x"); err != nil {
		t.Fatal(err)
	}
	got, err := collectActive(alwaysDead, alwaysOK)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 0 {
		t.Fatalf("got %+v, want empty (dead PID must be pruned)", got)
	}
	// 残骸はファイルからも消えていること。
	d, err := loadActiveFile()
	if err != nil {
		t.Fatal(err)
	}
	if len(d.Connections) != 0 {
		t.Fatalf("file still has %d entries, want 0", len(d.Connections))
	}
}

func TestCollectActivePrunesUnresponsiveHub(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	// PID は生きていても Hub が応答しないエントリ（PID 再利用の巻き添え相当）
	// は除外されること = 二重ガードの2段目。
	if err := RegisterActiveConnection("zombie", "http://127.0.0.1:1/?token=x"); err != nil {
		t.Fatal(err)
	}
	got, err := collectActive(alwaysAlive, alwaysFail)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 0 {
		t.Fatalf("got %+v, want empty (unresponsive hub must be pruned)", got)
	}
}

func TestLoadActiveFileToleratesCorruptJSON(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	path, err := activePath()
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("{not json"), 0o600); err != nil {
		t.Fatal(err)
	}
	d, err := loadActiveFile()
	if err != nil {
		t.Fatalf("loadActiveFile must tolerate corruption, got error: %v", err)
	}
	if len(d.Connections) != 0 {
		t.Fatalf("got %d connections, want 0", len(d.Connections))
	}
}
