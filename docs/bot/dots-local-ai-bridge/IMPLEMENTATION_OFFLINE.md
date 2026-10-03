---
type: reference
status: draft
tags: [dots, implementation, offline-tests]
owner: ishizakahiroshi
review_status: draft
related: [C2_TEST_PLAN.md, PROGRESS.md, REVIEW.md]
last_reviewed: 2026-10-03
---

# #5 実装指示：C2共通処理の内部実装とfake検証

最終更新: 2026-10-03 JST

## 背景・承認・固定基準

案件は#5、対象many-ai-cli。本人が残りの検収を包括承認し、ローカル指揮者が三担当で設計を並列レビューした。既存Hub関連6テストをGo 1.26.8/windowsで実行して成功。手元のbinary versionは0.9.0-72-g21d0bc79、ソースHEADは21d0bc7935a2c4696fb89ccff2e324157a528c2d。稼働Hubを再起動・変更していない。binaryの表記一致は実際の稼働設定/認証/全APIの検収合格ではない。

参照する試験設計は[固定C2_TEST_PLAN](https://github.com/ishizakahiroshi/many-ai-cli/blob/d5e0d022490d55ef26b446f5317bba06be5798b4/docs/bot/dots-local-ai-bridge/C2_TEST_PLAN.md)。詳細が大きいため複製せず固定版を読む。本指示が最新の実装範囲とレビュー補強を規定する。

有力複数経路を比較して主経路と補助経路を選ぶ方針は維持。最初に成功した方式で終了しない。この段階は共通処理の実装とfakeによる内部検証であり、MCP/Slackの本番接続、dotsの同job続行、UX、常設監視の成功ではない。

## 許可範囲とbranch

独立branch feat/dots-bridge-c2-offline-20261003 をコード基準21d0bc7935a2c4696fb89ccff2e324157a528c2dから作る。既存branchがある場合は衝突を確認して別名を報告。既存PR #8は設計文書用として保持し、実装は別のdevelop向けDraft PRへ出す。指示文書は固定URLから読み、設計branchの履歴全体を新実装PRへmergeしない。#3 Rust移植を触らない。

許可：新規共通bridge/CLI/fake transport/fake Hub/専用SQLite/fixture/必要テストと運用資料の実装、内部テスト、bridge prototype Build、必要な生成索引、commit/push、Draft PR、独立コードレビュー。実装/テストは外部通信や実AI起動なしで再現する。既存依存を使用し、新規外部依存が必要なら理由と固定版を先に報告する。

対象はcmd/dots-bridge-c2、internal/dotsbridge、合成testdata、docs/bot/dots-local-ai-bridgeの実装ガイドと検証/看板、新CLIを記すCHANGELOG Unreleased、必要な生成索引。Hub/認証/既存DB/schema、workflow、Makefile、既存provider/CLIを変更しない。本体API追加で相関問題を隠さない。

公開relay配備、MCP/Slack実接続/アプリ/購読設定、外部試験投稿、秘密値の取得・持ち込み、料金が発生する新契約、実Hub/CLI起動、常設化、merge/main push/releaseは行わない。本人の金額上限・配置先・アカウント可否は未確定。新規fake/mock処理の実装はこれらを待たず完成させる。

## 実装する最小構成

- 固定設計§3のenvelope/state/store、専用SQLite inbox/jobs/outbox、source eventとlogical messageの相関、payload hash競合、期限、単一実行/単一writer、route_epochとexecution_epoch。
- local_worker/hub_client/mailbox：既存API契約へ接続可能なインターフェースとfake Hub、ローカルlaunch intent/実行履歴/累積予算/lockの永続化。既存Hub DBへテーブル追加しない。実Hubへ自動接続しない。
- fake M/S transport：同じcolor/recovery fixtureを両経路から処理し、同一job経路切替と重複を確認する。これは実MCP/Slack protocol適合の証拠ではない。
- CLI：help/status、mock実験の再現、停止/未回収jobのexport。live modeは未実装/未設定として拒否し、既定で外部通信・AI起動なし。予定名relay/local/trial等は実装済みの契約だけをhelp/ガイドへ記載する。
- 作業領域・fixtureは合成データだけ。新bridgeでtokenをmailbox/prompt/log/relayへ広げない。任意shell/prompt/cwdをイベントから作らない。前提の既存OSユーザー信頼境界は変更しない。

## 並列レビューで追加された必須条件

1. **mailbox/run handshake**：固定input.jsonをPOST前に読める状態へ公開する案は修正する。active確認とPOSTの間にmanual/別runが先行すると、別runが新jobを読めるため。入力はstagingし、応答またはGETでrequest_id/run_id/sessionを照合した後にのみ、そのrunへ取得を許可する。既存wrapperのMANY_AI_CLI_SESSION_IDとrun対応を利用可能だが、session単体の再利用に備えHubインスタンス識別・run・一回限りのdispatch nonceを組み合わせる。正確な取得契約を実装ガイドへ記す。応答喪失/alias/不明runには入力を渡さない。job内のquestion/dispatch単位で安定request_idを作る。
2. 結果にjob/question/execution epochと実行run/session/dispatch相関を持つ。前jobのanswer、旧/別run、部分/過大JSON、finishedでもresult_available=false、期限切れは送信せず保存・隔離・照合待ちにする。runに紐づかないnonceの自己申告だけで認可しない。
3. launch intentと累積job予算をローカル専用台帳に保存し、保存成功前のHub POSTを禁止。relay outage、worker再起動、経路切替で予算や未回収runをリセットしない。claim/leaseを確認できないとき新規起動しない。
4. route切替は起動済みrunを維持し、正当な元runの結果を現経路の論理outboxへ一度だけ載せる。旧execution ownerは拒否。送信済み不明はneeds_reconcile、別経路へ盲目的に再送しない。
5. 10分期限は費用停止の保証ではない。期限後は新規claim/起動/配送を止め、起動済み実AIをkillする機能を追加しない。遅い回答は照合待ちへ保存する。

## 自動テストと独立レビュー

固定設計の10テストに加え、保存失敗POSTゼロ、relay断/再起動の予算保持、ACK前DB失敗/ACK喪失、同logical ID異hash、古い/部分/過大result、manual run割込みによる未認可入力取得ゼロ、期限後の遅い回答保持、POST応答喪失時のGET照合、session再利用時の誤相関ゼロを検証する。HTTP httptestはloopbackに限定し外部サービスは呼ばない。

内部コマンド：go test ./internal/dotsbridge/... -count=1、go test -race ./internal/dotsbridge/... -count=1、go vet ./internal/dotsbridge/... ./cmd/dots-bridge-c2。prototypeは出力先を作ってgo build -o ./out/dots-bridge-c2 ./cmd/dots-bridge-c2（Windowsは.exe）。race未対応環境は未実施と明記し、skip/no tests to runを合格にしない。Go/Bun等は既存repo規定を確認する。

作成者と別のレビュアーがmailbox相関、永続化、クラッシュ点、二重起動/二重返信、予算、秘密境界、offline既定とlive拒否を確認する。指摘は修正し最終head/treeを再レビューする。実装ガイドに正確な再現コマンド、予定と実装済みの差、未実機事項、次のliveゲートを記す。

提出は同じ#5スレッド。受付/固定指示読了、branch、段階、Draft PR、最終head、独立レビューSHA、実行結果、mock両候補の結果、未実機/未配備/未自動監視を報告。最終head CIを確認し、不成功を隠さない。ローカル指揮者は提出後に内部テストとprototype Buildを再現する。実機接続の範囲と費用は別途具体化する。
