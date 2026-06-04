//go:build windows

package launcher

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
)

func TestHandleDisconnectAllProxiesKillAllThenShutdownAndCancelsOwned(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	hub := newFakeDisconnectHub(t, nil)
	defer hub.server.Close()
	if err := RegisterActiveConnection("vps", hub.server.URL+"/?token=hubtok"); err != nil {
		t.Fatal(err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	s := newTestUIServer()
	s.conns["vps"] = &liveConn{cancel: cancel}

	rr := postDisconnect(t, s, `{"name":"vps","mode":"all"}`)
	if rr.Code != http.StatusOK {
		t.Fatalf("code = %d, body = %s", rr.Code, rr.Body.String())
	}
	hub.wantPosts(t, []string{
		"/api/kill-all?token=hubtok",
		"/api/shutdown?token=hubtok",
	})
	select {
	case <-ctx.Done():
	default:
		t.Fatal("owned connection was not cancelled")
	}
	if got, err := collectActive(alwaysAlive, alwaysOK); err != nil {
		t.Fatal(err)
	} else if len(got) != 0 {
		t.Fatalf("active entries = %+v, want none after local cleanup", got)
	}
}

func TestHandleDisconnectWebProxiesShutdownOnly(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	hub := newFakeDisconnectHub(t, nil)
	defer hub.server.Close()
	if err := RegisterActiveConnection("wsl", hub.server.URL+"/?token=hubtok"); err != nil {
		t.Fatal(err)
	}

	s := newTestUIServer()
	rr := postDisconnect(t, s, `{"name":"wsl","mode":"web"}`)
	if rr.Code != http.StatusOK {
		t.Fatalf("code = %d, body = %s", rr.Code, rr.Body.String())
	}
	hub.wantPosts(t, []string{"/api/shutdown?token=hubtok"})
}

func TestHandleDisconnectRejectsInactiveProfile(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	s := newTestUIServer()
	rr := postDisconnect(t, s, `{"name":"missing","mode":"web"}`)
	if rr.Code != http.StatusNotFound {
		t.Fatalf("code = %d, want %d, body = %s", rr.Code, http.StatusNotFound, rr.Body.String())
	}
}

func TestHandleDisconnectRejectsInvalidMode(t *testing.T) {
	s := newTestUIServer()
	rr := postDisconnect(t, s, `{"name":"vps","mode":"local"}`)
	if rr.Code != http.StatusBadRequest {
		t.Fatalf("code = %d, want %d, body = %s", rr.Code, http.StatusBadRequest, rr.Body.String())
	}
}

func TestHandleDisconnectRejectsUnownedDisconnectOnly(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	hub := newFakeDisconnectHub(t, nil)
	defer hub.server.Close()
	if err := RegisterActiveConnection("tunnel", hub.server.URL+"/?token=hubtok"); err != nil {
		t.Fatal(err)
	}

	s := newTestUIServer()
	rr := postDisconnect(t, s, `{"name":"tunnel","mode":"disconnect"}`)
	if rr.Code != http.StatusBadRequest {
		t.Fatalf("code = %d, want %d, body = %s", rr.Code, http.StatusBadRequest, rr.Body.String())
	}
	hub.wantPosts(t, nil)
}

func TestHandleDisconnectContinuesShutdownWhenKillAllFails(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	hub := newFakeDisconnectHub(t, map[string]int{"/api/kill-all": http.StatusInternalServerError})
	defer hub.server.Close()
	if err := RegisterActiveConnection("vps", hub.server.URL+"/?token=hubtok"); err != nil {
		t.Fatal(err)
	}

	s := newTestUIServer()
	rr := postDisconnect(t, s, `{"name":"vps","mode":"all"}`)
	if rr.Code != http.StatusOK {
		t.Fatalf("code = %d, body = %s", rr.Code, rr.Body.String())
	}
	var body struct {
		OK      bool   `json:"ok"`
		Warning string `json:"warning"`
	}
	if err := json.Unmarshal(rr.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if !body.OK || body.Warning == "" {
		t.Fatalf("body = %+v, want ok with warning", body)
	}
	hub.wantPosts(t, []string{
		"/api/kill-all?token=hubtok",
		"/api/shutdown?token=hubtok",
	})
}

func TestHandleDisconnectCleansOwnedConnectionWhenShutdownFails(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	hub := newFakeDisconnectHub(t, map[string]int{"/api/shutdown": http.StatusBadGateway})
	defer hub.server.Close()
	if err := RegisterActiveConnection("vps", hub.server.URL+"/?token=hubtok"); err != nil {
		t.Fatal(err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	s := newTestUIServer()
	s.conns["vps"] = &liveConn{cancel: cancel}

	rr := postDisconnect(t, s, `{"name":"vps","mode":"web"}`)
	if rr.Code != http.StatusBadGateway {
		t.Fatalf("code = %d, want %d, body = %s", rr.Code, http.StatusBadGateway, rr.Body.String())
	}
	select {
	case <-ctx.Done():
	default:
		t.Fatal("owned connection was not cancelled after shutdown failure")
	}
	if got, err := collectActive(alwaysAlive, alwaysOK); err != nil {
		t.Fatal(err)
	} else if len(got) != 0 {
		t.Fatalf("active entries = %+v, want none after local cleanup", got)
	}
	hub.wantPosts(t, []string{"/api/shutdown?token=hubtok"})
}

func TestHandleProfilesMarksOwnedActiveConnections(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	hub := newFakeDisconnectHub(t, nil)
	defer hub.server.Close()
	if err := RegisterActiveConnection("owned", hub.server.URL+"/?token=hubtok"); err != nil {
		t.Fatal(err)
	}

	s := newTestUIServer()
	_, cancel := context.WithCancel(context.Background())
	defer cancel()
	s.conns["owned"] = &liveConn{cancel: cancel}

	req := httptest.NewRequest(http.MethodGet, "http://127.0.0.1/api/profiles?token=ui", nil)
	req.Host = "127.0.0.1"
	rr := httptest.NewRecorder()
	s.handleProfiles(rr, req)
	if rr.Code != http.StatusOK {
		t.Fatalf("code = %d, body = %s", rr.Code, rr.Body.String())
	}
	var body profilesResponse
	if err := json.Unmarshal(rr.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if len(body.Active) != 1 {
		t.Fatalf("active len = %d, want 1", len(body.Active))
	}
	if !body.Active[0].Owned {
		t.Fatalf("owned = false, want true: %+v", body.Active[0])
	}
}

func newTestUIServer() *UIServer {
	return &UIServer{
		token: "ui",
		conns: make(map[string]*liveConn),
	}
}

func postDisconnect(t *testing.T, s *UIServer, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, "http://127.0.0.1/api/disconnect?token=ui", strings.NewReader(body))
	req.Host = "127.0.0.1"
	rr := httptest.NewRecorder()
	s.handleDisconnect(rr, req)
	return rr
}

type fakeDisconnectHub struct {
	t        *testing.T
	server   *httptest.Server
	mu       sync.Mutex
	posts    []string
	statuses map[string]int
}

func newFakeDisconnectHub(t *testing.T, statuses map[string]int) *fakeDisconnectHub {
	t.Helper()
	f := &fakeDisconnectHub{t: t, statuses: statuses}
	f.server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/api/info":
			if r.Method != http.MethodGet {
				t.Errorf("/api/info method = %s, want GET", r.Method)
			}
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/api/kill-all", "/api/shutdown":
			if r.Method != http.MethodPost {
				t.Errorf("%s method = %s, want POST", r.URL.Path, r.Method)
			}
			f.mu.Lock()
			f.posts = append(f.posts, r.URL.Path+"?"+r.URL.RawQuery)
			f.mu.Unlock()
			if status := statuses[r.URL.Path]; status != 0 {
				w.WriteHeader(status)
				return
			}
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		default:
			http.NotFound(w, r)
		}
	}))
	return f
}

func (f *fakeDisconnectHub) wantPosts(t *testing.T, want []string) {
	t.Helper()
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.posts) != len(want) {
		t.Fatalf("posts = %+v, want %+v", f.posts, want)
	}
	for i := range want {
		if f.posts[i] != want[i] {
			t.Fatalf("posts = %+v, want %+v", f.posts, want)
		}
	}
}
