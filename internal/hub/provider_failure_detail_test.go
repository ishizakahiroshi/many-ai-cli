package hub

import (
	"bytes"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"many-ai-cli/internal/provider"
)

// syntheticHomeName stands in for the user's home folder name, which is
// usually the user name: a file error from the provider stores carries an
// absolute path through it.
const syntheticHomeName = "synthetic-user"

// newProviderAPIServerUnderHome is newProviderAPITestServer with every
// provider store under <tmp>/synthetic-user/.many-ai-cli, so a path leaking
// into a response is recognisable, and a logger the test can read.
func newProviderAPIServerUnderHome(t *testing.T, prepare func(home string)) (*Server, string, *bytes.Buffer) {
	t.Helper()
	home := filepath.Join(t.TempDir(), syntheticHomeName)
	base := filepath.Join(home, ".many-ai-cli")
	if err := os.MkdirAll(base, 0o700); err != nil {
		t.Fatal(err)
	}
	if prepare != nil {
		prepare(home)
	}
	s := newTestServer()
	s.cfg.Token = "test-token"
	store, err := provider.NewFileStore(filepath.Join(base, "providers.d"))
	if err != nil {
		t.Fatal(err)
	}
	s.providerStore = store
	history, err := provider.NewHistoryStore(filepath.Join(base, "provider-history"), filepath.Join(base, "provider-backups"))
	if err != nil {
		t.Fatal(err)
	}
	s.historyStore = history
	registry, diagnostics, err := buildProviderRegistry(s.cfg)
	if err != nil || len(diagnostics) != 0 {
		t.Fatalf("build provider registry: %v, %#v", err, diagnostics)
	}
	s.providers = registry
	var logs bytes.Buffer
	s.logger = slog.New(slog.NewTextHandler(&logs, nil))
	return s, home, &logs
}

// occupyWithFile puts a plain file where a store expects its folder, so every
// read and write under it fails with a file-system error naming the path.
func occupyWithFile(t *testing.T, path string) {
	t.Helper()
	if err := os.WriteFile(path, []byte("synthetic"), 0o600); err != nil {
		t.Fatal(err)
	}
}

func providerAPIRequest(t *testing.T, s *Server, method, path string, body []byte) *httptest.ResponseRecorder {
	t.Helper()
	var reader *bytes.Reader
	if body != nil {
		reader = bytes.NewReader(body)
	} else {
		reader = bytes.NewReader(nil)
	}
	req := httptest.NewRequest(method, path, reader)
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	if path == "/api/providers?token=test-token" {
		s.handleProviders(resp, req)
	} else {
		s.handleProviderRoute(resp, req)
	}
	return resp
}

