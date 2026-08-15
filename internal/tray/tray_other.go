//go:build !windows

package tray

import (
	"syscall"

	"many-ai-cli/internal/config"
)

// run は Windows 以外では何もしない。トレイが解こうとしている問題
// （デスクトップに起動用と停止用のアイコンが 2 個並ぶ）は Windows 固有で、
// 他 OS には対応する使いにくさが無い。
func run(_ *config.Config) error { return ErrUnsupported }

// detachSysProcAttr は Windows 以外では既定のまま（nil）でよい。
func detachSysProcAttr() *syscall.SysProcAttr { return nil }
