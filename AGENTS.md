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

<!-- OPENWIKI:START -->

## OpenWiki

This repository has a generated `openwiki/` evidence index. It is optional just-in-time context, not required startup reading.

- Treat source code and tests as authoritative. A brief's unknowns and review items are verification gaps, not automatic requirements.
- Prefer the narrowest quiet validation that proves the changed behavior. Preserve complete failure output.

The scheduled OpenWiki GitHub Actions workflow refreshes the repository wiki. Do not hand-edit generated OpenWiki pages unless explicitly asked; prefer updating source code/docs and letting OpenWiki regenerate.

<!-- OPENWIKI:END -->

## ファイル索引・テーブル逆引き（探す前に読む）

**どのファイルが何をして、どのテーブルを読み書きするかを聞かれたら、grep で探し回る前に
`.omitnix/index.json` を読む。** 全ファイルの索引とテーブルからの逆引きが入っている。

- **解析できなかったファイルも名前と理由付きで載っている。** 「索引に無い」と「読めなかった」を
  取り違えない。参照 0 件は「未使用」ではない
- `generated.commit` が現在の HEAD と違えば、索引はその commit 時点のもの。
  **古いまま断定せず、古いことを添えて答えるか `omitnix` で作り直す**
