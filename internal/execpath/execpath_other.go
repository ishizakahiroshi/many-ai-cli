//go:build !windows

// Package execpath resolves an npm-generated Windows shim down to the real
// executable it launches. See execpath_windows.go for why this exists.
package execpath

// Resolve is a no-op outside Windows: only npm's Windows shims need the
// unwrapping execpath_windows.go performs. Unix package managers already
// install (or symlink to) the real binary, so exec.LookPath's result is
// already what should run.
func Resolve(exePath string, args []string) (string, []string) {
	return exePath, args
}
