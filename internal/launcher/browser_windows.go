//go:build windows

package launcher

import "os/exec"

// OpenBrowser launches the Windows default browser at url.
// Uses rundll32 url.dll,FileProtocolHandler so query strings with '&'
// (e.g. tunnel URLs: ?token=...&via=ssh&host_label=...) are not re-parsed
// by cmd.exe as command separators. Same approach as internal/hub.
func OpenBrowser(url string) error {
	return exec.Command("rundll32", "url.dll,FileProtocolHandler", url).Start()
}
