// Active-connection registry for the launcher.
//
// Each launcher process records its established connections in
// ~/.any-ai-cli/launcher-active.json so that other launcher processes
// (= the profile selection UI) can show which profiles are already
// connected and reuse the existing Hub URL instead of starting a
// duplicate tunnel / serve.
//
// Staleness is handled with the same double-guard approach as the Hub
// PID file (internal/hub/lifecycle.go killStalePid): a recorded PID may
// have been reused by an unrelated process, so an entry is treated as
// alive only when BOTH (1) the launcher PID is still running AND
// (2) the recorded Hub URL responds to /api/info. Entries failing
// either check are pruned on read.
package launcher

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"time"
)

const activeFile = "launcher-active.json"

// activeProbeTimeout bounds the /api/info liveness probe per entry.
const activeProbeTimeout = 800 * time.Millisecond

// ActiveConnection is one established launcher connection recorded in
// launcher-active.json.
//
// HubURL contains the Hub access token in its query string. The file is
// written with 0600 permissions under the user's home directory — the
// same security model as launcher-profiles.yaml and the Hub itself.
type ActiveConnection struct {
	Profile   string    `json:"profile"`
	PID       int       `json:"pid"`
	HubURL    string    `json:"hub_url"`
	StartedAt time.Time `json:"started_at"`
}

// activeFileData is the top-level structure of launcher-active.json.
type activeFileData struct {
	Version     int                `json:"version"`
	Connections []ActiveConnection `json:"connections,omitempty"`
}

// activePath returns the path to ~/.any-ai-cli/launcher-active.json.
func activePath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("home dir: %w", err)
	}
	return filepath.Join(home, ".any-ai-cli", activeFile), nil
}

// loadActiveFile reads launcher-active.json. A missing, empty, or corrupt
// file yields an empty registry (the file is a best-effort cache, never a
// source of truth — corruption must not block connecting).
func loadActiveFile() (*activeFileData, error) {
	path, err := activePath()
	if err != nil {
		return nil, err
	}
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return &activeFileData{Version: 1}, nil
	}
	if err != nil {
		return nil, fmt.Errorf("read launcher active file: %w", err)
	}
	var d activeFileData
	if err := json.Unmarshal(data, &d); err != nil {
		return &activeFileData{Version: 1}, nil
	}
	if d.Version == 0 {
		d.Version = 1
	}
	return &d, nil
}

// saveActiveFile writes d atomically (temp file + rename), mirroring
// SaveProfiles.
func saveActiveFile(d *activeFileData) error {
	path, err := activePath()
	if err != nil {
		return err
	}
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("mkdir launcher active dir: %w", err)
	}

	data, err := json.MarshalIndent(d, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal launcher active file: %w", err)
	}

	tmp, err := os.CreateTemp(dir, "launcher-active-*.json.tmp")
	if err != nil {
		return fmt.Errorf("create temp launcher active file: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // no-op after successful Rename

	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return fmt.Errorf("write temp launcher active file: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return fmt.Errorf("sync temp launcher active file: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close temp launcher active file: %w", err)
	}
	if err := os.Chmod(tmpName, 0o600); err != nil {
		return fmt.Errorf("chmod temp launcher active file: %w", err)
	}
	return os.Rename(tmpName, path)
}

// RegisterActiveConnection records (or replaces) the calling process's
// connection for profile in launcher-active.json.
func RegisterActiveConnection(profile, hubURL string) error {
	d, err := loadActiveFile()
	if err != nil {
		return err
	}
	pid := os.Getpid()
	kept := d.Connections[:0]
	for _, c := range d.Connections {
		// Replace any previous record for the same (profile, pid) pair.
		if c.Profile == profile && c.PID == pid {
			continue
		}
		kept = append(kept, c)
	}
	d.Connections = append(kept, ActiveConnection{
		Profile:   profile,
		PID:       pid,
		HubURL:    hubURL,
		StartedAt: time.Now(),
	})
	return saveActiveFile(d)
}

// UnregisterActiveConnection removes the calling process's record for
// profile from launcher-active.json. Missing records are not an error.
func UnregisterActiveConnection(profile string) error {
	return removeOwnEntries(func(c ActiveConnection) bool {
		return c.Profile == profile
	})
}

// UnregisterAllForPID removes every record owned by the calling process.
// Called on graceful shutdown of a launcher that may hold several
// connections (UI server mode).
func UnregisterAllForPID() error {
	return removeOwnEntries(func(ActiveConnection) bool { return true })
}

// removeOwnEntries drops entries with the calling process's PID that also
// satisfy match.
func removeOwnEntries(match func(ActiveConnection) bool) error {
	d, err := loadActiveFile()
	if err != nil {
		return err
	}
	pid := os.Getpid()
	kept := d.Connections[:0]
	changed := false
	for _, c := range d.Connections {
		if c.PID == pid && match(c) {
			changed = true
			continue
		}
		kept = append(kept, c)
	}
	if !changed {
		return nil
	}
	d.Connections = kept
	return saveActiveFile(d)
}

// ActiveConnectionsPruned returns the connections that are verifiably
// alive and rewrites launcher-active.json without the stale entries.
func ActiveConnectionsPruned() ([]ActiveConnection, error) {
	return collectActive(pidAlive, func(hubURL string) bool {
		return probeHub(hubURL, activeProbeTimeout)
	})
}

// collectActive applies the double guard (PID alive + Hub probe) to every
// recorded entry. alive and probe are injectable for tests.
func collectActive(alive func(pid int) bool, probe func(hubURL string) bool) ([]ActiveConnection, error) {
	d, err := loadActiveFile()
	if err != nil {
		return nil, err
	}
	kept := make([]ActiveConnection, 0, len(d.Connections))
	for _, c := range d.Connections {
		if !alive(c.PID) || !probe(c.HubURL) {
			continue
		}
		kept = append(kept, c)
	}
	if len(kept) != len(d.Connections) {
		d.Connections = kept
		if err := saveActiveFile(d); err != nil {
			return nil, err
		}
	}
	return kept, nil
}

// probeHub reports whether the Hub behind hubURL responds 200 to
// /api/info within timeout. The token query of hubURL is preserved.
func probeHub(hubURL string, timeout time.Duration) bool {
	u, err := url.Parse(hubURL)
	if err != nil {
		return false
	}
	u.Path = "/api/info"
	client := &http.Client{Timeout: timeout}
	resp, err := client.Get(u.String())
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}
