# Source-selection regression checks. No application build, Hub operation, or Git mutation.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'rust-local-source.ps1')
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('mai-source-test-' + [guid]::NewGuid().ToString('N'))
$script:listing = @()
$script:gitCalls = 0
function git {
    $script:gitCalls++
    $global:LASTEXITCODE = 0
    $script:listing
}
function Assert-Throws([scriptblock]$Action) {
    $threw = $false
    try { & $Action | Out-Null } catch { $threw = $true }
    if (-not $threw) { throw 'Expected source selection to stop.' }
}
try {
    $canonical = Join-Path $testRoot 'repair source'
    $old = Join-Path $testRoot 'old copy'
    foreach ($dir in @($canonical, $old)) {
        New-Item -ItemType Directory -Path (Join-Path $dir 'rust') -Force | Out-Null
        Set-Content -LiteralPath (Join-Path $dir 'rust/Cargo.toml') -Value '# synthetic fixture'
    }
    $script:listing = @("worktree $old", 'HEAD fixture', 'detached', '', "worktree $canonical", 'HEAD fixture', 'branch refs/heads/dots/rust-recovery-resume-3', '')
    if ((Resolve-RustLocalSource -Repo $testRoot) -ne $canonical) { throw 'Daily build selected old copy.' }
    $script:listing = @("worktree $old", 'HEAD fixture', 'detached', '')
    Assert-Throws { Resolve-RustLocalSource -Repo $testRoot }
    if ((Resolve-RustLocalSource -Repo $testRoot -Source $old) -ne $old) { throw 'Explicit source was ignored.' }
    Assert-Throws { Resolve-RustLocalSource -Repo $testRoot -Source $testRoot }
    $calls = $script:gitCalls
    if ($null -ne (Resolve-RustLocalSource -Repo $testRoot -RemoteCandidate)) { throw 'Explicit remote selection used daily source.' }
    if ($calls -ne $script:gitCalls) { throw 'Explicit source/candidate made an unnecessary Git call.' }
    $script:listing = @("worktree $testRoot", 'branch refs/heads/dots/rust-recovery-resume-3', '')
    Assert-Throws { Resolve-RustLocalSource -Repo $testRoot }
    'Source selection: 6 regression cases passed.'
} finally {
    if ($testRoot -and (Split-Path -Leaf $testRoot).StartsWith('mai-source-test-') -and
        [IO.Path]::GetFullPath($testRoot).StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
