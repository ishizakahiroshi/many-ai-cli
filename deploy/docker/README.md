# Docker support scope for v1.0.0

Official Docker images, container builds and automatic updates are unsupported
for v1.0.0 and excluded from its release gate. The current container build has a
known failure. This scope decision does not repair that build.

Automatic image publishing is stopped. The former GHCR images were deleted on
2026-10-25: they bundled a Node.js release affected by CVE-2026-111497 and would
never be rebuilt with the fix, so the old tag names can no longer be pulled. The
Dockerfile, compose files, user examples and update scripts are retained as
historical / pre-v1 material. Their presence does not promise a current working
image, ongoing publishing or automatic updates. The legacy build and cron
examples in the root README are historical references, not v1.0.0 setup steps.

Linux binaries and SSH remote operation remain in the v1.0.0 release scope.
Run the Linux binary directly under a separate OS user and home per person,
with separate working directories and Hub ports, using `tmux`, `screen` or
`systemd` when a persistent Hub is needed. Connect through SSH rather than
depending on an official container image. See [the main README](../../README.md)
and [the Japanese README](../../README.ja.md) for installation and SSH guidance.

The [`docker-image` workflow](../../.github/workflows/docker-image.yml) is replaced
with a manual informational notice and read-only permissions. It has no image
build, login or push steps. Keep it disabled: this replacement source must reach
the default branch before any later re-enable. Restoring Docker support and any
publishing or automatic-update path requires separate approval after the build,
image contents and operation have been repaired and verified.

The existing input-pin checker and its tests remain as static protection for
the retained Docker inputs. Passing them does not prove a successful build,
runtime behavior, image freshness or acceptance of Docker support. They must
not be used as a v1.0.0 Docker release gate.
