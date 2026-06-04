//go:build windows

package launcher

import (
	"syscall"
	"unsafe"
)

var (
	kernel32            = syscall.NewLazyDLL("kernel32.dll")
	procSetConsoleCP    = kernel32.NewProc("SetConsoleCP")
	procSetConsoleOutCP = kernel32.NewProc("SetConsoleOutputCP")
	procGetStdHandle    = kernel32.NewProc("GetStdHandle")
	procGetConsoleMode  = kernel32.NewProc("GetConsoleMode")
	procSetConsoleMode  = kernel32.NewProc("SetConsoleMode")
)

const (
	// Win32 STD_OUTPUT_HANDLE / STD_ERROR_HANDLE — DWORD-cast of -11 / -12.
	StdOutputHandle = uint32(0xFFFFFFF5)
	StdErrorHandle  = uint32(0xFFFFFFF4)

	// SetConsoleMode flags. We make sure PROCESSED_OUTPUT (so "\n" expands to
	// CRLF) and VIRTUAL_TERMINAL_PROCESSING (so ANSI color escapes work) are
	// both on; their states are independent and both matter for the banner.
	enableProcessedOutput           = 0x0001
	enableWrapAtEOLOutput           = 0x0002
	enableVirtualTerminalProcessing = 0x0004
)

// ConfigureConsoleUTF8 switches the Windows console code page to UTF-8 and
// enables PROCESSED_OUTPUT + VT_PROCESSING on stdout and stderr so that ANSI
// escape codes and "\n" line endings render correctly.
func ConfigureConsoleUTF8() {
	const cpUTF8 = 65001
	_, _, _ = procSetConsoleCP.Call(cpUTF8)
	_, _, _ = procSetConsoleOutCP.Call(cpUTF8)
	EnsureConsoleOutputMode(StdOutputHandle)
	EnsureConsoleOutputMode(StdErrorHandle)
}

// EnsureConsoleOutputMode forces PROCESSED_OUTPUT and VT_PROCESSING on for the
// given std handle. Without PROCESSED_OUTPUT the Hub banner's "\n" line breaks
// only emit LF; cursor moves down without returning to column 0, and the next
// line is rendered starting at the previous line's right edge (the ASCII art
// "slides diagonally" and "Claude Code / Codex wrapper" stacks onto the logo's
// last row). Without VT_PROCESSING the ANSI color escapes in the banner are
// printed as raw text, blowing up apparent line width. Both modes are
// independent — we OR them in rather than overwriting, so any existing flags
// (line wrap, etc.) survive.
func EnsureConsoleOutputMode(stdHandleID uint32) {
	h, _, _ := procGetStdHandle.Call(uintptr(int32(stdHandleID)))
	if h == 0 || h == uintptr(^uintptr(0)) {
		return
	}
	var mode uint32
	r, _, _ := procGetConsoleMode.Call(h, uintptr(unsafe.Pointer(&mode)))
	if r == 0 {
		return
	}
	newMode := mode | enableProcessedOutput | enableVirtualTerminalProcessing | enableWrapAtEOLOutput
	if newMode == mode {
		return
	}
	_, _, _ = procSetConsoleMode.Call(h, uintptr(newMode))
}
