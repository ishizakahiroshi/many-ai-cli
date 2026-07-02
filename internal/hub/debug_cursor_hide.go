package hub

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/sessionlog"
)

// 一時デバッグ用: grok の応答が filterCursorHideShowBlocksForDisplay で破棄され
// extractAndSetLiveStatus 経由にのみ渡っているという仮説を実機で確認するための
// 計装。検証後に削除予定（terminal.ts 側の fetch 呼び出しとセットで撤去する）。

var debugCursorHideMu sync.Mutex

type debugCursorHideEntry struct {
	SessionID  int    `json:"session_id"`
	Provider   string `json:"provider"`
	Source     string `json:"source"`
	HasAbsPos  bool   `json:"has_abs_pos"`
	HasNewline bool   `json:"has_newline"`
	Text       string `json:"text"`
	// 観測強化フィールド（2026-07-02 spinner-scrollback-fossilization 調査）。
	// 未設定時は 0/空文字（既存 jsonl と互換）。
	TerminalRows int `json:"terminal_rows,omitempty"`
	TerminalCols int `json:"terminal_cols,omitempty"`
	MaxCUPRow    int `json:"max_cup_row,omitempty"`
	MaxCUPCol    int `json:"max_cup_col,omitempty"`
	LFCount      int `json:"lf_count,omitempty"`
	BlockBytes   int `json:"block_bytes,omitempty"`
}

func (s *Server) handleDebugCursorHideLog(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body debugCursorHideEntry
	if !decodeJSON(w, r, &body) {
		return
	}
	dir, err := config.Dir()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "server_error", "config dir")
		return
	}
	path := filepath.Join(dir, "debug-cursor-hide.jsonl")

	debugCursorHideMu.Lock()
	defer debugCursorHideMu.Unlock()
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, sessionlog.PrivateFileMode) // #nosec G304 -- パスは config.Dir() 配下の固定ファイル名
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "server_error", "open log")
		return
	}
	defer f.Close()

	line := struct {
		Time string `json:"time"`
		debugCursorHideEntry
	}{
		Time:                 time.Now().Format(time.RFC3339Nano),
		debugCursorHideEntry: body,
	}
	if err := json.NewEncoder(f).Encode(line); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "server_error", "write log")
		return
	}
	writeJSON(w, map[string]bool{"ok": true})
}
