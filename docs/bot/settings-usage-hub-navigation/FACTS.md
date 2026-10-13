# Facts and decision record
> 最終更新: 2026-10-13(火) 22:20:28

## User decision

On 2026-10-13 the user adopted this arrangement:

- One persistent lower-footer Settings entry.
- Opening Settings first shows the current general Settings content.
- The Settings navigation adds Usage and Hub as two destinations.
- The general Settings content is the default view. There is no duplicate Settings destination in the navigation.

Usage and Hub remain available. Their current menu contents move into Settings navigation.

## Source facts at the task baseline

Observed at PR #18 head d95effb70cf550ff93ca593160ee8aa827abc28b on 2026-10-13:

- PR #18 is the open Draft Rust UI component update: https://github.com/ishizakahiroshi/many-ai-cli/pull/18
- web/src/index.html places Settings, Usage, and Hub menu controls under session-footer. The Usage popup is usage-dropdown. The Hub popup is web-menu-dropdown.
- web/src/app/settings.ts builds the Settings navigation from existing settings sections and initializes the Usage dropdown.
- web/src/app/usage-panel.ts owns Usage popup behavior.
- web/src/app/web-menu.ts owns Hub menu open, close, and keyboard behavior.
- web/src/app/host-expose.ts updates Hub exposure state and refers to the Hub menu control.
- Settings navigation currently persists the last selected settings section. The adopted default view requires entering Settings at the existing general Settings content.

The task branch starts at the PR #18 head above. If the parent branch advances, the implementer records the actual new parent SHA in the Issue and PROGRESS before proceeding.

## Compatibility facts

- The Usage view has contract remaining amounts, provider links, unavailable-provider explanations, and local-run links.
- The Hub view has external access, mobile QR connection, Hub shutdown, and bug reporting actions.
- The main session token display is separate from these Usage destinations and remains where it is.
- Existing Settings Back, Close, Save, and Reset controls retain their current behavior. Save handling is not redesigned here.

## Public references

- User-approved proposal decision: captured in the task statement and Issue body.
- Source behavior: the repository files and PR #18 commit above.
- This task instruction was prepared for public storage on 2026-10-13 and contains only public product behavior and implementation guidance.
