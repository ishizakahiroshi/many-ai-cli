//go:build !windows

// Package wslutil centralizes WSL detection and path conversion so both the
// hub and the config packages can share one implementation without an import
// cycle (config -> hub would be invalid).
package wslutil

import (
	"os"
	"os/exec"
	"strings"
)

// IsWSL reports whether the current Linux process is running under WSL.
// On macOS these env vars are never set, so the result is always false there.
func IsWSL() bool {
	return os.Getenv("WSL_INTEROP") != "" || os.Getenv("WSL_DISTRO_NAME") != ""
}

// IsWindowsLauncherMode reports whether the current Linux process was started
// by the any-ai-cli-wsl.exe Windows launcher. Plain `any-ai-cli serve` invoked
// directly inside WSL (without the launcher) returns false even though IsWSL
// is true — that case is treated as a pure-Linux session where the Hub UI is
// expected to be opened by a WSL-side browser, not Windows Explorer / a
// Windows-side browser.
func IsWindowsLauncherMode() bool {
	return os.Getenv("ANY_AI_CLI_WSL_LAUNCHER") == "1"
}

// ToWindowsPath converts a Linux path to a Windows path via `wslpath -w`.
// On failure, returns the input unchanged so callers can still attempt the
// operation; some Windows tools tolerate /mnt/<drive>/-style paths.
func ToWindowsPath(p string) string {
	out, err := exec.Command("wslpath", "-w", p).Output()
	if err != nil {
		return p
	}
	return strings.TrimSpace(string(out))
}

// ToUnixPath converts a Windows path to a Linux path via `wslpath -u`.
// On failure, returns the input unchanged.
func ToUnixPath(p string) string {
	out, err := exec.Command("wslpath", "-u", p).Output()
	if err != nil {
		return p
	}
	return strings.TrimSpace(string(out))
}

// WindowsHomeAsUnix returns the Windows %USERPROFILE% expressed as a WSL
// Linux path (e.g. /mnt/c/Users/<name>). Returns "" when not on WSL or when
// resolution fails; callers should fall back to the Linux $HOME in that case.
// Note: this does not gate on IsWindowsLauncherMode — callers decide whether
// the Windows-side path is appropriate for their use case.
func WindowsHomeAsUnix() string {
	if !IsWSL() {
		return ""
	}
	out, err := exec.Command("cmd.exe", "/c", "echo %USERPROFILE%").Output()
	if err != nil {
		return ""
	}
	winHome := strings.TrimRight(strings.TrimSpace(string(out)), "\r\n")
	if winHome == "" || winHome == "%USERPROFILE%" {
		return ""
	}
	linHome := ToUnixPath(winHome)
	if linHome == "" || linHome == winHome {
		return ""
	}
	return linHome
}
