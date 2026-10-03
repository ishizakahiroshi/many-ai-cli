//go:build ignore

// Synthetic compatibility oracle. Run only against an explicit isolated root,
// after the Rust writer has closed. No Hub, provider, or home lookup is involved.
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"many-ai-cli/internal/sessionstore"
)

func main() {
	if len(os.Args) != 2 || !filepath.IsAbs(os.Args[1]) {
		panic("expected explicit absolute synthetic root")
	}
	root, err := filepath.EvalSymlinks(os.Args[1])
	if err != nil {
		panic(err)
	}
	temporary, err := filepath.EvalSymlinks(os.TempDir())
	if err != nil {
		panic(err)
	}
	relative, err := filepath.Rel(temporary, root)
	if err != nil || relative == "." || relative == ".." || strings.HasPrefix(relative, ".."+string(filepath.Separator)) || !strings.HasPrefix(filepath.Base(root), "many-ai-storage-oracle-") {
		panic("isolated temporary oracle root required")
	}
	markerPath := filepath.Join(root, "synthetic-storage-fixture")
	info, err := os.Lstat(markerPath)
	if err != nil || !info.Mode().IsRegular() || info.Size() > 64 {
		panic("bounded regular marker required")
	}
	markerFile, err := os.Open(markerPath)
	if err != nil {
		panic(err)
	}
	marker, err := io.ReadAll(io.LimitReader(markerFile, 65))
	markerFile.Close()
	if err != nil || string(marker) != "Go/Rust storage compatibility only\n" {
		panic("synthetic fixture marker required")
	}
	store, err := sessionstore.OpenForLogDir(filepath.Join(root, "logs"))
	if err != nil {
		panic(err)
	}
	defer store.Close()
	rows, err := store.ListSessions(100, true)
	if err != nil {
		panic(err)
	}
	if len(rows) != 1 || rows[0].Provider != "copilot" || rows[0].Label != "edited card" {
		panic("Rust session row is not readable by Go baseline")
	}
	messages, err := store.ChatMessagesBySessionID(rows[0].ID, 100)
	if err != nil {
		panic(err)
	}
	if len(messages) != 2 || messages[0].RawText != "synthetic rollback query" || messages[1].RawText != "synthetic answer" {
		panic("Rust messages are not readable by Go baseline")
	}
	search, err := store.SearchMessages("rollback", 10)
	if err != nil || len(search) != 1 {
		panic("Rust FTS index is not readable by Go baseline")
	}
	approvals, err := store.ApprovalsBySessionID(rows[0].ID, 10, false)
	if err != nil || len(approvals) != 1 || approvals[0].State != "resolved" || approvals[0].SourceEpoch != 3 {
		panic("Rust approval ledger is not readable by Go baseline")
	}
	if _, err = store.StartSession(sessionstore.SessionStart{LiveSessionID: 1, Provider: "copilot", JSONLPath: "virtual-live-1"}); err != nil {
		panic(err)
	}
	if err = store.StoreEvent(1, map[string]any{"type": "user_input", "text": "synthetic Go rollback continuation", "ts": "2025-01-01T02:00:00Z"}); err != nil {
		panic(err)
	}
	if err = json.NewEncoder(os.Stdout).Encode(map[string]any{"go_baseline": "21d0bc7935a2c4696fb89ccff2e324157a528c2d", "sessions": len(rows), "messages": len(messages), "approvals": len(approvals), "search": len(search), "go_write": true}); err != nil {
		panic(err)
	}
	fmt.Fprintln(os.Stderr, "synthetic Go read/write compatibility verified")
}
