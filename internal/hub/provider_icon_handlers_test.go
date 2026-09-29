package hub

import (
	"bytes"
	"encoding/json"
	"io/fs"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"many-ai-cli/internal/provider"
)

// plan_provider-icon-single-source.md C4: a picture a user chooses as an AI's icon.

var (
	testIconPNG  = []byte("\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR")
	testIconGIF  = []byte("GIF89a\x01\x00\x01\x00")
	testIconJPEG = []byte("\xff\xd8\xff\xe0\x00\x10JFIF\x00")
	testIconWebP = []byte("RIFF\x24\x00\x00\x00WEBPVP8 ")
)

// newProviderIconTestServer is a provider API server whose home folder is a
// temp dir, so every file the handler writes can be listed afterwards.
func newProviderIconTestServer(t *testing.T) (*Server, string) {
	t.Helper()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	return newProviderAPITestServer(t), home
}

func providerIconRequest(s *Server, method, path string, body []byte) *httptest.ResponseRecorder {
	req := httptest.NewRequest(method, path, bytes.NewReader(body))
	req.Header.Set("Origin", "http://127.0.0.1:47777")
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviderIcon(resp, req)
	return resp
}

func iconURL(id string) string { return "/api/provider-icons/" + id + "?token=test-token" }

// filesUnder lists every regular file below root, relative to it.
func filesUnder(t *testing.T, root string) []string {
	t.Helper()
	var files []string
	err := filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !d.IsDir() {
			rel, _ := filepath.Rel(root, path)
			files = append(files, filepath.ToSlash(rel))
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	return files
}

func iconPathFor(t *testing.T, home, id string) string {
	t.Helper()
	return filepath.Join(home, ".many-ai-cli", "provider_icons", id+".bin")
}

func listedIconVersion(t *testing.T, s *Server, id string) string {
	t.Helper()
	req := httptest.NewRequest(http.MethodGet, "/api/providers?token=test-token", nil)
	req.Host = "127.0.0.1:47777"
	resp := httptest.NewRecorder()
	s.handleProviders(resp, req)
	if resp.Code != http.StatusOK {
		t.Fatalf("GET /api/providers status = %d body=%s", resp.Code, resp.Body.String())
	}
	var body struct {
		Providers []provider.Summary `json:"providers"`
	}
	if err := json.Unmarshal(resp.Body.Bytes(), &body); err != nil {
		t.Fatal(err)
	}
	for _, summary := range body.Providers {
		if summary.ID == id {
			return summary.IconImageVersion
		}
	}
	t.Fatalf("provider %q is not in the list", id)
	return ""
}

func TestProviderIconPutStoresAndServesPicture(t *testing.T) {
	s, home := newProviderIconTestServer(t)

	if v := listedIconVersion(t, s, "claude"); v != "" {
		t.Fatalf("version before any picture = %q, want empty", v)
	}
	resp := providerIconRequest(s, http.MethodPut, iconURL("claude"), testIconPNG)
	if resp.Code != http.StatusOK {
		t.Fatalf("PUT status = %d body=%s, want 200", resp.Code, resp.Body.String())
	}
	path := iconPathFor(t, home, "claude")
	data, err := os.ReadFile(path)
	if err != nil || !bytes.Equal(data, testIconPNG) {
		t.Fatalf("stored file = %q err=%v", data, err)
	}
	if runtime.GOOS != "windows" {
		info, _ := os.Stat(path)
		if info.Mode().Perm() != 0o600 {
			t.Fatalf("mode = %v, want 0600", info.Mode().Perm())
		}
	}
	if files := filesUnder(t, home); len(files) != 1 {
		t.Fatalf("files under home = %v, want only the picture (no leftover temp file)", files)
	}

	get := providerIconRequest(s, http.MethodGet, iconURL("claude"), nil)
	if get.Code != http.StatusOK {
		t.Fatalf("GET status = %d, want 200", get.Code)
	}
	if ct := get.Header().Get("Content-Type"); ct != "image/png" {
		t.Fatalf("Content-Type = %q, want image/png", ct)
	}
	if get.Header().Get("X-Content-Type-Options") != "nosniff" {
		t.Fatal("GET must send X-Content-Type-Options: nosniff")
	}
	if !bytes.Equal(get.Body.Bytes(), testIconPNG) {
		t.Fatal("GET returned different bytes")
	}
}

func TestProviderIconAcceptsEveryRasterFormat(t *testing.T) {
	for name, body := range map[string][]byte{"png": testIconPNG, "gif": testIconGIF, "jpeg": testIconJPEG, "webp": testIconWebP} {
		t.Run(name, func(t *testing.T) {
			s, _ := newProviderIconTestServer(t)
			// The client's Content-Type is not trusted, so send a wrong one on purpose.
			req := httptest.NewRequest(http.MethodPut, iconURL("codex"), bytes.NewReader(body))
			req.Header.Set("Content-Type", "text/plain")
			req.Header.Set("Origin", "http://127.0.0.1:47777")
			req.Host = "127.0.0.1:47777"
			resp := httptest.NewRecorder()
			s.handleProviderIcon(resp, req)
			if resp.Code != http.StatusOK {
				t.Fatalf("PUT %s status = %d body=%s, want 200", name, resp.Code, resp.Body.String())
			}
		})
	}
}

func TestProviderIconRejectsSVGAndUnknownBytes(t *testing.T) {
	s, home := newProviderIconTestServer(t)
	for name, body := range map[string][]byte{
		"svg":     []byte(`<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>`),
		"svg-xml": []byte(`<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>`),
		"html":    []byte(`<html><script>alert(1)</script></html>`),
		"text":    []byte("hello"),
		"empty":   nil,
	} {
		resp := providerIconRequest(s, http.MethodPut, iconURL("claude"), body)
		if resp.Code != http.StatusUnsupportedMediaType {
			t.Errorf("%s: status = %d body=%s, want 415", name, resp.Code, resp.Body.String())
		}
	}
	if files := filesUnder(t, home); len(files) != 0 {
		t.Fatalf("a refused upload wrote %v", files)
	}
}

func TestProviderIconRejectsTooLarge(t *testing.T) {
	s, home := newProviderIconTestServer(t)
	big := append(append([]byte{}, testIconPNG...), bytes.Repeat([]byte{0}, providerIconMaxBytes)...)
	resp := providerIconRequest(s, http.MethodPut, iconURL("claude"), big)
	if resp.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("status = %d body=%s, want 413", resp.Code, resp.Body.String())
	}
	if !strings.Contains(resp.Body.String(), "provider_icon_too_large") {
		t.Fatalf("body = %s, want provider_icon_too_large", resp.Body.String())
	}
	if files := filesUnder(t, home); len(files) != 0 {
		t.Fatalf("a refused upload wrote %v", files)
	}
	// Exactly at the limit is accepted.
	atLimit := append(append([]byte{}, testIconPNG...), bytes.Repeat([]byte{0}, providerIconMaxBytes-len(testIconPNG))...)
	if resp := providerIconRequest(s, http.MethodPut, iconURL("claude"), atLimit); resp.Code != http.StatusOK {
		t.Fatalf("at-limit status = %d body=%s, want 200", resp.Code, resp.Body.String())
	}
}

