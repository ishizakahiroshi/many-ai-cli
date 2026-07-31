package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"many-ai-cli/internal/config"
)

func TestAutoApprovalSimulationDefaultsToLastHundred(t *testing.T) {
	s := newTestServer()
	s.cfg.Token = "tok"
	for i := 0; i < 150; i++ {
		s.autoApprovalHistory = append(s.autoApprovalHistory, autoApprovalCandidate{SessionID: i + 1})
	}
	req := httptest.NewRequest(http.MethodGet, "/api/auto-approval/simulation?token=tok", nil)
	req.Host = testHubHost
	req.RemoteAddr = testLoopbackAddr
	w := httptest.NewRecorder()
	s.handleAutoApprovalSimulation(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("code = %d", w.Code)
	}
	var body struct {
		Total int `json:"total"`
		Items []struct {
			SessionID int `json:"session_id"`
		} `json:"items"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	if body.Total != 100 || len(body.Items) != 100 || body.Items[0].SessionID != 51 {
		t.Fatalf("simulation window = total:%d len:%d first:%d", body.Total, len(body.Items), body.Items[0].SessionID)
	}
}

func TestSanitizeNotifySoundPrefUsesOnlyManagedAudio(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	managed, err := notifySoundCustomPath()
	if err != nil {
		t.Fatal(err)
	}
	valid := sanitizeNotifySoundPref(config.UserPrefsNotifySound{CustomFile: managed, CustomMime: "audio/wav"})
	if valid.CustomFile != managed || valid.CustomMime != "audio/wav" {
		t.Fatalf("valid sound was changed: %+v", valid)
	}
	invalid := sanitizeNotifySoundPref(config.UserPrefsNotifySound{
		CustomFile: filepath.Join(home, "config.yaml"),
		CustomMime: "text/html",
	})
	if invalid.CustomFile != "" || invalid.CustomMime != "" {
		t.Fatalf("unsafe sound survived: %+v", invalid)
	}
}

func TestLocalModelHTTPClientBlocksPrivateHostWithoutOptIn(t *testing.T) {
	client := newLocalModelHTTPClient("http://10.0.0.1:11434/api/tags", 100*time.Millisecond, false)
	_, err := client.Get("http://10.0.0.1:11434/api/tags")
	if err == nil || !strings.Contains(strings.ToLower(err.Error()), "private") {
		t.Fatalf("private host request error = %v", err)
	}
}
