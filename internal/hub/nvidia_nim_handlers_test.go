package hub

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/nvidianim"
)

func setupNVIDIANIMHandlerTest(t *testing.T) (*Server, string) {
	t.Helper()
	home := t.TempDir()
	t.Setenv("USERPROFILE", home)
	t.Setenv("HOME", home)
	t.Setenv("NVIDIA_API_KEY", "")
	s := newTestServer()
	s.cfg.Token = "test-token"
	return s, home
}

func nvidiaNIMRequest(s *Server, method, path, body string, handler http.HandlerFunc) *httptest.ResponseRecorder {
	req := httptest.NewRequest(method, path+"?token=test-token", strings.NewReader(body))
	req.Host = "127.0.0.1:47777"
	req.RemoteAddr = "127.0.0.1:12345"
	rec := httptest.NewRecorder()
	handler(rec, req)
	return rec
}

func decodeNVIDIANIMJSON(t *testing.T, rec *httptest.ResponseRecorder) map[string]any {
	t.Helper()
	var got map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatalf("invalid JSON response: %v", err)
	}
	return got
}

func TestNVIDIANIMSettingsNeverReturnsKey(t *testing.T) {
	s, _ := setupNVIDIANIMHandlerTest(t)
	const key = "synthetic-nim-secret"
	if err := nvidianim.SaveAPIKey(mustConfigDir(t), key); err != nil {
		t.Fatal("SaveAPIKey failed")
	}
	s.cfg.NVIDIANIM.Enabled = true

	rec := nvidiaNIMRequest(s, http.MethodGet, "/api/nvidia-nim", "", s.handleNVIDIANIMSettings)
	if rec.Code != http.StatusOK {
		t.Fatalf("GET status = %d, want 200", rec.Code)
	}
	if strings.Contains(rec.Body.String(), key) {
		t.Fatal("GET response exposed the API key")
	}
	got := decodeNVIDIANIMJSON(t, rec)
	if got["enabled"] != true || got["api_key_configured"] != true || got["api_key_source"] != "file" {
		t.Fatalf("GET state = %v, want enabled/configured/file", got)
	}
}

func TestNVIDIANIMSettingsPutSavesKeyAndEnabledStateWithoutEcho(t *testing.T) {
	s, home := setupNVIDIANIMHandlerTest(t)
	const key = "synthetic-nim-secret"
	rec := nvidiaNIMRequest(s, http.MethodPut, "/api/nvidia-nim", `{"enabled":true,"api_key":"`+key+`"}`, s.handleNVIDIANIMSettings)
	if rec.Code != http.StatusOK {
		t.Fatalf("PUT status = %d, want 200; body=%s", rec.Code, rec.Body.String())
	}
	if strings.Contains(rec.Body.String(), key) {
		t.Fatal("PUT response exposed the submitted API key")
	}
	got := decodeNVIDIANIMJSON(t, rec)
	if got["enabled"] != true || got["api_key_configured"] != true || got["api_key_source"] != "file" {
		t.Fatalf("PUT state = %v, want enabled/configured/file", got)
	}
	stored, err := os.ReadFile(filepath.Join(home, ".many-ai-cli", "secrets", "nvidia_api_key"))
	if err != nil || string(stored) != key {
		t.Fatal("API key was not saved to the private secret file")
	}
	data, err := os.ReadFile(filepath.Join(home, ".many-ai-cli", "config.yaml"))
	if err != nil {
		t.Fatal("config was not persisted")
	}
	if strings.Contains(string(data), key) {
		t.Fatal("config.yaml contains the API key")
	}
}

func TestNVIDIANIMEnvironmentKeyCannotBeDeletedOrReplaced(t *testing.T) {
	s, _ := setupNVIDIANIMHandlerTest(t)
	const key = "synthetic-env-key"
	t.Setenv("NVIDIA_API_KEY", key)

	deleteRec := nvidiaNIMRequest(s, http.MethodDelete, "/api/nvidia-nim/key", "", s.handleNVIDIANIMKeyDelete)
	if deleteRec.Code != http.StatusConflict || !strings.Contains(deleteRec.Body.String(), "key_managed_by_environment") {
		t.Fatalf("DELETE status/body = %d/%s, want managed-by-environment conflict", deleteRec.Code, deleteRec.Body.String())
	}
	putRec := nvidiaNIMRequest(s, http.MethodPut, "/api/nvidia-nim", `{"enabled":true,"api_key":"replacement"}`, s.handleNVIDIANIMSettings)
	if putRec.Code != http.StatusConflict || !strings.Contains(putRec.Body.String(), "key_managed_by_environment") {
		t.Fatalf("PUT status/body = %d/%s, want managed-by-environment conflict", putRec.Code, putRec.Body.String())
	}
	if strings.Contains(deleteRec.Body.String()+putRec.Body.String(), key) {
		t.Fatal("environment key leaked in conflict response")
	}
}

