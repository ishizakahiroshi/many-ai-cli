# Security Policy

## Supported versions

Security fixes are made on `develop` and ship in the next release. Only the newest release line receives them; earlier versions are not patched and fixes are not backported.

The Go version (0.9.x and earlier) is frozen and will not get further fixes. Its successor, 0.10.0 (built from Rust), has not been released yet, so the fixes made since 0.9.0 are so far only in the source on `develop` and `main`. Once 0.10.0 is out, update to it to receive them.

| Version | Supported |
|---|---|
| 0.10.x (Rust, after its release) | :white_check_mark: |
| 0.9.x (Go) | :x: no further fixes; update to 0.10.x once it is released |
| < 0.9.0 | :x: |

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

- **Hub API & Token Boundary**: Access to the Hub API requires authentication via a random bearer/cookie/query token, Host header validation (DNS rebinding protection), Origin / Sec-Fetch-Site validation for state-changing requests, and optional remote PIN verification.
- **Process & Workspace Isolation**: Wrapped CLI processes run with the local user's operating system privileges. Child sessions launched via relay or derivation are subject to explicit approval or configured permission tiers. A tier's list of allowed commands does not limit what an allowed command runs (a test or a script the child edited, a Git hook): in the bounded tier, a Claude child on macOS, Linux and WSL2 runs its shell commands in Claude Code's sandbox (writes only in its working folder, no network), and where there is no sandbox (Claude on native Windows, Copilot) the built-in list leaves out commands that run repository code and the child's Git commands run no repository hooks.
- **Files API**: Files preview and Git diff APIs enforce strict path confinement within allowed workspace roots, rejection of sensitive files (denylist), and prevention of symlink escapes.
