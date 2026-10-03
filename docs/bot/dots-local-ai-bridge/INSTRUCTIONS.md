# #5 many-ai-cli dots-bridge-c1-20261003 — C1 調査・比較HTML

## 目的・今回の範囲

dotsとローカルAIが、人の取り次ぎなしで質問・停止・完了・回答を交換できる方式を比較する。今回はC1の調査・比較HTML・候補の順位付けまで。通信機能の実装や実接続実験は後のC2以降で行う。

本人はC1をdotsへ依頼することを承認済み。調査文書の作成・commit・専用ブランチへのpush・develop向けdraft PRの提出を許可する。本体コード変更、依存追加、workflow変更、Hub起動、アプリ/Webhookの新規設定、外部への試験送信、継続的なAI起動、main取り込み、配備、秘密の取得・転記は許可しない。

## 基準・作業の分離

- 対象はこのmany-ai-cliリポジトリ。
- ソース基準：`21d0bc7935a2c4696fb89ccff2e324157a528c2d`（依頼準備時のdevelop）。
- 読む指示書はSlack起動メッセージのcommit固定URL。作業ブランチはその指示commitから作る。
- 推奨作業ブランチ：`docs/dots-bridge-c1-research-20261003`。既存の場合は衝突しない別名を報告する。
- 別件の#3 Rust移植・#4サービス実装を変更・再起動しない。今回の調査は基準版で行い、将来のRust版との統合は未確認として明記する。
- ローカル専用docs/local、他の非公開運用リポ、本人PCのファイルは読めない前提。本指示だけで着手する。

## 作成・変更してよいファイル

- `docs/bot/dots-local-ai-bridge/comparison.html`：主成果物。
- `docs/bot/dots-local-ai-bridge/FINDINGS.md`：公式根拠、ソース上の根拠、比較結果、C2実験計画の詳細。
- `docs/bot/dots-local-ai-bridge/PROGRESS.md`：現在地と証跡。
- `docs/bot/README.md`：この案件の成果物・PR・ボードリンクと受付番号の更新だけ。
- `.omitnix/index.json`：文書の追加に伴いリポの鮮度検査が要求する場合のみ、omitnixで生成し差分を確認する。手編集や本体ソース変更はしない。

固定版のINSTRUCTIONS.md・REVIEW.mdは変更しない。指示に疑問があれば同じSlackスレッドで質問する。

## 確認済みの前提と未確認事項

既存運用では、本人アカウント経由のSlackコネクタ投稿にdotsが応答し、GitHubの固定commit指示を読んで作業・返信する経路は実動した。これを別Botからの投稿への応答や自動通知の証明にしない。ローカルAIは現状、Slackへ読み取りに行って返信を確認する必要がある。

dotsのSlackプロフィール説明ではチャンネルのメンション・グループDMに対応すると案内されていたが、新規受信Botとの共存・自動応答が実際に成立するかは未実験。本人とdotsの既存DMを別Botが読めると仮定しない。

many-ai-cliのREADMEにはルーチンから新規AIセッションを起動する仕組み、履歴、日次・平日スケジュール、同じルーチンの重複実行防止が記載されている。ソース候補は `internal/hub/routine_runner.go`、`routine_handlers.go`、`routine_store.go`、`routine_test.go`。呼び出し先・設定・認可を読み、外部イベント起動や既存セッション再開が可能か別途調べる。本人PCでの動作はローカル確認待ちとする。

## 比較対象

1. Slack Events API / Socket Mode。
2. Slack HTTPイベントと中継・永続化。
3. GitHub Webhook。
4. GitHubまたはSlackの軽量ポーリング（接続先の制約が違うので分けて説明）。
5. dotsから専用HTTP/MCP通知口への直接通知（実際に送信・ツール登録できるか未確認）。
6. A2A等のエージェント間通信（規格の存在とdots対応を区別）。

有用なら既製の中継も比較するが、製品名の羅列で終わらせない。通知、AI起動、状態記録は別の軸で選び、Slack通知＋GitHub記録などの組み合わせも認める。Socket Modeは会話時点の有力候補に過ぎず、一位と決め打ちしない。

## 調査方法とHTMLの内容

最新の公式資料を読み、確認日・仕様版・直接の根拠URLを残す。製品名・未公開API・権限・自動再開機能は推測しない。dots自身の環境について、公開可能な能力の有無は確認してよいが、他者への送信や設定変更を伴う試験は行わない。

