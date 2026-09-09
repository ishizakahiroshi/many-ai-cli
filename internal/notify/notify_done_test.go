package notify

import (
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

// captured は webhook backend が受け取った 1 通分の本文。
type captured struct {
	body string
}

// newCapturingBackend は受信内容を channel に流す webhook backend を立てる。
func newCapturingBackend(t *testing.T) (BackendConfig, <-chan captured) {
	t.Helper()
	ch := make(chan captured, 4)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		ch <- captured{body: string(b)}
		w.WriteHeader(http.StatusOK)
	}))
	t.Cleanup(srv.Close)
	return BackendConfig{Type: "webhook", URL: srv.URL}, ch
}

func waitCaptured(t *testing.T, ch <-chan captured) captured {
	t.Helper()
	select {
	case c := <-ch:
		return c
	case <-time.After(3 * time.Second):
		t.Fatal("timeout waiting for notification")
		return captured{}
	}
}

// 既定（include_body 未設定）では完了サマリー本文を外部へ送らない。
func TestSendDoneOmitsSummaryByDefault(t *testing.T) {
	backend, ch := newCapturingBackend(t)
	m := New(Config{
		Backends: []BackendConfig{backend},
		Events:   []string{"done"},
	}, slog.New(slog.NewTextHandler(io.Discard, nil)))

	m.SendDone(DonePayload{
		SessionID: 7,
		Provider:  "claude",
		Title:     "task",
		Summary:   "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY を消しました",
		Kind:      "success",
	})

	got := waitCaptured(t, ch)
	if strings.Contains(got.body, "wJalrXUtnFEMI") {
		t.Errorf("summary leaked into notification body:\n%s", got.body)
	}
	if !strings.Contains(got.body, "session #7") {
		t.Errorf("body should still identify the session:\n%s", got.body)
	}
	if !strings.Contains(got.body, "成功") {
		t.Errorf("body should carry the kind label:\n%s", got.body)
	}
}

// include_body 有効時は本文を送るが、シークレットはマスクを通す。
func TestSendDoneMasksSummaryWhenIncludeBody(t *testing.T) {
	backend, ch := newCapturingBackend(t)
	m := New(Config{
		Backends:    []BackendConfig{backend},
		Events:      []string{"done"},
		IncludeBody: true,
	}, slog.New(slog.NewTextHandler(io.Discard, nil)))

	m.SendDone(DonePayload{
		SessionID: 9,
		Title:     "task",
		Summary:   "done: sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
		Kind:      "success",
	})

	got := waitCaptured(t, ch)
	if strings.Contains(got.body, "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA") {
		t.Errorf("api key survived MaskSecrets:\n%s", got.body)
	}
	if !strings.Contains(got.body, "done:") {
		t.Errorf("summary text should be present when include_body is on:\n%s", got.body)
	}
}

// events に "done" が無ければ何も送らない。
func TestSendDoneRespectsEventFilter(t *testing.T) {
	backend, ch := newCapturingBackend(t)
	m := New(Config{
		Backends: []BackendConfig{backend},
		Events:   []string{"approval"},
	}, slog.New(slog.NewTextHandler(io.Discard, nil)))

	m.SendDone(DonePayload{SessionID: 1, Title: "task", Summary: "x", Kind: "success"})

	select {
	case c := <-ch:
		t.Fatalf("unexpected notification sent: %s", c.body)
	case <-time.After(300 * time.Millisecond):
	}
}

func TestSendApprovalRetriesAfterAllBackendsFail(t *testing.T) {
	var mu sync.Mutex
	calls := 0
	firstDone := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		calls++
		call := calls
		mu.Unlock()
		if call == 1 {
			close(firstDone)
			w.WriteHeader(http.StatusBadGateway)
			return
		}
		w.WriteHeader(http.StatusNoContent)
	}))
	defer srv.Close()
	m := New(Config{
		Backends: []BackendConfig{{Type: "webhook", URL: srv.URL}},
		Events:   []string{"approval"},
	}, slog.New(slog.NewTextHandler(io.Discard, nil)))
	payload := ApprovalPayload{ID: "approval-retry", SessionID: 1, Title: "approval"}
	m.SendApproval(payload)
	select {
	case <-firstDone:
	case <-time.After(2 * time.Second):
		t.Fatal("first failed notification did not reach the backend")
	}
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		m.mu.Lock()
		_, pending := m.pending[payload.ID]
		m.mu.Unlock()
		if !pending {
			break
		}
		time.Sleep(5 * time.Millisecond)
	}

	// The first request was a total failure, so the same ID must be eligible
	// for a retry instead of being suppressed for sentTTL.
	m.SendApproval(payload)
	deadline = time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		mu.Lock()
		got := calls
		mu.Unlock()
		if got >= 2 {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	mu.Lock()
	defer mu.Unlock()
	t.Fatalf("backend calls = %d, want retry after total failure", calls)
}

func TestDoneKindLabel(t *testing.T) {
	cases := map[string]string{
		"success":      "成功",
		"failure":      "失敗",
		"aborted":      "中断",
		"needs_action": "要判断",
		"  SUCCESS  ":  "成功",
		"":             "完了",
		"unknown":      "完了",
	}
	for kind, want := range cases {
		if got := doneKindLabel(kind); got != want {
			t.Errorf("doneKindLabel(%q) = %q, want %q", kind, got, want)
		}
	}
}
