package hub

import (
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestBrowserCommandUsesPlatformLauncher(t *testing.T) {
	const url = "http://127.0.0.1:47777/?token=test"

	cmd := browserCommand(url)
	name := strings.ToLower(filepath.Base(cmd.Path))

	switch runtime.GOOS {
	case "windows":
		if name != "rundll32.exe" && name != "rundll32" {
			t.Fatalf("browserCommand path = %q, want rundll32", cmd.Path)
		}
		if len(cmd.Args) != 3 || cmd.Args[1] != "url.dll,FileProtocolHandler" || cmd.Args[2] != url {
			t.Fatalf("browserCommand args = %#v", cmd.Args)
		}
	case "darwin":
		if name != "open" {
			t.Fatalf("browserCommand path = %q, want open", cmd.Path)
		}
		if len(cmd.Args) != 2 || cmd.Args[1] != url {
			t.Fatalf("browserCommand args = %#v", cmd.Args)
		}
	default:
		if name != "xdg-open" {
			t.Fatalf("browserCommand path = %q, want xdg-open", cmd.Path)
		}
		if len(cmd.Args) != 2 || cmd.Args[1] != url {
			t.Fatalf("browserCommand args = %#v", cmd.Args)
		}
	}
}
