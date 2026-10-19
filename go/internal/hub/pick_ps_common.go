//go:build windows

package hub

// wrapWithForegroundOwner は Windows ネイティブダイアログ (FolderBrowserDialog /
// OpenFileDialog 等) を表示する PowerShell スクリプトを、
// 「隠しオーナーウィンドウ + Win32Focus (SetForegroundWindow/ShowWindow) による
// フォアグラウンド強制」でラップした完全スクリプトを返す共通ヘルパ。
//
// C2 (plan_audit_score_s_promotion_2026-07-05.md): 従来は pick_directory_windows.go
// と pick_file_windows.go の 2 ファイルに全く同じ Win32Focus 定型が展開されていた
// （片方だけ修正して他方が取り残される保守事故のリスクあり）。両呼び出しを
// このヘルパへ寄せる。
//
// 引数:
//   - dialogSetup: `$xx = New-Object ...FooDialog` の初期化スクリプト
//     （filter 設定などの追加行を含めてもよい・末尾改行なしで渡す）
//   - dialogVar: dialog 変数名（`$folder` / `$picker` 等・`$` を含む）
//   - resultProperty: OK 時に取り出すプロパティ名（`SelectedPath` / `FileName` 等）
//
// 戻り値の完全スクリプトは exec.Command("powershell.exe", "-NoProfile", "-STA",
// "-NonInteractive", "-Command", script) に渡してそのまま実行できる。
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
