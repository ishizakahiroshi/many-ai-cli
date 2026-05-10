Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\.." )).Path
$target = Join-Path $repoRoot "THIRD_PARTY_NOTICES.md"
$vendorLicenses = Join-Path $repoRoot "web\src\vendor\THIRD_PARTY_LICENSES.txt"
$tmp = Join-Path $env:TEMP "third_party_notices.$PID.md"

try {
  & (Join-Path $PSScriptRoot "gen-third-party-notices.ps1") -OutputPath $tmp
  if (-not (Test-Path -LiteralPath $target)) {
    Write-Error "THIRD_PARTY_NOTICES.md is missing. Run scripts/local/gen-third-party-notices.ps1"
    exit 1
  }
  if (-not (Test-Path -LiteralPath $vendorLicenses)) {
    Write-Error "web/src/vendor/THIRD_PARTY_LICENSES.txt is missing. Restore the vendored browser-side license texts."
    exit 1
  }

  $targetHash = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash
  $tmpHash = (Get-FileHash -LiteralPath $tmp -Algorithm SHA256).Hash
  if ($targetHash -ne $tmpHash) {
    Write-Host "----- Diff between repo notices and freshly generated notices -----"
    Write-Host "repo : $target ($targetHash, $((Get-Item -LiteralPath $target).Length) bytes)"
    Write-Host "fresh: $tmp ($tmpHash, $((Get-Item -LiteralPath $tmp).Length) bytes)"
    $repoLines = Get-Content -LiteralPath $target
    $freshLines = Get-Content -LiteralPath $tmp
    $diff = Compare-Object -ReferenceObject $repoLines -DifferenceObject $freshLines | Select-Object -First 40
    if ($diff) {
      $diff | Format-Table -AutoSize | Out-String | Write-Host
    }
    Write-Host "----- End of diff -----"
    Write-Error "THIRD_PARTY_NOTICES.md is outdated. Run scripts/local/gen-third-party-notices.ps1 and commit the result."
    exit 1
  }

  Write-Host "THIRD_PARTY_NOTICES.md is up to date."
}
finally {
  if (Test-Path -LiteralPath $tmp) {
    Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
  }
}
