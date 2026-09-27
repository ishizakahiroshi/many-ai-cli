//go:build maidebug

package hub

import (
	"encoding/json"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"sync"
	"time"

	"gopkg.in/natefinch/lumberjack.v2"
)

// Dedicated, bounded diagnostics: independent of log.enabled, debug builds only.
// The schema intentionally cannot carry speech, terminal text, URLs or errors.
type uiFreezeEvent struct {
	ID    int64  `json:"id"`
	Phase string `json:"phase"`
	Edge  string `json:"edge"`
	At    int64  `json:"at"`
	Size  int64  `json:"size"`
}

type uiFreezePacket struct {
	Schema  int             `json:"schema"`
	Tab     string          `json:"tab"`
	Reason  string          `json:"reason"`
	At      int64           `json:"at"`
	GapMS   int64           `json:"gapMs"`
	Visible bool            `json:"visible"`
	Dropped int64           `json:"dropped"`
	Events  []uiFreezeEvent `json:"events"`
	Open    []uiFreezeEvent `json:"open"`
}

var uiFreezeTabID = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)

func validUIFreezePacket(p uiFreezePacket) bool {
	if p.Schema != 1 || !uiFreezeTabID.MatchString(p.Tab) || p.At <= 0 || p.At > 1e14 || p.GapMS < 0 || p.GapMS > 1e12 || p.Dropped < 0 || p.Dropped > 1e15 || len(p.Events) > 64 || len(p.Open) > 16 {
		return false
	}
	switch p.Reason {
	case "checkpoint", "suspected-stall", "recovered", "worker-gap":
	default:
		return false
	}
	validEvent := func(e uiFreezeEvent) bool {
		if e.ID <= 0 || e.ID > 1e15 || e.At <= 0 || e.At > 1e14 || e.Size < 0 || e.Size > 1e9 || (e.Edge != "begin" && e.Edge != "end") {
			return false
		}
		switch e.Phase {
		case "voice.result", "input.layout", "terminal.fit", "terminal.links":
			return true
		}
		return false
	}
	for _, e := range p.Events {
		if !validEvent(e) {
			return false
		}
	}
	for _, e := range p.Open {
		if !validEvent(e) || e.Edge != "begin" {
			return false
		}
	}
	return true
}

func init() {
	registerProbeRoute("/api/debug/ui-freeze", newUIFreezeHandler)
}

func newUIFreezeHandler(s *Server) http.HandlerFunc {
	var mu sync.Mutex
	var lastWrite time.Time
	dir := s.snapshotCfg().Hub.LogDir
	writer := &lumberjack.Logger{
		Filename: filepath.Join(dir, "ui-freeze.jsonl"),
		MaxSize:  1, MaxBackups: 2, MaxAge: 7,
	}
	return func(w http.ResponseWriter, r *http.Request) {
		if !s.guard(w, r, http.MethodPost) {
			return
		}
		var packet uiFreezePacket
		decoder := json.NewDecoder(http.MaxBytesReader(w, r.Body, 16384))
		decoder.DisallowUnknownFields()
		if err := decoder.Decode(&packet); err != nil {
			http.Error(w, "invalid capture", http.StatusBadRequest)
			return
		}
		var extra any
		if err := decoder.Decode(&extra); err != io.EOF || !validUIFreezePacket(packet) {
			http.Error(w, "invalid capture", http.StatusBadRequest)
			return
		}
		mu.Lock()
		defer mu.Unlock()
		// Bound aggregate writes even with multiple authenticated tabs. A failed
		// request is retried by the worker; no per-tab unbounded server state.
		if time.Since(lastWrite) < 100*time.Millisecond {
			w.WriteHeader(http.StatusTooManyRequests)
			return
		}
		if dir == "" || os.MkdirAll(dir, 0o700) != nil {
			http.Error(w, "capture unavailable", http.StatusServiceUnavailable)
			return
		}
		record := struct {
			ReceivedAt string `json:"receivedAt"`
			Version    string `json:"hubVersion"`
			Commit     string `json:"hubCommit"`
			WebHash    string `json:"hubWebHash"`
			uiFreezePacket
		}{time.Now().UTC().Format(time.RFC3339Nano), s.version, s.gitCommit, s.webSrcHash, packet}
		line, err := json.Marshal(record)
		if err == nil {
			_, err = writer.Write(append(line, '\n'))
		}
		// No long-lived file handle or shutdown hook; also permits shared-read
		// collection on Windows while the Hub continues running.
		closeErr := writer.Close()
		if err != nil || closeErr != nil {
			http.Error(w, "capture unavailable", http.StatusServiceUnavailable)
			return
		}
		lastWrite = time.Now()
		w.WriteHeader(http.StatusNoContent)
	}
}
