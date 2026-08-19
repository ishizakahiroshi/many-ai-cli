# `resources/` — 実行時配信リソース（リリース不要で更新できるもの）

> 最終更新: 2026-08-19(水) — `CLAUDE.md` から本文を移設（常時ロード分を索引だけにする再編）

このディレクトリの 4 つは **バイナリに焼き込まれず、`main` の raw URL から実行時に fetch される**。URL 定数は `internal/config/config.go` の `Default*Source`。

| ディレクトリ | 中身 | 消費側 |
|---|---|---|
| `approval-patterns/` | 承認 trigger phrase | `internal/hub/` の承認検出 |
| `models/` | spawn パネルのモデル候補（`defaults.json`） | `internal/hub/models_fetch.go` |
| `slash-commands/` | スラッシュコマンドピッカー | `internal/hub/slash_cmd_fetch.go` |
| `usage-links/` | 利用状況リンク | Hub UI |

## 帰結（毎回調べ直さない）

- 内容を直すのに **リビルド・タグ・リリースは一切不要**。`main` へ push した時点で全ユーザーへ反映される
- 逆に **`develop` に置いても誰にも届かない**。配信元は `main` だけ
- 全ユーザーへ即時 live 配信されるため、**誤りもそのまま公開される**。自動反映せず採否は人間が決める
- 形式制約を破るとパーサが壊れる。`slash-commands/` は `scripts/check-slash-commands.mjs` が CI で検査する（2026-08-11、エイリアスを 1 行にまとめた 6 行がパーサに 1 件もマッチせず、6 コマンドがピッカーに一度も出ていなかったのを発見）

## 鮮度チェックがあるのは `slash-commands` だけ

`.claude/skills/slash-commands-update` の report / apply / preflight。release の前提チェックから preflight が呼ばれる。

**`models` / `usage-links` / `approval-patterns` には無く、陳腐化しても誰も気づかない。** 2026-08-11、モデル一覧が Claude 5 世代を丸ごと欠いたまま放置されていたのを発見した。provider を増やすとこの追従先が 1 本増える（`CLAUDE.md` の見送り台帳 D-02 が挙げている本当のコスト）。

## 本家との差分を根拠に `slash-commands/*.md` から削除しない（2026-08-15 制定）

鮮度チェックは本家の一覧と突き合わせて「追加候補」と「削除候補」を出すが、**削除候補は原則そのまま採用しない**。

- **公式 docs は built-in コマンドの完全一覧ではない。** 2026-08-15 の照合で、codex の docs に `/model` `/status` `/usage` `/resume` `/logout` `/mcp` が載っていなかった。docs との差分をそのまま削除すると、実在する基本コマンドがピッカーから消える
- **`/orchestrate` は many-ai-cli 自身が提供する Hub の機能**で、本家 CLI には無い（`claude` / `codex` / `copilot` / `cursor-agent` の 4 本に同一行）。**本家と比べる限り毎回「削除候補」として出続けるのが正常**。`freshness-report.ps1` の `$script:HouseCommands` で除外してある
- **差分は追加候補から先に数える。** 「本家にあって md に無い」が 0 件なら、逆向きが何件でも利用者への実害は無い（ピッカーから欠けているものが無い）。件数の多い削除候補側から見ると判断を誤る
- 削除してよいのは **実機の `/help` で「無い」ことを確かめたとき**だけ

## 参照

- 設計書: [`../docs/v0.3.x-many-ai-cli-design.md`](../docs/v0.3.x-many-ai-cli-design.md) の承認パターン / モデル / スラッシュコマンド各節
- 手順書: [`../docs/manual_slash_commands_update.md`](../docs/manual_slash_commands_update.md)