func TestProviderIconIDNeverLeavesTheIconDirectory(t *testing.T) {
	s, home := newProviderIconTestServer(t)
	// An id that reaches the handler as a path component must never become a path.
	for _, id := range []string{
		"", ".", "..", "../claude", "..%2fclaude", `..\claude`, "claude/../codex", "a/b", "/claude", "claude/",
		"claude.bin", "Claude", "cla ude", "claude\x00", strings.Repeat("a", 65), "my.ai",
	} {
		if path, ok, _ := providerIconPath(id); ok {
			t.Errorf("providerIconPath(%q) = %q, want it refused", id, path)
		}
		req := httptest.NewRequest(http.MethodPut, "/api/provider-icons/x?token=test-token", bytes.NewReader(testIconPNG))
		req.URL.Path = "/api/provider-icons/" + id
		req.Header.Set("Origin", "http://127.0.0.1:47777")
		req.Host = "127.0.0.1:47777"
		resp := httptest.NewRecorder()
		s.handleProviderIcon(resp, req)
		if resp.Code != http.StatusNotFound {
			t.Errorf("PUT %q: status = %d body=%s, want 404", id, resp.Code, resp.Body.String())
		}
	}
	if files := filesUnder(t, home); len(files) != 0 {
		t.Fatalf("a refused id wrote %v", files)
	}
	if path, ok, err := providerIconPath("cursor-agent"); err != nil || !ok || filepath.Base(path) != "cursor-agent.bin" || filepath.Base(filepath.Dir(path)) != "provider_icons" {
		t.Fatalf("providerIconPath(cursor-agent) = %q ok=%v err=%v", path, ok, err)
	}
}

