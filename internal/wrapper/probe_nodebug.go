//go:build !maidebug

package wrapper

// probe_nodebug.go — 記録点フックの空実装。既定（タグ無し）のビルドはこちらを使う。
//
// 記録点は製品コードに残るが、ここが空なのでコンパイラが呼び出しごと畳む。
// 「何も実行されない」ことは保証するが、引数の評価コストが完全に消えることまでは
// 保証しない。probe へ渡す値は安価なものに限ること。
//
// 対になる実装は probe_debug.go。シグネチャは 2 枚で完全に一致させること。

import (
	"log/slog"

	"many-ai-cli/internal/config"
)

func installProbes(logger *slog.Logger, cfg *config.Config) {}

func probe(channel string, args ...any) {}
