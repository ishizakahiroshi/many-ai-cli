//go:build windows

package hub

import "syscall"

// processQueryLimitedInformation は PROCESS_QUERY_LIMITED_INFORMATION。
// syscall パッケージが公開していないため定数で持つ。PROCESS_QUERY_INFORMATION と違い
// 別ユーザー所有のプロセスにも開けるので、Hub が別権限で起動していても判定できる。
const processQueryLimitedInformation = 0x1000

// stillActive は GetExitCodeProcess が「まだ動いている」ときに返す STILL_ACTIVE。
const stillActive = 259

// pidAlive reports whether pid refers to a running process.
//
// Windows では「ハンドルが開けること」は生存を意味しない。終了したプロセスの
// カーネルオブジェクトは、誰かがハンドルを保持している間（親が Wait した直後を含む）
// 残り続けるため OpenProcess は成功する。os.FindProcess の成否だけを見ていた旧実装は
// これを生存と誤判定していた:
//   - stop 経路で終了済み Hub をタイムアウトまで待ち、force kill へ倒れる
//   - その頃には PID が再利用され、無関係なプロセスを kill しうる
//
// GetExitCodeProcess で STILL_ACTIVE かどうかを見るのが正しい判定。
// 終了コードがちょうど 259 のプロセスだけは生存と誤判定するが（STILL_ACTIVE の
// 既知の曖昧さ）、旧実装が常に踏んでいた誤判定をこの 1 ケースまで狭められる。
//
// これは第一段のガードにすぎない — 呼び出し側は記録された Hub ポートも確認すること
// （killStalePid / internal/launcher/pid_windows.go と同じ理由で PID は再利用される）。
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
