#!/usr/bin/env bash
# Publish the npm packages: the 4 platform optionalDependency packages first
# (so the root can resolve them), then the root package last.
#
# Hardened for the failure modes that made the first release painful:
#   - dist-tag selection: prerelease tags (vX.Y.Z-rc.1) -> "next", stable -> "latest"
#   - E429 / rate-limit retry with exponential backoff. npm rate-limits the PUT
#     that CREATES a brand-new package; without backoff the very first release of
#     each package flakes. Once a package exists, version bumps rarely hit this.
#   - idempotent: a version already on the registry is treated as success, so a
#     re-run (e.g. the npm-only workflow_dispatch) safely skips what's done.
#
# Usage: scripts/publish-npm.sh <tag>      (NODE_AUTH_TOKEN must be set)
set -euo pipefail

TAG="${1:?usage: publish-npm.sh <tag>}"
DIST_TAG=latest
case "$TAG" in
  *-*) DIST_TAG=next ;;
esac
echo "publishing tag=$TAG dist-tag=$DIST_TAG"

# Order matters: platform packages before root.
PKGS=(
  many-ai-cli-windows-x64
  many-ai-cli-linux-x64
  many-ai-cli-macos-intel
  many-ai-cli-macos-apple-silicon
  many-ai-cli
)

publish_one() {
  local dir="$1" attempt=1 max=6 delay=30 out rc
  while :; do
    if out="$(npm publish "$dir" --access public --provenance --tag "$DIST_TAG" 2>&1)"; then
      echo "OK       $dir"
      return 0
    fi
    rc=$?
    # Already on the registry -> idempotent success (safe re-run).
    if printf '%s' "$out" | grep -qiE 'cannot publish over|previously published|EPUBLISHCONFLICT|409 Conflict'; then
      echo "SKIP     $dir (version already published)"
      return 0
    fi
    # Rate limited -> exponential backoff, then retry.
    if printf '%s' "$out" | grep -qiE 'rate.?limit|429|too many requests' && [ "$attempt" -lt "$max" ]; then
      echo "RETRY    $dir (rate limited; attempt ${attempt}/${max}, sleeping ${delay}s)"
      sleep "$delay"
      attempt=$((attempt + 1))
      delay=$((delay * 2))
      continue
    fi
    echo "FAILED   $dir (rc=${rc}):"
    printf '%s\n' "$out" | tail -n 12
    return 1
  done
}

for p in "${PKGS[@]}"; do
  publish_one "./npm/$p"
done

VER="${TAG#v}"
echo ""
echo "== verify on registry (want ${VER}) =="
for p in "${PKGS[@]}"; do
  got="$(npm view "${p}@${VER}" version 2>/dev/null || true)"
  echo "  ${p}: ${got:-<not visible yet>}"
done
echo "done."
