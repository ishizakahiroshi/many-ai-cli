# Gemini Entry Point (many-ai-cli)

> 最終更新: 2026-05-07(木) 19:24:03

このリポジトリの運用ガイドは `CLAUDE.md` にある。

- Project overview & task index: `./CLAUDE.md`
- Detailed guides (read on demand):
  - `./CLAUDE/coding.md` — Go / Vue 3 conventions, PTY, detector
  - `./CLAUDE/development.md` — plan_*.md context split, AI work-model
  - `./CLAUDE/operations.md` — Git, commit messages, output rules
  - `./CLAUDE/deployment.md` — cross-compile build & distribution
  - `./CLAUDE/windows_setup.md` — Windows dev environment specifics
- Design (source of truth): `./docs/v0.3.x-many-ai-cli-design.md`
- Local/private additions (if present): `./CLAUDE.local.md`

個人グローバルルールは各 AI ツールのグローバル設定に置く。公開リポジトリ内のガイドが衝突したときは `CLAUDE.md` を優先。

## Gemini 特記

- **本リポジトリでは Gemini CLI を wrap 対象外とした**（2026-05-06 決定 / ToS グレーゾーンのため）。
- 詳細: [docs/provider_tos_review.md](docs/provider_tos_review.md)
- 本リポジトリ自体は `many-ai-cli`（Claude Code / Codex 向け承認ハブ）の開発リポジトリであり、開発補助に Gemini CLI を使う場合の手引きとして本ファイルを残置している。
