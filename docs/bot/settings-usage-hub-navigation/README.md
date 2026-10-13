# #P-20261013-001 many-ai-cli: Usage and Hub inside Settings
> 最終更新: 2026-10-13(火) 22:20:28

## Task

- Task ID: #P-20261013-001
- Product repository: ishizakahiroshi/many-ai-cli
- GitHub Issue: created after the initial task-instruction commit; URL is added in the next instruction commit
- Instruction directory: docs/bot/settings-usage-hub-navigation/
- Implementation branch: dots/P-20261013-001-settings-usage-hub-nav
- Initial product base: wt/rust-ui-publication-20261011 at d95effb70cf550ff93ca593160ee8aa827abc28b
- Parent Draft PR: [PR #18](https://github.com/ishizakahiroshi/many-ai-cli/pull/18)
- Planned Draft PR base: wt/rust-ui-publication-20261011
- Initial instruction commit SHA: recorded in PROGRESS.md after publication

## Adopted behavior

The user selected the “設定画面＋UsageHub” proposal on 2026-10-13.

Keep one persistent Settings entry in the lower session footer. Opening it shows the existing general settings content first. The Settings navigation then offers two destinations: Usage and Hub. The existing general settings content is the default view; it does not become another Settings navigation destination.

Move the existing Usage and Hub destinations into that navigation while keeping their content and actions available. The lower footer retains the Settings entry only. Usage and Hub should open within the Settings screen, not as separate footer menus or independent overlays.

## Implementation scope

1. Keep the existing Settings button and its open/close behavior.
2. Remove the persistent Usage and Hub footer menu buttons after moving their current contents into Settings navigation.
3. Add labeled Usage and Hub destinations to the Settings navigation. Use the existing Settings content as the first view whenever Settings is opened.
4. Preserve all current Usage items, provider availability information, external links, and explanatory text.
5. Preserve all current Hub menu actions: external access, mobile QR connection, Hub shutdown, and bug report. Keep their existing confirmations and state indicators.
6. Keep selected-session token counts and other session usage displays in their existing places.
7. Keep the current Settings back, close, Save, and Reset behavior. Save behavior is outside this task.
8. Keep visible labels, focus, keyboard operation, and responsive layout. The Usage destination must remain readable at narrow widths and its icon must not clip.
9. Add a CHANGELOG Unreleased line. Update README only if the user-facing instructions actually change.

Inspect the current handlers before moving DOM. The Settings navigation is created from settings sections; Usage and Hub currently have separate handlers and dropdown roots. Reuse those behaviors without duplicating event handlers or losing actions.

## Source baseline

The initial product base is PR #18 head d95effb70cf550ff93ca593160ee8aa827abc28b. The current UI contains these relevant implementation points:

- web/src/index.html: session footer, Settings panel, Usage dropdown, and Hub menu
- web/src/app/settings.ts: Settings navigation and Usage menu initialization
- web/src/app/usage-panel.ts: Usage panel behavior
- web/src/app/web-menu.ts: Hub menu open/close behavior
- web/src/app/host-expose.ts: Hub exposure state and menu button references

At task start, check PR #18 head and the implementation branch. If the parent head has advanced, preserve this instruction commit, merge the new parent head into the task branch, update the recorded product base, and check for conflicts before coding. Stop and report if the merge conflicts or changes the behavior this task depends on.

## Branch and write permissions

Allowed:

- Commit and push task files, implementation, and relevant tests on dots/P-20261013-001-settings-usage-hub-nav.
- Create and update one Draft PR from that branch to wt/rust-ui-publication-20261011.
- Update this task directory and report exact evidence in the task Issue.

Prohibited:

- Push directly to wt/rust-ui-publication-20261011, dots/rust-recovery-resume-3, develop, or main.
- Update parent PR #18.
- Merge any PR, release, deploy, add a dependency, change CI workflows, or use credentials.
- Change provider usage collection, billing, or account data behavior.
- Remove Settings Save, Reset, Back, or Close behavior.

The Draft PR is a child of PR #18. Keep it Draft. Do not retarget it to develop or the Rust recovery branch.

## Checks and completion

- Run the permitted web source check from the web directory: bun run check.
- Add or update focused tests for initial Settings view, Usage/Hub navigation, persistent footer controls, and preserved actions, following existing project test patterns.
- Record exact commands, exit codes, tested SHA, and CI run status. Queued or skipped checks are not passes.
- Do not start the Hub or open/reload the local application. Browser and device acceptance remains a separate local check.
- Have a reviewer other than the implementer inspect the implementation. Record review SHA and findings.
- Update PROGRESS.md at each handoff point and keep Issue comments as the source of truth for status.

Use synthetic data. Do not add screenshots, local absolute paths, private conversation locators, tokens, or credentials to this public repository.

## Reply format

Every reply in the task conversation starts with:

#P-20261013-001 many-ai-cli：<status>

Reply in the same task conversation. Start with receipt including the instruction SHA, current base SHA, branch, and available checks. Then report implementation, independent review, and final status separately.
