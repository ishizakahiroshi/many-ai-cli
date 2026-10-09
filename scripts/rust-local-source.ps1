# Resolve the daily repair source without fetching, checking out, or discarding local edits.
function Resolve-RustLocalSource {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Repo,
        [string]$Source,
        [string]$Branch = 'dots/rust-recovery-resume-3',
        [switch]$RemoteCandidate
    )

    if ($Source) {
        if (-not (Test-Path -LiteralPath (Join-Path $Source 'rust\Cargo.toml') -PathType Leaf)) {
            throw "-Source に rust\Cargo.toml がありません: $Source"
        }
        return (Resolve-Path -LiteralPath $Source).Path
    }
    # Remote candidate selection is an explicit operation, never the daily default.
    if ($RemoteCandidate) { return $null }

    $records = @(& git -C $Repo -c core.quotePath=false worktree list --porcelain)
    if ($LASTEXITCODE -ne 0) { throw 'Rust修正元のGit worktree一覧を取得できませんでした。' }
    $path = $null
    foreach ($line in $records) {
        if ($line.StartsWith('worktree ')) { $path = $line.Substring(9) }
        elseif ($line -eq "branch refs/heads/$Branch") {
            if (-not $path -or -not (Test-Path -LiteralPath (Join-Path $path 'rust\Cargo.toml') -PathType Leaf)) {
                throw "Rust修正元に rust\Cargo.toml がありません: $path"
            }
            return (Resolve-Path -LiteralPath $path).Path
        }
    }
    throw "Rust修正元のブランチ $Branch が手元のworktreeにありません。修正元を開くか -Source で指定してください。旧コピーを自動ビルドしません。"
}
