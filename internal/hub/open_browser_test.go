package hub

import (
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func TestBrowserCommandUsesPlatformLauncher(t *testing.T) {
	const url = "http://127.0.0.1:47777/?token=test"

	t.Setenv("WSL_INTEROP", "")
	t.Setenv("WSL_DISTRO_NAME", "")

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

func TestBrowserCommandUnderWSLUsesExplorer(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("WSL detection only applies to the linux build")
	}
	const url = "http://127.0.0.1:47777/?token=test"

	// browserCommand picks explorer.exe only in Windows-launcher (WSL) mode,
	// which wslutil.IsWindowsLauncherMode() detects via MANY_AI_CLI_WSL_LAUNCHER.
	t.Setenv("MANY_AI_CLI_WSL_LAUNCHER", "1")

	cmd := browserCommand(url)
	name := strings.ToLower(filepath.Base(cmd.Path))
	if name != "explorer.exe" {
		t.Fatalf("browserCommand under WSL path = %q, want explorer.exe", cmd.Path)
	}
	if len(cmd.Args) != 2 || cmd.Args[1] != url {
		t.Fatalf("browserCommand under WSL args = %#v", cmd.Args)
	}
}
