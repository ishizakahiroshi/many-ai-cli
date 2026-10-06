#requires -Version 5.1
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$ReceiptPath)

$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$fetch = Join-Path $repo 'internal/whisperruntime/fetch_windows_runtime.ps1'
$payload = Join-Path $repo 'internal/whisperruntime/files/windows-amd64'
# Inherit the existing Go VS-Redist acquisition and licence decision. This
# wrapper deliberately exposes no AllowSystem32 option or alternate source.
& $fetch
$names = @('vcomp140.dll', 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
$files = @()
foreach ($name in $names) {
    $path = Join-Path $payload $name
    $item = Get-Item -LiteralPath $path
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Runtime input must be a regular file: $name"
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $path
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft Corporation') {
        throw "Runtime signature verification failed: $name"
    }
    $bytes = [IO.File]::ReadAllBytes($path)
    if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) {
        throw "Invalid PE header: $name"
    }
    $offset = [BitConverter]::ToUInt32($bytes, 0x3c)
    if ([uint64]$offset + 6 -gt [uint64]$bytes.Length -or
        [BitConverter]::ToUInt32($bytes, $offset) -ne 0x00004550 -or
        [BitConverter]::ToUInt16($bytes, $offset + 4) -ne 0x8664) {
        throw "Runtime input must be x64 PE: $name"
    }
    $version = $item.VersionInfo.FileVersion
    if ([string]::IsNullOrWhiteSpace($version)) { throw "Missing FileVersion: $name" }
    $files += [ordered]@{
        name = $name
        file_version = $version
        bytes = $item.Length
        sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        signature_status = 'Valid'
        microsoft_signer = $true
        pe_machine = '0x8664'
    }
}
$source = (& git -C $repo rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $source -notmatch '^[0-9a-f]{40}$') { throw 'Invalid source revision' }
$receipt = [ordered]@{
    schema_version = 1
    source_sha = $source
    target = 'x86_64-pc-windows-msvc'
    acquisition = 'visual-studio-redist'
    selection = 'Latest installed VC/Redist/MSVC x64 CRT/OpenMP; existing Go script; no System32'
    source_script = 'internal/whisperruntime/fetch_windows_runtime.ps1'
    source_script_sha256 = (Get-FileHash -LiteralPath $fetch -Algorithm SHA256).Hash.ToLowerInvariant()
    version_policy = 'Observed per build; not a fixed version or hash'
    licence_basis = 'Existing Go Visual Studio VC/redist redistribution decision; Microsoft components, not MIT-licensed project code'
    licence_url = 'https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution#visual-c-runtime-files'
    files = $files
}
[IO.File]::WriteAllText([IO.Path]::GetFullPath($ReceiptPath), ($receipt | ConvertTo-Json -Depth 8) + "`n", [Text.UTF8Encoding]::new($false))
Write-Host 'Prepared four VS-only Windows runtime inputs with FileVersion, signature, architecture and SHA256 receipt.'
