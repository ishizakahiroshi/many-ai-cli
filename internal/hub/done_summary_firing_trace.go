//go:build maidebug

package hub

// done_summary_firing_trace.go — 観測専用の一時コード（原因が確定したら撤去する）。
//
// フォールバック候補の入力と Git 差分結果を、本文なしの数値・フラグだけで
// 同じセッションログへ記録する。build 側は maidebug、runtime 側は
// log.session_enabled の二重ゲートにする。台帳: instrumentation.json の
// id=done-summary-firing-trace

func init() {
	registerProbeSink("done.fallback", func(s *Server, args ...any) {
		if s == nil || s.logger == nil {
			return
		}
		s.cfgMu.Lock()
		enabled := s.cfg.Log.SessionEnabled
		s.cfgMu.Unlock()
		if !enabled {
			return
		}
		s.logger.Info("done_summary_firing_trace", args...)
	})
}
