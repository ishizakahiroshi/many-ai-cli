#requires -Version 5.1
<#
.SYNOPSIS
  whisper-server.exe が依存する x64 の VC++ ランタイム DLL 4 点を取得・検証し、
  go:embed 対象ディレクトリ (files/windows-amd64/) へ配置する。

.DESCRIPTION
  plan C2 / D2 / D7。0xC0000135 (STATUS_DLL_NOT_FOUND) を回避するため、
  whisper-server.exe と同梱する 4 点を「正規入手元」から取得する。

  入手元は次の優先順で探索する（サードパーティ DLL 配布サイトは使用しない）:
    1) Visual Studio の Redist フォルダ (Microsoft.VCxxx.CRT + .OpenMP) を vswhere 経由で探索
    2) System32 の Microsoft 署名済みコピー（フォールバック。servicing 版）

  取得後、各 DLL について以下を検証する:
    - Authenticode 署名が Valid かつ Subject に "Microsoft Corporation"
    - PE machine が 0x8664 (x64)

  この処理は CI / リリース前に一度実行する。実 DLL は .gitignore 済み
  （リポジトリには含めない）。go:embed は working tree を読むため、
  配置後にビルドすれば本体バイナリへ同梱される。

.PARAMETER Dest
  配置先。既定は本スクリプトと同じ階層の files/windows-amd64/。
#>
[CmdletBinding()]
param(
  [string]$Dest = (Join-Path $PSScriptRoot 'files/windows-amd64'),
  # System32 のコピーは OS servicing 版で、VS の \VC\redist フォルダ単位許諾
  # （plan D7）の対象外＝再頒布ライセンスでカバーされない。ローカル検証専用の
  # 明示オプトインとし、リリース/CI では使わない（既定は VS Redist のみ）。
  [switch]$AllowSystem32
)

$ErrorActionPreference = 'Stop'
$needed = @('vcomp140.dll', 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')

function Write-Info($msg) { Write-Host "[fetch-vcruntime] $msg" }

function Get-PEMachineHex([string]$path) {
  $bytes = [IO.File]::ReadAllBytes($path)
  $peOff = [BitConverter]::ToInt32($bytes, 0x3C)   # IMAGE_DOS_HEADER.e_lfanew
  # 'PE\0\0' の直後 2 バイトが Machine
  $machine = [BitConverter]::ToUInt16($bytes, $peOff + 4)
  return ('0x{0:X4}' -f $machine)
}

function Test-MicrosoftDll([string]$path) {
  $sig = Get-AuthenticodeSignature -FilePath $path
  if ($sig.Status -ne 'Valid') {
    throw "$([IO.Path]::GetFileName($path)): Authenticode status = $($sig.Status) (want Valid)"
  }
  $subject = $sig.SignerCertificate.Subject
  if ($subject -notmatch 'Microsoft Corporation') {
    throw "$([IO.Path]::GetFileName($path)): signer subject does not contain Microsoft Corporation: $subject"
  }
  $machine = Get-PEMachineHex $path
  if ($machine -ne '0x8664') {
    throw "$([IO.Path]::GetFileName($path)): PE machine = $machine (want 0x8664 x64)"
  }
  $ver = (Get-Item $path).VersionInfo.FileVersion
  Write-Info ("verified {0}  ver={1}  machine={2}  signer=OK" -f ([IO.Path]::GetFileName($path)), $ver, $machine)
}

function Find-VSRedistDir {
  $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  if (-not (Test-Path $vswhere)) { return $null }
  $vsPath = & $vswhere -latest -products * -property installationPath 2>$null
  if (-not $vsPath) { return $null }
  $redistRoot = Join-Path $vsPath 'VC\Redist\MSVC'
  if (-not (Test-Path $redistRoot)) { return $null }
  # 最新の v14x toolset ディレクトリを選ぶ
  $verDir = Get-ChildItem $redistRoot -Directory |
    Where-Object { $_.Name -match '^\d+\.' } |
    Sort-Object Name -Descending | Select-Object -First 1
  if (-not $verDir) { return $null }
  return $verDir.FullName
}

function Resolve-FromVS([string]$redistVerDir) {
  $x64 = Join-Path $redistVerDir 'x64'
  if (-not (Test-Path $x64)) { return $null }
  $crt = Get-ChildItem $x64 -Directory -Filter 'Microsoft.VC*.CRT'   | Select-Object -First 1
  $omp = Get-ChildItem $x64 -Directory -Filter 'Microsoft.VC*.OpenMP' | Select-Object -First 1
  $map = @{}
  foreach ($dll in $needed) {
    foreach ($dir in @($crt, $omp)) {
      if ($dir) {
        $candidate = Join-Path $dir.FullName $dll
        if (Test-Path $candidate) { $map[$dll] = $candidate; break }
      }
    }
  }
  return $map
}

function Resolve-FromSystem32 {
  $sys = Join-Path $env:WINDIR 'System32'
  $map = @{}
  foreach ($dll in $needed) {
    $candidate = Join-Path $sys $dll
    if (Test-Path $candidate) { $map[$dll] = $candidate }
  }
  return $map
}

# --- 入手元解決 ---
$source = $null
$resolved = @{}

$vsDir = Find-VSRedistDir
if ($vsDir) {
  Write-Info "trying Visual Studio Redist: $vsDir"
  $resolved = Resolve-FromVS $vsDir
  if ($resolved.Count -eq $needed.Count) { $source = "VS Redist ($vsDir)" }
}

if (-not $source -and $AllowSystem32) {
  Write-Info 'falling back to System32 (DEV ONLY)'
  Write-Warning 'System32 copies are OS-serviced builds, NOT the VS \VC\redist grant (plan D7). Do NOT ship these; use only for local testing.'
  $resolved = Resolve-FromSystem32
  if ($resolved.Count -eq $needed.Count) { $source = 'System32 (dev-only, do not redistribute)' }
}

if (-not $source) {
  if (-not $AllowSystem32) {
    throw "Visual Studio \VC\redist (x64 CRT + OpenMP) not found. Install Visual Studio with the C++ workload, or the standalone VC++ x64 redistributable into a VS layout, then rerun. For LOCAL TESTING ONLY you may rerun with -AllowSystem32 (those copies are OS-licensed and must NOT be redistributed; plan D7)."
  }
  $missing = $needed | Where-Object { -not $resolved.ContainsKey($_) }
  throw "could not locate all required DLLs. Missing: $($missing -join ', '). Install the VC++ x64 redistributable or Visual Studio with the C++ workload, then rerun."
}

Write-Info "source: $source"

# --- 配置 + 検証 ---
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
foreach ($dll in $needed) {
  $src = $resolved[$dll]
  $dst = Join-Path $Dest $dll
  Copy-Item -Path $src -Destination $dst -Force
  Test-MicrosoftDll $dst
}

Write-Info "done. placed $($needed.Count) DLLs in $Dest"
Write-Info 'next: build the binary (go:embed will bundle these). The .dll files are gitignored.'
