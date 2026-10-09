//go:build ignore
package main
import("encoding/json";"os")
func wrapWithForegroundOwner(dialogSetup, dialogVar, resultProperty string) string {
	return `
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = $utf8NoBom
$OutputEncoding = $utf8NoBom
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
` + dialogSetup + `
try {
  $owner.Show()
  $null = [Win32Focus]::ShowWindow($owner.Handle, 5)
  $null = [Win32Focus]::SetForegroundWindow($owner.Handle)
  $owner.Activate()
  $owner.BringToFront()
  if (` + dialogVar + `.ShowDialog($owner) -eq 'OK') { Write-Output ` + dialogVar + `.` + resultProperty + ` }
} finally {
  ` + dialogVar + `.Dispose()
  $owner.Close()
  $owner.Dispose()
}`
}

func main(){json.NewEncoder(os.Stdout).Encode(map[string]string{
"directory":wrapWithForegroundOwner(`$folder = New-Object System.Windows.Forms.FolderBrowserDialog`,`$folder`,`SelectedPath`),
"file":wrapWithForegroundOwner(`$picker = New-Object System.Windows.Forms.OpenFileDialog`,`$picker`,`FileName`),
"exe":wrapWithForegroundOwner("$picker = New-Object System.Windows.Forms.OpenFileDialog\n"+`$picker.Filter = "Executable (*.exe)|*.exe|All files (*.*)|*.*"`,`$picker`,`FileName`),
})}
