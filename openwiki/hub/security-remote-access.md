---
type: architecture-component
title: Security, Auth, and Remote Access Protection
description: The Hub's token-based auth model, its loopback-only binding, the kill-switch revoke-all endpoint, the optional PIN gate for non-loopback access, and new-device connection notifications.
tags: [security, auth, token, pin, revoke-all, loopback, remote-access]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-b5500eb12b9efc8794e83269
    resource: repo://docs/v0.3.x-many-ai-cli-design.md
  - id: openwiki-source-e9ea19da619dfd1bbcc7b961
    resource: repo://internal/hub/auth_handlers.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## The trust boundary is "the OS user," not "possesses a network path"

The design's unchanging core (design doc §14.4): the Hub binds to `127.0.0.1` only, a token is required for essentially everything, and a loopback caller is treated as comfortably trusted with just that token — see [Configuration and On-Disk Layout](/openwiki/architecture/configuration.md) for `token`'s storage and [Files and Git Tabs](/openwiki/hub/files-and-git.md) for how the direct-loopback-vs-logically-remote distinction also shapes the Files API's read scope. Rather than bolting a heavyweight authentication system onto the Hub itself, remote-access risk (a lost phone, a leaked QR code) is handled with a cheap, effective kill switch plus an opt-in PIN for anyone who wants it, and the manual's loss-response playbook is candid that these are the practical mitigations, not a claim of enterprise-grade access control. This mirrors declined-direction D-11 in the project's own tracked list of rejected proposals: the Hub deliberately does not narrow an authenticated token holder below "equivalent to the OS user" via capability tokens, remote deny, or mandatory PIN.

## Revoke-all: the kill switch

`POST /api/auth/revoke-all` regenerates and persists both `cfg.Token` and `cfg.AuthCookieSecret` (the cookie-signing key), which simultaneously invalidates the token URL, the token cookie, and every PIN-session cookie — every device including a lost one is locked out at once, including the PC that issued the request, which must re-authenticate against the new token/URL returned in the response. If persisting the new values to disk fails, the handler explicitly rolls the in-memory config back to the previous token/secret before returning an error, rather than leaving the running process holding new in-memory credentials that were never written to disk and never returned to the caller either — a state that would otherwise cut off all external Hub access until a manual process restart.

Revoke-all carries its own extra authorization gate beyond the normal token check: a **remote** (non-loopback) caller must also already hold a valid PIN cookie to invoke it, even though `remotePINRequired()` normally lets a bootstrap caller through when no PIN is configured yet. Without this extra gate, any remote holder of the current token could rotate the token and cookie secret and lock the legitimate owner out first — the comment in `auth_handlers.go` states this plainly: "a loopback session, or an already-PIN-authenticated remote session" is required to invoke the kill switch remotely.

## Optional PIN for non-loopback access

`cfg.RemotePINHash` (bcrypt-hashed, plaintext never stored) is off by default. When set, `requireRemotePIN`'s gate inside `guard()` adds a PIN-login requirement specifically for non-loopback access — a loopback connection always passes through untouched. A successful PIN login issues a signed, expiring cookie (`MANY_AI_CLI_pin`, HMAC-SHA256 over `AuthCookieSecret`) rather than treating the PIN as a bearer credential on every request, and that cookie is invalidated wholesale the same way the token is — by a revoke-all rotation of `AuthCookieSecret`. Because a numeric PIN of 6+ digits is brute-forceable, failed attempts are rate-limited per IP with exponential backoff (5 failures → 1 minute lockout → 5 minutes → 30 minutes) plus an overall cap intended to blunt a distributed brute-force attempt, both surfaced as HTTP `429`. `GET /api/auth/status` (pin_enabled/authed/locked/retry_after) and `POST /api/auth/login` deliberately go through `guardBase` (skipping the PIN gate itself, since that would be circular), while `POST /api/auth/set-pin` requires the full `guard()` — meaning a *remote* caller can only change the PIN once already PIN-authenticated. Every `/api/auth/*` response sets `Cache-Control: no-store`, and PIN values, tokens, auth cookies, and failed-PIN input are never written to the structured log — a failure is logged only as a count/lockout state (`retry_after`), never the attempted value.

## Session logout vs. revoke-all: two different weights

`POST /api/auth/logout` is a deliberately lighter-weight action than revoke-all: it clears only the calling browser's own `MANY_AI_CLI_token`/`MANY_AI_CLI_pin` cookies (`MaxAge=-1`) without rotating the shared token or cookie secret, so it has no effect on any other device's access — useful after handing a borrowed device back, distinct from the "something may have leaked" case revoke-all is for. Its authorization is intentionally asymmetric from a normal write endpoint: it runs through `guardBase` (token + method + Host + Origin) without the PIN gate, on the reasoning that possessing a valid session cookie already proves you may end that same session, while a request with no cookie at all has nothing to log out — closing off both a CSRF path and a "force-logout someone else" path, and satisfying the same expectation `TestRegisteredAPIRoutesRequireToken` checks for every route.

## Tailscale `serve`: a proxy in front of loopback, not a bind change

The optional mobile-connect-via-Tailscale path (design doc §14.3) never changes the Hub's bind address — it stays `127.0.0.1` — and instead relies on `tailscale serve` proxying from the tailnet into that same loopback port, so a tailnet device can reach the Hub without the Hub itself ever listening beyond loopback. Tailscale **Funnel** (public internet exposure) is deliberately never used; the mobile-connect UI actively steers a user toward leaving Funnel unchecked during first-time HTTPS setup. HTTPS `serve` is preferred over reaching the tailnet IP directly, because a bare `127.0.0.1`-bound HTTP origin loses "secure context" status in the browser, which disables the service worker, Web Push, PWA installation, and microphone access — an SSH local-forward to `http://127.0.0.1:<port>` is kept as the one plain-HTTP path that still qualifies as a secure context (since the browser treats `127.0.0.1` itself as trustworthy). Enabling `serve` idempotently adds the tailnet's own DNS name to `hub.allowed_hosts` so the Host/Origin guard and the WebSocket handshake accept it. In an environment with no reachable Tailscale CLI (a Docker container, headless host), this path degrades gracefully and the mobile-connect flow instead points the user at an SSH tunnel or the launcher.

## New-device connection notifications

A first connection from a device the Hub has not seen before — identified by a truncated SHA-256 hash of IP+User-Agent, not the raw values — triggers a notification to the account owner through the existing push/ntfy/webhook channels (see [Notifications and Done Summaries](/openwiki/hub/notifications.md)), regardless of the user's configured `notify.events` filter, since this is treated as a security signal rather than an ordinary event a user might have opted out of. This is checked on `handleIndex`, on a WebSocket UI attach, and on a successful PIN login. A previously-seen device is not re-notified on every reconnect (a 24-hour TTL with a soft cap of 256 tracked devices bounds the "known" set), so the intent is specifically to catch first use of a token or QR code that should not have been usable — an intrusion-detection signal for a leaked credential, not a general connection log.
