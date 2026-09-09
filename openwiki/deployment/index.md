# Files

- [Build, Packaging, and Release Pipeline](build-and-release.md) - How many-ai-cli is built locally with make, how git tags are the single source of version truth, and how the tag-driven GitHub Actions workflow publishes to GitHub Releases, winget, Homebrew, and npm.
- [Docker and Remote Server Deployment](docker-remote-server.md) - The per-user GHCR container model for running many-ai-cli on a remote server — Dockerfile stages, the loopback-only socat relay, entrypoint lifecycle, and opt-in cron-based image updates.
