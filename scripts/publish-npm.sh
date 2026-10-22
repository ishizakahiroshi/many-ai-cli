#!/usr/bin/env bash
# Platform packages first, root last; immutable existing versions must match the
# exact local tarball integrity. Lifecycle scripts are disabled in the helper.
set -euo pipefail
exec node "$(dirname "$0")/publish-npm.mjs" "$@"
