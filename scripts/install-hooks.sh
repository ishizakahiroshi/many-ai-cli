#!/usr/bin/env sh
# One-shot installer for git pre-commit hooks (non-pnpm projects).
#
# What it does:
#   - Sets core.hooksPath to .githooks (which is git-tracked, unlike .git/hooks/).
#   - Ensures the .githooks/pre-commit is executable (POSIX only).
#
# Why:
#   secrets-scan layer 2 needs a hook that survives `git clone`. The default
#   .git/hooks/ is not tracked, so we use core.hooksPath -> .githooks/.
#
# Pairs with: scripts/install-hooks.ps1 (Windows equivalent).
# Conflicts with: husky (also sets core.hooksPath). Use one or the other.

set -e

repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

if [ ! -d .githooks ]; then
  mkdir -p .githooks
fi

if [ -f .githooks/pre-commit ]; then
  chmod +x .githooks/pre-commit
fi

git config core.hooksPath .githooks

printf '%s\n' "OK: hooks active (core.hooksPath = .githooks)"
printf '%s\n' "    pre-commit: .githooks/pre-commit"
