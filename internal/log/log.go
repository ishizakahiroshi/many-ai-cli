package log

import (
	"io"
	"log/slog"
	"os"
	"path/filepath"

	"gopkg.in/natefinch/lumberjack.v2"

	"ai-cli-hub/internal/config"
)

// NewFileLogger はファイル出力付き slog.Logger を返す。
// cfg.Enabled が false のとき stderr のみにフォールバックする。
// alsoStderr=true のとき file + stderr の MultiWriter になる。
// debug=true のとき LevelDebug、それ以外は LevelInfo。
func NewFileLogger(logDir string, cfg config.LogConfig, debug bool, alsoStderr bool) *slog.Logger {
	level := slog.LevelInfo
	if debug {
		level = slog.LevelDebug
	}
	var w io.Writer
	if cfg.Enabled {
		roller := &lumberjack.Logger{
			Filename:   filepath.Join(logDir, "hub.log"),
			MaxSize:    cfg.MaxSizeMB,
			MaxBackups: cfg.MaxBackups,
			Compress:   cfg.Compress,
		}
		if alsoStderr {
			w = io.MultiWriter(roller, os.Stderr)
		} else {
			w = roller
		}
	} else {
		w = os.Stderr
	}
	return slog.New(slog.NewTextHandler(w, &slog.HandlerOptions{Level: level}))
}
