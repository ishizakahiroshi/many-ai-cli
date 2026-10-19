//go:build !windows

package launcher

// ConfigureConsoleUTF8 is a no-op on non-Windows platforms; terminals are
// already UTF-8 and support ANSI escape codes by default.
func ConfigureConsoleUTF8() {}

// EnsureConsoleOutputMode is a no-op on non-Windows platforms.
func EnsureConsoleOutputMode(stdHandleID uint32) {}
