//go:build maidebug

package hub

// debug_terminal_geometry.go
//
// 一時観測: 承認ポップアップ表示中にスクロールすると画面が重なって描かれ、直前の内容が
// 読めなくなる症状の切り分け用。ブラウザ側（web/src/debug/terminal-geometry.ts）が
// URL に ?geodebug=1 を付けて開かれたときだけ POST してくる。既定では 1 件も届かない。
//
// 受け取るのは xterm と PTY の寸法・要素サイズ・状態フラグだけで、ターミナル本文や
// 入力テキストは含まない。原因が確定したら撤去する
// （instrumentation.json の terminal-grid-divergence）。

import (
	"net/http"
	"sort"
)

// ルートは自分で登録する。server.go 側は s.registerProbeRoutes(mux) の 1 行だけで、
// このファイルを消せば登録も一緒に消える。
func init() {
	registerProbeRoute("/api/debug/terminal-geometry", func(s *Server) http.HandlerFunc {
		return s.handleDebugTerminalGeometry
	})
}

const (
	debugTerminalGeometryMaxKeys     = 32
	debugTerminalGeometryMaxValueLen = 80
)

func (s *Server) handleDebugTerminalGeometry(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var body map[string]any
	if !decodeJSON(w, r, &body) {
		return
	}
	keys := make([]string, 0, len(body))
	for k := range body {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	if len(keys) > debugTerminalGeometryMaxKeys {
		keys = keys[:debugTerminalGeometryMaxKeys]
	}
	attrs := make([]any, 0, len(keys)*2)
	for _, k := range keys {
		v := body[k]
		if sv, ok := v.(string); ok && len(sv) > debugTerminalGeometryMaxValueLen {
			v = sv[:debugTerminalGeometryMaxValueLen]
		}
		attrs = append(attrs, k, v)
	}
	s.logger.Info("debug terminal geometry", attrs...)
	writeJSON(w, map[string]any{"ok": true})
}
