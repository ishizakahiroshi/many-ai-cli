//go:build windows

package subscription

import (
	"encoding/binary"
	"fmt"
	"os"
	"path/filepath"
	"unicode/utf16"

	"golang.org/x/sys/windows"

	"many-ai-cli/internal/config"
)

// Windows directory links, and why this file exists.
//
// os.Symlink creates a directory symlink, which needs
// SeCreateSymbolicLinkPrivilege — an administrator, or Developer Mode turned
// on. Most people running many-ai-cli have neither, so relying on os.Symlink
// alone would silently degrade seeding to "skills are missing" on the platform
// this tool is used on most.
//
// A junction (an IO_REPARSE_TAG_MOUNT_POINT reparse point) does the same job
// for a local directory and needs no privilege at all: it is what `mklink /J`
// makes. The Go standard library has no API for it, so the reparse buffer is
// assembled here. Local directories are the only thing seeding ever links, so
// the junction's inability to cross to a network share does not matter.
const (
	fsctlSetReparsePoint   = 0x000900A4
	ioReparseTagMountPoint = 0xA0000003
	// maxReparseDataBufferLen is MAXIMUM_REPARSE_DATA_BUFFER_SIZE from the
	// Windows headers. DeviceIoControl rejects anything larger.
	maxReparseDataBufferLen = 16 * 1024
)

// linkDir points dst at src, preferring a symlink and falling back to a
// junction when the process lacks the privilege to create one.
func linkDir(src, dst string) error {
	if err := os.Symlink(src, dst); err == nil {
		return nil
	}
	return createJunction(src, dst)
}

// createJunction makes dst an empty directory and turns it into a junction
// pointing at target. A half-made directory is removed on failure so a later
// pass sees "nothing here" and retries, rather than "already seeded".
func createJunction(target, dst string) error {
	absTarget, err := filepath.Abs(target)
	if err != nil {
		return fmt.Errorf("resolve junction target: %w", err)
	}
	if err := os.Mkdir(dst, config.DirMode); err != nil {
		return err
	}
	if err := setMountPoint(dst, absTarget); err != nil {
		_ = os.Remove(dst)
		return err
	}
	return nil
}

func setMountPoint(dst, target string) error {
	buf, err := mountPointReparseBuffer(target)
	if err != nil {
		return err
	}
	dstPtr, err := windows.UTF16PtrFromString(dst)
	if err != nil {
		return err
	}
	// FILE_FLAG_BACKUP_SEMANTICS is required to open a directory at all;
	// FILE_FLAG_OPEN_REPARSE_POINT opens the directory itself rather than
	// following any reparse point already on it.
	handle, err := windows.CreateFile(dstPtr, windows.GENERIC_WRITE, 0, nil,
		windows.OPEN_EXISTING,
		windows.FILE_FLAG_OPEN_REPARSE_POINT|windows.FILE_FLAG_BACKUP_SEMANTICS, 0)
	if err != nil {
		return fmt.Errorf("open junction target: %w", err)
	}
	defer func() { _ = windows.CloseHandle(handle) }()

	var returned uint32
	if err := windows.DeviceIoControl(handle, fsctlSetReparsePoint,
		&buf[0], uint32(len(buf)), nil, 0, &returned, nil); err != nil {
		return fmt.Errorf("set reparse point: %w", err)
	}
	return nil
}

// mountPointReparseBuffer builds a REPARSE_DATA_BUFFER for a mount point.
//
// Layout (all little-endian):
//
//	 0  uint32  ReparseTag
//	 4  uint16  ReparseDataLength   (everything after byte 8)
//	 6  uint16  Reserved
//	 8  uint16  SubstituteNameOffset
//	10  uint16  SubstituteNameLength
//	12  uint16  PrintNameOffset
//	14  uint16  PrintNameLength
//	16  WCHAR   PathBuffer[]
//
// The substitute name is the NT-namespace path (\??\C:\dir) that the filesystem
// resolves; the print name is the plain path shown to users. Both are stored
// NUL-terminated, and both lengths exclude their terminator.
func mountPointReparseBuffer(target string) ([]byte, error) {
	substitute := utf16.Encode([]rune(`\??\` + target))
	display := utf16.Encode([]rune(target))
	pathBytes := (len(substitute) + 1 + len(display) + 1) * 2
	total := 16 + pathBytes
	if total > maxReparseDataBufferLen {
		return nil, fmt.Errorf("junction target %q is too long for a reparse point", target)
	}

	buf := make([]byte, total)
	binary.LittleEndian.PutUint32(buf[0:], ioReparseTagMountPoint)
	binary.LittleEndian.PutUint16(buf[4:], uint16(pathBytes+8))
	binary.LittleEndian.PutUint16(buf[6:], 0)
	binary.LittleEndian.PutUint16(buf[8:], 0)
	binary.LittleEndian.PutUint16(buf[10:], uint16(len(substitute)*2))
	binary.LittleEndian.PutUint16(buf[12:], uint16((len(substitute)+1)*2))
	binary.LittleEndian.PutUint16(buf[14:], uint16(len(display)*2))

	offset := 16
	for _, unit := range substitute {
		binary.LittleEndian.PutUint16(buf[offset:], unit)
		offset += 2
	}
	offset += 2 // NUL terminator for the substitute name
	for _, unit := range display {
		binary.LittleEndian.PutUint16(buf[offset:], unit)
		offset += 2
	}
	return buf, nil
}
