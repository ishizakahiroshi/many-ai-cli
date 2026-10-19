//go:build windows

package launcher

import "syscall"

// createNoWindow は CREATE_NO_WINDOW（0x08000000）。子コンソールアプリ
// （ssh.exe / wsl.exe）に新規コンソールウィンドウを割り当てない。
const createNoWindow = 0x08000000

// noWindowSysProcAttr は ssh.exe / wsl.exe を「無窓」で起動するための
// SysProcAttr を返す。Hub がこれらの子プロセスを抱える場合、Hub 自身が
// GUI 起動（コンソール無し）だと Windows は子ごとに新規コンソールを開いて
// しまう。CREATE_NO_WINDOW + HideWindow でその窓を抑止する。
// 入出力は呼び出し側で pipe 済みのため、コンソールが無くても支障はない。
// ランチャー exe（コンソールあり）経路でも子の窓が出なくなるだけで無害。
func noWindowSysProcAttr() *syscall.SysProcAttr {
	return &syscall.SysProcAttr{
		HideWindow:    true,
		CreationFlags: createNoWindow,
	}
}
