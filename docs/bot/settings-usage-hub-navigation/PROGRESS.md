# Progress board
> 最終更新: 2026-10-13 17:14:11 UTC

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
| CI at last verified PR head | `25a91db2b6189104fac0092e4fbd761e42809614`; Validate run [#383](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/38436223040) completed with failure only in `govulncheck`; secret-scan [#531](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/38436223029) succeeded. This final planned progress update is documentation-only; verify the new head once and put its result in Issue #20. |

## Work state

| Work | Owner | Status | Evidence and next action |
|---|---|---|---|
| Accepted design and task instruction | User / Codex | Adopted / handed off | One persistent footer Settings entry. Settings opens at General; Usage and Hub are destinations inside that screen, with no duplicate Settings destination. |
| Implementation | Dots | Submitted on task branch | Usage/Hub content and actions moved into Settings navigation; the lower footer has one Settings control. Existing session token usage remains separate. Commit `4498465`, tree `716e770365985e9c547001acb9352efeb9222c95`. |
| Independent implementation review | Dots reviewer | Complete | Separate reviewer inspected exact tree `716e770365985e9c547001acb9352efeb9222c95`; no required product code findings reported. |
| Adversarial source review | Codex | Complete | Audited exact commit/tree above. The earlier CSS-specificity concern was corrected as a selector-reading error; the explicit narrow-width wrap rule/test is responsive hardening. No remaining required findings. |
| Draft PR and CI | Codex | CI completed on `25a91db2b6189104fac0092e4fbd761e42809614`; final progress update is documentation-only | Validate run #383 failed only at `govulncheck`; all three platform Go test jobs, web-check, instrumentation, staticcheck, gosec, third-party, web-package-manager-consistency, and go-mod-tidy succeeded. secret-scan run #531 succeeded. `release-token-scopes` was skipped. Verify the new head once and record the result in Issue #20. |
| Actual screen acceptance | User | Pending | Check the initial General view, Usage/Hub navigation, and narrow-width icon/content after the user has the candidate. No screen evidence is claimed here. |

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

PR CI at head `25a91db2b6189104fac0092e4fbd761e42809614`: Validate run #383 failed because govulncheck found 10 Go standard-library vulnerabilities, 1 vulnerability in imported packages, and 3 module vulnerabilities not reached by the code. The log lists fixes in Go `1.26.9` and `golang.org/x/net@v0.60.0`. This task diff changes no Go files or module versions. The other completed Validate jobs passed as listed above; release-token-scopes was skipped, not a pass. This final progress-only update will move the PR head; verify its workflows once, then keep the Issue comment as the status source of truth.

These are source-level checks. No Rust/product build, Hub startup, local application reload, browser/device inspection, merge, release, or deployment has been performed. The actual Settings screen and responsive icon remain separate acceptance checks.
