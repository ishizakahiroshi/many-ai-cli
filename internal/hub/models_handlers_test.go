package hub

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"many-ai-cli/internal/nvidianim"
)

func TestHandleModelsLoadsNVIDIANIMOnlyWhenEnabledAndKeyExists(t *testing.T) {
	for _, tc := range []struct {
		name         string
		enabled      bool
		apiKey       string
		wantFetch    bool
		wantNIMGroup bool
	}{
		{name: "enabled with key", enabled: true, apiKey: "synthetic-key", wantFetch: true, wantNIMGroup: true},
		{name: "disabled with key", enabled: false, apiKey: "synthetic-key"},
		{name: "enabled without key", enabled: true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			t.Setenv(nvidianim.APIKeyEnv, tc.apiKey)
			s := newTestServer()
			s.cfg.Token = "test-token"
			s.cfg.NVIDIANIM.Enabled = tc.enabled
			s.modelsCache = newNVIDIANIMTestModelsCache()
			fetchCalls := 0
			s.modelsCache.nvidiaNIMLoader = func(key string) ([]string, error) {
				fetchCalls++
				if key != tc.apiKey {
					t.Errorf("loader key = %q, want configured key", key)
				}
				return []string{"moonshotai/kimi-k3"}, nil
			}
			emptyDefaults := modelsDefaults{}
			s.modelsRemoteCache = &ttlCache[modelsDefaults]{
				data:      &emptyDefaults,
				fetchedAt: time.Now(),
				ttl:       time.Hour,
			}

			req := httptest.NewRequest(http.MethodGet, "/api/models?token=test-token", nil)
			req.Host = "127.0.0.1:47777"
			req.RemoteAddr = "127.0.0.1:12345"
			response := httptest.NewRecorder()
			s.handleModels(response, req)
			if response.Code != http.StatusOK {
				t.Fatalf("GET /api/models status = %d, body = %s", response.Code, response.Body.String())
			}
			if (fetchCalls == 1) != tc.wantFetch {
				t.Fatalf("NVIDIA catalog fetch calls = %d, want fetch=%v", fetchCalls, tc.wantFetch)
			}
			var result ModelsResponse
			if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
				t.Fatal(err)
			}
			hasNIMGroup := false
			for _, group := range result.Groups {
				if group.Route == RouteNVIDIANIM {
					hasNIMGroup = true
				}
			}
			if hasNIMGroup != tc.wantNIMGroup {
				t.Fatalf("NVIDIA NIM group present = %v, want %v", hasNIMGroup, tc.wantNIMGroup)
			}
		})
	}
}
