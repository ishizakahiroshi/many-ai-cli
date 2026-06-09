@echo off
setlocal

set "AAC_DIR=%~dp0"

powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ErrorActionPreference = 'Stop';" ^
  "$targets = Get-ChildItem -LiteralPath $env:AAC_DIR -Filter 'any-ai-cli*.exe' -File;" ^
  "if (-not $targets) { Write-Host 'No any-ai-cli executables found in:' $env:AAC_DIR; exit 1 };" ^
  "$targets | Unblock-File;" ^
  "Write-Host 'Unblocked any-ai-cli executables in:' $env:AAC_DIR"

if errorlevel 1 (
  echo.
  echo Failed to unblock any-ai-cli executables in:
  echo "%AAC_DIR%"
  echo.
  pause
  exit /b 1
)

echo.
echo Done. You can now start any-ai-cli.exe or any-ai-cli-launcher.exe manually.
echo This script does not launch any-ai-cli.
echo.
pause
