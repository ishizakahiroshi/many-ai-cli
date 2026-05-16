//go:build windows

package wslutil

// On Windows native the WSL interop helpers are no-ops; IsWSL is always false.

func IsWSL() bool { return false }

func IsWindowsLauncherMode() bool { return false }

func ToWindowsPath(p string) string { return p }

func ToUnixPath(p string) string { return p }

func WindowsHomeAsUnix() string { return "" }
