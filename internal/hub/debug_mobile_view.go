//go:build maidebug

package hub

// debug_mobile_view.go
//
// 一時観測: スマホ幅で mobile-terminal-lite のチャットが空のまま表示される症状の切り分け用。
// ブラウザ側（web/src/app/debug-mobile-view.ts）が URL に ?mtldebug=1 を付けて開かれた
// ときだけ POST してくる。既定では 1 件も届かない。
//
// 受け取るのは行数・文字数・寸法・状態フラグと userAgent 先頭のみで、ターミナル本文や
// 入力テキストは含まない。原因が確定したら撤去する（instrumentation.json の mobile-lite-empty）。

import (
	"net/http"
	"sort"
)

// ルートは自分で登録する。server.go 側は s.registerProbeRoutes(mux) の 1 行だけで、
// このファイルを消せば登録も一緒に消える。
func init() {
	registerProbeRoute("/api/debug/mobile-view", func(s *Server) http.HandlerFunc {
		return s.handleDebugMobileView
	})
}

const (
	debugMobileViewMaxKeys     = 64
	debugMobileViewMaxValueLen = 200
)

func (s *Server) handleDebugMobileView(w http.ResponseWriter, r *http.Request) {
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
	if len(keys) > debugMobileViewMaxKeys {
		keys = keys[:debugMobileViewMaxKeys]
	}
	attrs := make([]any, 0, len(keys)*2)
	for _, k := range keys {
		v := body[k]
		if sv, ok := v.(string); ok && len(sv) > debugMobileViewMaxValueLen {
			v = sv[:debugMobileViewMaxValueLen]
		}
		attrs = append(attrs, k, v)
	}
	s.logger.Info("debug mobile view sample", attrs...)
	writeJSON(w, map[string]any{"ok": true})
}
