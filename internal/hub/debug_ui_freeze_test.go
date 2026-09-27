//go:build maidebug

package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func uiFreezeTestBody() string {
	return `{"schema":1,"tab":"12345678-1234-1234-1234-123456789abc","reason":"suspected-stall","at":1000,"gapMs":5000,"visible":true,"dropped":0,"events":[{"id":1,"phase":"voice.result","edge":"begin","at":900,"size":12}],"open":[{"id":1,"phase":"voice.result","edge":"begin","at":900,"size":12}]}`
}

func TestUIFreezeCapture(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "test-auth"
	s.cfg.Hub.LogDir = t.TempDir()
	s.cfg.Log.Enabled = false
	s.version, s.gitCommit = "test-version", "test-commit"
	handler := newUIFreezeHandler(s)
	request := func(body, token, origin string) *httptest.ResponseRecorder {
		r := httptest.NewRequest(http.MethodPost, "http://127.0.0.1:47777/api/debug/ui-freeze", strings.NewReader(body))
		if token != "" {
			r.Header.Set("Authorization", "Bearer "+token)
		}
		if origin != "" {
			r.Header.Set("Origin", origin)
		}
		w := httptest.NewRecorder()
		handler(w, r)
		return w
	}
	if w := request(uiFreezeTestBody(), "", ""); w.Code != http.StatusUnauthorized {
		t.Fatalf("unauthenticated: %d", w.Code)
	}
	if w := request(uiFreezeTestBody(), "test-auth", "https://other.example"); w.Code != http.StatusForbidden {
		t.Fatalf("cross-origin: %d", w.Code)
	}
	for _, body := range []string{
		strings.Replace(uiFreezeTestBody(), `"schema":1`, `"transcript":"private text","schema":1`, 1),
		strings.Replace(uiFreezeTestBody(), `"size":12`, `"text":"private text","size":12`, 1),
		strings.Replace(uiFreezeTestBody(), "voice.result", "arbitrary private text", 1),
		strings.Replace(uiFreezeTestBody(), "suspected-stall", "private error", 1),
		strings.Replace(uiFreezeTestBody(), `"size":12`, `"size":-1`, 1),
		uiFreezeTestBody() + `{}`,
		strings.Repeat(" ", 16385) + uiFreezeTestBody(),
	} {
		if w := request(body, "test-auth", ""); w.Code != http.StatusBadRequest {
			t.Fatalf("invalid capture accepted: %d", w.Code)
		}
	}
	path := filepath.Join(s.cfg.Hub.LogDir, "ui-freeze.jsonl")
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatal("rejected requests must not create a log")
	}
	if w := request(uiFreezeTestBody(), "test-auth", ""); w.Code != http.StatusNoContent {
		t.Fatalf("capture: %d %s", w.Code, w.Body.String())
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var record map[string]any
	if err := json.Unmarshal(data, &record); err != nil {
		t.Fatal(err)
	}
	if record["reason"] != "suspected-stall" || record["hubCommit"] != "test-commit" || record["receivedAt"] == nil {
		t.Fatal("missing diagnostic context")
	}
	if strings.Contains(string(data), "test-auth") || strings.Contains(string(data), "private text") {
		t.Fatal("private input leaked")
	}
	if w := request(uiFreezeTestBody(), "test-auth", ""); w.Code != http.StatusTooManyRequests {
		t.Fatalf("write limit: %d", w.Code)
	}
}

func TestUIFreezeBoundsAndRegistration(t *testing.T) {
	if probeRoutes["/api/debug/ui-freeze"] == nil {
		t.Fatal("missing debug route")
	}
	var p uiFreezePacket
	if err := json.Unmarshal([]byte(uiFreezeTestBody()), &p); err != nil {
		t.Fatal(err)
	}
	p.Events = make([]uiFreezeEvent, 65)
	if validUIFreezePacket(p) {
		t.Fatal("unbounded events")
	}
	p.Events = nil
	p.Open = make([]uiFreezeEvent, 17)
	if validUIFreezePacket(p) {
		t.Fatal("unbounded open spans")
	}
}
