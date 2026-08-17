# Agent Entry Point (many-ai-cli)

This repository's operational guidance is maintained in `CLAUDE.md`.

- Project overview & task index: `./CLAUDE.md`
- Detailed guides (read on demand):
  - `./CLAUDE/coding.md` — Go / Web TypeScript conventions, PTY, detector
  - `./CLAUDE/development.md` — plan_*.md context split, AI work-model
  - `./CLAUDE/operations.md` — Git, commit messages, output rules
  - `./CLAUDE/deployment.md` — cross-compile build & distribution
  - `./CLAUDE/windows_setup.md` — Windows dev environment specifics
- Design (source of truth): `./docs/v0.3.x-many-ai-cli-design.md`
- Local/private additions (if present, not committed): `./CLAUDE.local.md`
- Tool-specific local notes (if present, not committed): `./AGENTS.local.md`

Personal/global AI rules are intentionally kept outside this repository. Use each AI tool's supported global instruction location for user-specific rules; this file must remain valid for a fresh public clone with no private files.

If any project guidance conflicts, follow `CLAUDE.md`.

## AI 作業共通ルール

- ビルド・コミット禁止、secrets-scan 責務、plan/bugfix/pending md の作成ルール等の AI 作業共通ルールは、各利用者のグローバル AI 設定に従う（作者環境の例: `~/.claude/CLAUDE.md` および `~/.claude/guides/`）