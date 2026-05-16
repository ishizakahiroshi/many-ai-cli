package hub

import (
	"runtime"

	"any-ai-cli/internal/wslutil"
)

func runtimeMode() string {
	if wslutil.IsWindowsLauncherMode() {
		return "windows-wsl"
	}
	if wslutil.IsWSL() {
		return "wsl"
	}
	if runtime.GOOS == "windows" {
		return "windows-native"
	}
	return runtime.GOOS
}

func runtimeLabel(mode string) string {
	switch mode {
	case "windows-wsl":
		return "Windows + WSL (any-ai-cli-wsl.exe)"
	case "windows-native":
		return "Windows native"
	case "wsl":
		return "WSL Linux"
	case "darwin":
		return "macOS"
	case "linux":
		return "Linux"
	default:
		return mode
	}
}
