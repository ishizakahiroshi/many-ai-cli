# Progress board
> 最終更新: 2026-10-13(火) 17:34:47 UTC

## Task

- Task: #P-20261013-001 many-ai-cli
- Issue URL: https://github.com/ishizakahiroshi/many-ai-cli/issues/20
- Instruction repo/path: ishizakahiroshi/many-ai-cli / docs/bot/settings-usage-hub-navigation/
- Implementation branch: `dots/P-20261013-001-settings-usage-hub-nav`
- Draft PR: https://github.com/ishizakahiroshi/many-ai-cli/pull/21
- PR base: `wt/rust-ui-publication-20261011`
- Product base: `d95effb70cf550ff93ca593160ee8aa827abc28b`
- Parent Draft PR: https://github.com/ishizakahiroshi/many-ai-cli/pull/18

## Current handoff

The General navigation contract needs correction. The previous implementation generated a General navigation button from the existing General section. The adopted behavior is initial General content with no General navigation destination. Usage, Hub, and all other preexisting non-General navigation destinations remain available.

This repair was prepared and checked as working-tree changes based on PR head `ee34c46d2554759b2c9e3a322818e7c8dac3e268`. A new committed repair SHA, independent review, publication verification, and new-head CI results are pending at this handoff. Earlier review conclusions do not constitute approval of this repair or the corrected navigation contract.

| Work | Status | Evidence and next action |
|---|---|---|
| General navigation correction | Source repair prepared | General remains selectable as initial content, a search match, and a deep-link target; it no longer creates a navigation button. Selection validation uses sections rather than navigation buttons. |
| Preserved navigation | Synthetic checks passed | Every non-General section has exactly one navigation destination. Desktop/mobile reopen selects only General and clears stale search. Usage, Hub, existing sections, Close, footer toggle, Save, and Reset remain covered. |
| Mobile focus and General search | Synthetic checks passed | Back/Escape focuses a visible active destination or the search input. Search keeps the input visible while typing; Enter opens the first matching content, including General. IME confirmation and already-handled key events do not navigate. |
| Usage lifecycle | Synthetic checks passed | Returning to General through search or section/deep-link selection stops polling. A late response after General selection cannot restart polling. |
| Independent review | Pending | Review the final repair SHA against the product base and fixed instructions. Do not carry forward an earlier pass. |
| Draft PR publication and CI | Pending for repair | Keep PR #21 Draft with the existing base. Record the published repair SHA and exact-head workflow results in Issue #20. |
| Actual screen acceptance | Pending | No browser/device inspection has been performed. Initial General content, mobile search/Enter, focus, Usage/Hub navigation, and narrow-width labels/icons remain real-screen acceptance checks. |

Changed files: `web/src/app/settings.ts`, `web/tests/settings-usage-hub.test.ts`, and this progress record. General content/markup, Usage/Hub actions, and the previously adopted explicit exposure-button wrap rule remain unchanged. No dependency, Go/module, or CI workflow change is included.

## Current source and synthetic validation

Checks used Node `24.19.0` and existing cached TypeScript `5.9.3` / esbuild `0.21.5`; no dependency was installed or updated. These cache versions are not the pinned package versions.

Commands from `web/`:

- `bun run check`: exit `127` (`bun: command not found`). The required Bun/pinned-toolchain check did not run.
- `node node_modules/typescript/bin/tsc --noEmit`: exit `0`, cached TypeScript `5.9.3`. This is supplementary source evidence, not the pinned TypeScript `6.0.3` check.
- `node --experimental-transform-types --test tests/settings-usage-hub.test.ts`: exit `0`, **14/14 passed**. Actual source handlers execute against synthetic DOM/HTTP; this is not browser evidence. Test assertions require the absence of a General navigation item and retain all non-General destinations. The synthetic selector implementation now handles comma-separated selectors and hidden-item exclusion correctly; mock storage no longer supplies an invalid `usage` font-size value.

Commands from the repository root:

- `node --test scripts/check-ui-components.test.mjs`: exit `0`, **14/14 passed**.
- `node scripts/check-ui-components.mjs`: exit `0`, **39 CSS / 152 code files**, no syntax/config errors, new appearance declarations, or stale exceptions.
- `node scripts/check-web-module-init.mjs`: exit `0`, **182 modules**. Synthetic module-init guard; no product build or `web/dist` output.
- `node scripts/check-wheel-overlays.mjs`: exit `0`, **33 overlays / 1 exemption**.
- `node scripts/check-text-hygiene.mjs`: exit `0`.
- `node scripts/check-instrumentation.mjs`: exit `0`.
- `git diff --check`: exit `0`.

Checked source SHA-256:

- `web/src/app/settings.ts`: `736764211c4a2f5e325efa0978ef8d4f6733da0f82750d542cd2bc279f8dae65`
- `web/tests/settings-usage-hub.test.ts`: `c1edf1884385a053fbd44662753e73987f478b0f110d61637789d4130bf6e0fe`

No Rust/product build, Hub startup, local application reload, browser/device inspection, merge, release, or deployment was performed. Full test/build scripts were not run. The scope remains source and synthetic validation only.

## Historical evidence, superseded for current acceptance

- Fixed task instruction: [README.md at 9db2462](https://github.com/ishizakahiroshi/many-ai-cli/blob/9db24622dad1c2fc5ee48d8661b337a1e98a7249/docs/bot/settings-usage-hub-navigation/README.md).
- Initial implementation: [44984653b9f32cf66b3c359556e0893b2cebedb1](https://github.com/ishizakahiroshi/many-ai-cli/commit/44984653b9f32cf66b3c359556e0893b2cebedb1), tree `716e770365985e9c547001acb9352efeb9222c95`, parent `8b90f475c56df30f47709fe538b8938c35f0187f`.
- Earlier Dots/Codex source reviews reported no required findings on that implementation. Those reviews missed the General navigation contract and are superseded for current acceptance. The earlier CSS-specificity concern was a selector-reading error; the explicit two-ID wrapping rule is retained as responsive hardening.
- Earlier source evidence included 11/11 focused tests, cached TypeScript success, and the guards above. That earlier focused test expected a General navigation button and therefore did not prove the adopted contract.
- CI previously recorded at head `25a91db2b6189104fac0092e4fbd761e42809614`: Validate [run #383](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/38436223040) failed only in `govulncheck`; secret-scan [run #531](https://github.com/ishizakahiroshi/many-ai-cli/actions/runs/38436223029) succeeded. All three platform Go test jobs, web-check, instrumentation, staticcheck, gosec, third-party, web-package-manager-consistency, and go-mod-tidy succeeded; `release-token-scopes` was skipped.
- That `govulncheck` log reported 10 Go standard-library vulnerabilities, 1 imported-package vulnerability, and 3 module vulnerabilities not reached by the code, with fixes in Go `1.26.9` and `golang.org/x/net@v0.60.0`. This repair changes no Go source, module versions, or CI configuration. New-head CI remains separate evidence and must be recorded after publication.
