---
type: reference
status: draft
tags: [ui, handoff]
owner: user
related: [README.md, REVIEW.md, FACTS.md]
last_reviewed: 2026-10-08
---

# Progress board — #P-20261008-007 many-ai-cli

> 最終更新: 2026-10-08(木) 15:44:51 UTC

Updated: 2026-10-08. This folder is public and may be read by the delegated implementation/review workers.

Work repo: `ishizakahiroshi/many-ai-cli`.
Work branch: `dots/P-20261008-007-session-list-tabs`.
PR base: `dots/rust-recovery-resume-3`.
Behavior oracle: `4a3c141a1e6573459d3ff2a8cc6c9962bce6def3`.
Management Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/12
First instruction commit: `c29b58fa1c5bff2298197d9700c61b503b581b18`.
Fixed instruction: [4102dd2aeb4df8452c8e3f8c2751d9fdeaa0abda](https://github.com/ishizakahiroshi/many-ai-cli/blob/4102dd2aeb4df8452c8e3f8c2751d9fdeaa0abda/docs/bot/session-list-tabs/README.md).
C2 initial code: `f2ba035558882b34e465bc91481809235d93d40c`.
C2-R1 fix code: `e7eea35dc5c31f7f17a92bf0175cc7960e86d520`.
Connected C3/C4 candidate: included in this publication; exact code SHA will be recorded in the Issue after publication. Draft PR and final independently reviewed SHA: pending.

| Stage | Owner | Issue evidence | Remaining gate |
|---|---|---|---|
| Publication/dispatch | Local coordinator / dots implementer | [Acceptance and read-back](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6062609938) | Accepted; partial scope continuation authorized |
| C2 state/save | dots implementer | [C2 code receipt](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6063138449), [C2-R1 finding](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6063190078) | C2-R1 fixed at e7eea35; final review remains |
| C3 UI/manage | dots implementer | Connected candidate prepared | Exact-SHA independent review and product acceptance |
| C4 drag/move/status | dots implementer | Connected candidate prepared | Synthetic caller checks; browser/native-input acceptance still unavailable |
| Independent review | Different dots worker | Preliminary findings fixed in candidate | Exact code SHA re-review pending |
| Product acceptance | Owner/local coordinator | Not started | CI/diff, Windows/native input/product UI, no merge implied |

The management Issue comments are the state-transition source of truth once created. Link accepted/submitted/reviewed/blocked/owner-accepted receipts here; do not store private chat locators or machine paths. Keep instruction SHA, code SHA, reviewed SHA and board commit distinct. Do not self-reference the current board commit inside itself.

Next: publish the connected candidate after focused checks, obtain an exact-code-SHA independent re-review, and submit a Draft PR against the specified base. [Owner scope continuation](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6062835156) authorizes independent work while requirement 11 remains **unimplemented and awaiting a decision**. No API expansion, requirement relaxation, guessed spawn affiliation or label correlation is authorized. This is a partial submission, not whole-task acceptance.

Synthetic state validation: C2-R1 reproduction failed before the fix and passed after it. New state fixtures (17) plus existing sidebar-tree fixtures (21) pass. Connected UI tests exercise actual menu, pointer and scroll callers with the repository's existing synthetic-DOM/TypeScript-transpiler testing pattern; counts and exact commands are recorded in the publication Issue receipt. These are not browser layout/native-input tests.

Capability receipt: repository/branch reads/writes and Issue comment writes verified. Node v24.19.0 is available. Existing TypeScript 5.9.3 provides an alternate noEmit check; the repository specifies ^6.0.3. `bun run check` from `web/` exited 127 (`bun: command not found`), and the official module-init check cannot run because esbuild is not installed. No dependencies installed. Standalone Chromium could not start due to IPC restrictions; the cloud browser rejected local-file fixture navigation by URL policy. No bypass was attempted. Browser layout/native input, product build, Hub operation and product/Windows acceptance remain unverified. Owner-provided Windows Bun fixture results are a separate, limited piece of evidence, not product acceptance.

Rollback: revert this task branch's feature commits or remove the browser-local `many-ai-cli-session-list-tabs-v1` item to reset only tab organization. Sessions, processes, history and server preferences are not deleted by tab reset.
