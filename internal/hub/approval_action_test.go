package hub

import (
	"bytes"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"
)

func newTestOneTapApprovalManager(t *testing.T, now time.Time) *oneTapApprovalManager {
	t.Helper()
	m, err := newOneTapApprovalManager()
	if err != nil {
		t.Fatalf("newOneTapApprovalManager: %v", err)
	}
	m.now = func() time.Time { return now }
	m.tokenTTL = time.Minute
	return m
}

func TestOneTapApprovalEndpointRejectsHighRiskApprove(t *testing.T) {
	s := newTestServer()
	now := time.Unix(1_700_000_000, 0)
	s.oneTapApprovals = newTestOneTapApprovalManager(t, now)
	ses := registerTestSession(s, 9, "codex")
	raw, err := os.ReadFile("testdata/approval_codex_shortcut_ansi.ansi")
	if err != nil {
		t.Fatal(err)
	}
	raw = bytes.ReplaceAll(raw, []byte(`\x1b`), []byte{0x1b})
	raw = bytes.ReplaceAll(raw, []byte("\n"), []byte("\r\n"))
	ses.vt = newVTBuffer(120, 30)
	ses.vt.Write(raw)
	pending := detectNativeApproval("codex", ses.vt.TailLines(vtTailLinesForApproval))
	if pending == nil || pending.Summary.Risk != "high" {
		t.Fatalf("pending approval = %#v, want high risk", pending)
	}
	ses.nativeApprovalSig = pending.Sig
	token, err := s.oneTapApprovals.issue(9, pending.Sig, pending.Sig, oneTapApprove)
	if err != nil {
		t.Fatal(err)
	}
	req := httptest.NewRequest(http.MethodPost, "/api/approval-action/"+token, nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleOneTapApproval(w, req)
	if w.Code != http.StatusForbidden {
		t.Fatalf("status = %d, want %d; body=%s", w.Code, http.StatusForbidden, w.Body.String())
	}
}

func TestOneTapApprovalEndpointRejectsExpiredToken(t *testing.T) {
	s := newTestServer()
	now := time.Unix(1_700_000_000, 0)
	s.oneTapApprovals = newTestOneTapApprovalManager(t, now)
	token, err := s.oneTapApprovals.issue(1, "approval-1", "approval-1", oneTapReject)
	if err != nil {
		t.Fatal(err)
	}
	s.oneTapApprovals.now = func() time.Time { return now.Add(2 * time.Minute) }
	req := httptest.NewRequest(http.MethodPost, "/api/approval-action/"+token, nil)
	req.Host = "127.0.0.1:47777"
	w := httptest.NewRecorder()
	s.handleOneTapApproval(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d; body=%s", w.Code, http.StatusUnauthorized, w.Body.String())
	}
}

func TestOneTapApprovalTokenIsBoundAndOneShot(t *testing.T) {
	m := newTestOneTapApprovalManager(t, time.Unix(1_700_000_000, 0))
	token, err := m.issue(7, "approval-7", "approval-7", oneTapApprove)
	if err != nil {
		t.Fatalf("issue: %v", err)
	}
	if strings.Contains(token, "hub-token") {
		t.Fatal("action token must not contain a Hub token")
	}
	claim, err := m.verify(token)
	if err != nil {
		t.Fatalf("verify: %v", err)
	}
	if claim.SessionID != 7 || claim.ApprovalID != "approval-7" || claim.ApprovalSig != "approval-7" || claim.Action != oneTapApprove {
		t.Fatalf("claim binding = %#v", claim)
	}
	if err := m.consume(claim); err != nil {
		t.Fatalf("first consume: %v", err)
	}
	if err := m.consume(claim); !errors.Is(err, errOneTapConsumed) {
		t.Fatalf("second consume = %v, want replay rejection", err)
	}
}

func TestOneTapApprovalTokenRejectsTamperAndExpiry(t *testing.T) {
	now := time.Unix(1_700_000_000, 0)
	m := newTestOneTapApprovalManager(t, now)
	token, err := m.issue(2, "approval-2", "approval-2", oneTapReject)
	if err != nil {
		t.Fatalf("issue: %v", err)
	}
	if _, err := m.verify(token + "x"); !errors.Is(err, errOneTapInvalid) {
		t.Fatalf("tampered verify = %v, want invalid", err)
	}
	m.now = func() time.Time { return now.Add(2 * time.Minute) }
	if _, err := m.verify(token); !errors.Is(err, errOneTapExpired) {
		t.Fatalf("expired verify = %v, want expired", err)
	}
}

func TestApplyOneTapApprovalRevertsOnSendFailure(t *testing.T) {
	s := newTestServer()
	now := time.Unix(1_700_000_000, 0)
	s.oneTapApprovals = newTestOneTapApprovalManager(t, now)
	ses := registerTestSession(s, 12, "codex")
	raw, err := os.ReadFile("testdata/approval_codex_shortcut_ansi.ansi")
	if err != nil {
		t.Fatal(err)
	}
	raw = bytes.ReplaceAll(raw, []byte(`\x1b`), []byte{0x1b})
	raw = bytes.ReplaceAll(raw, []byte("\n"), []byte("\r\n"))
	ses.vt = newVTBuffer(120, 30)
	ses.vt.Write(raw)
	pending := detectNativeApproval("codex", ses.vt.TailLines(vtTailLinesForApproval))
	if pending == nil {
		t.Fatal("pending approval = nil")
	}
	ses.nativeApprovalSig = pending.Sig
	token, err := s.oneTapApprovals.issue(12, pending.Sig, pending.Sig, oneTapReject)
	if err != nil {
		t.Fatal(err)
	}
	claim, err := s.oneTapApprovals.verify(token)
	if err != nil {
		t.Fatal(err)
	}

	err = s.applyOneTapApproval(claim)
	if !errors.Is(err, errOneTapNoInput) {
		t.Fatalf("applyOneTapApproval error = %v, want errOneTapNoInput", err)
	}
	if ses.nativeApprovalSig != pending.Sig {
		t.Fatalf("nativeApprovalSig = %q, want %q after send failure", ses.nativeApprovalSig, pending.Sig)
	}
	if err := s.oneTapApprovals.consume(claim); err != nil {
		t.Fatalf("consume after send failure = %v, want token to remain retryable", err)
	}
}
