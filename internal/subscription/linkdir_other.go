//go:build !windows

package subscription

import "os"

// linkDir points dst at src with a symbolic link. Every supported non-Windows
// platform lets an unprivileged user create one, so there is nothing to fall
// back to.
func linkDir(src, dst string) error {
	return os.Symlink(src, dst)
}
