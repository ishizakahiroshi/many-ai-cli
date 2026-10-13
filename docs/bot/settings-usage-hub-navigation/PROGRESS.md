# Progress board
> 最終更新: 2026-10-13 17:00:19 UTC

## Task

- Task: #P-20261013-001 many-ai-cli
- Issue URL: https://github.com/ishizakahiroshi/many-ai-cli/issues/20
- Instruction repo/path: ishizakahiroshi/many-ai-cli / docs/bot/settings-usage-hub-navigation/
- Implementation branch: `dots/P-20261013-001-settings-usage-hub-nav`
- Planned PR base: `wt/rust-ui-publication-20261011`
- Product base at instruction creation: `d95effb70cf550ff93ca593160ee8aa827abc28b`
- Parent Draft PR: https://github.com/ishizakahiroshi/many-ai-cli/pull/18

## Evidence pointers

| Evidence | Value |
|---|---|
| Fixed task instruction | [README.md](https://github.com/ishizakahiroshi/many-ai-cli/blob/9db24622dad1c2fc5ee48d8661b337a1e98a7249/docs/bot/settings-usage-hub-navigation/README.md) |
| Implementation commit | [4498465](https://github.com/ishizakahiroshi/many-ai-cli/commit/44984653b9f32cf66b3c359556e0893b2cebedb1), parent `8b90f475c56df30f47709fe538b8938c35f0187f` |
| Implementation tree | `716e770365985e9c547001acb9352efeb9222c95` |
| Dots independent review | Exact implementation tree `716e770365985e9c547001acb9352efeb9222c95`; 0 required code findings reported |
| Codex source review | Exact commit `44984653b9f32cf66b3c359556e0893b2cebedb1` / tree `716e770365985e9c547001acb9352efeb9222c95`; no required findings |
| Draft PR / initial CI | [PR #21](https://github.com/ishizakahiroshi/many-ai-cli/pull/21), open Draft to `wt/rust-ui-publication-20261011` | At creation, head `1ab86a403304dce69ca21793042ce359cf2cd209`; Validate was queued and secret-scan was in progress. This PROGRESS update is documentation-only and advances the PR head; re-read checks on the new head before reporting CI. |
| Draft PR | Created as [PR #21](https://github.com/ishizakahiroshi/many-ai-cli/pull/21), open and Draft; base head was still `d95effb70cf550ff93ca593160ee8aa827abc28b` at creation |
| Actual screen acceptance | Pending; no Hub start, app reload, browser, or device check performed |

## Work state

| Work | Owner | Status | Evidence and next action |
|---|---|---|---|
| Accepted design and task instruction | User / Codex | Adopted / handed off | One persistent footer Settings entry. Settings opens at General; Usage and Hub are destinations inside that screen, with no duplicate Settings destination. |
| Implementation | Dots | Submitted on task branch | Usage/Hub content and actions moved into Settings navigation; the lower footer has one Settings control. Existing session token usage remains separate. Commit `4498465`, tree `716e770365985e9c547001acb9352efeb9222c95`. |
| Independent implementation review | Dots reviewer | Complete | Separate reviewer inspected exact tree `716e770365985e9c547001acb9352efeb9222c95`; no required product code findings reported. |
| Adversarial source review | Codex | Complete | Audited exact commit/tree above. The earlier CSS-specificity concern was corrected as a selector-reading error; the explicit narrow-width wrap rule/test is responsive hardening. No remaining required findings. |
| Draft PR and CI | Codex | Draft PR created; exact-head checks being re-read | PR #21 is open/Draft against the planned base. Before this documentation-only follow-up, head was `1ab86a403304dce69ca21793042ce359cf2cd209` with Validate queued and secret-scan in progress. Verify the new head, diff, and its checks after this update. |
| Windows build and actual screen acceptance | User | Pending | Check initial General view, Usage/Hub navigation, and narrow-width icon/content after the user has the candidate. No screen evidence is claimed here. |

## Validation evidence

Dots reports these checks on the candidate source:

- `bun run check`: exit 127; Bun is unavailable, so the required Bun check did not run.
- TypeScript `tsc --noEmit`: exit 0 with cached TypeScript 5.9.3. This does not substitute for the pinned Bun / TypeScript 6.0.3 check.
- Focused `settings-usage-hub.test.ts`: 11/11 passed with Node 24.19.0 and cached TypeScript 5.9.3.
- UI component guard tests: 14/14; UI guard covered 39 CSS and 152 code files.
- Module-init guard: 182 modules; wheel-overlay guard: 33 overlays / 1 exemption.
- Text hygiene, instrumentation guard, and `git diff --check`: passed.
- No dependency or CI workflow changes. The 12 removed UI exception entries correspond to obsolete Usage/Hub popup/footer selectors; no new exception was added.

The Dots reviewer independently reran the focused 11/11 tests, UI guard tests, module-init guard, wheel-overlay guard, TypeScript check, text hygiene, and instrumentation check on the same tree.

These are source-level checks. No Rust/product build, Hub startup, local application reload, browser/device inspection, merge, release, or deployment has been performed. The actual Settings screen and responsive icon remain separate acceptance checks.
