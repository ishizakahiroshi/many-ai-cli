# dist/linux/any-ai-cli を WSL の ~/.local/bin/ へ転送する
# Usage:
#   .\scripts\deploy-wsl.ps1                # 既定 (Distro=Ubuntu, Dest=~/.local/bin/any-ai-cli)
#   .\scripts\deploy-wsl.ps1 -Distro Ubuntu -Dest '~/.local/bin/any-ai-cli'

[CmdletBinding()]
param(
    [string]$Distro = 'Ubuntu',
    [string]$Dest   = '~/.local/bin/any-ai-cli'
)

$ErrorActionPreference = 'Stop'

$RepoRoot = Split-Path -Parent $PSScriptRoot
$SrcWin   = Join-Path $RepoRoot 'dist\linux\any-ai-cli'

if (-not (Test-Path -LiteralPath $SrcWin)) {
    Write-Error "Source not found: $SrcWin  (先に 'make build-linux' を実行してください)"
    exit 1
}

# C:\foo\bar -> /mnt/c/foo/bar
$drive  = $SrcWin.Substring(0,1).ToLower()
$rest   = ($SrcWin.Substring(2)) -replace '\\','/'
$SrcWsl = "/mnt/$drive$rest"

$srcInfo = Get-Item -LiteralPath $SrcWin
Write-Host "Source : $SrcWin"
Write-Host "       : size=$($srcInfo.Length) bytes, mtime=$($srcInfo.LastWriteTime)"
Write-Host "Target : ${Distro}:${Dest}"
Write-Host ''

# wsl.exe は `bash -c "...; $VAR"` のような引数を bash に渡すときに `$VAR` `$(...)` を壊すので、
# bash 側でシェル変数・コマンド置換は使わず、値を直接埋め込んで && でチェインする。
# 前提: 転送先パスに空白を含まないこと (any-ai-cli のデフォルトパスは満たす)。
$DestDir = $Dest -replace '/[^/]+$', ''
if ([string]::IsNullOrEmpty($DestDir)) { $DestDir = '.' }

$bashCmd = "mkdir -p $DestDir && cp $SrcWsl $Dest && chmod +x $Dest && echo '--- deployed ---' && ls -la $Dest && echo '--- version ---' && ($Dest --version 2>/dev/null || $Dest version 2>/dev/null || echo '(version subcommand not found)')"

wsl -d $Distro -- bash -c $bashCmd
if ($LASTEXITCODE -ne 0) {
    Write-Error "Deploy failed (wsl exit $LASTEXITCODE)"
    exit $LASTEXITCODE
}

Write-Host ''
Write-Host "Done."
