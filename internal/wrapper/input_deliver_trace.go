//go:build maidebug

package wrapper

import (
	"log/slog"

	"many-ai-cli/internal/config"
)

// input_deliver_trace.go — 観測専用の一時コード（原因が確定したら撤去する）。
//
// internal/hub/input_deliver_trace.go の相棒。Hub 側が「送った」と記録した
// pty_input フレームが、wrapper で本当に PTY へ書かれたかを 1 行で刻む。
// Hub 側に stage=sent があって wrapper 側に stage=pty_write が無ければ
// Hub から wrapper への送達で止まっており、両方あって画面が変わらなければ
// provider CLI 側のキー解釈という切り分けになる。
//
// ゲートは 2 層。build 側（maidebug タグが無ければこのファイルごと存在しない）と、
// runtime 側（セッションログの opt-in = log.session_enabled）。
// 記録するのは stage・session_id・input_seq・バイト長・エラー有無だけで、
// 入力本文は含まない。台帳: instrumentation.json の id=hub-input-deliver-trace

func init() {
	registerProbeInstaller(func(logger *slog.Logger, cfg *config.Config) {
		if logger == nil || cfg == nil || !cfg.Log.SessionEnabled {
			return
		}
		for _, channel := range []string{"input.pty_write", "input.pty_write_dup"} {
			stage := channel[len("input."):]
			registerProbeSink(channel, func(args ...any) {
				logger.Info("input_deliver_wrapper", append([]any{"stage", stage}, args...)...)
			})
		}
	})
}
