package hub

import (
	"encoding/json"
	"io"
	"net/http"
	"path/filepath"
	"time"

	"gopkg.in/natefinch/lumberjack.v2"
)

// handleDebugBatchLog は Hub Web UI から fire-and-forget で送られる 1 行 JSON を
// ~/.many-ai-cli/logs/debug/batch-log.jsonl に append する一時 endpoint。
// 一括承認 action-bar が消えない bug (docs/local/bugfix_batch-approval-actionbar-not-hidden_2026-07-21.md)
// の原因判別のため、CLAUDE.md feedback-no-manual-devtools-ask に沿ってログ計装用に導入した。
// 過去に同種の一時 endpoint /api/debug/cursor-hide-log を導入→回収した前例あり (CHANGELOG.md)。
// 原因判明後は endpoint / dlog 呼び出しごと削除する。
func (s *Server) handleDebugBatchLog(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	body, err := io.ReadAll(io.LimitReader(r.Body, 64*1024))
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "read body failed")
		return
	}
	defer r.Body.Close()
	var payload map[string]any
	if err := json.Unmarshal(body, &payload); err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid json")
		return
	}
	payload["server_ts"] = time.Now().Format(time.RFC3339Nano)

	s.cfgMu.Lock()
	logDir, logCfg := s.cfg.Hub.LogDir, s.cfg.Log
	s.cfgMu.Unlock()
	if logDir == "" {
		writeJSON(w, map[string]any{"ok": false, "reason": "log dir unavailable"})
		return
	}

	line, err := json.Marshal(payload)
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "marshal_failed", err.Error())
		return
	}
	roller := &lumberjack.Logger{
		Filename:   filepath.Join(logDir, "debug", "batch-log.jsonl"),
		MaxSize:    logCfg.MaxSizeMB,
		MaxBackups: logCfg.MaxBackups,
		Compress:   logCfg.Compress,
	}
	if _, err := roller.Write(append(line, '\n')); err != nil {
		s.logger.Warn("debug batch log write failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "write_failed", err.Error())
		return
	}
	_ = roller.Close()
	writeJSON(w, map[string]any{"ok": true})
}