func TestNVIDIANIMFileKeyCanBeDeleted(t *testing.T) {
	s, home := setupNVIDIANIMHandlerTest(t)
	if err := nvidianim.SaveAPIKey(mustConfigDir(t), "synthetic-file-key"); err != nil {
		t.Fatal("SaveAPIKey failed")
	}
	rec := nvidiaNIMRequest(s, http.MethodDelete, "/api/nvidia-nim/key", "", s.handleNVIDIANIMKeyDelete)
	if rec.Code != http.StatusOK {
		t.Fatalf("DELETE status = %d, want 200", rec.Code)
	}
	got := decodeNVIDIANIMJSON(t, rec)
	if got["api_key_configured"] != false || got["api_key_source"] != "none" {
		t.Fatalf("DELETE state = %v, want unconfigured/none", got)
	}
	if _, err := os.Stat(filepath.Join(home, ".many-ai-cli", "secrets", "nvidia_api_key")); !errors.Is(err, os.ErrNotExist) {
		t.Fatal("secret file still exists after delete")
	}
}

func TestNVIDIANIMTestReturnsOnlySafeResultCode(t *testing.T) {
	s, _ := setupNVIDIANIMHandlerTest(t)
	const key = "synthetic-test-key"
	if err := nvidianim.SaveAPIKey(mustConfigDir(t), key); err != nil {
		t.Fatal("SaveAPIKey failed")
	}
	s.nvidiaNIMTester = func(got string) error {
		if got != key {
			t.Fatal("NVIDIA connection tester received a different key")
		}
		return &nvidianim.CatalogHTTPError{StatusCode: http.StatusUnauthorized}
	}
	rec := nvidiaNIMRequest(s, http.MethodPost, "/api/nvidia-nim/test", "", s.handleNVIDIANIMTest)
	if rec.Code != http.StatusOK {
		t.Fatalf("test status = %d, want 200", rec.Code)
	}
	if strings.Contains(rec.Body.String(), key) {
		t.Fatal("test response exposed the API key")
	}
	got := decodeNVIDIANIMJSON(t, rec)
	if got["ok"] != false || got["code"] != "unauthorized" {
		t.Fatalf("test result = %v, want unauthorized classification", got)
	}
}

func TestNVIDIANIMTestRequiresConfiguredKey(t *testing.T) {
	s, _ := setupNVIDIANIMHandlerTest(t)
	calls := 0
	s.nvidiaNIMTester = func(string) error { calls++; return nil }
	rec := nvidiaNIMRequest(s, http.MethodPost, "/api/nvidia-nim/test", "", s.handleNVIDIANIMTest)
	if rec.Code != http.StatusBadRequest || !strings.Contains(rec.Body.String(), "api_key_missing") {
		t.Fatalf("test status/body = %d/%s, want missing-key response", rec.Code, rec.Body.String())
	}
	if calls != 0 {
		t.Fatal("connection tester ran without a configured key")
	}
}

func TestNVIDIANIMTestErrorClassifications(t *testing.T) {
	tests := []struct {
		status int
		want   string
	}{
		{http.StatusUnauthorized, "unauthorized"},
		{http.StatusPaymentRequired, "payment_required"},
		{http.StatusForbidden, "forbidden"},
		{http.StatusNotFound, "not_found"},
		{http.StatusRequestTimeout, "timeout"},
		{http.StatusTooManyRequests, "rate_limited"},
		{http.StatusBadGateway, "server_error"},
	}
	for _, tc := range tests {
		err := &nvidianim.CatalogHTTPError{StatusCode: tc.status}
		if got := nvidiaNIMTestErrorCode(err); got != tc.want {
			t.Errorf("status %d classified as %q, want %q", tc.status, got, tc.want)
		}
	}
}

func mustConfigDir(t *testing.T) string {
	t.Helper()
	dir, err := config.Dir()
	if err != nil {
		t.Fatal("config.Dir failed")
	}
	return dir
}
