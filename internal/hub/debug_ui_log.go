package hub

import (
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
)

// 一時観測用エンドポイント（Codex ターミナルに空行が積もる / rows が発振する件）。
//
// Web 側でしか観測できない「どのレイアウト変化が pty_resize を誘発したか」を
// 受け取り、日次 jsonl へ追記する。Hub 側で既に記録している pty_resize 履歴
// （forwardResize の writeHistory）と時刻で突き合わせるための相方。
// 原因が確定したら本ファイルと web/src/app/debug-ui-log.ts ごと撤去する。
const (
	debugUILogMaxEvents     = 400
	debugUILogDailyMaxBytes = 16 << 20
	debugUILogRetentionDays = 3
)

var debugUILogMu sync.Mutex

type debugUILogRequest struct {
	Events []map[string]any `json:"events"`
}

func (s *Server) handleDebugUILog(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body debugUILogRequest
	if !decodeJSON(w, r, &body) {
		return
	}
	written := s.appendDebugUILog(body.Events, time.Now())
	writeJSON(w, map[string]any{"ok": true, "written": written})
}

// appendDebugUILog は受け取ったイベントを jsonl へ 1 行ずつ追記し、書けた件数を返す。
// 観測用なので失敗は握り潰す（本体動作に影響させない）。
func (s *Server) appendDebugUILog(events []map[string]any, at time.Time) int {
	if len(events) == 0 || s.cfg == nil {
		return 0
	}
	if len(events) > debugUILogMaxEvents {
		events = events[:debugUILogMaxEvents]
	}
	dir := strings.TrimSpace(s.cfg.Hub.LogDir)
	if dir == "" {
		return 0
	}
	dir = filepath.Join(dir, "debug")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return 0
	}
	path := filepath.Join(dir, "ui-log-"+at.Format("2006-01-02")+".jsonl")

	debugUILogMu.Lock()
	defer debugUILogMu.Unlock()

	if fi, err := os.Stat(path); err == nil && fi.Size() >= debugUILogDailyMaxBytes {
		return 0
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return 0
	}
	defer f.Close()

	recvTS := at.Format(time.RFC3339Nano)
	written := 0
	for _, ev := range events {
		if ev == nil {
			continue
		}
		// クライアント時刻がずれていても並べ替えられるよう、受信時刻も残す。
		ev["recv_ts"] = recvTS
		line, err := json.Marshal(ev)
		if err != nil {
			continue
		}
		if _, err := f.Write(append(line, '\n')); err != nil {
			break
		}
		written++
	}

	pruneDebugUILogs(dir, at)
	return written
}

// pruneDebugUILogs は保持日数を超えた日次ファイルを削除する。
// ファイル名の日付で判定する（mtime だとコピー・同期で狂う）。
func pruneDebugUILogs(dir string, now time.Time) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	names := make([]string, 0, len(entries))
	for _, e := range entries {
		if e.IsDir() || !strings.HasPrefix(e.Name(), "ui-log-") || !strings.HasSuffix(e.Name(), ".jsonl") {
			continue
		}
		names = append(names, e.Name())
	}
	if len(names) <= debugUILogRetentionDays {
		return
	}
	sort.Strings(names)
	for _, name := range names[:len(names)-debugUILogRetentionDays] {
		_ = os.Remove(filepath.Join(dir, name))
	}
}
