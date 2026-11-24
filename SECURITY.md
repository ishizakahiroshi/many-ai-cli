# Security Policy

## Supported versions

Security fixes are made on `develop` and ship in the next release. Only the newest release line receives them; earlier versions are not patched and fixes are not backported.

The Go implementation (0.9.x and earlier) is frozen. The Rust implementation shipped starting with 0.10.0. A fix on `develop` reaches installed copies when a release containing it is installed.

| Version | Supported |
|---|---|
| Newest Rust release line | :white_check_mark: |
| Earlier Rust releases and the Go implementation | :x: |

## Reporting a vulnerability

Please report security issues privately via GitHub Security Advisories for this repository:

https://github.com/ishizakahiroshi/many-ai-cli/security/advisories/new

If Security Advisories are unavailable to you, please contact via email: `ishizakahiroshi.dev@gmail.com` with the subject prefix `[SECURITY] many-ai-cli`.

Please do not open a public issue for an unfixed vulnerability.

When reporting, please include:
- A clear description of the vulnerability and its potential impact
- Step-by-step reproduction instructions or a minimal proof-of-concept (PoC)
- The affected component (Hub HTTP/WS API, Wrapper PTY, Web UI, Provider Registry, or Files API)
- The OS platform (Windows / macOS / Linux) and many-ai-cli version

## Security model and trust boundaries

- **Provider session credentials.** An updated wrapper gives the provider a credential for its own live session. The Hub accepts it locally for usage reports and the child and relay operations exposed by `orchestrate`. The wrapper keeps the administrator credential for its own connection. When upgrading from a version that passed the administrator credential to providers, update both the Hub and wrapper, restart those provider sessions, and rotate the administrator credential if it may have been disclosed. A running provider needs a fresh session after administrator credential rotation or a session ID change. File access still follows the OS user's permissions.
- **Subscription settings.** New profile settings are seeded from supported preferences. Credentials or custom configuration previously copied to an existing profile can remain there. Review those profiles when upgrading, especially when switching between personal and work accounts.
- **Token-free access.** The default requires a token. Enabling `hub.allow_loopback_without_token` separately grants access to local clients and clients in `hub.trusted_networks`. Enable it only if you trust those clients with Hub access.
- **Hub API & Token Boundary**: Access to the Hub API requires authentication via a random token in a Bearer header, the query or the first WebSocket message (never a cookie; images, video and downloads use a short-lived read-only ticket derived from it), Host header validation (DNS rebinding protection), Origin / Sec-Fetch-Site validation for state-changing requests, and optional remote PIN verification.
- **Process & Workspace Isolation**: Wrapped CLI processes run with the local user's operating system privileges. Child sessions launched via relay or derivation are subject to explicit approval or configured permission tiers. A tier's list of allowed commands does not limit what an allowed command runs (a test or a script the child edited, a Git hook): in the bounded tier, a Claude child on macOS, Linux and WSL2 runs its shell commands in Claude Code's sandbox (writes only in its working folder, no network), and where there is no sandbox (Claude on native Windows, Copilot) the built-in list leaves out commands that run repository code and the child's Git commands run no repository hooks.
- **Files API**: Files preview and Git diff APIs enforce strict path confinement within allowed workspace roots, rejection of sensitive files (denylist), and prevention of symlink escapes.
