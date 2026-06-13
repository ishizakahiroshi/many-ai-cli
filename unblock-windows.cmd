@echo off
setlocal

set "AAC_DIR=%~dp0"

powershell.exe -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ErrorActionPreference = 'Stop';" ^
  "$targets = Get-ChildItem -LiteralPath $env:AAC_DIR -Filter 'many-ai-cli*.exe' -File;" ^
  "if (-not $targets) { Write-Host 'No many-ai-cli executables found in:' $env:AAC_DIR; exit 1 };" ^
  "$targets | Unblock-File;" ^
  "Write-Host 'Unblocked many-ai-cli executables in:' $env:AAC_DIR"

if errorlevel 1 (
  echo.
  echo Failed to unblock many-ai-cli executables in:
  echo "%AAC_DIR%"
  echo.
  pause
  exit /b 1
)

echo.
echo Done. You can now start many-ai-cli.exe or many-ai-cli-launcher.exe manually.
echo This script does not launch many-ai-cli.
echo.
pause
