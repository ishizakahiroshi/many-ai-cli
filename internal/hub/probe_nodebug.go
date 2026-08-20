//go:build !maidebug

package hub

// probe_nodebug.go — 記録点フックの空実装。既定（タグ無し）のビルドはこちらを使う。
//
// 記録点は製品コードに残るが、ここが空なのでコンパイラが呼び出しごと畳む。
// 「何も実行されない」ことは保証するが、引数の評価コストが完全に消えることまでは
// 保証しない。probe へ渡す値は安価なもの（整数・短い文字列）に限り、重い
// スナップショットは probeLazy のクロージャ越しに渡すこと。
//
// 対になる実装は probe_debug.go。シグネチャは 2 枚で完全に一致させること。

import "net/http"

func (s *Server) probe(channel string, args ...any) {}

func (s *Server) probeLazy(channel string, build func() []any) {}

func (s *Server) registerProbeRoutes(mux *http.ServeMux) {}
