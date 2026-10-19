//go:build windows

package wrapper

import "syscall"

var (
	wrapKernel32           = syscall.NewLazyDLL("kernel32.dll")
	procSetConsoleCP       = wrapKernel32.NewProc("SetConsoleCP")
	procSetConsoleOutputCP = wrapKernel32.NewProc("SetConsoleOutputCP")
)

// applyUTF8Session は現在のプロセスのコンソールコードページを UTF-8 (65001) に設定する。
// ConPTY 作成前に呼ぶことで、PTY 内の stdin/stdout が UTF-8 で動作する。
func applyUTF8Session() {
	_, _, _ = procSetConsoleCP.Call(65001)
	_, _, _ = procSetConsoleOutputCP.Call(65001)
}
