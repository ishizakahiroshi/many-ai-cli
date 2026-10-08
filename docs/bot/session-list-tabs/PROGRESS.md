---
type: reference
status: draft
tags: [ui, handoff]
owner: user
related: [README.md, REVIEW.md, FACTS.md]
last_reviewed: 2026-10-08
---

# Progress board — #P-20261008-007 many-ai-cli

> 最終更新: 2026-10-08(木) 14:58:51 UTC

Updated: 2026-10-08. This folder is public and may be read by the delegated implementation/review workers.

Work repo: `ishizakahiroshi/many-ai-cli`.
Work branch: `dots/P-20261008-007-session-list-tabs`.
PR base: `dots/rust-recovery-resume-3`.
Behavior oracle: `4a3c141a1e6573459d3ff2a8cc6c9962bce6def3`.
Management Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/12
First instruction commit: `c29b58fa1c5bff2298197d9700c61b503b581b18`.
Fixed instruction: [4102dd2aeb4df8452c8e3f8c2751d9fdeaa0abda](https://github.com/ishizakahiroshi/many-ai-cli/blob/4102dd2aeb4df8452c8e3f8c2751d9fdeaa0abda/docs/bot/session-list-tabs/README.md).
Draft PR/code SHA/reviewed code SHA: not submitted.

| Stage | Owner | Issue evidence | Remaining gate |
|---|---|---|---|
| Publication/dispatch | Local coordinator / dots implementer | [Acceptance and read-back](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6062609938) | Accepted; product code not submitted |
| C2 state/save | dots implementer | [Stopped before code edits](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6062677527) | Owner decision: exact spawn correlation conflicts with Web-only API boundary |
| C3 UI/manage | dots implementer | Not started | Strip/icons/menu/rename/delete |
| C4 drag/move/status | dots implementer | Not started | Actual listeners/root family/overflow/touch/waiting |
| Independent review | Different dots worker | Not started | Exact code SHA and fixes re-reviewed |
| Product acceptance | Owner/local coordinator | Not started | CI/diff, Windows/native input/product UI, no merge implied |

The management Issue comments are the state-transition source of truth once created. Link accepted/submitted/reviewed/blocked/owner-accepted receipts here; do not store private chat locators or machine paths. Keep instruction SHA, code SHA, reviewed SHA and board commit distinct. Do not self-reference the current board commit inside itself.

Next: await the owner's explicit scope decision. Ordinary spawn returns only `ok` (plus an orchestration ID for orchestration); grid returns `ok`, `layout`, and `count`. Concurrent external or other-browser creation cannot be correlated exactly without a request/creation identifier. No hidden label workaround or backend/API change is authorized. Options are a safe default-tab fallback for uncorrelated creation, or separately authorized minimal API correlation work. No product code has been edited.

Capability receipt: repository/branch reads and Issue comment writes verified. Node v24.19.0, Chromium 154 and existing Playwright are available. Existing TypeScript 5.9.3 is available in another workspace, while this repo specifies ^6.0.3 and has no installed dependencies. `bun run check` from `web/` exited 127 (`bun: command not found`). No dependencies installed, product build, Hub operation, browser-product acceptance or Windows acceptance performed. The board-only update does not constitute a product implementation or reviewed code SHA.

