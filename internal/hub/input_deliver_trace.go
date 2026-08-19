package hub

// input_deliver_trace.go — 観測専用の一時コード（原因が確定したら撤去する）。
//
// 症状: セッション 1 本だけが、ある時点から Web からの入力を PTY へ通さなくなる。
// wrapper プロセスは生きたままで、hub.log には切断も再接続も WARN も ERROR も
// 出ない（2026-08-19 / session #7 / claude / bypass permissions 同意画面で発生。
// 詳細は docs/local/bugfix_hub-input-dead-without-reconnect_2026-08-19.md）。
//
// なぜログが要るか: submitInputWithGate は「初期プロンプト注入ゲート中」または
// 「保留キューに 1 件でもある」場合、入力を直送せず pendingInput へ積む。
// pendingInput を吐き出すのは flushPendingInput だけで、その呼び出し元は wrapper の
// (再)接続と orchestration の 2 箇所しかない。したがって再接続が起きないまま保留が
// 立つと、以降の入力は永久に PTY へ届かない。この経路には現在ログが 1 行も無く、
// 「滞留しているのか、そもそも送っていないのか」を既存ログから区別できない。
//
// 記録するのは stage・session_id・バイト長・フラグ・件数だけで、入力本文は含まない。
// 台帳: instrumentation.json の id=hub-input-deliver-trace
func (s *Server) traceInputDeliver(stage string, sessionID int, attrs ...any) {
	// ゲートはセッションログの opt-in に合わせる。既定 OFF の利用者環境では 1 行も出ない。
	s.cfgMu.Lock()
	enabled := s.cfg.Log.SessionEnabled
	s.cfgMu.Unlock()
	if !enabled || s.logger == nil {
		return
	}
	args := make([]any, 0, len(attrs)+4)
	args = append(args, "stage", stage, "session_id", sessionID)
	args = append(args, attrs...)
	s.logger.Info("input_deliver", args...)
}
