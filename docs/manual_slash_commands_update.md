# any-ai-cli スラッシュコマンド一覧の更新手順

> 最終更新: 2026-05-29(金) 09:04:04 — 初版作成（claude/codex 一覧の最新化作業を手順化）

Claude Code / Codex CLI に新しいスラッシュコマンドが追加された際、ダッシュボードのスラッシュコマンドピッカーに反映させるための運用メモ。

## 仕組み（前提）

- 一覧の正本は `resources/slash-commands/claude.md` と `resources/slash-commands/codex.md`（markdown テーブル）
- Hub は実行時にこのファイルを **GitHub の raw URL 経由で取得・パース**する。デフォルト取得元は `internal/config/config.go` の以下:
  - `DefaultClaudeSlashCmdSource = https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/claude.md`
  - `DefaultCodexSlashCmdSource = https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/codex.md`
- **リビルド不要・トークン消費 0** で更新できる（md を更新して `main` に push するだけ）。バイナリには影響しない。
- 取得結果は provider ごとに **24h キャッシュ**（`slashCmdCacheTTL`）。

## 手順

### C1. 権威ある現行コマンド一覧を調べる

- **Claude Code**: claude-code-guide エージェントに「組み込みスラッシュコマンドを網羅列挙」と依頼するのが確実（記憶ベースで書かない）。
- **Codex CLI**: 公式ドキュメント [Slash commands in Codex CLI](https://developers.openai.com/codex/cli/slash-commands) を正本とする。
- 削除済みコマンドは除外する（例: Claude の `/vim` `/pr-comments` は削除済み）。

### C2. md ファイルを更新する

各行は `| \`/cmd\` | 目的（1文） | いつ使うか（1文） |` のテーブル形式。並び順は **コマンド名の ABC 順**（claude/codex 統一）。

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
git add resources/slash-commands/claude.md resources/slash-commands/codex.md
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
https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/claude.md
https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/codex.md
```

### C6. ダッシュボードで再取得する

Hub は 24h キャッシュのため、即時反映には手動で強制再取得する。

- **設定画面ではない**。設定画面の「スラッシュコマンドソース」欄は URL を変更したときだけキャッシュを無効化する作りなので、URL が同じなら保存しても再取得されない。
- 正しい操作: プロンプト入力欄下のクイックボタン列の **`/ ▾`** を押してピッカーを開き、右上の **`⟳`（再取得）ボタン**を押す。これが `POST /api/slash-commands?provider=...` を投げてキャッシュを無視し強制再取得する。
- ピッカーは **アクティブなセッションの provider**（claude / codex）に応じて対象が切り替わる。両方確認するならそれぞれの provider のセッションでピッカーを開いて `⟳` する。
- ピッカー上部の時刻表示が「たった今」になり、新コマンドが一覧に出れば成功。

## 関連

- パーサ実装: `internal/hub/slash_cmd_fetch.go`
- API ハンドラ: `internal/hub/slash_handlers.go`(`handleSlashCommands` が GET=キャッシュ / POST=強制再取得)
- 取得元 URL 定義: `internal/config/config.go`
- ピッカー UI: `web/src/app.js`(スラッシュコマンドピッカー節) / `web/src/index.html`(`slash-picker-refresh`)
- リリース手順: [manual_release.md](manual_release.md)（runtime-served resources の差分確認手順あり）
