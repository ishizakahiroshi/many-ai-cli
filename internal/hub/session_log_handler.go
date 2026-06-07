package hub

import (
	"encoding/base64"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strconv"

	"any-ai-cli/internal/sessionlog"
)

const (
	// sessionLogChunkDefault は /api/session-log の limit 省略時の読み出しサイズ。
	sessionLogChunkDefault = 128 * 1024
	// sessionLogChunkMax は 1 リクエストで返す生ログの上限バイト数。
	sessionLogChunkMax = 512 * 1024
)

// handleSessionLog は稼働中セッションの生 PTY ログ（.log）を範囲指定で返す。
// 過去ログビューア（ターミナルのスクロールバック上限より前を遡る UI）用。
//
//	GET /api/session-log?session_id=<id>&offset=<bytes>&limit=<bytes>
//
// offset 省略時または負値は「末尾から limit バイト」。レスポンスは JSON:
//
//	{ok, size, offset, length, data_b64}
//
// 生バイトは ANSI を含むため base64 で包む（PTY 生バイトを JSON に直接入れない規約）。
// ログファイルは wrapper が書き込み中でも共有読み取りで開ける。
func (s *Server) handleSessionLog(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	q := r.URL.Query()
	sessionID, err := strconv.Atoi(q.Get("session_id"))
	if err != nil || sessionID <= 0 {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid session_id")
		return
	}

	s.sessionsMu.Lock()
	var logPath string
	if ses := s.sessions[sessionID]; ses != nil {
		logPath = ses.LogPath
	}
	logDir := s.cfg.Hub.LogDir
	s.sessionsMu.Unlock()
	if logPath == "" {
		writeJSONError(w, http.StatusNotFound, "not_found", "session log not found")
		return
	}

	// LogPath は reattach 時に wrapper から渡される値を含むため、
	// Hub 設定の logs/sessions 配下に限定する（パストラバーサル防止）。
	cleanPath := filepath.Clean(logPath)
	allowedDir := filepath.Join(logDir, "sessions")
	if logDir == "" {
		writeJSONError(w, http.StatusNotFound, "not_found", "session log not available")
		return
	}
	if ok, _ := isPathUnderAllowedRoots(cleanPath, allowedDir); !ok {
		writeJSONError(w, http.StatusForbidden, "forbidden", "log path outside log dir")
		return
	}

	f, err := os.Open(cleanPath)
	if err != nil {
		writeJSONError(w, http.StatusNotFound, "not_found", "session log not readable")
		return
	}
	defer f.Close()
	// 書き込み中ファイルのディレクトリエントリはサイズ反映が遅延するため、
	// 開いたハンドル経由の Stat で実サイズを取る。
	info, err := f.Stat()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "internal_error", errorDetail("stat failed", err))
		return
	}
	size := info.Size()

	limit := int64(sessionLogChunkDefault)
	if v := q.Get("limit"); v != "" {
		n, err := strconv.ParseInt(v, 10, 64)
		if err != nil || n <= 0 {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid limit")
			return
		}
		limit = min(n, sessionLogChunkMax)
	}

	offset := int64(-1)
	if v := q.Get("offset"); v != "" {
		n, err := strconv.ParseInt(v, 10, 64)
		if err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid offset")
			return
		}
		offset = n
	}
	if offset < 0 {
		// 末尾から limit バイト
		offset = max(size-limit, 0)
	}
	if offset > size {
		offset = size
	}

	readN := min(limit, size-offset)
	buf := make([]byte, readN)
	n, err := f.ReadAt(buf, offset)
	if err != nil && err != io.EOF {
		writeJSONError(w, http.StatusInternalServerError, "internal_error", errorDetail("read failed", err))
		return
	}
	// jsonl 同様、生ログ返却時も既知の秘密文字列を伏字化する（ベストエフォート）。
	masked := sessionlog.MaskSecrets(string(buf[:n]))
	writeJSON(w, map[string]any{
		"ok":       true,
		"size":     size,
		"offset":   offset,
		"length":   len(masked),
		"data_b64": base64.StdEncoding.EncodeToString([]byte(masked)),
	})
}
