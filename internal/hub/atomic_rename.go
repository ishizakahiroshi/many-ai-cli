package hub

// atomicRenameNoReplace は src を dst にリネームするが、dst が既存の場合は失敗する。
// os.Rename (POSIX rename(2) / Windows MoveFileEx with REPLACE_EXISTING) は事前
// 存在チェックとの間に TOCTOU があり、files_move.go / files_rename.go が黙って上書き
// する事故を起こしていた (HUB-5)。OS 別 syscall で「存在すれば失敗する」プリミティブに
// 切り替えて事前チェックと本操作の atomicity を確保する。
//
// 実装は OS 別ファイルで build tag 分岐:
//   - atomic_rename_linux.go   : unix.Renameat2(AT_FDCWD, src, AT_FDCWD, dst, RENAME_NOREPLACE)
//   - atomic_rename_windows.go : windows.MoveFileEx(src, dst, 0)（REPLACE_EXISTING なし）
//   - atomic_rename_other.go   : 上記が使えない OS 向けのフォールバック
//
// 呼び出し側は既存 error に加えて、返された error が「dst 既存で失敗」ケース
// (isRenameTargetExistsErr) かどうかを判定して 409 Conflict 応答するのに使える。
