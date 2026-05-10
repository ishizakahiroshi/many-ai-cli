//go:build windows

package wrapper

import "testing"

func TestKnownWindowsShellName(t *testing.T) {
	tests := []struct {
		exe  string
		want string
	}{
		{"pwsh.exe", "PowerShell 7"},
		{"powershell.exe", "Windows PowerShell"},
		{"cmd.exe", "cmd"},
		{"bash.exe", "bash"},
		{"zsh.exe", "zsh"},
		{"nu.exe", "nushell"},
		{"wsl.exe", "WSL"},
		{"PWSH.EXE", "PowerShell 7"},
	}

	for _, tt := range tests {
		got, ok := knownWindowsShellName(tt.exe)
		if !ok {
			t.Fatalf("knownWindowsShellName(%q) returned ok=false", tt.exe)
		}
		if got != tt.want {
			t.Fatalf("knownWindowsShellName(%q) = %q, want %q", tt.exe, got, tt.want)
		}
	}
}

func TestWindowsExeToShellUnknown(t *testing.T) {
	if got := windowsExeToShell("Code.exe"); got != "Code" {
		t.Fatalf("windowsExeToShell(Code.exe) = %q, want Code", got)
	}
	if got := windowsExeToShell("custom-host"); got != "custom-host" {
		t.Fatalf("windowsExeToShell(custom-host) = %q, want custom-host", got)
	}
}

func TestShouldSkipWindowsShellProcess(t *testing.T) {
	skip := []string{
		"ai-cli-hub.exe",
		"explorer.exe",
		"WindowsTerminal.exe",
		"wt.exe",
		"conhost.exe",
		"OpenConsole.exe",
	}
	for _, exe := range skip {
		if !shouldSkipWindowsShellProcess(exe) {
			t.Fatalf("shouldSkipWindowsShellProcess(%q) = false, want true", exe)
		}
	}
	if shouldSkipWindowsShellProcess("powershell.exe") {
		t.Fatal("shouldSkipWindowsShellProcess(powershell.exe) = true, want false")
	}
}

func TestDetectShell_EnvOverride(t *testing.T) {
	t.Setenv(parentShellEnv, "PowerShell 7 7.6.1.500")
	if got := DetectShell(); got != "PowerShell 7 7.6.1.500" {
		t.Fatalf("DetectShell() = %q, want env override value", got)
	}
}

func TestShellVersionSuffix_PowerShell51Hardcoded(t *testing.T) {
	if got := shellVersionSuffix("powershell.exe", 0); got != "5.1" {
		t.Fatalf("shellVersionSuffix(powershell.exe) = %q, want 5.1", got)
	}
}
