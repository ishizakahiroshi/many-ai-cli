//go:build !windows

package clitrust

import "path/filepath"

// canonicalPath is realpath(3), which is what Rust's std::fs::canonicalize
// calls outside Windows (dunce::simplified is a no-op there): absolute, with
// every symbolic link resolved.
func canonicalPath(path string) (string, error) {
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	return filepath.EvalSymlinks(abs)
}
