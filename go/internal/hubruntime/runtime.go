// Package hubruntime owns the small runtime ledger shared by Hub and wrapper.
//
// The configured Hub port is not necessarily the port the process actually
// bound: Hub can move to a nearby free port. Keeping the ledger here avoids
// having each caller implement a subtly different fallback-port lookup.
package hubruntime

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"time"
)

const (
	// FileName is the per-user runtime ledger name under the CLI config dir.
	FileName     = "hub-runtime.json"
	probeTimeout = 500 * time.Millisecond
)

// Data is the content of hub-runtime.json. The token is intentionally not
// stored here; callers combine the recorded port with their current config
// token when probing the Hub.
type Data struct {
	Version   int       `json:"version"`
	PID       int       `json:"pid"`
	Port      int       `json:"port"`
	StartedAt time.Time `json:"started_at"`
}

// Path returns the path to ~/.many-ai-cli/hub-runtime.json.
func Path() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("home dir: %w", err)
	}
	return filepath.Join(home, ".many-ai-cli", FileName), nil
}

// Write records the calling process as the Hub bound to port. The write is
// atomic so readers never observe a partially-written JSON document.
func Write(port int) error {
	if port <= 0 {
		return fmt.Errorf("invalid hub runtime port: %d", port)
	}
	path, err := Path()
	if err != nil {
		return err
	}
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return fmt.Errorf("mkdir hub runtime dir: %w", err)
	}

	data, err := json.MarshalIndent(Data{
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
	defer os.Remove(tmpName)

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

// Read loads hub-runtime.json. A missing or corrupt file is an empty cache,
// not a fatal configuration error.
func Read() (*Data, error) {
	path, err := Path()
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
	var rt Data
	if err := json.Unmarshal(data, &rt); err != nil {
		return nil, nil
	}
	if rt.PID <= 0 || rt.Port <= 0 {
		return nil, nil
	}
	return &rt, nil
}

// RemoveIfPID removes the ledger only when it still belongs to pid. A late
// exiting old Hub must not remove a newer Hub's record.
func RemoveIfPID(pid int) {
	rt, err := Read()
	if err != nil || rt == nil || rt.PID != pid {
		return
	}
	if path, err := Path(); err == nil {
		_ = os.Remove(path)
	}
}

// RunningPort returns the port of a verifiably running Hub. It checks the
// configured port first, then the runtime ledger using a PID plus /api/info
// double guard.
func RunningPort(configuredPort int, token string) (int, bool) {
	return RunningPortWith(configuredPort, pidAlive, func(port int) bool {
		return probeHubInfo(port, token)
	})
}

// RunningPortWith is RunningPort with injectable liveness and probe checks.
// It is kept small so Hub and wrapper tests can exercise the fallback decision
// without starting a real Hub process.
func RunningPortWith(configuredPort int, alive func(pid int) bool, probe func(port int) bool) (int, bool) {
	if configuredPort > 0 && probe(configuredPort) {
		return configuredPort, true
	}
	rt, err := Read()
	if err != nil || rt == nil {
		return 0, false
	}
	if !alive(rt.PID) {
		RemoveIfPID(rt.PID)
		return 0, false
	}
	if rt.Port == configuredPort || !probe(rt.Port) {
		return 0, false
	}
	return rt.Port, true
}

func probeHubInfo(port int, token string) bool {
	if port <= 0 {
		return false
	}
	u := fmt.Sprintf("http://127.0.0.1:%d/api/info?token=%s", port, url.QueryEscape(token))
	client := &http.Client{Timeout: probeTimeout}
	resp, err := client.Get(u)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}