func TestProviderIconOnlyForAnAIInTheRegistry(t *testing.T) {
	s, home := newProviderIconTestServer(t)
	for _, method := range []string{http.MethodPut, http.MethodGet, http.MethodDelete} {
		resp := providerIconRequest(s, method, iconURL("no-such-ai"), testIconPNG)
		if resp.Code != http.StatusNotFound {
			t.Errorf("%s of an unregistered id: status = %d, want 404", method, resp.Code)
		}
	}
	if files := filesUnder(t, home); len(files) != 0 {
		t.Fatalf("an unregistered id wrote %v", files)
	}
}

func TestProviderIconRequiresToken(t *testing.T) {
	s, home := newProviderIconTestServer(t)
	for _, method := range []string{http.MethodPut, http.MethodGet, http.MethodDelete} {
		resp := providerIconRequest(s, method, "/api/provider-icons/claude", testIconPNG)
		if resp.Code != http.StatusUnauthorized {
			t.Errorf("%s without a token: status = %d, want 401", method, resp.Code)
		}
	}
	if files := filesUnder(t, home); len(files) != 0 {
		t.Fatalf("an unauthenticated request wrote %v", files)
	}
}

func TestProviderIconServesOnlyRasterTypes(t *testing.T) {
	s, home := newProviderIconTestServer(t)
	// A file that is not an image (planted by hand) must never be sent as HTML.
	path := iconPathFor(t, home, "claude")
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(`<html><script>alert(1)</script></html>`), 0o600); err != nil {
		t.Fatal(err)
	}
	resp := providerIconRequest(s, http.MethodGet, iconURL("claude"), nil)
	if ct := resp.Header().Get("Content-Type"); ct != "application/octet-stream" {
		t.Fatalf("Content-Type = %q, want application/octet-stream", ct)
	}
	if resp.Header().Get("X-Content-Type-Options") != "nosniff" {
		t.Fatal("missing nosniff")
	}
	if missing := providerIconRequest(s, http.MethodGet, iconURL("codex"), nil); missing.Code != http.StatusNotFound {
		t.Fatalf("GET of an AI with no picture: status = %d, want 404", missing.Code)
	}
}

func TestProviderIconVersionAppearsAndChangesWithThePicture(t *testing.T) {
	s, _ := newProviderIconTestServer(t)
	if resp := providerIconRequest(s, http.MethodPut, iconURL("claude"), testIconPNG); resp.Code != http.StatusOK {
		t.Fatalf("PUT status = %d", resp.Code)
	}
	first := listedIconVersion(t, s, "claude")
	if first == "" {
		t.Fatal("the list carries no icon_image_version after a picture was stored")
	}
	if other := listedIconVersion(t, s, "codex"); other != "" {
		t.Fatalf("codex has no picture but version = %q", other)
	}
	// A different picture (different size) is a different version, so ?v= busts the cache.
	if resp := providerIconRequest(s, http.MethodPut, iconURL("claude"), append(append([]byte{}, testIconPNG...), 0, 0, 0)); resp.Code != http.StatusOK {
		t.Fatalf("second PUT status = %d", resp.Code)
	}
	if second := listedIconVersion(t, s, "claude"); second == "" || second == first {
		t.Fatalf("version after replacing = %q, before = %q, want a different one", second, first)
	}
	if resp := providerIconRequest(s, http.MethodDelete, iconURL("claude"), nil); resp.Code != http.StatusOK {
		t.Fatalf("DELETE status = %d", resp.Code)
	}
	if v := listedIconVersion(t, s, "claude"); v != "" {
		t.Fatalf("version after removing the picture = %q, want empty", v)
	}
	// Removing what is not there is fine.
	if resp := providerIconRequest(s, http.MethodDelete, iconURL("claude"), nil); resp.Code != http.StatusOK {
		t.Fatalf("second DELETE status = %d, want 200", resp.Code)
	}
}

