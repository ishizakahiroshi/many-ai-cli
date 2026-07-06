# many-ai-cli スラッシュコマンド一覧の更新手順

> 最終更新: 2026-06-19(金) 08:06:57 — 半自動の `slash-commands-update` スキル連携と freshness レポート運用を追記

Claude Code / Codex CLI / GitHub Copilot CLI / Cursor Agent CLI に新しいスラッシュコマンドが追加された際、ダッシュボードのスラッシュコマンドピッカーに反映させるための運用メモ。

## 推奨: `slash-commands-update` スキルで半自動化する

手作業で本家を追う前に、`C:\dev\workshop\skills\slash-commands-update` スキルを使うのが基本。本家との差分検出・md 形式への正規化案・人間確認用レポート作成を半自動化する（採否は人間が差分だけ見て決める）。

- 起動: 「スラッシュコマンド鮮度確認」「slash-commands-update」等。
- モード:
  - `report`（既定）: 全 provider の差分を検出し `docs/local/slash-command-freshness_YYYY-MM-DD.md` を作る。`resources/slash-commands/` の `*.md` から provider を動的検出するので、`opencode.md` など新 provider も自動で対象に入る。
  - `apply`: レポートで `decision = accepted` にした差分だけを `resources/slash-commands/*.md` へ反映する（`pending` / `unknown` / `deferred` は触らない）。
  - `preflight`: release 前のゲート判定（後述の release 連携）。
- スクリプト:
  - `scripts/freshness-report.ps1`: ローカルインベントリ抽出＋レポート雛形生成。実機 `copilot help commands` は自動 diff まで行う。docs ベース（codex / cursor-agent）と claude / opencode は誤検出を避けるため `unknown` とし、AI が `report` モードで WebFetch / claude-code-guide / 実機採取により確認する。**commit / push は一切しない**。
  - `scripts/freshness-preflight.ps1`: 最新レポートの鮮度（既定 7 日）と未判断差分（`pending`）を判定し、exit code でゲート結果を返す（0=可 / 2=要対応 / 3=stale・要 report）。
- 契約の正本: スキルの `references/provider-sources.md`（provider 別の source-of-truth・md 形式制約・レポート形式）。

下記 C1〜C6 は、スキルが内部で踏む手順の詳細（手で回す場合の正本でもある）。

## 仕組み（前提）

## 仕組み（前提）

- 一覧の正本は `resources/slash-commands/claude.md` / `resources/slash-commands/codex.md` / `resources/slash-commands/copilot.md` / `resources/slash-commands/cursor-agent.md`（markdown テーブル）
- Hub は実行時にこのファイルを **GitHub の raw URL 経由で取得・パース**する。デフォルト取得元は `internal/config/config.go` の以下:
  - `DefaultClaudeSlashCmdSource = https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/claude.md`
  - `DefaultCodexSlashCmdSource = https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/codex.md`
  - `DefaultCopilotSlashCmdSource = https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/copilot.md`
  - `DefaultCursorAgentSlashCmdSource = https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/cursor-agent.md`
- **リビルド不要・トークン消費 0** で更新できる（md を更新して `main` に push するだけ）。バイナリには影響しない。
- 取得結果は provider ごとに **24h キャッシュ**（`slashCmdCacheTTL`）。

## 手順

### C1. 権威ある現行コマンド一覧を調べる

