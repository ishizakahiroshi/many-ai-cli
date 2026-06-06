// Hub runtime file (~/.any-ai-cli/hub-runtime.json).
//
// The Hub records its actually-bound port and PID here after Listen
// succeeds. The configured port (config.yaml hub.port) and the actual
// port can differ: when the configured port is occupied (e.g. an SSH
// tunnel forwarding a remote Hub, or another local Hub) the Hub moves to
// the next free port, and that move is never persisted to config.yaml.
// Without this file, a no-arg launch probes only the configured port,
// concludes "no Hub running", and spawns a duplicate Hub terminal.
//
// Staleness is handled with the same double-guard approach as
// launcher-active.json (internal/launcher/active.go): a recorded PID may
// have been reused by an unrelated process, so readers treat the entry as
// alive only when BOTH (1) the PID is still running AND (2) the recorded
// port answers /api/info with this machine's token.
package hub

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"
)

const hubRuntimeFile = "hub-runtime.json"

// hubRuntimeData is the content of hub-runtime.json. The token is NOT
// stored here — it lives in config.yaml and is stable across restarts, so
// readers combine this port with cfg.Token.
type hubRuntimeData struct {
	Version   int       `json:"version"`
	PID       int       `json:"pid"`
	Port      int       `json:"port"`
	StartedAt time.Time `json:"started_at"`
}

// hubRuntimePath returns the path to ~/.any-ai-cli/hub-runtime.json.
func hubRuntimePath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("home dir: %w", err)
	}
	return filepath.Join(home, ".any-ai-cli", hubRuntimeFile), nil
}

// writeHubRuntime records the calling process as the Hub bound to port.
// Written atomically (temp file + rename) with 0600, mirroring
// launcher/active.go saveActiveFile.
func writeHubRuntime(port int) error {
	path, err := hubRuntimePath()
	if err != nil {
		return err
	}
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("mkdir hub runtime dir: %w", err)
	}

	data, err := json.MarshalIndent(hubRuntimeData{
		Version:   1,
		PID:       os.Getpid(),
		Port:      port,
		StartedAt: time.Now(),
	}, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal hub runtime file: %w", err)
	}

	tmp, err := os.CreateTemp(dir, "hub-runtime-*.json.tmp")
	if err != nil {
		return fmt.Errorf("create temp hub runtime file: %w", err)
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // no-op after successful Rename

	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("write temp hub runtime file: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("sync temp hub runtime file: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("close temp hub runtime file: %w", err)
	}
	if err := os.Chmod(tmpName, 0o600); err != nil {
		return fmt.Errorf("chmod temp hub runtime file: %w", err)
	}
	return os.Rename(tmpName, path)
}

// readHubRuntime loads hub-runtime.json. A missing or corrupt file yields
// (nil, nil) — the file is a best-effort cache, never a source of truth.
func readHubRuntime() (*hubRuntimeData, error) {
	path, err := hubRuntimePath()
	if err != nil {
		return nil, err
	}
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return nil, nil
	}
	if err != nil {
		return nil, fmt.Errorf("read hub runtime file: %w", err)
	}
	var rt hubRuntimeData
	if err := json.Unmarshal(data, &rt); err != nil {
		return nil, nil
	}
	if rt.PID <= 0 || rt.Port <= 0 {
		return nil, nil
	}
	return &rt, nil
}

// removeHubRuntimeIfPID deletes hub-runtime.json only when it still
// records pid, so a late-exiting old Hub never deletes the file a newer
// Hub has already overwritten. Errors are ignored — best-effort cleanup,
// stale entries are excluded by the readers' double guard anyway.
func removeHubRuntimeIfPID(pid int) {
	rt, err := readHubRuntime()
	if err != nil || rt == nil || rt.PID != pid {
		return
	}
	if path, err := hubRuntimePath(); err == nil {
		_ = os.Remove(path)
	}
}
