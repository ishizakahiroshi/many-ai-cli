# 看板（MCP ボード）への進捗の書き込み

> 最終更新: 2026-11-06 — 初版

many-ai-cli で動かした AI の作業を、MCP で書き込める看板（例: Many Iris の看板）へ自動で載せる機能です。**設定ファイルを置いた人だけが使う機能**で、置かなければ何も起きません。

## 何が載るか

送ってよいと指定したリポジトリの中で始めたセッションごとに、看板へ項目を1つ作ります。

1. セッションが始まったとき: 項目（種類 `task`、状態 `in_progress`、題名は「AI の名前: セッション名」）と、「開始」の出来事
2. AI が作業を1回終えるたび（完了マーカーの要約が届いたとき）: 要約を `note` の出来事として追加
3. セッションが終わったとき: 状態を `done` にし、「終了」の出来事

指定していないリポジトリのセッションは送りません。お試し用の環境（trial）でも送りません。要約は、秘密に見える文字列を伏せてから送ります。

## 設定

many-ai-cli のデータフォルダ（`config.yaml` と同じ場所。既定は `~/.many-ai-cli/`）に `board.json` を置きます。

```json
{
  "schema_version": 1,
  "url": "https://board.example.com/board/mcp",
  "key_file": "C:\\keys\\board-writer.key",
  "repos": [
    { "path": "C:\\work\\my-app", "project": "my-app" },
    { "path": "C:\\work\\worktrees\\my-app", "project": "my-app" }
  ]
}
```

| 項目 | 内容 |
|---|---|
| `url` | 看板の MCP の住所。`https` のみ（`http` は `127.0.0.1` / `localhost` だけ）。`?` 以降や利用者名入りの URL は受け付けません |
| `key_file` | 書き込み用の鍵を1行だけ書いたファイルの絶対パス。送るたびに読み、`Authorization: Bearer` で送ります。鍵そのものを `board.json` や `config.yaml` に書かないでください |
| `repos` | 送ってよいリポジトリ。`path` の中（その下のフォルダを含む）で始めたセッションだけを送ります。`project` は看板に出る案件名で、省略するとフォルダ名になります。worktree を使う場合は、その置き場も足してください |

設定ファイルは書き込みのたびに読み直すので、編集した後に Hub を再起動する必要はありません。設定の誤り、鍵が読めない、看板が書き込みを断った、といった問題は Hub のログに警告として残ります（鍵の値はログに出しません）。

## 看板側の条件

看板は MCP の `tools/call` で次の2つの道具を受け付ける必要があります（Many Iris の看板の口と同じ形です）。

- `board_upsert_item`（`item_id`・`project`・`kind`・`title`・`state`）
- `board_add_event`（`item_id`・`kind` が `session_start` / `note` / `session_end`・`text`）

項目の ID は `session/<many-ai-cli のセッション番号>` です。Hub を再起動した後に同じセッションが続いても、同じ項目へ書き足します。

## 止めるとき

`board.json` を消すか、`repos` から外します。看板に書いたものは、看板側の決まりに従って残ります。
