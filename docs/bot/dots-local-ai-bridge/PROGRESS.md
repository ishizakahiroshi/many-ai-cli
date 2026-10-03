# #5 many-ai-cli dots-bridge-c1-20261003

> 最終更新: 2026-10-03(土) 17:51:00 JST

## 現在地

- 受付番号：#5。C1の調査・比較HTML・順位付けを受付し、固定版 INSTRUCTIONS.md / REVIEW.md を読了。
- 段階：公開実践例8件の追加調査・文書反映が完了し、静的確認と独立レビューへ進む。実表示QAは環境制約により未実施。C2接続実験・構築は未着手。
- ソース基準：`21d0bc7935a2c4696fb89ccff2e324157a528c2d`。
- 指示commit・作業起点：`c2822a8f39a18809bca8fe9484f1c84bd387f64b`。
- 専用文書branch：`docs/dots-bridge-c1-comparison-20261003`。指示branch `docs/dots-local-ai-bridge-c1-20261003` は変更しない。
- 参考：指示branchの `59300409408c24d6040445d77733da7c5b55d2e8` はSlack送信状態の更新のみ。固定指示・レビュー差分なしを確認。起点は変更しない。
- 初稿公開SHA：`df1e01f80daf3641361eee0939549c74baa98d97`。draft PR：[#8](https://github.com/ishizakahiroshi/many-ai-cli/pull/8)。独立レビュー対象のローカルSHA：`6f41211f05dbe56578c0b6c41352a89a09c64744`（同一tree `8a35556ec1e6c9ec2edce6ac7d0f3d32854122f2`）。
- 次の一手：公開事例を5〜10件の目安で絞り、PUBLIC_RESEARCH・FINDINGS・HTML・候補別C2手順を更新し、独立レビューへ回す。C1からC2へ自動移行しない。

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
- リンクの静的検査・GitHub読戻し、独立レビュー、最終diff確認は実施済み。HTMLの実表示とスマホ実画面QAのみ未実施。
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
- gitの直接pushは認証情報が無い環境のため失敗。ローカルhookを通した同内容をGitHubコネクタで公開し、ツリー一致と読戻しを確認した。hookは無効化しない。
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
- GitHub上のHTML・FINDINGS・PROGRESS・READMEを修正公開SHAで読戻し、各blob SHAとURLを確認。独立レビューも完了。

## 独立レビューと修正

- 作成者と別の担当が固定指示と初稿SHAをレビューし、修正後を再確認。中1件・低2件はすべて解消、内容の未解決blockerなし。
- 中重要度1件：active routineへの衝突時に新request_idも既存runへ永続aliasされるため、単にqueue待ちとして同ID再試行するとjobが進まない。ソースroutine_runner.go L31–42を根拠にFINDINGSとHTMLを修正。active中dispatch回避、衝突時の停止・照合、制御された再投入を明記。Hubコードは変更なし。
- 再レビュー対象ローカルSHA：`5a4b017e91b64641ef901591ad5b54a5df308a1a`。修正後公開SHA：`fb8bcbb8f20516b7e6298b869c56867dc871d3dd`。同一tree：`87b4bf614ec49edbac81c31627098f98943cbbb9`。固定SHAの修正差分・静的検査を再確認し合格。
- 公開初稿はローカル検査済treeと全5blob SHAが一致。draft PR #8はdevelop向け、draft=trueを確認。

- 低重要度の補足も反映：A2AのHTTPSは本提案の採用条件（仕様はSHOULD）と明記。HTML比較表にcaption・列scopeを追加。scroll領域は元からkeyboard focus可能・label付き。

## 初回C1提出時の証跡

- 初回成果物本文の固定SHA：`fb8bcbb8f20516b7e6298b869c56867dc871d3dd`。初回提出後、追加指示0f14ba5により本文を更新する。ボード更新SHAはGitHubのこのファイルの履歴で区別する。
- [draft PR #8](https://github.com/ishizakahiroshi/many-ai-cli/pull/8) はdevelop向け。C1差分は許可された4文書と必要な生成indexのみ。固定指示commitを含むためPR全体にはINSTRUCTIONS/REVIEWも載るが、作業側で両者は変更していない。
- 初回提出のローカル検査：git diff --check合格、working tree clean、HTML静的検査・独立再レビュー・hook検査合格。製品build/testと通信実験は未実施。
- CIのスナップショット（2026-10-03 05:20 UTC、成果物SHA fb8bcbb）：secret-scan成功、Validate queued。これは最終headのCI成功を意味しない。最新結果はPR checksを確認。CI待ちを接続実測の成功と扱わない。
- 未確認：実ブラウザ/スマホ表示、本人PC/Hub/CLI実動、plugin登録・購読、Slack別Bot受信、dots同一jobの自動再開、Rust版統合、実費・遅延。
- C2で本人に必要な操作：方式の選択と外部試験送信の許可、必要なplugin/app/webhook権限の承認、PCと既存Hub/CLIの起動・認証・専用cwd確認。秘密は本人が安全な設定画面で扱い、チャットや成果物へ転記しない。

## 公開実践例の追加調査（受付）

- 追加固定指示：[SUPPLEMENT_PUBLIC_RESEARCH.md / 0f14ba5](https://github.com/ishizakahiroshi/many-ai-cli/blob/0f14ba574aaf6e00bacaa92744f2db1e55845345/docs/bot/dots-local-ai-bridge/SUPPLEMENT_PUBLIC_RESEARCH.md) を全文読了。原指示・レビュー条件を維持。
- 更新前のremote head：`755db76e6823d75920b526d62e014ceb712fafd7`。専用作業branchとローカルが一致、未保存変更なしを確認。
- X、Zenn、Qiita、note、技術ブログ、公開GitHubを日本語・英語で検索する。公式仕様・公開コード・著者報告・提案を区別し、dots以外の成功をdots対応へ一般化しない。
- 新しい方式・権限・保存/再開条件が増えなくなれば検索を止める。ログイン・有料記事・取得制限は回避せず記録。
- 初回headのmacOS Goテスト失敗は原因・再現性未確定の既存結果。今回headのCIと分け、文書の範囲外の製品修正は行わない。

## 公開実践例の追加調査（原稿完成）

- 受付チェックポイント公開SHA：`1d70de98797670907e1435dc88ca9896ecf3e797`。remote読戻し確認済み。
- [PUBLIC_RESEARCH](PUBLIC_RESEARCH.md)へ8事例、固定コードSHA、検索語・日付・媒体・採否・証拠水準・条件を記録。FINDINGSとHTML、候補別C2最小手順へ反映。
- 7候補の順位変更なし。MCP Eventsはdots対応の公式根拠、Socketはローカルbridge実例があるがdots間往復未確認。n8nは中継部品、Claude Channelsはローカル入力部品の別案。
- 新しい制約：対応表の永続化≠待ちjobの永続化、MCP呼出し待ち≠死んだAIの起動、Bot除外と過去の再帰課金報告、sampleのdemo認証/メモリ保持。
- Xでは対象一往復の原典を得られず、noteの2件は本文取得エラー。追加のMCP/dots・poll・A2A検索で順位を変える根拠が増えず終了。取得制限の迂回なし。
- 静的検査・独立レビューの対象SHAと提出headは確定後に記録。実画面QAとC2は未実施を維持。

- 追加稿の静的確認：7方式・14図・8事例、リンク106件、ID51件、重複ID/内部anchor/ローカル相対パス欠損なし。外部asset読込0、ja/viewport/600px幅CSSを確認。HTMLは約44KB。実表示QA合格の意味ではない。
- PUBLIC_RESEARCH追加後のomitnix鮮度検査が1件追加を検知したため生成。coverage 1133→1134、unclaimed 205→206、analyzed 922のまま。既存解析警告は変更せず記録。

## 追加稿の独立レビュー修正

- 初回レビュー対象ローカルSHA：`380b5109506dbbc770d4056745717ea9b96fac0d`、tree `3d0da77bf670727ebbb96078c2d8456cc19f02f3`。
- 中1件：R2のfresh headless resumeは実行中TUIへ注入できず、TUIとの同時書込みで会話が分岐する。固定READMEのHandoff modelを根拠にPUBLIC_RESEARCH/FINDINGS/HTMLへ追記し、C2の既存session案では単一書込み担当を条件にした。最小試験の新規合成runは維持。
- 低1件：初回C1の提出証跡を見出し・本文で明示し、追加稿の未完了レビューと区別。修正後SHAの再レビュー待ち。
- 初回追加稿の公開処理：index blobとtree作成は完了したが、commit作成が取消で終了。ref更新未実行、remote headは受付時の1d70de9を確認。本文の提出完了とは扱わない。
