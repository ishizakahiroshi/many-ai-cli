package hub

import (
	"bytes"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"many-ai-cli/internal/proto"
)

func installBatchApproval(t *testing.T, s *Server, id int, provider, cwd, command string) *nativeApproval {
	t.Helper()
	ses := registerTestSession(s, id, provider)
	lines := []string{
		"Command requires approval",
		"Run: " + command,
		"",
		"❯ Yes (y)",
		"  Yes, and don't ask again for this command (p)",
		"  No (n)",
		"  Cancel (esc)",
	}
	approval := detectNativeApproval(provider, lines)
	if approval == nil {
		t.Fatalf("detectNativeApproval(%q, %q) returned nil", provider, command)
	}
	ses.CWD = cwd
	ses.vt = newVTBuffer(120, 30)
	ses.vt.Write([]byte(strings.Join(lines, "\r\n")))
	vtApproval := detectNativeApproval(provider, ses.vt.Lines())
	if vtApproval == nil {
		t.Fatalf("detectNativeApproval from VT returned nil for %q", command)
	}
	ses.nativeApprovalSig = vtApproval.Sig
	s.sessionsMu.Lock()
	s.wrappers[id] = &wrapperConn{}
	s.sessionsMu.Unlock()
	return vtApproval
}

func callApprovalBatch(t *testing.T, s *Server, req approvalBatchRequest) *httptest.ResponseRecorder {
	t.Helper()
	body, err := json.Marshal(req)
	if err != nil {
		t.Fatal(err)
	}
	r := httptest.NewRequest(http.MethodPost, "/api/approval/batch?token=test-token", bytes.NewReader(body))
	r.Host = "127.0.0.1:47777"
	r.Header.Set("Origin", "http://127.0.0.1:47777")
	w := httptest.NewRecorder()
	s.handleApprovalBatch(w, r)
	return w
}

func TestApprovalBatchAutoRuleRequiresLow(t *testing.T) {
	withApprovalTestHome(t)
	s := newTestServer()
	s.cfg.Token = "test-token"
	cwd := t.TempDir()
	mid := installBatchApproval(t, s, 1, "codex", cwd, "git commit -m test")
	w := callApprovalBatch(t, s, approvalBatchRequest{Signature: approvalBatchSignature("codex", cwd, mid.Summary), Action: "auto_rule"})
	if w.Code != http.StatusForbidden {
		t.Fatalf("mid-risk auto_rule status = %d, want %d: %s", w.Code, http.StatusForbidden, w.Body.String())
	}

	low := installBatchApproval(t, s, 2, "codex", cwd, "git status")
	w = callApprovalBatch(t, s, approvalBatchRequest{Signature: approvalBatchSignature("codex", cwd, low.Summary), Action: "auto_rule"})
	if w.Code != http.StatusOK {
		t.Fatalf("low-risk auto_rule status = %d, want %d: %s", w.Code, http.StatusOK, w.Body.String())
	}
}

func TestApprovalBatchDenySession(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "test-token"
	cwd := t.TempDir()
	target := installBatchApproval(t, s, 10, "codex", cwd, "git status")
	other := installBatchApproval(t, s, 11, "codex", cwd, "git diff")

	w := callApprovalBatch(t, s, approvalBatchRequest{Action: "deny_session", SessionID: 10})
	if w.Code != http.StatusOK {
		t.Fatalf("deny_session status = %d, want %d: %s", w.Code, http.StatusOK, w.Body.String())
	}
	if s.sessions[10].nativeApprovalSig != "" {
		t.Fatalf("target approval sig = %q, want cleared", s.sessions[10].nativeApprovalSig)
	}
	if s.sessions[11].nativeApprovalSig != other.Sig {
		t.Fatalf("other approval sig = %q, want %q", s.sessions[11].nativeApprovalSig, other.Sig)
	}
	_ = target
}

func TestApprovalBatchApproveSkipsMidHigh(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "test-token"
	cwd := t.TempDir()
	low := installBatchApproval(t, s, 20, "codex", cwd, "git status")
	installBatchApproval(t, s, 21, "codex", cwd, "git commit -m test")
	high := installBatchApproval(t, s, 22, "codex", cwd, "rm -rf ./tmp")

	w := callApprovalBatch(t, s, approvalBatchRequest{
		Signature: approvalBatchSignature("codex", cwd, low.Summary),
		Action:    "approve",
	})
	if w.Code != http.StatusOK {
		t.Fatalf("approve status = %d, want %d: %s", w.Code, http.StatusOK, w.Body.String())
	}
	if s.sessions[20].nativeApprovalSig != "" {
		t.Fatalf("low approval sig = %q, want cleared", s.sessions[20].nativeApprovalSig)
	}
	if s.sessions[21].nativeApprovalSig == "" || s.sessions[22].nativeApprovalSig != high.Sig {
		t.Fatalf("mid/high approvals changed: mid=%q high=%q", s.sessions[21].nativeApprovalSig, s.sessions[22].nativeApprovalSig)
	}
	if low.Summary.Risk != proto.ApprovalRiskLow {
		t.Fatalf("low fixture risk = %q", low.Summary.Risk)
	}
}
