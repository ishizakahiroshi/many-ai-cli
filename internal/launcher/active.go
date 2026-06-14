// Active-connection registry for the launcher.
//
// Each launcher process records its established connections in
// ~/.many-ai-cli/launcher-active.json so that other launcher processes
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
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sync"
	"time"
)

const activeFile = "launcher-active.json"
const connectLockPrefix = "launcher-connect-"

// activeProbeTimeout bounds the /api/info liveness probe per entry.
const activeProbeTimeout = 800 * time.Millisecond
const activePollInterval = 500 * time.Millisecond

// activeFileMu serializes the load→modify→save sequences against
// launcher-active.json within this process. saveActiveFile already provides
// cross-process atomicity (temp file + rename), but that is per-write, not
// transactional: without this guard, two goroutines in the same process
// (e.g. UI server mode running RegisterActiveConnection and
// ActiveConnectionsPruned concurrently) can read the same state and clobber
// each other's update on save. loadActiveFile/saveActiveFile themselves do
// NOT take this lock; callers hold it across the whole read-modify-write.
var activeFileMu sync.Mutex

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

type connectLockData struct {
	Profile   string    `json:"profile"`
	PID       int       `json:"pid"`
	StartedAt time.Time `json:"started_at"`
}

// ProfileConnectLock is a best-effort cross-process startup guard for one
// launcher profile. It covers the gap before a Hub URL is known and recorded
// in launcher-active.json.
type ProfileConnectLock struct {
	path string
}

// activePath returns the path to ~/.many-ai-cli/launcher-active.json.
func activePath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("home dir: %w", err)
	}
	return filepath.Join(home, ".many-ai-cli", activeFile), nil
}

func connectLockPath(profile string) (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("home dir: %w", err)
	}
	sum := sha256.Sum256([]byte(profile))
	name := connectLockPrefix + fmt.Sprintf("%x", sum[:8]) + ".json"
	return filepath.Join(home, ".many-ai-cli", name), nil
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
		_ = tmp.Close()
		return fmt.Errorf("write temp launcher active file: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
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

// TryAcquireProfileConnectLock creates a startup lock for profile. If another
// live launcher process is already connecting the same profile, acquired is
// false. Stale locks whose owner PID no longer exists are removed and retried.
func TryAcquireProfileConnectLock(profile string) (*ProfileConnectLock, bool, error) {
	path, err := connectLockPath(profile)
	if err != nil {
		return nil, false, err
	}
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, false, fmt.Errorf("mkdir launcher lock dir: %w", err)
	}

	for {
		f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
		if err == nil {
			data, marshalErr := json.Marshal(connectLockData{
				Profile:   profile,
				PID:       os.Getpid(),
				StartedAt: time.Now(),
			})
			if marshalErr != nil {
				_ = f.Close()
				_ = os.Remove(path)
				return nil, false, fmt.Errorf("marshal launcher lock: %w", marshalErr)
			}
			if _, writeErr := f.Write(data); writeErr != nil {
				_ = f.Close()
				_ = os.Remove(path)
				return nil, false, fmt.Errorf("write launcher lock: %w", writeErr)
			}
			if closeErr := f.Close(); closeErr != nil {
				_ = os.Remove(path)
				return nil, false, fmt.Errorf("close launcher lock: %w", closeErr)
			}
			return &ProfileConnectLock{path: path}, true, nil
		}
		if !os.IsExist(err) {
			return nil, false, fmt.Errorf("create launcher lock: %w", err)
		}

		data, readErr := os.ReadFile(path)
		if readErr != nil {
			return nil, false, fmt.Errorf("read launcher lock: %w", readErr)
		}
		var lock connectLockData
		if json.Unmarshal(data, &lock) == nil && pidAlive(lock.PID) {
			return nil, false, nil
		}
		if removeErr := os.Remove(path); removeErr != nil && !os.IsNotExist(removeErr) {
			return nil, false, fmt.Errorf("remove stale launcher lock: %w", removeErr)
		}
	}
}

// Release removes the startup lock if it is still owned by this process.
func (l *ProfileConnectLock) Release() error {
	if l == nil || l.path == "" {
		return nil
	}
	data, err := os.ReadFile(l.path)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return fmt.Errorf("read launcher lock: %w", err)
	}
	var lock connectLockData
	if err := json.Unmarshal(data, &lock); err == nil && lock.PID != os.Getpid() {
		return nil
	}
	if err := os.Remove(l.path); err != nil && !os.IsNotExist(err) {
		return fmt.Errorf("remove launcher lock: %w", err)
	}
	l.path = ""
	return nil
}

// RegisterActiveConnection records (or replaces) the calling process's
// connection for profile in launcher-active.json.
func RegisterActiveConnection(profile, hubURL string) error {
	activeFileMu.Lock()
	defer activeFileMu.Unlock()
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
	activeFileMu.Lock()
	defer activeFileMu.Unlock()
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

// WaitForActiveConnection polls launcher-active.json until profile becomes a
// verified active connection or timeout elapses.
func WaitForActiveConnection(profile string, timeout time.Duration) (ActiveConnection, bool) {
	deadline := time.Now().Add(timeout)
	for {
		conns, err := ActiveConnectionsPruned()
		if err == nil {
			for _, c := range conns {
				if c.Profile == profile {
					return c, true
				}
			}
		}
		if time.Now().After(deadline) {
			return ActiveConnection{}, false
		}
		time.Sleep(activePollInterval)
	}
}

// collectActive applies the double guard (PID alive + Hub probe) to every
// recorded entry. alive and probe are injectable for tests.
func collectActive(alive func(pid int) bool, probe func(hubURL string) bool) ([]ActiveConnection, error) {
	activeFileMu.Lock()
	defer activeFileMu.Unlock()
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
