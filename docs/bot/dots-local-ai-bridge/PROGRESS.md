# #5 many-ai-cli dots-bridge-c1-20261003

> 最終更新: 2026-10-03(土) 13:31:45 JST

## 現在地

- 受付番号：#5。C1の調査・比較HTML・順位付けを受付し、固定版 INSTRUCTIONS.md / REVIEW.md を読了。
- 段階：開始。公式資料と基準ソースの調査中。C2接続実験・構築は未着手。
- ソース基準：`21d0bc7935a2c4696fb89ccff2e324157a528c2d`。
- 指示commit・作業起点：`c2822a8f39a18809bca8fe9484f1c84bd387f64b`。
- 専用文書branch：`docs/dots-bridge-c1-comparison-20261003`。指示branch `docs/dots-local-ai-bridge-c1-20261003` は変更しない。
- 参考：指示branchの `59300409408c24d6040445d77733da7c5b55d2e8` はSlack送信状態の更新のみ。固定指示・レビュー差分なしを確認。起点は変更しない。
- 成果物SHA／draft PR／独立レビュー対象SHA：未取得。
- 次の一手：方式比較と既存Hub能力を整理し、HTMLを作成する。

## 開始時の能力確認

| 項目 | 確認結果 | 証拠の種類 |
|---|---|---|
| 指示・基準ソース読取 | GitHub経由の固定版取得、独立した作業用cloneに成功 | 読取実施 |
| 公式資料調査 | Slack/GitHub/MCP/A2Aの公式ページを読取可能 | 読取実施 |
| HTML表示用ブラウザ | クラウド側のChromiumとブラウザ接続を確認。成果物の表示QAは未実施 | 環境確認のみ |
| 本人PC・Hub・AI | 読取や起動を行っていない。実機接続、権限、cwd、再開は未確認 | ローカル確認待ち |
| dotsと別Botの疎通 | 本人投稿経由の成功と区別。C1では送信実験しない | 未確認 |

## 証跡と未実施

- C1の許可文書だけを変更し、製品ビルド・Hub起動・接続試験は行わない。
- HTMLの実表示、狭い画面幅、リンク、独立レビュー、最終diff確認は後続のC1工程。
- 外部APIの仕様があることと、dotsが自動的に起動・再開することを分ける。
- ボード更新commitはこのファイルのGitHub履歴に記録。成果物・レビュー対象SHAは別項目へ記す。

## リンク

- [固定Instructions](https://github.com/ishizakahiroshi/many-ai-cli/blob/c2822a8f39a18809bca8fe9484f1c84bd387f64b/docs/bot/dots-local-ai-bridge/INSTRUCTIONS.md)
- [固定Review](https://github.com/ishizakahiroshi/many-ai-cli/blob/c2822a8f39a18809bca8fe9484f1c84bd387f64b/docs/bot/dots-local-ai-bridge/REVIEW.md)
- [Index](../README.md)
