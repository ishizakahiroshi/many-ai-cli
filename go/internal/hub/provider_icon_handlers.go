package hub

import (
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/provider"
)

// A picture a user chose as an AI's icon (plan_provider-icon-single-source.md
// C4). It is stored as one file per AI, ~/.many-ai-cli/provider_icons/<id>.bin,
// the same way the avatar is (user_avatar.bin): PNG / JPEG / GIF / WebP only,
// judged by the bytes and never by the client's Content-Type. SVG is refused
// because it can carry script. The picture is deliberately not part of the
// provider definition: a definition string is capped at MaxStringLength, and
// history / backups would grow with every picture.
//
// The file lives under ~/.many-ai-cli, which is our own data directory (removed
// as a whole by `many-ai-cli uninstall`) and is not written into a user's
// repository, so the "reclaim on the next start" rule in
// internal/doctor/residue.go does not apply to it.

// providerIconMaxBytes is far smaller than avatarMaxBytes: the icon is drawn at
// 14-32 px.
const providerIconMaxBytes = 512 * 1024

// providerIconIDPattern is the only shape of id that is turned into a file
// name. It is narrower than a provider id (which may contain "."): no path
// separator, no dot, so an id can never point outside provider_icons/.
var providerIconIDPattern = regexp.MustCompile(`^[a-z0-9_-]{1,64}$`)

// providerIconMu keeps a PUT / DELETE pair for the same AI from interleaving
// its temp file and rename.
var providerIconMu sync.Mutex

func providerIconDir() (string, error) {
	dir, err := config.Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "provider_icons"), nil
}

// providerIconPath returns the file that holds id's picture. ok is false for an
// id that must never become a file name.
func providerIconPath(id string) (path string, ok bool, err error) {
	if !providerIconIDPattern.MatchString(id) {
		return "", false, nil
	}
	dir, err := providerIconDir()
	if err != nil {
		return "", false, err
	}
	path = filepath.Join(dir, id+".bin")
	// Belt and braces on top of the pattern: whatever the join produced must sit
	// directly inside dir.
	if filepath.Dir(path) != dir {
		return "", false, nil
	}
	return path, true, nil
}

// providerIconVersion is a short token that changes whenever the picture is
// replaced; "" means there is no picture.
func providerIconVersion(id string) string {
	path, ok, err := providerIconPath(id)
	if err != nil || !ok {
		return ""
	}
	info, err := os.Stat(path)
	if err != nil || !info.Mode().IsRegular() {
		return ""
	}
	return fmt.Sprintf("%x-%x", info.ModTime().UnixNano(), info.Size())
}

// removeProviderIcon deletes id's picture. A missing file is not an error. It is
// called when an AI is deleted or reset to its distributed default, so the
// picture does not outlive the definition it belonged to.
func (s *Server) removeProviderIcon(id string) {
	path, ok, err := providerIconPath(id)
	if err != nil || !ok {
		return
	}
	providerIconMu.Lock()
	defer providerIconMu.Unlock()
	if err := os.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
		s.logger.Warn("remove provider icon failed", "provider", id, "err", err)
	}
}

// providerSummariesWithIcons is the registry list plus the picture version of
// every AI that has one.
func providerSummariesWithIcons(registry *provider.Registry) []provider.Summary {
	summaries := registry.List()
	for i := range summaries {
		summaries[i].IconImageVersion = providerIconVersion(summaries[i].ID)
	}
	return summaries
}

// handleProviderIcon serves GET / PUT / DELETE /api/provider-icons/<id>.
func (s *Server) handleProviderIcon(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPut, http.MethodDelete) {
		return
	}
	id := strings.TrimPrefix(r.URL.Path, "/api/provider-icons/")
	path, ok, err := providerIconPath(id)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "home_dir_error", "home dir error")
		return
	}
	if !ok {
		// Includes "", "../x", "a/b" and ids that contain a dot.
		writeJSONError(w, http.StatusNotFound, "provider_icon_not_found", "provider icon was not found")
		return
	}
	// Only an AI the registry knows can have a picture.
	registry := s.providerRegistrySnapshot()
	if registry == nil {
		writeJSONError(w, http.StatusServiceUnavailable, "provider_registry_unavailable", "provider registry is unavailable")
		return
	}
	if _, known := registry.Lookup(id); !known {
		writeJSONError(w, http.StatusNotFound, "provider_not_found", "provider was not found")
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.handleProviderIconGet(w, r, path)
	case http.MethodPut:
		s.handleProviderIconPut(w, r, path)
	case http.MethodDelete:
		s.removeProviderIcon(id)
		writeJSON(w, map[string]bool{"ok": true})
	}
}

func (s *Server) handleProviderIconGet(w http.ResponseWriter, r *http.Request, path string) {
	data, err := os.ReadFile(path)
	if err != nil {
		http.NotFound(w, r)
		return
	}
	ct := http.DetectContentType(data)
	if !avatarImageAllowed(ct) {
		ct = "application/octet-stream"
	}
	w.Header().Set("Content-Type", ct)
	w.Header().Set("X-Content-Type-Options", "nosniff")
	// The page puts the picture's version after "?v=", so a new picture is a new URL.
	w.Header().Set("Cache-Control", "max-age=3600")
	_, _ = w.Write(data)
}

func (s *Server) handleProviderIconPut(w http.ResponseWriter, r *http.Request, path string) {
	r.Body = http.MaxBytesReader(w, r.Body, providerIconMaxBytes)
	data, err := io.ReadAll(r.Body)
	if err != nil {
		var tooLarge *http.MaxBytesError
		if errors.As(err, &tooLarge) {
			writeJSONError(w, http.StatusRequestEntityTooLarge, "provider_icon_too_large", fmt.Sprintf("image must be %d KB or smaller", providerIconMaxBytes/1024))
			return
		}
		writeJSONError(w, http.StatusBadRequest, "bad_request", "could not read the image")
		return
	}
	// The type comes from the bytes, not from the client's header.
	if !avatarImageAllowed(http.DetectContentType(data)) {
		writeJSONError(w, http.StatusUnsupportedMediaType, "provider_icon_bad_type", "image/png, image/jpeg, image/gif, or image/webp required")
		return
	}
	providerIconMu.Lock()
	defer providerIconMu.Unlock()
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		s.logger.Warn("provider icon dir failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "write_error", "could not save the image")
		return
	}
	// Write beside the target and rename, so a reader never sees half a picture.
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		s.logger.Warn("provider icon write failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "write_error", "could not save the image")
		return
	}
	if err := os.Rename(tmp, path); err != nil {
		_ = os.Remove(tmp)
		s.logger.Warn("provider icon rename failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "write_error", "could not save the image")
		return
	}
	writeJSON(w, map[string]bool{"ok": true})
}
