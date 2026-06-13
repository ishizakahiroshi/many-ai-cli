package wrapper

import (
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
)

func TestReconnectGraceClampsNegative(t *testing.T) {
	cfg := &config.Config{}
	cfg.Hub.WrapperReconnectGraceSec = -10
	if got := reconnectGrace(cfg); got != 0 {
		t.Fatalf("reconnectGrace = %v, want 0", got)
	}

	cfg.Hub.WrapperReconnectGraceSec = 3
	if got := reconnectGrace(cfg); got != 3*time.Second {
		t.Fatalf("reconnectGrace = %v, want 3s", got)
	}
}

func TestFindFreePortSkipsOccupiedPreferred(t *testing.T) {
	preferred := findFreePort(47777)
	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", preferred))
	if err != nil {
		t.Skipf("preferred port became unavailable before test bind: %v", err)
	}
	defer ln.Close()

	got := findFreePort(preferred)
	if got == preferred {
		t.Fatalf("findFreePort returned occupied preferred port %d", preferred)
	}
	if got < preferred || got >= preferred+100 {
		t.Fatalf("findFreePort = %d, want in [%d,%d)", got, preferred, preferred+100)
	}
}

func TestProbeHubAlive(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("token") != "tok" {
			t.Fatalf("token = %q, want tok", r.URL.Query().Get("token"))
		}
		w.WriteHeader(http.StatusOK)
	}))
	defer ts.Close()

	cfg := &config.Config{}
	cfg.Token = "tok"
	cfg.Hub.Port = mustServerPort(t, ts)

	if !probeHubAlive(cfg) {
		t.Fatal("probeHubAlive returned false for healthy hub")
	}
}

func TestProbeHubAliveRejectsNonOK(t *testing.T) {
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
	}))
	defer ts.Close()

	cfg := &config.Config{}
	cfg.Token = "tok"
	cfg.Hub.Port = mustServerPort(t, ts)

	if probeHubAlive(cfg) {
		t.Fatal("probeHubAlive returned true for non-OK status")
	}
}

func mustServerPort(t *testing.T, ts *httptest.Server) int {
	t.Helper()
	raw := strings.TrimPrefix(ts.URL, "http://")
	_, port, err := net.SplitHostPort(raw)
	if err != nil {
		t.Fatalf("SplitHostPort(%q): %v", raw, err)
	}
	n, err := strconv.Atoi(port)
	if err != nil {
		t.Fatalf("Atoi(%q): %v", port, err)
	}
	if n <= 0 {
		t.Fatalf("invalid port: %d", n)
	}
	return n
}
