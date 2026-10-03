# #5 many-ai-cli dots-bridge-c1-20261003

> 最終更新: 2026-10-03(土) 13:48:25 JST

## 現在地

- 受付番号：#5。C1の調査・比較HTML・順位付けを受付し、固定版 INSTRUCTIONS.md / REVIEW.md を読了。
- 段階：比較文書の初稿完成、静的検査と独立レビュー準備。C2接続実験・構築は未着手。
- ソース基準：`21d0bc7935a2c4696fb89ccff2e324157a528c2d`。
- 指示commit・作業起点：`c2822a8f39a18809bca8fe9484f1c84bd387f64b`。
- 専用文書branch：`docs/dots-bridge-c1-comparison-20261003`。指示branch `docs/dots-local-ai-bridge-c1-20261003` は変更しない。
- 参考：指示branchの `59300409408c24d6040445d77733da7c5b55d2e8` はSlack送信状態の更新のみ。固定指示・レビュー差分なしを確認。起点は変更しない。
- 成果物SHA／draft PR／独立レビュー対象SHA：未取得。
- 次の一手：初稿をcommit/pushし、固定SHAで独立レビューを依頼する。

## 開始時の能力確認

| 項目 | 確認結果 | 証拠の種類 |
|---|---|---|
| 指示・基準ソース読取 | GitHub経由の固定版取得、独立した作業用cloneに成功 | 読取実施 |
| 公式資料調査 | Slack/GitHub/MCP/A2Aの公式ページを読取可能 | 読取実施 |
| HTML表示用ブラウザ | クラウド側のブラウザ接続は存在するが、file URLはポリシー拒否、loopback HTTPはERR_BLOCKED_BY_CLIENT。表示QA未実施 | 表示経路の失敗を確認 |
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

## 初稿の到達点

- 第一候補：専用MCP tools＋MCP Events＋永続キュー。OpenAI公開仕様にdots対応を確認。ただし登録・購読・本環境の往復は未確認。
- 順位：1 MCP Events、2 Slack Socket、3 Slack HTTP＋中継、4 GitHub Webhook、5 GitHub poll、6 Slack poll、7 A2A。PC停止耐性優先ならSlack内の2/3を入れ替える。
- 初稿：comparison.html と FINDINGS.md。方式ごとに往路・復路・AI起動・保存・権限・費用・切断条件とC2順位順の試行計画を記載。
- 起点c2822a8と基準21d0bc7は文書・索引以外同一。ソースは基準SHAで参照。Rust版は未評価。
- 受付ボード公開commit：`adc730ba8926ad29756e40b3ba685f046b1ed7ae`。公開後、GitHubから読戻し確認。
- gitの直接pushは認証情報が無い環境のため失敗。ローカルhookを通した同内容をGitHubコネクタで公開し、ツリー一致と読戻しを確認する。hookは無効化しない。
- omitnix 0.1.3の解析依存を作業用ツール領域へ導入。製品依存は未変更。初回の不足解析環境は修正し、既存coverageと同じ鮮度check合格を確認。
- 既存の解析警告（13 statements、TS unknown 1件等）は基準にも存在。製品コードの修正は行わない。
- 静的HTML確認だけでは図・表の実表示やスマホ幅を保証しない。一時的な静的文書previewは停止済み。Hub起動なし。

### 初稿リンク

- [Comparison HTML](comparison.html)
- [Findings](FINDINGS.md)

## 初稿の静的検査

- HTML parser検査：7方式、往路/復路14図、リンク85件、重複IDなし、内部anchor・ローカル相対リンクの欠損なし、外部asset読込0。viewport・600px以下responsive CSSあり。これは実画面QAの代替合格ではない。
- omitnix鮮度検査が文書2件追加を検知したため0.1.3で生成。coverageは1131→1133、analyzed 921→922、unclaimed 204→205。既存fileレコードは変化なし。生成環境差としてRust adapterのcapabilities metadataがsummaryのみへ変化（基準にRustソースなし）。手編集はしていない。
- secrets-scanは今回の公開文書4件の構造検査合格。非公開watchlistは未設定で未実施、本人の秘密を取得して補完しない。承認ルール混入検査、instrumentation検査合格。
- GitHub上の成果物リンク読戻し・独立レビューは初稿公開後に実施。
