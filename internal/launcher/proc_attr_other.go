//go:build !windows

package launcher

import "syscall"

// noWindowSysProcAttr は非 Windows では何もしない（コンソールウィンドウの
// 概念が無いため）。nil を返すと exec.Cmd はデフォルトの起動属性を使う。
func noWindowSysProcAttr() *syscall.SysProcAttr {
	return nil
}
