# Failure injection against disposable artifacts. No Rust build or Hub operation.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'rust-local-build.ps1')
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$work = Join-Path $tempRoot ('mai-build-test-' + [guid]::NewGuid().ToString('N'))
$live = Join-Path $work 'rust/target/debug'
$main = Join-Path $live 'many-ai-cli.exe'
$launcher = Join-Path $live 'many-ai-cli-launcher.exe'
$script:cargoMode = 'ok'
function cargo {
    $candidateTarget = $args[[Array]::IndexOf($args, '--target-dir') + 1]
    if ($candidateTarget -ne (Join-Path $work 'rust/target/local-build')) { throw 'unexpected Cargo target path' }
    $dir = Join-Path $candidateTarget 'debug'
    [void][IO.Directory]::CreateDirectory($dir)
    [IO.File]::WriteAllText((Join-Path $dir 'many-ai-cli.exe'), 'new main')
    if ($script:cargoMode -eq 'ok') { [IO.File]::WriteAllText((Join-Path $dir 'many-ai-cli-launcher.exe'), 'new launcher') }
    $global:LASTEXITCODE = if ($script:cargoMode -eq 'cargo failure') { 1 } else { 0 }
}
function Assert-Binaries([string]$expectedMain, [string]$expectedLauncher) {
    if ([IO.File]::ReadAllText($main) -ne $expectedMain -or [IO.File]::ReadAllText($launcher) -ne $expectedLauncher) {
        throw "Expected $expectedMain and $expectedLauncher at their launch paths."
    }
}
function Assert-BuildFailure {
    $failed = $false
    try { Invoke-RustLocalBuild -Worktree $work -Toolchain 'test' } catch { $failed = $true }
    if (!$failed) { throw "Expected failure in fixture mode $script:cargoMode." }
    Assert-Binaries 'old main' 'old launcher'
}
try {
    [void][IO.Directory]::CreateDirectory($live)
    [IO.File]::WriteAllText($main, 'old main')
    [IO.File]::WriteAllText($launcher, 'old launcher')
    $script:cargoMode = 'cargo failure'
    Assert-BuildFailure
    $script:cargoMode = 'missing launcher'
    Assert-BuildFailure
    $script:cargoMode = 'ok'
    # Block replacing only the second file, after the first file has been installed.
    $held = [IO.File]::Open($launcher, 'Open', 'Read', 'Read')
    try { Assert-BuildFailure } finally { $held.Dispose() }
    Invoke-RustLocalBuild -Worktree $work -Toolchain 'test'
    Assert-Binaries 'new main' 'new launcher'
    $installed = (Get-Item -LiteralPath $main).LastWriteTimeUtc
    Invoke-RustLocalBuild -Worktree $work -Toolchain 'test'
    Assert-Binaries 'new main' 'new launcher'
    if ((Get-Item -LiteralPath $main).LastWriteTimeUtc -ne $installed) { throw 'Unchanged build replaced the runtime.' }
    # A live Windows executable can still be invoked at the same path after an interrupted build.
    Copy-Item -LiteralPath (Join-Path $env:SystemRoot 'System32/ping.exe') -Destination $main -Force
    $process = Start-Process -FilePath $main -ArgumentList '-t', '127.0.0.1' -WindowStyle Hidden -PassThru
    try {
        $script:cargoMode = 'cargo failure'
        $failed = $false
        try { Invoke-RustLocalBuild -Worktree $work -Toolchain 'test' } catch { $failed = $true }
        if (!$failed -or $process.HasExited) { throw 'Fault injection did not preserve the running process.' }
        & $main -n 1 127.0.0.1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'Original executable no longer starts.' }
        $script:cargoMode = 'ok'
        Invoke-RustLocalBuild -Worktree $work -Toolchain 'test'
        Assert-Binaries 'new main' 'new launcher'
        if ($process.HasExited) { throw 'Successful publication interrupted the old process.' }
    } finally {
        if (!$process.HasExited) { $process.Kill(); $process.WaitForExit() }
        $process.Dispose()
    }
    'Rust local build publication checks passed.'
} finally {
    $resolved = [IO.Path]::GetFullPath($work)
    if ([IO.Path]::GetDirectoryName($resolved) -ne $tempRoot.TrimEnd('\', '/') -or
        [IO.Path]::GetFileName($resolved) -notmatch '^mai-build-test-[0-9a-f]{32}$') { throw 'Unsafe fixture cleanup path' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
