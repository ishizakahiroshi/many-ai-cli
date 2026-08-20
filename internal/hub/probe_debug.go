//go:build maidebug

package hub

// probe_debug.go — 記録点フックの実装。maidebug タグ付きビルドにだけ入る。
//
// 記録点（server.go / input_gate.go などにある probe 呼び出し）は製品コードに
// 残り続け、収集する側だけをここに登録する。登録は各計測ファイルの init() から
// 行い、計測ファイル自身も同じ build tag を持つので、タグ無しビルドでは
// 登録側もろとも存在しない。
//
// 対になる空実装は probe_nodebug.go。シグネチャは 2 枚で完全に一致させること。

import "net/http"

type probeSink func(s *Server, args ...any)

// 登録は init() の中だけで行う（読み出しはその後なので mutex を持たない）。
var probeSinks = map[string]probeSink{}

func registerProbeSink(channel string, sink probeSink) { probeSinks[channel] = sink }

func (s *Server) probe(channel string, args ...any) {
	if f := probeSinks[channel]; f != nil {
		f(s, args...)
	}
}

// 引数の組み立て自体が重い場合に使う（sink が無ければ build を呼ばない）。
func (s *Server) probeLazy(channel string, build func() []any) {
	if f := probeSinks[channel]; f != nil {
		f(s, build()...)
	}
}

// HTTP ルートも同じ仕組みで着脱する。handler は *Server のメソッドとして
// 書かれるので、登録側は *Server を受けて HandlerFunc を返す形にする。
var probeRoutes = map[string]func(*Server) http.HandlerFunc{}

func registerProbeRoute(path string, h func(*Server) http.HandlerFunc) { probeRoutes[path] = h }

func (s *Server) registerProbeRoutes(mux *http.ServeMux) {
	for path, h := range probeRoutes {
		mux.HandleFunc(path, h(s))
	}
}
