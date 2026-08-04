package wrapper

// trace.go: 「Web から送信した文字列が CLI 入力欄に滞留し、もう一度 Enter を押さないと
// 送信されない」症状（2026-08-04 調査）の wrapper 側観測。
//
// wrapper の slog は stderr 直行（cmd/many-ai-cli/main.go:200）で、PTY を占有した
// 内側 CLI の画面と同じ端末へ書き込む。観測のためにここへ Info を足すと TUI が壊れる
// ため、専用ファイルへ追記するだけの最小ロガーを別に持つ。
//
// 出力先: <cfg.Hub.LogDir>/wrapper-trace.jsonl（全 wrapper プロセス共有・pid で区別）
// Hub 側 hub.log の input_trace と ts_ns で突き合わせる前提（別プロセスなので
// 単調時計ではなく壁時計の ns で揃える）。
//
// 調査完了後に撤去する前提の一時計測コードであり、恒久機能ではない。

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
	"time"
)

type inputTracer struct {
	mu  sync.Mutex
	f   *os.File
	pid int
}

// newInputTracer は追記モードでトレースファイルを開く。失敗しても nil を返すだけで
// エラーにしない（観測が本体の起動を妨げてはいけない）。nil レシーバでも emit /
// close は安全に呼べる。
func newInputTracer(logDir string) *inputTracer {
	if logDir == "" {
		return nil
	}
	if err := os.MkdirAll(logDir, 0o755); err != nil {
		return nil
	}
	f, err := os.OpenFile(filepath.Join(logDir, "wrapper-trace.jsonl"), os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o644)
	if err != nil {
		return nil
	}
	return &inputTracer{f: f, pid: os.Getpid()}
}

func (t *inputTracer) emit(event string, sessionID int, fields map[string]any) {
	if t == nil {
		return
	}
	rec := make(map[string]any, len(fields)+5)
	for k, v := range fields {
		rec[k] = v
	}
	now := time.Now()
	rec["ts"] = now.Format(time.RFC3339Nano)
	rec["ts_ns"] = now.UnixNano()
	rec["event"] = event
	rec["session_id"] = sessionID
	rec["pid"] = t.pid
	b, err := json.Marshal(rec)
	if err != nil {
		return
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	_, _ = t.f.Write(append(b, '\n'))
}

func (t *inputTracer) close() {
	if t == nil {
		return
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	_ = t.f.Close()
}

// traceHex は先頭または末尾 n バイトを hex で返す。ペースト終端（1b5b3230317e）と
// 確定 \r（0d）を目視で区別するために使う。
func traceHex(b []byte, n int, tail bool) string {
	if len(b) > n {
		if tail {
			b = b[len(b)-n:]
		} else {
			b = b[:n]
		}
	}
	return hex.EncodeToString(b)
}
