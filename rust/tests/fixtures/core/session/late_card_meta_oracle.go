//go:build ignore

// Deterministic fixed-Go commit ordering. StoreEvent represents an earlier
// queued lifecycle event that reaches the DB after a synchronous card edit.
// Synthetic roots only; no Hub, providers or actual profile data are accessed.
package main

import (
	"encoding/json"
	"flag"
	"os"
	"path/filepath"
	"time"

	"many-ai-cli/internal/sessionstore"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func cardValue(card sessionstore.SessionCardMeta) map[string]any {
	return map[string]any{"label": card.Label, "pinned": card.Pinned, "color": card.Color, "note": card.Note, "auto_title": card.AutoTitle}
}
func observe(workRoot, kind string) map[string]any {
	root, err := os.MkdirTemp(workRoot, "many-ai-late-card-")
	must(err)
	defer os.RemoveAll(root)
	store, err := sessionstore.OpenForLogDir(filepath.Join(root, "logs"))
	must(err)
	defer func() { must(store.Close()) }()
	stamp := "2026-01-02T03:04:05Z"
	at, err := time.Parse(time.RFC3339, stamp)
	must(err)
	start := sessionstore.SessionStart{LiveSessionID: 7, Provider: "copilot", CWD: root, Label: "launch-label", State: "standby", StartedAt: stamp, JSONLPath: filepath.Join(root, "sessions", "synthetic.jsonl")}
	_, err = store.StartSession(start)
	must(err)
	event := func(typ string) map[string]any {
		return map[string]any{"type": typ, "session_id": 7, "ts": stamp, "label": "launch-label"}
	}
	if kind == "session_reattach" {
		must(store.StoreEvent(7, event("session_start")))
	}
	must(store.UpdateSessionCardMeta(7, sessionstore.SessionCardMeta{Label: "renamed card", Pinned: true, Color: "blue", Note: "synthetic note", AutoTitle: "synthetic title"}))
	if kind == "session_reattach" {
		store.EndSession(7, "disconnected", "", at)
		start.State = "running"
		_, err = store.StartSession(start)
		must(err)
	}
	before, err := store.SessionCardMetaByLiveSession(7)
	must(err)
	must(store.StoreEvent(7, event(kind)))
	after, err := store.SessionCardMetaByLiveSession(7)
	must(err)
	rows, err := store.TimelineByLiveSession(7, 10)
	must(err)
	historyLabel := ""
	for _, row := range rows {
		if row.Type == kind {
			historyLabel, _ = row.Payload["label"].(string)
		}
	}
	store.EndSession(7, "disconnected", "", at)
	start.State = "running"
	_, err = store.StartSession(start)
	must(err)
	next, err := store.SessionCardMetaByLiveSession(7)
	must(err)
	return map[string]any{"kind": kind, "before_event": cardValue(before), "after_event": cardValue(after), "after_next_start": cardValue(next), "history_label": historyLabel}
}
func main() {
	root := flag.String("work-root", "", "absolute owned synthetic work root")
	flag.Parse()
	if !filepath.IsAbs(*root) {
		panic("absolute synthetic work root required")
	}
	must(os.MkdirAll(*root, 0700))
	values := []map[string]any{observe(*root, "session_start"), observe(*root, "session_reattach")}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(values))
}