HTMLは日本語、単体でブラウザから開ける1ファイル。外部CDN・画像・JS依存なし。スマートフォン幅でも読めるようにし、各方式に図（inline SVG等）と平易な説明を付ける。

各方式で以下を説明する：

- dots、Slack/GitHub/中継、ローカル受信プログラム、ローカルAIを置いた通信図。
- 質問・停止・完了・回答の往路と復路、相手を起こす担当、通知と保存の違い。
- メリット・デメリット：導入の手間、必要権限、遅延、費用・AI利用枠、PC停止・切断、履歴・復旧性、運用負担。
- 必要な設定・常駐箇所・公開受信口の有無、dots/Hub側の能力、未確認条件。
- 根拠リンク・確認日と「公式仕様／ソース確認／実測／未確認」の区別。

全方式の横断比較と、第一候補・次点以降の順位・理由・除外条件を示す。必須条件はdotsへの送信、dotsからの受信、ローカルAI起動、回答後のdots再開が成立し得ること。未確認を成立済みにせず、C2で検証する項目として示す。

既存Hubの認証・ローカルbindなどを維持できるか確認する。別Botの投稿へのdotsの反応、イベントの配送範囲、DM/スレッド、切断中の補完、起動先の認証・cwd・履歴・承認待ち、ループ防止も評価する。毎回AIを起こす巡回方式は推奨しない。

## C2へ渡す試行計画

C1の順位順に専用の合成試験案件で「依頼→dotsの質問→ローカル受信・AI起動→回答→dots再開→完了受信」を試す手順を作る。本C1では実行しない。

候補ごとに、実施条件、必要なオーナー操作、最小構成、成功条件、証跡、試行上限と理由、失敗の切り分け、後片付け、次に試す候補を明記する。不成立なら理由を残して次点へ進み、一往復が成立した方式だけをC3構築へ渡す。権限の未許可は条件待ちとし、方式の不成立と混同しない。共通のローカルAI起動障害なら、通知方式を替えて同じ失敗を繰り返さない。全候補が不成立なら候補・条件を再評価する。

## 公式資料の調査入口

- https://docs.slack.dev/apis/events-api/using-socket-mode/
- https://docs.slack.dev/reference/events/app_mention/
- https://docs.slack.dev/reference/events/message.channels/
- https://docs.slack.dev/reference/events/message/bot_message/
- https://docs.slack.dev/reference/methods/chat.postMessage/
- https://docs.github.com/en/webhooks/using-webhooks/handling-webhook-deliveries
- https://docs.github.com/en/webhooks/testing-and-troubleshooting-webhooks/using-the-github-cli-to-forward-webhooks-for-testing （開発・テスト用途の制限を確認）
- https://docs.github.com/en/rest/issues/comments
- https://developers.googleblog.com/a2a-a-new-era-of-agent-interoperability/ （背景。採用条件は現行仕様で確認）

## 提出・完了条件

HTMLに本指示・FINDINGS・PROGRESSへのリンクを付け、READMEから成果物へ辿れるようにする。リンクは作業ブランチ上の公開GitHub URLまたはブラウザで解決できる相対パスを使う。MarkdownをHTMLと同じディレクトリへ置くだけでなく、GitHub上で辿れることも確認する。

REVIEW.mdの観点で、作成担当とは別の担当に文書・HTMLをレビューしてもらう。レビューしたSHA、指摘、修正後SHAを記録する。独立レビューができなければ未実施として報告する。

HTMLを実際に開いて図・横断比較・小さい画面幅・リンクを確認する。ブラウザが使えなければ静的確認の範囲と未実施を明記する。製品のBuildやHub起動は不要。git diff --checkと許可ファイル範囲を確認し、成果物をcommit/pushしてGitHubから読めることを確認する。コミットメッセージは日本語にする。文書追加でomitnixの鮮度検査が止めたら生成して差分を確認し、フックを無効化して回避しない。生成時の既存ソースの解析警告を、本体ソースを直す許可と扱わない。

同じSlack案件へ、draft PR URL、head SHA、HTML/FINDINGS/最新PROGRESSのURL、確認した項目と未確認、順位と理由、C2で必要なローカル操作を報告する。受付番号・指示読了・開始時の能力確認も返信する。

工程開始・質問・停止・提出時にPROGRESSを更新する。更新を統合する担当は一人。仕様の根拠、自己申告、実測、ローカル確認待ちを分け、C2以降を実施済みにしない。