// assertNoPathInBody fails when the response names the home folder, in any of
// the spellings a JSON body can carry it.
func assertNoPathInBody(t *testing.T, resp *httptest.ResponseRecorder, home string) {
	t.Helper()
	body := resp.Body.String()
	for _, word := range []string{syntheticHomeName, home, filepath.ToSlash(home), strings.ReplaceAll(home, `\`, `\\`)} {
		if strings.Contains(body, word) {
			t.Errorf("response body carries %q (the home folder); it must not:\n%s", word, body)
		}
	}
}

// v0.9 release review C4-D 観点 4 F1 / Q8: when a provider operation fails in
// the file system, the settings dialog shows the 4xx reason as it is sent
// (provider-manager-view.ts providerErrorMessage), and the error text carries
// an absolute path through the user's home folder. The body now holds a fixed
// sentence; the original error, path and all, goes to the Hub log only.
func TestProviderFileFailureAnswersWithoutThePath(t *testing.T) {
	cases := []struct {
		name       string
		occupy     string // relative to ~/.many-ai-cli; made a file
		request    func(t *testing.T, s *Server) *httptest.ResponseRecorder
		wantStatus int
		wantCode   string
	}{
		{
			name:   "built-in override save",
			occupy: "provider-history",
			request: func(t *testing.T, s *Server) *httptest.ResponseRecorder {
				definition, ok := s.providerRegistrySnapshot().Lookup("claude")
				if !ok {
					t.Fatal("claude is not registered")
				}
				definition.Definition.DisplayName = "Claude (edited)"
				return patchProviderRequest(t, s, "claude", "", definition.Definition)
			},
			wantStatus: http.StatusUnprocessableEntity,
			wantCode:   "provider_override_failed",
		},
		{
			name:   "custom provider create",
			occupy: "providers.d",
			request: func(t *testing.T, s *Server) *httptest.ResponseRecorder {
				definition := provider.Definition{SchemaVersion: 1, ID: "synthetic-cli", DisplayName: "Synthetic CLI", Launch: &provider.LaunchDefinition{Executable: "synthetic-cli"}}
				return providerAPIRequest(t, s, http.MethodPost, "/api/providers?token=test-token", mustJSON(t, definition))
			},
			wantStatus: http.StatusUnprocessableEntity,
			wantCode:   "provider_save_failed",
		},
		{
			name: "history diff of a revision that is not on disk",
			request: func(t *testing.T, s *Server) *httptest.ResponseRecorder {
				return providerAPIRequest(t, s, http.MethodGet, "/api/providers/claude/history/0123456789abcdef01234567/diff?token=test-token", nil)
			},
			wantStatus: http.StatusNotFound,
			wantCode:   "history_not_found",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			s, home, logs := newProviderAPIServerUnderHome(t, func(home string) {
				if tc.occupy != "" {
					occupyWithFile(t, filepath.Join(home, ".many-ai-cli", tc.occupy))
				}
			})

			resp := tc.request(t, s)

			if resp.Code != tc.wantStatus {
				t.Fatalf("status = %d, want %d, body=%s", resp.Code, tc.wantStatus, resp.Body.String())
			}
			body := decodeProviderErrorBody(t, resp)
			if body.Error != tc.wantCode || body.Detail == "" {
				t.Fatalf("body = %#v, want error %q with a reason", body, tc.wantCode)
			}
			assertNoPathInBody(t, resp, home)
			if !strings.Contains(logs.String(), syntheticHomeName) {
				t.Errorf("the Hub log lacks the original error (with its path):\n%s", logs.String())
			}
		})
	}
}

// The other side of the same line: a failure about what the request sent
// still names the field or the id and why (C10-C4 完了条件).
func TestProviderInputFailureStillNamesTheReason(t *testing.T) {
	s, _, _ := newProviderAPIServerUnderHome(t, nil)

	invalid := provider.Definition{SchemaVersion: 1, ID: "synthetic-cli", DisplayName: "Synthetic CLI"}
	resp := providerAPIRequest(t, s, http.MethodPost, "/api/providers?token=test-token", mustJSON(t, invalid))
	if body := decodeProviderErrorBody(t, resp); resp.Code != http.StatusUnprocessableEntity || body.Error != "provider_save_failed" || body.Detail != "provider definition is invalid" {
		t.Errorf("custom create without a launch: status %d body %#v, want 422 provider_save_failed \"provider definition is invalid\"", resp.Code, body)
	}

	resp = providerAPIRequest(t, s, http.MethodGet, "/api/providers/Not_A_Slug/history?token=test-token", nil)
	if body := decodeProviderErrorBody(t, resp); resp.Code != http.StatusNotFound || !strings.Contains(body.Detail, "lowercase slug") {
		t.Errorf("history of a malformed id: status %d body %#v, want 404 naming the id rule", resp.Code, body)
	}

	resp = providerAPIRequest(t, s, http.MethodDelete, "/api/providers/never-stored-cli?token=test-token&expected_revision="+s.providerRegistrySnapshot().Revision(), nil)
	if body := decodeProviderErrorBody(t, resp); resp.Code != http.StatusNotFound || !strings.Contains(body.Detail, "is not stored") {
		t.Errorf("delete of an id that is not stored: status %d body %#v, want 404 saying so", resp.Code, body)
	}
}
