# Security Policy

## Supported versions

Security fixes are applied to the latest release on `main` and active development on `develop`.

| Version | Supported |
|---|---|
| >= 0.8.x | :white_check_mark: |
| < 0.8.0 | :x: |

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
- **Process & Workspace Isolation**: Wrapped CLI processes run with the local user's operating system privileges. Child sessions launched via relay or derivation are subject to explicit approval or configured permission tiers.
- **Files API**: Files preview and Git diff APIs enforce strict path confinement within allowed workspace roots, rejection of sensitive files (denylist), and prevention of symlink escapes.
