# Progress board
> 最終更新: 2026-10-13(火) 23:07:27 +09:00

## Task

- Task: #P-20261013-001 many-ai-cli
- Issue URL: https://github.com/ishizakahiroshi/many-ai-cli/issues/20
- Instruction repo/path: ishizakahiroshi/many-ai-cli / docs/bot/settings-usage-hub-navigation/
- Implementation branch: dots/P-20261013-001-settings-usage-hub-nav
- Planned PR base: wt/rust-ui-publication-20261011
- Product base at instruction creation: d95effb70cf550ff93ca593160ee8aa827abc28b
- Parent PR: https://github.com/ishizakahiroshi/many-ai-cli/pull/18

## Evidence pointers

| Evidence | Value |
|---|---|
| Initial instruction commit | 4d744dfbf5cea15eef16c3351694690d352f910c |
| Final fixed instruction URL | https://github.com/ishizakahiroshi/many-ai-cli/blob/9db24622dad1c2fc5ee48d8661b337a1e98a7249/docs/bot/settings-usage-hub-navigation/README.md |
| Latest PROGRESS URL | https://github.com/ishizakahiroshi/many-ai-cli/blob/dots/P-20261013-001-settings-usage-hub-nav/docs/bot/settings-usage-hub-navigation/PROGRESS.md |
| Product base SHA | d95effb70cf550ff93ca593160ee8aa827abc28b |
| Slack send/read-back | 2026-10-13; sent via verified configured DM; fixed README SHA and adopted behavior confirmed on read-back |
| Receipt | Issue #20 response reports fixed instruction, parent PR, and task branch SHAs; environment/check details are self-reported and not independently verified |
| Implementation code SHA | This implementation commit; exact SHA is recorded in the next Issue #20 comment |
| dots independent review SHA | not submitted |
| Codex review SHA | pending |
| CI SHA and runs | pending |
| Branch commit containing latest board | record from branch history after commit |

## Work state

| Work | Owner | Issue status | Evidence and next action |
|---|---|---|---|
| Accepted design and task instruction | Codex | sent; read back; receipt received | Issue #20 contains the commit-pinned README and current board URL; receipt recorded in Issue #20 |
| Implementation | dots | initial implementation submitted for independent review | One footer entry, default General view, embedded Usage/Hub, preserved actions and panel lifecycle. Parent receipt-board update 8b90f475 is retained. See validation below; no screen acceptance claimed |
| Independent implementation review | dots reviewer | not started | Reviewer must be separate from the implementation author |
| Adversarial source review and canonical-source reflection | Codex | not started | Review the exact submitted SHA; retain unrelated dirty work |
| Windows build, Hub reload, and actual screen acceptance | user | pending | Record after the user checks the Settings initial view and both navigation destinations |

Issue comments are the source of truth for send, receipt, implementation, review, CI, and acceptance state. This board is a navigation aid. Keep send success, receipt, implementation start, submitted code, review, and CI as separate states.

## Initial implementation validation

Source snapshot: task head `8b90f475c56df30f47709fe538b8938c35f0187f` (product code unchanged from `d95effb70cf550ff93ca593160ee8aa827abc28b`) plus this implementation. Exact submitted code SHA and review handoff are recorded separately in Issue #20.

- `cd web && bun run check`: exit 127, Bun unavailable. Required Bun check remains **not run**.
- `cd web && node node_modules/typescript/bin/tsc --noEmit`: exit 0 using existing TypeScript 5.9.3 cache. This is alternate source evidence, not the pinned TypeScript 6.0.3/Bun result.
- `node --experimental-transform-types --test web/tests/settings-usage-hub.test.ts`: exit 0, 10/10 synthetic/source tests. Node 24.19.0, cached TypeScript 5.9.3. Covers reopened General/search reset, navigation/deep links, mobile Back/Escape, Save/Reset, Usage polling/late responses, exposure help/state, retained links and shutdown cancellation/branches.
- `node scripts/check-ui-components.mjs`: exit 0, 39 CSS / 152 code files, no syntax errors/new appearance declarations/stale exceptions; cached esbuild 0.21.5 and TypeScript 5.9.3.
- `node --test scripts/check-ui-components.test.mjs`: exit 0, 14/14 guard tests, using the same cached parsers.
- `node scripts/check-web-module-init.mjs`: exit 0, 182 modules, no TDZ initialization failure; cached parser.
- `node scripts/check-wheel-overlays.mjs`: exit 0, 33 overlays / 1 exemption.
- `node scripts/check-text-hygiene.mjs`: exit 0.
- `node scripts/check-instrumentation.mjs`: exit 0, existing 17 entries / 2 active.
- `git diff --check`: exit 0.

No dependency or lockfile changes. The deleted CSS rules' 12 obsolete exact exceptions were removed; no exceptions were added. README English/Japanese navigation instructions and CHANGELOG Unreleased were updated; the dated Vietnamese README snapshot is unchanged, and existing Japanese/English/Vietnamese UI labels are reused.

Independent code review and exact-SHA CI are pending. No Hub launch, product build/reload, browser/device screen inspection, release, merge or deployment has been performed. Windows source reflection and actual screen acceptance remain with Codex/user as assigned above.
