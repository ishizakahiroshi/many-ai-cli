# Agent Entry Point (ai-cli-hub)

This repository's operational guidance is maintained in `CLAUDE.md`.

- Global rules (all AIs): `~/.claude/CLAUDE.md` — load this first (actual path: see `./AGENTS.local.md`)
- Project overview & task index: `./CLAUDE.md`
- Detailed guides (read on demand):
  - `./CLAUDE/coding.md` — Go / Vue 3 conventions, PTY, detector
  - `./CLAUDE/development.md` — plan_*.md context split, AI work-model
  - `./CLAUDE/operations.md` — Git, commit messages, output rules
  - `./CLAUDE/deployment.md` — cross-compile build & distribution
  - `./CLAUDE/windows_setup.md` — Windows dev environment specifics
- Design (source of truth): `./docs/ai-cli-hub-design-v0.1.0.md`
- Local/private additions (if present): `./CLAUDE.local.md`
- Codex-specific SSH/local notes: `./AGENTS.local.md`

If any guidance conflicts, follow `CLAUDE.md`.
