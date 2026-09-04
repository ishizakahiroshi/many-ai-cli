//go:build maidebug

package hub

// debug_alt_scroll.go
//
// 一時観測: 代替画面のセッションで「上へは一部行くが、昔の内容まで遡れない」症状の
// 切り分け用。ブラウザ側（web/src/debug/alt-scroll-travel.ts）が URL に
// ?scrolldebug=1 を付けて開かれたときだけ POST してくる。既定では 1 件も届かない。
//
// 受け取るのは件数・行数・方向・セッション ID だけで、ターミナル本文や入力テキストは
// 含まない。原因が確定したら撤去する（instrumentation.json の alt-scroll-travel）。

import (
	"net/http"
	"sort"
)

// ルートは自分で登録する。server.go 側は s.registerProbeRoutes(mux) の 1 行だけで、
// このファイルを消せば登録も一緒に消える。
func init() {
	registerProbeRoute("/api/debug/alt-scroll", func(s *Server) http.HandlerFunc {
		return s.handleDebugAltScroll
	})
}

const (
	debugAltScrollMaxKeys     = 24
	debugAltScrollMaxValueLen = 40
)

func (s *Server) handleDebugAltScroll(w http.ResponseWriter, r *http.Request) {
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
	if len(keys) > debugAltScrollMaxKeys {
		keys = keys[:debugAltScrollMaxKeys]
	}
	attrs := make([]any, 0, len(keys)*2)
	for _, k := range keys {
		v := body[k]
		if sv, ok := v.(string); ok && len(sv) > debugAltScrollMaxValueLen {
			v = sv[:debugAltScrollMaxValueLen]
		}
		attrs = append(attrs, k, v)
	}
	s.logger.Info("debug alt scroll", attrs...)
	writeJSON(w, map[string]any{"ok": true})
}
