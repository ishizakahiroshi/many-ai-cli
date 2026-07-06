//go:build windows
// +build windows

package hub

import (
	"errors"
	"os"

	"golang.org/x/sys/windows"
)

// atomicRenameNoReplace は Windows で MoveFileExW を REPLACE_EXISTING フラグなしで
// 呼び出す。既存だと ERROR_ALREADY_EXISTS を返す。UTF-16 変換は windows.UTF16PtrFromString
// で行う（`syscall.StringToUTF16Ptr` は deprecated）。
func atomicRenameNoReplace(src, dst string) error {
	srcPtr, err := windows.UTF16PtrFromString(src)
	if err != nil {
		return err
	}
	dstPtr, err := windows.UTF16PtrFromString(dst)
	if err != nil {
		return err
	}
	// flag=0 は「dst 既存なら失敗」を意味する。REPLACE_EXISTING や COPY_ALLOWED は付けない。
	if err := windows.MoveFileEx(srcPtr, dstPtr, 0); err != nil {
		return err
	}
	return nil
}

// isRenameTargetExistsErr は atomicRenameNoReplace が「dst 既存」を理由に失敗
// したことを返す。呼び出し側の 409 Conflict 応答用。
func isRenameTargetExistsErr(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, windows.ERROR_ALREADY_EXISTS) || errors.Is(err, windows.ERROR_FILE_EXISTS) {
		return true
	}
	return errors.Is(err, os.ErrExist)
}
