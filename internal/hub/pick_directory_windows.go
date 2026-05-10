//go:build windows

package hub

import (
	"os/exec"
	"strings"
)

func pickDirectoryNative() (string, error) {
	cmd := exec.Command("powershell.exe", "-NoProfile", "-STA", "-NonInteractive", "-Command", `
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win32Focus {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
}
"@
[System.Windows.Forms.Application]::EnableVisualStyles()
$owner = New-Object System.Windows.Forms.Form
$owner.StartPosition = 'CenterScreen'
$owner.Width = 1
$owner.Height = 1
$owner.ShowInTaskbar = $false
$owner.TopMost = $true
$owner.Opacity = 0.01
$folder = New-Object System.Windows.Forms.FolderBrowserDialog
try {
  $owner.Show()
  $null = [Win32Focus]::ShowWindow($owner.Handle, 5)
  $null = [Win32Focus]::SetForegroundWindow($owner.Handle)
  $owner.Activate()
  $owner.BringToFront()
  if ($folder.ShowDialog($owner) -eq 'OK') { Write-Output $folder.SelectedPath }
} finally {
  $folder.Dispose()
  $owner.Close()
  $owner.Dispose()
}`)
	out, err := cmd.Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}
