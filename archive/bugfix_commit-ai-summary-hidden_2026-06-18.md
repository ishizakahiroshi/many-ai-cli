# [完了] 障害対応記録: Commit all の AI 導線を非表示にする

> 最終更新: 2026-06-18(木) 08:38:02

## 症状

Git タブの Commit all モーダルにある `Ask AI` 導線が、遅い・反映が分かりにくい・待ち時間が長い、という理由で実運用に向いていなかった。

利用者の意図としては「機能を削除したい」というより、「画面上の入口を隠して、必要なら後で戻せるようにしたい」というものだった。

## 根本原因

Commit all モーダルには、2系統のメッセージ生成導線が並んでいた。

- `Generate`: ローカルの簡易生成
- `Ask AI`: 接続中の AI セッションへ diff を読ませて生成

このうち `Ask AI` は、Hub 側の応答待ちと WS イベント受信を前提にしており、応答が遅い場合に UI だけが待機状態に見えやすかった。特に Commit メッセージのような短文用途では、体感が悪く、成功可否の判断もしづらかった。

## 修正内容

`Ask AI` ボタンを UI から非表示にした。処理本体は残してあり、後から再表示しやすい。

具体的には `web/src/app/git-view.ts` の Commit all モーダル HTML から `data-commit-ai` ボタンを `hidden` 化した。

## 変更ファイル

- `web/src/app/git-view.ts`
  - Commit all モーダルの `Ask AI` ボタンを `hidden` に変更

## 検証

- `bun run check` を `web` 配下で実行し、TypeScript チェックが通ることを確認
- 既存の `Generate` / `Review` / `Commit` の流れはそのまま維持

## 備忘

- 削除ではなく非表示にしたので、必要なら将来また戻せる
- もし `Ask AI` 自体を完全に廃止するなら、UI だけでなく `internal/hub/git_commit_ai.go` / WS イベント / i18n もまとめて整理する必要がある
