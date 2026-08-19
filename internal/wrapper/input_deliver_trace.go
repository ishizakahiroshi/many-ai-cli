package wrapper

import "log/slog"

// input_deliver_trace.go — 観測専用の一時コード（原因が確定したら撤去する）。
//
// internal/hub/input_deliver_trace.go の相棒。Hub 側が「送った」と記録した
// pty_input フレームが、wrapper で本当に PTY へ書かれたかを 1 行で刻む。
// Hub 側に stage=sent があって wrapper 側に stage=pty_write が無ければ
// Hub から wrapper への送達で止まっており、両方あって画面が変わらなければ
// provider CLI 側のキー解釈という切り分けになる。
//
// 記録するのは stage・session_id・input_seq・バイト長・エラー有無だけで、
// 入力本文は含まない。ゲートはセッションログの opt-in（log.session_enabled）に従う。
// 台帳: instrumentation.json の id=hub-input-deliver-trace
func traceInputDeliver(logger *slog.Logger, enabled bool, stage string, sessionID int, attrs ...any) {
	if !enabled || logger == nil {
		return
	}
	args := make([]any, 0, len(attrs)+4)
	args = append(args, "stage", stage, "session_id", sessionID)
	args = append(args, attrs...)
	logger.Info("input_deliver_wrapper", args...)
}
