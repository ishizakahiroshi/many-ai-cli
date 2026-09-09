---
type: architecture-component
title: Notifications and Done Summaries
description: How the Hub always surfaces completion/approval state in-UI, and how the two separate opt-in external channels — Web Push and outbound ntfy/webhook — deliver it without leaking the Hub auth token.
tags: [notifications, web-push, done-summary, ntfy, webhook, vapid]
verified:
  - by: openwiki/0.5.0
    at: 2026-09-08T13:17:25.310Z
sources:
  - id: openwiki-source-58a68ef24682e5bbb97ed70e
    resource: repo://internal/hub/done_summary.go
  - id: openwiki-source-8c3778a2069897f6ea1fe695
    resource: repo://internal/hub/push_test.go
  - id: openwiki-source-3735fd619becc2d912d40b6c
    resource: repo://internal/hub/push.go
  - id: openwiki-source-a2e03051b566e646c5cd8cff
    resource: repo://internal/notify/notify.go
generated: { by: "claude-code", at: "2026-09-08T13:17:25.310Z" }
---

## In-UI visibility is not opt-in; external delivery is

`publishDoneSummary` (`done_summary.go`) makes a session's completion text visible in the Hub UI unconditionally — its own doc comment states this is deliberate, "even when external notifications are disabled," and that external delivery (Web Push, ntfy, webhook) remains a separate, explicit opt-in user preference layered on top. Every summary passes through `sessionlog.MaskSecrets` and is truncated to `doneSummaryMaxRunes` (320 runes) before being stored or sent anywhere. A parallel `publishRelayDoneSummary` sends the same browser/external notification but deliberately skips starting a git-turn snapshot for the parent session, because relay work already has its own representation (a dedicated worktree or shared-tree state) that a done-summary-triggered snapshot would be redundant with. There are two independent external channels beyond the in-UI display, and they are wired from the same event sites (approval detected, task done) but implemented as separate backends.

## Web Push (PWA)

The Hub UI can be installed as a PWA and, opt-in, receive W3C Web Push notifications even when no browser tab is open. The moving parts:

| Piece | Location | Role |
|---|---|---|
| Manifest | `web/src/manifest.webmanifest` | name/icons/`display: standalone` |
| Service worker | `web/src/sw.ts` (served as `dist/sw.js`) | receives push events, shows the notification, handles `notificationclick`, and caches the Hub token locally for building the click-through URL |
| Frontend glue | `web/src/app/pwa.ts` | registers the service worker; `subscribeWebPush()`/`unsubscribeWebPush()` |
| Server | `internal/hub/push.go` | VAPID key generation, subscription CRUD, sending pushes |

VAPID keys and subscriptions live in `~/.many-ai-cli/push_store.json` (perm `0o600`, written via a temp-file-plus-atomic-rename) — deliberately kept out of `config.yaml`, which holds only the boolean opt-in (`user_prefs.push_notifications.enabled`). Keys are generated once at Hub startup (`newPushManager()` calling `webpush.GenerateVAPIDKeys()` via the `github.com/SherClockHolmes/webpush-go` library) and reused across restarts if already present.

**The Hub token never reaches the push payload or an external push service.** `approvalPushURL(id)` builds only `/?session_id=<id>` — no `token=` query parameter at all — and this is locked in by a regression test, `TestApprovalPushURLDoesNotIncludeToken`. Instead, the service worker keeps a locally cached copy of the token (in a Cache Storage entry, populated via a `postMessage` from the page while it is open) and reads that cache to construct the full authenticated URL only at the moment the user clicks the notification — so the token is never transmitted through the push provider (e.g., a browser vendor's FCM-equivalent relay) at all, only ever locally between the page and its own service worker. Push notifications also carry a 300-second TTL and are deduplicated against re-sends within one hour via `pushManager.sent`, and any send-failure log records only the first 12 hex characters of a hash of the push endpoint URL, never the endpoint itself.

## Outbound notifications: ntfy / webhook

`internal/notify` (package doc: "implements outbound HTTP notification backends (ntfy / webhook). Each backend is fire-and-forget: errors are logged but never propagate to the caller. All methods are safe to call concurrently") is a second, independent external channel from Web Push, configured under `config.yaml`'s `notify:` block rather than the push subscription store. `notifyApprovalOutbound` fires from the exact same event site and with the same arguments as the Web Push send (`notifyApprovalPush`), so both channels observe identical trigger conditions even though they are separately implemented and separately opted into. Notifications are sent only for event kinds explicitly listed in `notify.events` — an empty list (the default) sends nothing, keeping outbound notifications opt-in the same way Web Push is.

## Path/open handlers stay loopback-only

Local-filesystem-touching endpoints reachable from Terminal/Chat/Files (`/api/open-file`, `/api/open-default-file`, `/api/open-folder`, `/api/open-terminal`, `/api/file-open-app`, `/api/terminal-app`, `/api/pick-file`, `/api/pick-directory`, `/api/path-exists`, `/api/open-dir`) are treated as host-operation APIs under the same localhost-only assumption as the rest of the Hub: they verify the request's remote address is loopback before acting. On Windows they shell out to Explorer or a configured app; under the WSL launcher mode they convert the path through `wslpath -w` first, since a WSL-side path is meaningless to a Windows-side opener.
