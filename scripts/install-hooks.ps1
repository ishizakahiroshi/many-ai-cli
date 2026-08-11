# One-shot installer for git pre-commit hooks (non-pnpm projects).
#
# What it does:
#   - Sets core.hooksPath to .githooks (git-tracked, unlike .git/hooks/).
#
# Why:
#   secrets-scan layer 2 needs a hook that survives `git clone`. The default
#   .git/hooks/ is not tracked, so we use core.hooksPath -> .githooks/.
#
# Pairs with: scripts/install-hooks.sh (POSIX equivalent).
# Conflicts with: husky (also sets core.hooksPath). Use one or the other.

$ErrorActionPreference = 'Stop'

$repoRoot = (& git rev-parse --show-toplevel).Trim()
Set-Location $repoRoot

if (-not (Test-Path .githooks)) {
  New-Item -ItemType Directory .githooks | Out-Null
}

& git config core.hooksPath .githooks

Write-Host "OK: hooks active (core.hooksPath = .githooks)"
Write-Host "    pre-commit: .githooks/pre-commit"