- **Claude Code**: claude-code-guide エージェントに「組み込みスラッシュコマンドを網羅列挙」と依頼するのが確実（記憶ベースで書かない）。
- **Codex CLI**: 公式ドキュメント [Slash commands in Codex CLI](https://developers.openai.com/codex/cli/slash-commands) を正本とする。
- **GitHub Copilot CLI**: `copilot help commands` の実機出力を正本とする。公式ドキュメントと差分があれば実機出力を優先する。
- **Cursor Agent CLI**: 実機 `cursor-agent` の `/help` を正本とする。実機が無ければ公式ドキュメント [Slash commands | Cursor Docs](https://cursor.com/docs/cli/reference/slash-commands) を使い、差分があれば実機出力を優先する。
- 削除済みコマンドは除外する（例: Claude の `/vim` `/pr-comments` は削除済み）。

### C2. md ファイルを更新する

各行は `| \`/cmd\` | 目的（1文） | いつ使うか（1文） |` のテーブル形式。並び順は **コマンド名の ABC 順**（provider 間で統一）。

**パーサ仕様による必須ルール**（`internal/hub/slash_cmd_fetch.go` の `cleanDescMarkdown` / `tableRowRe`）:

- **コマンド列は引数なしのコマンド名のみ**書く。`/effort [level|auto]` のように `|` を含む引数を入れるとテーブル行パースが壊れる。
- **説明文に `( )` を入れない**。`cleanDescMarkdown` が丸括弧・角括弧の中身を除去するため、括弧内に書いた情報はダッシュボード表示時に消える。
  - 例: レベル一覧は `Set the model effort level: low, medium, high, ...` のようにコロン区切りで書く（括弧で囲まない）。
- 説明文に `|`（パイプ）を入れない（列区切りと衝突する）。
- markdown 装飾（`**bold**` / `[link](url)` / `` `code` ``）は自動で剥がされるので、残ってもよいが避けたほうが無難。

### C3. ローカル検証（任意・推奨）

実ファイルをパーサに通して、新コマンドが取得できるか確認できる。`internal/hub/` に一時テストを置いて `fetchAndParseSlashCmds` を実ファイルパスで呼ぶ:

```powershell
go test ./internal/hub/ -run <一時テスト名> -v
```

- パース件数とファイルの行数（`| \`/` で始まる行数）が一致すればパース漏れなし。
- `/effort` などの説明文に括弧由来の欠落がないか目視する。
- 確認後、一時テストファイルは削除する（コミットしない）。

### C4. コミットして main に反映する

配信元は `main` の raw URL なので、**develop でコミットしただけでは反映されない**。`main` へマージして push する必要がある。

```powershell
# develop でコミット
git add resources/slash-commands/claude.md resources/slash-commands/codex.md resources/slash-commands/copilot.md resources/slash-commands/cursor-agent.md
git commit -m "feat: スラッシュコマンド一覧を最新化"

# main へマージして push
git checkout main
git merge develop --no-edit
git push origin main
git checkout develop
```

> コミットメッセージに複数行 here-string を使う場合、PowerShell では `@'...'@`、Bash ツールでは別構文。Bash ツールに `@'...'@` を渡すと `@` がメッセージに混入するので注意（混入したら `git commit --amend -F <file>` で修正）。

### C5. 配信内容を確認する

push 後、raw URL に反映されたか確認する:

```
https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/claude.md
https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/codex.md
https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/copilot.md
https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/cursor-agent.md
```

### C6. ダッシュボードで再取得する

Hub は 24h キャッシュのため、即時反映には手動で強制再取得する。

- **設定画面ではない**。設定画面の「スラッシュコマンドソース」欄は URL を変更したときだけキャッシュを無効化する作りなので、URL が同じなら保存しても再取得されない。
- 正しい操作: プロンプト入力欄下のクイックボタン列の **`/ ▾`** を押してピッカーを開き、右上の **`⟳`（再取得）ボタン**を押す。これが `POST /api/slash-commands?provider=...` を投げてキャッシュを無視し強制再取得する。
- ピッカーは **アクティブなセッションの provider**（claude / codex / copilot / cursor-agent）に応じて対象が切り替わる。それぞれの provider のセッションでピッカーを開いて `⟳` する。
- ピッカー上部の時刻表示が「たった今」になり、新コマンドが一覧に出れば成功。

## 関連

- 半自動化スキル: `C:\dev\workshop\skills\slash-commands-update\SKILL.md`（契約: `references/provider-sources.md`）
- パーサ実装: `internal/hub/slash_cmd_fetch.go`
- API ハンドラ: `internal/hub/slash_handlers.go`(`handleSlashCommands` が GET=キャッシュ / POST=強制再取得)
- 取得元 URL 定義: `internal/config/config.go`
- ピッカー UI: `web/src/app.js`(スラッシュコマンドピッカー節) / `web/src/index.html`(`slash-picker-refresh`)
- リリース手順: [manual_release.md](manual_release.md)（runtime-served resources の差分確認手順あり）
