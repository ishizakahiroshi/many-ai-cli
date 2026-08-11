//go:build windows

package launcher

import "syscall"

// processQueryLimitedInformation は PROCESS_QUERY_LIMITED_INFORMATION。
// syscall パッケージが公開していないため定数で持つ。
const processQueryLimitedInformation = 0x1000

// stillActive は GetExitCodeProcess が「まだ動いている」ときに返す STILL_ACTIVE。
const stillActive = 259

// pidAlive reports whether pid refers to a running process.
//
// Windows では終了済みプロセスにもハンドルを開けるため、os.FindProcess の成否では
// 生存を判定できない（internal/hub/pid_windows.go に同じ修正と経緯あり）。
// GetExitCodeProcess が STILL_ACTIVE を返すかどうかで判定する。
//
// これは第一段のガードにすぎない — 呼び出し側は記録された Hub URL も確認すること
// （死んだ launcher の PID は無関係なプロセスに再利用される）。
func pidAlive(pid int) bool {
	if pid <= 0 {
		return false
	}
	h, err := syscall.OpenProcess(processQueryLimitedInformation, false, uint32(pid))
	if err != nil {
		return false
	}
	defer func() { _ = syscall.CloseHandle(h) }()
	var code uint32
	if err := syscall.GetExitCodeProcess(h, &code); err != nil {
		return false
	}
	return code == stillActive
}