func TestDeletingACustomProviderRemovesItsPicture(t *testing.T) {
	s, home := newProviderIconTestServer(t)
	definition := provider.Definition{
		SchemaVersion: 1,
		ID:            "picture-cli",
		DisplayName:   "Picture CLI",
		Launch:        &provider.LaunchDefinition{Executable: "picture-cli"},
	}
	createReq := httptest.NewRequest(http.MethodPost, "/api/providers?token=test-token", bytes.NewReader(mustJSON(t, definition)))
	createReq.Header.Set("Origin", "http://127.0.0.1:47777")
	createReq.Host = "127.0.0.1:47777"
	createResp := httptest.NewRecorder()
	s.handleProviders(createResp, createReq)
	if createResp.Code != http.StatusCreated {
		t.Fatalf("create status = %d body=%s", createResp.Code, createResp.Body.String())
	}
	if resp := providerIconRequest(s, http.MethodPut, iconURL("picture-cli"), testIconPNG); resp.Code != http.StatusOK {
		t.Fatalf("PUT status = %d body=%s", resp.Code, resp.Body.String())
	}
	if _, err := os.Stat(iconPathFor(t, home, "picture-cli")); err != nil {
		t.Fatalf("picture was not stored: %v", err)
	}

	del := deleteProviderRequest(t, s, "picture-cli", s.providerRegistrySnapshot().Revision())
	if del.Code != http.StatusOK {
		t.Fatalf("delete status = %d body=%s", del.Code, del.Body.String())
	}
	if _, err := os.Stat(iconPathFor(t, home, "picture-cli")); !os.IsNotExist(err) {
		t.Fatalf("the picture of a deleted AI is still there: err=%v", err)
	}
}

func TestResettingABuiltinProviderRemovesItsPicture(t *testing.T) {
	s, home := newProviderIconTestServer(t)
	definition, ok := s.providerRegistrySnapshot().Lookup("claude")
	if !ok {
		t.Fatal("claude is not registered")
	}
	definition.Definition.DisplayName = "Claude (edited)"
	if resp := patchProviderRequest(t, s, "claude", "", definition.Definition); resp.Code != http.StatusOK {
		t.Fatalf("PATCH status = %d body=%s", resp.Code, resp.Body.String())
	}
	if resp := providerIconRequest(s, http.MethodPut, iconURL("claude"), testIconPNG); resp.Code != http.StatusOK {
		t.Fatalf("PUT status = %d", resp.Code)
	}
	current, err := s.historyStore.Current("claude")
	if err != nil {
		t.Fatal(err)
	}
	resetReq := httptest.NewRequest(http.MethodPost, "/api/providers/claude/reset?token=test-token", bytes.NewReader(mustJSON(t, map[string]any{"expected_revision": current.Revision})))
	resetReq.Header.Set("Origin", "http://127.0.0.1:47777")
	resetReq.Host = "127.0.0.1:47777"
	resetResp := httptest.NewRecorder()
	s.handleProviderRoute(resetResp, resetReq)
	if resetResp.Code != http.StatusOK {
		t.Fatalf("reset status = %d body=%s", resetResp.Code, resetResp.Body.String())
	}
	if _, err := os.Stat(iconPathFor(t, home, "claude")); !os.IsNotExist(err) {
		t.Fatalf("the picture survived a reset to the distributed default: err=%v", err)
	}
}

func TestDisablingABuiltinProviderKeepsItsPicture(t *testing.T) {
	// Disable is a soft switch (the definition stays), so turning it back on must find the picture.
	s, home := newProviderIconTestServer(t)
	if resp := providerIconRequest(s, http.MethodPut, iconURL("claude"), testIconPNG); resp.Code != http.StatusOK {
		t.Fatalf("PUT status = %d", resp.Code)
	}
	if resp := deleteProviderRequest(t, s, "claude", ""); resp.Code != http.StatusOK {
		t.Fatalf("disable status = %d body=%s", resp.Code, resp.Body.String())
	}
	if _, err := os.Stat(iconPathFor(t, home, "claude")); err != nil {
		t.Fatalf("disabling removed the picture: %v", err)
	}
}
