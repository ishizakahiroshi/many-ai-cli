//go:build windows

package launcher

import "os/exec"

// OpenBrowser launches the Windows default browser at url.
// Uses `cmd /c start "" <url>` which is the documented stable way to invoke
// the user's registered URL handler. The empty "" is start's window-title
// argument and prevents start from treating the URL itself as a title.
func OpenBrowser(url string) error {
	return exec.Command("cmd", "/c", "start", "", url).Start()
}
