# Grok Approval Patterns

> many-ai-cli の Hub UI が Grok Build CLI の承認 / permission UI を検出するためのトリガー文言。
> 1 行 1 パターン。バッククオートで囲んだ部分のみをパースする。
> Grok Build は Claude Code 互換 harness（CLAUDE.md / `--permission-mode`）のため、暫定で Claude Code 系 + 汎用の文言を採用している。実機 TUI の承認プロンプトで確定する。

- `do you want to`
- `esc to cancel`
- `press enter to confirm`
- `allow this command`
- `permission required`
- `approve?`
- `proceed?`
