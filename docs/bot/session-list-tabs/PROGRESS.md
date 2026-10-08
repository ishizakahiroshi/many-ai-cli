---
type: reference
status: draft
tags: [ui, handoff]
owner: user
related: [README.md, REVIEW.md, FACTS.md]
last_reviewed: 2026-10-09
---

# Progress board — #P-20261008-007 many-ai-cli

> 最終更新: 2026-10-09(金) JST — publication and independent review recorded

Updated: 2026-10-09. This folder is public and may be read by the delegated implementation/review workers.

Work repo: `ishizakahiroshi/many-ai-cli`.
Work branch: `dots/P-20261008-007-session-list-tabs`.
PR base: `dots/rust-recovery-resume-3`.
Behavior oracle: `4a3c141a1e6573459d3ff2a8cc6c9962bce6def3`.
Management Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/12
First instruction commit: `c29b58fa1c5bff2298197d9700c61b503b581b18`.
Fixed instruction: [4102dd2aeb4df8452c8e3f8c2751d9fdeaa0abda](https://github.com/ishizakahiroshi/many-ai-cli/blob/4102dd2aeb4df8452c8e3f8c2751d9fdeaa0abda/docs/bot/session-list-tabs/README.md).
C2 initial code: `f2ba035558882b34e465bc91481809235d93d40c`.
C2-R1 fix code: `e7eea35dc5c31f7f17a92bf0175cc7960e86d520`.
Published code SHA = independently reviewed code SHA: `081b0679022934eca7d19577d7c3b80b60580196`.
Draft PR: https://github.com/ishizakahiroshi/many-ai-cli/pull/13
Publication receipt: https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6070797580
Independent review receipt: https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6070849986
Validate CI for the code SHA: https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/37857413422 (success; release-token-scopes intentionally skipped for pull requests).

| Stage | Owner | Issue evidence | Remaining gate |
|---|---|---|---|
| Publication/dispatch | Local coordinator / dots implementer | [Acceptance and read-back](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6062609938) | Accepted; partial scope continuation authorized |
| C2 state/save | dots implementer | [C2 code receipt](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6063138449), [C2-R1 finding](https://github.com/ishizakahiroshi/many-ai-cli/issues/12#issuecomment-6063190078) | Fix verified; requirement 11 remains unimplemented |
| C3 UI/manage | dots implementer | Published at 081b0679; C3-R1 long-press lifecycle fix verified | Product layout/native-input acceptance |
| C4 drag/move/status | dots implementer | Actual-listener synthetic checks and exact-source review passed | Browser/native-input acceptance |
| Independent review | Different dots worker | Fixed public SHA 081b0679 verified; no further requested fixes | Complete for this source; product acceptance remains separate |
| Product acceptance | Owner/local coordinator | Typecheck, module-init, related tests and CI passed | Windows/native input/product UI, no merge implied |

The management Issue comments are the state-transition source of truth once created. Link accepted/submitted/reviewed/blocked/owner-accepted receipts here; do not store private chat locators or machine paths. Keep instruction SHA, code SHA, reviewed SHA and board commit distinct. Do not self-reference the current board commit inside itself.

Next: implement requirement 11 and obtain actual browser/Windows product acceptance. On 2026-10-09 the owner explicitly approved all remaining steps and parallel subagents, including the focused Rust-only API/registration/WS extension in README. Requirement 11 remains **unimplemented** until a new candidate and its evidence are submitted. Go changes, guessed spawn affiliation and label correlation remain outside scope. The earlier partial-continuation receipt is historical. This is a partial submission, not whole-task acceptance. Merge, release and deployment have not been performed.

Synthetic state validation: C2-R1 reproduction failed before the fix and passed after it. New state fixtures (17) plus existing sidebar-tree fixtures (21) pass. Connected UI tests exercise actual menu, pointer and scroll callers with the repository's existing synthetic-DOM/TypeScript-transpiler testing pattern; counts and exact commands are recorded in the publication Issue receipt. These are not browser layout/native-input tests.

Capability receipt: dots originally lacked Bun and the specified TypeScript dependencies. The local coordinator subsequently used existing Bun 1.3.14 and TypeScript 6.0.3: `bun run check` passed, 177 modules evaluated without TDZ, and 94 related tests passed. CI web-check passed all steps, including 356 unit tests and 412 fixtures. No new local dependencies were installed, and no local product build or running Hub operation occurred. Dots browser attempts were blocked by IPC/URL restrictions; no bypass was attempted. Browser layout/native input and Windows product UI acceptance remain unverified. CI build/test success is not product acceptance.

Rollback: revert this task branch's feature commits or remove the browser-local `many-ai-cli-session-list-tabs-v1` item to reset only tab organization. Sessions, processes, history and server preferences are not deleted by tab reset.
