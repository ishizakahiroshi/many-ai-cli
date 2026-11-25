# Build away from the executable paths used by running Hubs to spawn wrappers.
function Invoke-RustLocalBuild {
    param([string]$Worktree, [string]$Toolchain, [switch]$Release)

    $kind = if ($Release) { 'release' } else { 'debug' }
    $target = Join-Path $Worktree 'rust/target'
    $live = Join-Path $target $kind
    $cache = Join-Path $target 'local-build'
    [void][IO.Directory]::CreateDirectory($live)
    # Serialize the build and copy together. Cargo's own lock ends before publication.
    $lock = [IO.File]::Open((Join-Path $live '.local-publish.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
    $attempt = [guid]::NewGuid().ToString('N')
    $files = @()
    try {
        $argsForCargo = @("+$Toolchain", 'build', '--locked', '--manifest-path',
            (Join-Path $Worktree 'rust/Cargo.toml'), '--target-dir', $cache)
        if ($Release) { $argsForCargo += '--release' }
        & cargo @argsForCargo
        if ($LASTEXITCODE -ne 0) { throw "cargo build が失敗しました（exit $LASTEXITCODE）" }

        foreach ($name in 'many-ai-cli', 'many-ai-cli-launcher') {
            $source = Join-Path $cache "$kind/$name.exe"
            $destination = Join-Path $live "$name.exe"
            if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw "候補に $name.exe がありません: $source" }
            $exists = [IO.File]::Exists($destination)
            if ($exists -and (Get-FileHash -LiteralPath $source).Hash -eq (Get-FileHash -LiteralPath $destination).Hash) { continue }
            $file = [pscustomobject]@{
                Destination = $destination
                Staged = Join-Path $live "$name.$attempt.new"
                Backup = Join-Path $live "$name.old.$attempt.exe"
                Existed = $exists
                Installed = $false
            }
            $files += $file
            [IO.File]::Copy($source, $file.Staged)
        }
        foreach ($file in $files) {
            if ($file.Existed) {
                [IO.File]::Replace($file.Staged, $file.Destination, $file.Backup)
            } else {
                [IO.File]::Move($file.Staged, $file.Destination)
            }
            $file.Installed = $true
        }
        foreach ($name in 'many-ai-cli', 'many-ai-cli-launcher') {
            Get-ChildItem -LiteralPath $live -Filter "$name.old.*.exe" -File |
                Remove-Item -Force -ErrorAction SilentlyContinue
        }
    } catch {
        $failure = $_
        foreach ($file in $files) {
            try {
                if ([IO.File]::Exists($file.Backup)) {
                    if ([IO.File]::Exists($file.Destination)) {
                        $rejected = [IO.Path]::ChangeExtension($file.Backup, 'rejected.exe')
                        [IO.File]::Replace($file.Backup, $file.Destination, $rejected)
                        Remove-Item -LiteralPath $rejected -Force -ErrorAction SilentlyContinue
                    } else {
                        [IO.File]::Move($file.Backup, $file.Destination)
                    }
                } elseif ($file.Installed -and !$file.Existed) {
                    [IO.File]::Delete($file.Destination)
                }
            } catch {
                Write-Warning "旧版の復元に失敗しました。退避先 $($file.Backup): $_"
            }
        }
        throw $failure
    } finally {
        foreach ($file in $files) { Remove-Item -LiteralPath $file.Staged -Force -ErrorAction SilentlyContinue }
        $lock.Dispose()
    }
}
