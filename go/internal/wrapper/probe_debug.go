//go:build maidebug

package wrapper

// probe_debug.go — 記録点フックの実装。maidebug タグ付きビルドにだけ入る。
//
// hub 側と違い wrapper には共通のレシーバが無く、収集には logger と cfg が要る。
// これを記録点から追い出すため、起動時に installProbes() で 1 度だけ sink へ
// 注入する。計測ファイルは init() で registerProbeInstaller() を呼び、
// 注入されたタイミングで自分の sink を registerProbeSink() する。
//
// 対になる空実装は probe_nodebug.go。シグネチャは 2 枚で完全に一致させること。

import (
	"log/slog"

	"many-ai-cli/internal/config"
)

type probeSink func(args ...any)

var (
	// 登録は installProbes() の中だけで行う（読み出しはその後）。
	probeSinks      = map[string]probeSink{}
	probeInstallers []func(logger *slog.Logger, cfg *config.Config)
)

func registerProbeSink(channel string, sink probeSink) { probeSinks[channel] = sink }

// 計測ファイルの init() から呼ぶ。
func registerProbeInstaller(f func(*slog.Logger, *config.Config)) {
	probeInstallers = append(probeInstallers, f)
}

// wrapper 起動時に 1 度だけ呼ぶ。計測ファイルが無ければ何もしない。
func installProbes(logger *slog.Logger, cfg *config.Config) {
	for _, f := range probeInstallers {
		f(logger, cfg)
	}
}

func probe(channel string, args ...any) {
	if f := probeSinks[channel]; f != nil {
		f(args...)
	}
}
