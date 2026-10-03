# dots とローカルAIの複数経路を比較するC2試験設計

> 最終更新: 2026-10-03(土) 19:05:49 JST

## 目的と現在の許可範囲

**MCP tools＋MCP EventsとSlack Socket Modeを同じ二つの合成課題で比較する。第一候補が成功しても比較を終えない。** PC停止中の受付、またはSocket固有の制約が重要ならSlack HTTP＋中継を追加する。主経路と補助／手動復旧経路を選び、入口が増えても案件・実行・返信の正本は共通queueへ集約する。全7方式の実装・常設は求めない。

本書は [固定追加指示d533a72](https://github.com/ishizakahiroshi/many-ai-cli/blob/d533a72f66752d1f9e97a40d4ccba468c65e4f6f/docs/bot/dots-local-ai-bridge/SUPPLEMENT_MULTI_PATH_PLAN.md)に基づく**設計のみ**。C2実装、試験、設定、認証情報取得、外部送信、Hub／CLI起動、Build、常設監視は未実施・未許可。以下のファイル・コマンド・契約は、既存と明記したもの以外は追加実装提案であり、現在使える機能ではない。

ソース基準は `21d0bc7935a2c4696fb89ccff2e324157a528c2d`。初回C1追加調査の固定成果は `855786ad207a775b145e22725215df24ab9125bd`。#3 Rust作業を変更・起動しない。実装前にローカル指揮者が稼働版・OS・CLI版・対象branchを記録し、Go基準とのAPI差があれば本契約を更新する。

原INSTRUCTIONS／REVIEWとPUBLIC_RESEARCHは調査根拠として保持する。そこから読み取れる「順位順に試し最初の成功で採用」は本書で置き換える。現在の選定手順の正本は本書である。

## 1 比較する構成と担当

共通relayはPCとは別の常時稼働するHTTPSサービスと専用DB。ローカルworkerはPCから外向きに取得し、Hubはloopbackのままにする。これは比較条件を揃えるための提案構成であり、サービスはまだ存在しない。

### M MCP Events

```text
local workerの依頼/回答 → relayのoutbox → MCP Events callback → dotsの購読会話
dotsの質問/停止状態/完了 → 登録MCP tool → relay inbox → 共通job queue
共通queue → PC worker → 既存Hub routine → 新規ローカルAI → PC worker → outbox
```

relayが受信・認証・保存・Events配信を担当する。dotsは合成依頼の質問・回答後の続行・完了を担当し、PC workerだけがHubを呼ぶ。OpenAIはMCP 2.0／2026-07-28のEvents webhookとdots対応を説明しているが、本人のplugin追加・購読・同job続行は未確認。[公式Events](https://developers.openai.com/plugins/build/mcp-events)。通常MCP通知、Agents APIやWorkspace Agentの別APIで代用しない。

### S Slack Socket Mode

```text
relay outbox → Slack Web API → 専用thread → dots
dots質問/停止状態/完了 → Slack → PCのSocket receiver → relay inbox → 共通queue
共通queue → 同じPC worker → 同じHub routine/AI → outbox → 同じthread → dots続行
```

最小比較ではSocket receiverをPCに置く。app-level接続と投稿をPC側のSlack adapterが担当し、共通relayにはSlack tokenを複製しない。上図のWeb API送信はoutboxをPC adapterがclaimして実施する。M案にも共通relayがあるため、**この組合せ全体が公開HTTPS不要という意味ではない**。Socket受信自体にSlack用公開URLが不要という利点である。[Socket公式](https://docs.slack.dev/apis/events-api/using-socket-mode/)。

PC停止中はSocket受信・Slack outbox送信も止まる。Slackに残る履歴とrelayが受理済みのjobを区別する。再接続後の欠落補完が実証できなければ、Socket単体のPC停止耐性は未成立。Socketを将来relay側に置く案は追加常駐条件として別記録し、試験途中で黙って構成を変えない。

### H 必要な場合だけSlack HTTP

```text
dotsの投稿 → Slack署名HTTPS event → 常時relay inbox/queue
queue → PC復帰後のworker → Hub/AI → relay outbox → Slack Web API → dots
```

HTTP案ではSlack受信・投稿adapterを常時relay側へ置く。PC停止中にもrelayが受理・保存できる点を比較する。既存Socket用appを使って設定を切り替える場合、元の設定を記録して試験を直列化する。二つのSlack transportが同じappで同時配信されるとは仮定しない。M＋Sの複数入口試験には別々のMCP／Slack入口を使う。[HTTP設定公式](https://docs.slack.dev/apis/events-api/using-http-request-urls/)。Bot受理拒否が共通原因ならHへ切り替えても解決しない。

GitHubは固定指示・実装差分・レビュー・検収要約の記録に使う。Webhook／pollを追加するのは復旧への実益とdotsの起動条件を説明できた場合だけ。GitHubコメント保存をdots起動と同一にしない。

## 2 オーナーの設定と起動操作

実装・外部試験を別途許可した後、採用する試験経路に必要な行だけ実施する。公開ページからアカウント内の可否は断定しない。秘密は本人がサービスの安全な設定画面／許可済み秘密管理へ入力し、文書・chat・コマンド引数へ転記しない。

| 対象 | 本人が確認・操作する導線 | 最小条件と理由 | 未提供／未許可の扱い |
|---|---|---|---|
| 共通 | 対象dot、専用Slack会話、検収記録用repo、合成データ、試験時刻・費用上限を確定 | 実データを使わず送信先を限定。relayホストと認証管理者を決める | 支出額・アクセス未確定は開始待ち |
| M plugin | [ChatGPT Plugins](https://chatgpt.com/plugins)の＋から接続。公式手順ではSettings → Security and login → Developer modeの利用可否を先に確認 | 公開MCP URL、認証、tools/eventsの検出。本人に操作が表示されない場合はworkspace管理者へ可否確認 | 不可ならcondition_wait、別アカウントや未公開APIで回避しない |
| M 購読 | plugin詳細でeventsが見えることを確認、対象dotへ試験event・job filter・許可された応答を指示 | subscribe/callback検証と購読保存。dotの質問・完了は専用toolへ戻す | 実験開始前に検出・購読不可を記録。単なるtool接続成功で代替しない |
| S app | [Slack Your apps](https://api.slack.com/apps) → 専用app → Socket Modeを有効化、Basic Information → App-Level Tokens | app-level connections:write。app作成・恒久アクセスの承認は本人が行う | 管理者承認待ちは技術的不成立と分ける |
| S 会話権限 | OAuth & Permissions／Event Subscriptions／Install App、専用会話へappとdotを参加 | chat:write、app_mentions:read＋app_mention、公開ならchannels:history＋message.channels、非公開ならgroups:history＋message.groups。選んだ会話種類だけ付ける | DMを最小試験に使わない。im/history全件・files・reactions等を不要に要求しない |
| S sender | 対象dotの実際の送信identityを受信eventと照合し許可リストへ登録する手順を承認 | 公開bridgeにあるbot_id一律除外を採用しない一方、任意Botを許可しない。dotが別Botを受理するかは別の未確認条件 | 本人投稿での代行成功は無人往復の成功にしない |
| H 追加時 | 同appのEvent Subscriptions → Request URLを設定、Basic Informationの署名設定を使用。Socket設定の旧値を控える | relayのHTTPS検証、raw body署名、3秒ACK、最小の同じevent/scope | PC停止中受付の比較が必要な場合のみ。不要な二重app登録を求めない |
| Hub | 本人が既存手順でPC・Hub・認可済CLIを起動。Hub sidebar／mobile homeのRoutinesから専用manual routineを作る | 専用の狭い既存cwd、provider/model、既存承認設定。日次scheduleは不要。[README](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/README.md#shared-instructions-and-routines) | 起動・認証・cwd・権限不足は共通ゲートで止める |
| ローカルAI | 試験cwdで使うproviderの認証とファイル読書き権限、必要なら個別connectorを確認 | 現在のdot会話のauth／connector／cwdを新規AIへ継承できると仮定しない。最小試験はmailboxだけを読むためSlack/MCP tokenをAIへ渡さない | 承認待ちを記録。権限を弱めて通さない |
| GitHub記録 | 既存認可経路でPR #8と後続の許可済み試験記録を読む | 最小試験の起動用Webhookや追加PATは不要 | 書込み先未承認ならローカル検収ファイルを本人へ返す |

MのUI導線は[公式接続手順](https://developers.openai.com/plugins/deploy/connect-chatgpt)、Slackの認証・scope・Bot条件は[FINDINGS](FINDINGS.md)の公式根拠を参照。初回設定操作数、日常開始操作数、質問後の人手、復旧操作数を別々に実測し、少なさを先に合格値として書かない。

## 3 既存APIと追加実装の境界

### 読んだ既存ソースと試験

| 固定基準の既存ファイル | 確認内容と設計への制約 |
|---|---|
| [routine_handlers.go](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/routine_handlers.go) | POST /api/routines/{id}/runs はrequest_idだけを受理、最大128 bytes。GET /api/routine-runs/{id}、GET /api/routine-runs?routine_id={id}で照合可能（一覧は最大200件） |
| [routine_runner.go](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/routine_runner.go) | 保存済prompt/cwd/providerで新規sessionを起動。active runへの新request_idは永続aliasとなる。起動POSTへ動的promptを渡せない |
| [routine_store.go](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/routine_store.go) | 保存後にメモリ公開。starting/running/waitingがactive。result_availableとfinishedは別。cwdは実在する狭い絶対directory |
| [http_helpers.go](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/http_helpers.go) | Bearer認証を受理し、Host/Origin等のguardがある。workerはlocalhostへ既存認証で接続し、Hub tokenをrelayへ送らない |
| [routine_test.go](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/routine_test.go) | ConcurrentAdmissionAndRetryRemainOneRunはactive中の別IDも完了後まで同じrunへ戻ることを検証。SaveFailure、HTTPCreateRunAndReadStableDetail、FailedLaunchAndPersistenceRestartKeepIdempotency、MissingAfterRestartNeverReplaysPrompt、IdleWithoutCompletionNeedsConfirmationも読取済み |
| [wrapper.go](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/wrapper/wrapper.go#L1460-L1487) | 既存wrapperは実CLIへMANY_AI_CLI_HUB_TOKENを環境変数で渡す。headlessにも同じ受渡しがある。bridgeで秘密を増やさない設計と、既存同一OSユーザーの信頼境界を分ける |
| [go.mod](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/go.mod)、[sessionstore/store.go](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/sessionstore/store.go) | Go 1.26.8、modernc.org/sqlite v1.54.0、golang.org/x/netが既存依存。SQLiteを使う実例あり。Hub既存DBへbridgeテーブルを直接追加しない |

この読取りで製品テストは実行していない。ソースと関連テストは基準から無変更。Rust版に同じAPIがあるとは断定しない。

### 後続実装へ渡す新規ファイル案

以下は**未作成の提案名**。実装承認後に対象branchをローカル指揮者が指定する。dots本体の内部実装を変更する案ではなく、登録するbridge MCP serverとローカルadapterを作る案である。

| 提案ファイル | 担当する契約／テスト |
|---|---|
| cmd/dots-bridge-c2/main.go | relay/local/trial/stop/statusサブコマンド。live送信は明示opt-in、既定はmock。秘密値は引数に取らない |
| internal/dotsbridge/envelope.go、state.go | job/phase/論理message、状態遷移・期限・sender制約 |
| internal/dotsbridge/store.go、schema.sql | relay専用SQLite。inbox、jobs、launch_intents、outbox、subscriptions、route_epochsをtransactionで永続化 |
| internal/dotsbridge/mcp.go、mcp_events.go | 公開MCP 2.0契約、認証、tool入力、event購読・検証・署名・期限更新。Events製品が対応するwebhookだけを使用 |
| internal/dotsbridge/slack_socket.go、slack_http.go、slack_outbox.go | ACK前保存、認可済sender/thread、再接続・署名・配送。HTTPファイルはHを比較するときだけ実装 |
| internal/dotsbridge/local_worker.go、hub_client.go、mailbox.go | outbound claim、ローカル起動台帳、単一writer、Hub既存HTTP、結果照合、mailbox入出力 |
| internal/dotsbridge/*_test.go、testdata/color.json、testdata/recovery.json | 外部サービス／CLIを呼ばないfake transport・fake Hub・時計・クラッシュ点。固定合成fixture |
| docs/bot/dots-local-ai-bridge/C2_OPERATOR_GUIDE.md | 後続実装後の設定例・起動/停止/復旧手順。存在する実装に合わせて本設計を更新する |

依存の第一案はGo標準のnet/http、encoding/json、crypto/hmac等と**既存**modernc.org/sqlite。Socket clientも既存x/netの適合性を実装時に確認する。MCP/Slack公式SDKが必要になれば、対応protocol版・保守負担・lock差分を提示して追加判断する。今回の依存追加・インストールはない。n8n等は置換案であり、最小構成へ重ねて常設しない。

### 新規adapterの接続契約案

- **relay入口**：新規MCP tool名bridge_put_event／bridge_get_job、新規HTTP /v1/events、/v1/local/claim、/v1/local/result。いずれも提案名で、OpenAIや既存HubのAPIではない。toolは認可されたjobへの質問/停止状態/完了のみを保存する。
- **封筒**：campaign_id、case_id、job_id、logical_message_id、phase、question_id、sequence、source、source_event_id、sender、route_epoch、execution_epoch、payload_hash、received_at、expires_at。署名やtokenは監査記録に含めない。
- **重複**：sourceとsource_event_idの組で配送重複を除外。同じ報告を二経路へ載せる場合はlogical_message_idを同じにする。job_idだけでは複数の正当な段階を潰すため不足。同IDで本文hashが違えば競合として停止する。
- **Slack相関**：専用threadとjobを事前登録。合成本文にjob/phase/question/logical_message識別を付ける。識別子欠落・sender不一致・未知threadは隔離し、自然文だけから新しいjobや権限を推測しない。MCPとSlackで意味が一致するか試験する。
- **永続化**：ACKはinbox保存後。状態変更とoutbox登録を同じDB transactionへ入れる。PCはDBファイルを共有マウントせず認証APIでclaim。execution lease/fencingにより旧workerの結果を拒否する。route_epochは配送経路の選択だけに使い、実行所有権のepochと分ける。DBが止まれば受付成功を返さない。
- **ローカルmailbox**：専用cwd内の提案ファイル .bridge-c2/input.jsonへclaim済みjobをatomicに配置。固定routine promptはこの合成入力を読んで .bridge-c2/answer.jsonへjob_id/question_id/execution_epoch/answerを書き、結果を報告する。workerがrun完了とresult_available、JSON整合を確認してoutboxへ返す。部分ファイルやprocess終了だけでは送信しない。
- **秘密を広げない境界**：新bridgeはSlack/MCP/Hubの秘密をmailbox・prompt・ログへ書かず、Hub tokenをrelayへ送らない。workerがbridge通信を担う。ただし既存wrapperはCLIへHub tokenを環境変数で渡すため、AIがHub認証情報を持たない隔離は実現していない。同一OSユーザーの既存信頼境界とproviderの権限制御に依存する。token削除や製品の認証変更は今回の提案済み機能ではない。入力から任意shell/prompt/cwdを組み立てない。

### 配送のepochと実行のepoch

route_epochはoutbox配送先・配送attemptの世代。execution_epochはlocal workerがjob/runを担当する所有権の世代で、mailboxと結果へ記録する。M→Sの通常切替で変えるのはroute_epochだけであり、起動済みrunのexecution_epoch/request_id/run_idは維持する。正常な元runの回答を受理し、現在選んだ経路の同じ論理outboxへ一度だけ載せる。経路切替を再起動理由にしない。

execution_epochの更新は、元worker/runを照合して所有権移譲を確定した場合だけ。旧実行所有者からの遅い結果は拒否して証跡を残す。旧routeからの後着inboxは保存・重複/状態照合し、単に経路が古いだけで妥当な元runの回答・完了を捨てない。旧outbox配送leaseでの再送と自動的な経路戻しは認めない。送信済みか不明なattemptはneeds_reconcileとし、別経路へ盲目的に再送しない。物理配送のexactly-once保証と、論理処理/AI実行を一度に抑える条件は区別する。

### 起動の不確実性とrequest_id

workerは専用routineごとのローカルlockを取り、launch intentと安定request_id（job由来、128 bytes以内）を**POST前**に保存する。active runがあればdispatchせず待つ。manual UIで同routineを同時操作しない約束だけに依存せず、応答runのroutine_id/request_id/run_idを照合する。

POST応答を失ったら同じrequest_idのintentを保持してGET一覧/詳細で照合する。runを見つけられない、既存runが別job、履歴上限等で不明ならneeds_reconcileで停止。active衝突でaliasされたIDは終了後も元runへ戻るので、別IDを次々作らない。新しいdispatch keyは元jobの未起動が確定し、管理された再投入を承認した場合の別判断である。

lease期限切れだけで別workerを再起動させない。元run／元workerの生存をローカル指揮者が照合してから移譲する。relayのfencingだけでは古いプロセスのHub呼出しを取り消せないため、単一local workerとHubの同ID冪等性を併用し、不確実時は停止する。exactly-once実行を無条件には保証しない。

## 4 同じ合成課題で比較する順序

開始前に両候補の利用条件を確認。Mが成立してもSを評価する。M未提供でSだけ動いた場合も比較表を残し、未測定Mとの優劣は未確定とする。

| fixture | 共通課題と期待する会話 | jobの扱い |
|---|---|---|
| color | 「合成メモの色が未指定。色を質問し、回答後に1行で完了して」。dots質問→新規ローカルAIの固定回答「青」→dots同job続行→完了回収 | M/Sで内容・provider/model・承認設定を揃える。job_idはcampaign＋候補＋colorとし、別試行を誤って重複扱いしない |
| recovery | 同じ色質問課題に「回答待ち中はblocked状態を記録し、同じ回答を二度適用しない」を付加。停止は業務状態でありAI強制終了ではない | campaign＋候補＋recovery。二経路の同じ報告は共通logical_message_idで照合。下記障害試験を割り当てる |

各試行は依頼保存→dots質問→質問保存→Hub新規run→回答保存→dots続行→完了保存の7区間を追う。質問・回答・完了の件数、各ID・時刻、launch intent、Hub result_availableを証跡にする。ACKや片方向返信だけはpartial。

M color、S colorを順に測る。その後M recovery、S recoveryで相手経路を補助として使えるかを評価する。必要ならHでも同じ2fixtureを測る。候補間のjobは独立、**一つのjobの経路切替ではjob_idとlaunch request_idを変更しない**。同時に複数のローカルAIを起こす比較はしない。

## 5 UXと費用の記録表

初期値はすべて未実測。本人の操作回数は設定・開始・承認・復旧に分け、実際の画面で数える。遅延は7区間のtimestamp差、費用は各サービスの利用表示と試験時間から記録する。

| 比較軸 | M | S | H（追加時） | 判定方法 |
|---|---|---|---|---|
| 設定／日常開始／承認／復旧の本人操作数 | 未実測 | 未実測 | 未実測 | 初回設定と日常UXを混ぜない |
| 質問後の無人続行と完了回収 | 未実測 | 未実測 | 未実測 | 同jobの質問/回答/完了、本人の取り次ぎ有無 |
| 区間別・全体遅延 | 未実測 | 未実測 | 未実測 | 保存/起動/回答/続行/完了を分離。2件でSLAやp95を主張しない |
| 会話の見やすさ | 未評価 | 未評価 | 未評価 | 本人が同jobを追い、質問・停止理由・完了を見つけられるか |
| PC停止／再接続 | 未実測 | 未実測 | 未実測 | 停止中受理と未受信を区別、復帰後の未処理inbox/outboxを照合 |
| 導入・維持負担 | 未評価 | 未評価 | 未評価 | component数、認証更新、購読/接続復旧、監査作業 |
| AI利用枠・通信・常時ホスト/DB・PC費 | 未実測 | 未実測 | 未実測 | 本人の実契約に基づく。同じqueueを使う費用は共通費として別計上 |
| 総合結果 | planned | planned | 条件により追加 | passed / partial / condition_wait / failed / untested |

MCP Eventsのdots利用可否、Slackのworkspace/app承認・履歴制限、relayホストの課金プランを本人が確認する。金額はplan未指定のため未算定。支出上限・新しい契約を本人に代わって決めない。軽量queue取得にAIを使わず、空振りAI起動0を目標として測る。

## 6 複数入口と復旧の検査

まず全ケースをfake transport/fake Hubで自動テストし、ローカル指揮者も内部テストを再現する。実機では各候補のrecovery job内で可能なケースを実施し、残りは未実機確認として残す。mockの合格を実配送の合格へ読み替えない。

| ケース | 注入位置と期待する処理 | 合格証跡／停止条件 |
|---|---|---|
| 二経路から同一報告 | 同じjob・logical_messageをMとSから各1回。各transportのsource_event_idは別でもよい | inboxに2配送の対応、意味イベント1件、Hub run1件、回答1件、完了1件。同ID本文差は競合停止 |
| 遅延・順序逆転 | fixtureの質問/blocked/回答/完了を入替え、古いsequenceを後着させる | 状態を巻き戻さず保留/拒否を記録。前提を満たさない完了を確定しない |
| 自己返信ループ | 自分のoutbox返信・ACK・完了通知を再受信 | origin/phase/jobで除外し、追加AI起動0。許可されたdotの返信まで一律Bot除外しない |
| 主経路停止 | 質問保存後、主経路の配送adapterだけを停止。共通queueとHubは動作を維持 | queueのoutboxを補助へ明示移管、同jobの回答とdots続行を確認。補助が別dots会話なら相関可能性を実測し、未知なら自動切替不可 |
| 元経路の復帰 | route_epochだけを進めて切替後、元経路の後着と起動済みrunの結果を受ける | 元runの正当なexecution_epochの回答を受理し、現経路の論理outboxへ1回。旧配送leaseは再送せず、後着inboxは重複/状態照合。元経路へ戻すのは明示操作 |
| 途中クラッシュ | fakeでは保存前後、POST前後、結果保存前後で停止。実機は許可済み専用workerを質問保存後／起動後に中断して再開 | inbox/outbox/launch intentとHub runを照合。保存済jobを失わず、未知runを再起動しない。単なるsession mapping保存だけでは不十分 |

実機のPC電源断・Hub/CLI強制終了は基本試験へ含めない。まず専用receiver/workerの停止で境界を確認し、本当のPC停止復帰が必要なら本人が追加許可して実施する。アプリ停止模擬と電源断耐性を別欄へ記録する。

障害の切分けは、経路固有（Socket接続、MCP購読、Slack HTTP URL）、共有queue/DB、共有認証、PC/Hub/CLI、dots起動/同job続行の5層。共有queue、同じ認証基盤、同じPC、同じdots側の制限が落ちた場合は別入口も代替にならない。二経路を並べただけで冗長化済みとはしない。

## 7 再開の種類と共通ゲート

| resume_kind | 意味 | C2での位置付け |
|---|---|---|
| live_tool | 実行中のMCP呼出しが回答を待ち、戻り値で続く | 生きたプロセスへの返却。PC停止後の起動証拠にしない |
| existing_session | 保存されたCLI会話をheadless --resume等で開く | 最小試験の必須条件にしない。追加採用時はTUIとbridgeの単一writer・session lockを別検証 |
| fresh_run | Hub routineから新規AI sessionを起こす | 最小C2のローカルAI。専用cwdと認証/権限を本人確認 |
| dots_job | 回答event後、dotsが同じ論理jobを続行する | M/Sとも必須。別会話で新しい回答を作るだけでは合格にしない |

既存Hubのactive衝突/request_id alias、権限待ち、cwd、起動失敗は全経路で共通。`needs_reconcile`中にroute変更で別IDのrunを増やさない。Slack Bot拒否はHTTPやpollへの変更で解消すると仮定しない。認証/権限拒否を本人token流用、名前偽装、guard無効化で回避しない。

## 8 役割別の実装と検収

| 担当 | 別途許可後の担当範囲 | 提出する証拠／合格範囲 |
|---|---|---|
| dots | bridge/adapter設計・許可された実装・fakeを使う自動テスト・独立レビュー依頼 | 固定commit/tree、変更file一覧、再現コマンド、mockログ、未対応条件。アカウント設定・実機検収を代行済みとしない |
| 独立レビュアー | source契約、二重実行/分岐、認証境界、旧選定方針の残骸、固定SHAを確認 | 指摘・修正後SHA。実測なしで稼働合格としない |
| ローカル指揮者（本人PCのローカルAI） | 稼働版差の照合、内部テスト、合成一往復、複数入口/復旧の実機検収、run照合 | 候補別の7区間ログ、UX表、Hub run/result、失敗理由、停止/撤収結果。本人の承認を超えて設定しない |
| 本人 | アカウント・plugin/app権限、必要なPC/Hub/CLI起動、費用上限、UXと主/補助経路の採否 | 設定可否、操作数、許可対象、最終判断。秘密値は報告しない |

### 検証コマンドの区別

**この文書作成では以下は実行していない。** repo rootで実行する将来の契約。Go 1.26.8、Hub側検査にはweb/distが必要（[web/web.go](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/web/web.go)）。

追加実装後、dotsまたはローカル指揮者が許可範囲で行うmock検査：

```sh
go test ./internal/dotsbridge/... -count=1
go test -race ./internal/dotsbridge/... -count=1
go vet ./internal/dotsbridge/... ./cmd/dots-bridge-c2
```

必須の新規test名案はTestCrossIngressDedup、TestOutOfOrder、TestSelfReplyIgnored、TestRouteFailoverAndReturn、TestCrashReconcile、TestHubAliasCollision、TestStaleLeaseCannotLaunch、TestMailboxResultCorrelation、TestInFlightRouteSwitchKeepsResult、TestStaleExecutionOwnerRejected。追加2件は「run実行中→経路切替→元runの正常回答→論理outbox1件」と「所有権移譲後の旧execution_epoch結果の拒否」を別々に検証する。packageやtestが未作成なら検査不能であり、no tests to runを合格にしない。fake以外の外部接続やAI起動をこれらへ混ぜない。race対応toolchainがなければその検査は未実施として分離する。

本人が通常の製品Buildを行う経路は、[CLAUDE.mdの規約](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/CLAUDE.md#ai-作業共通ルール)に従いmake build。ローカル指揮者は本人が用意したweb/distを使って下記の既存Hub試験を実行する。bun run build単体を通常手順へ置かない。いずれもBuild/内部試験の追加許可後に行う：

```sh
make build
go test ./internal/hub -run '^TestRoutine(ConcurrentAdmissionAndRetryRemainOneRun|SaveFailureDoesNotLaunchOrChangeMemory|HTTPCreateRunAndReadStableDetail|FailedLaunchAndPersistenceRestartKeepIdempotency|MissingAfterRestartNeverReplaysPrompt|IdleWithoutCompletionNeedsConfirmation)$' -count=1
```

追加adapter専用の試験binaryは新規提案なので、通常製品make buildにはまだ含まれない。実装後に本人がこのprototype Buildを個別許可する場合だけ、出力directoryを用意して次を実行する（Makefileへ組み込む変更は別判断）：

```sh
go build -o ./out/dots-bridge-c2 ./cmd/dots-bridge-c2
```

参考として基準CIはBun 1.3.14でfrontendを生成するが、その個別手順を通常ローカルBuildの代替にはしない。Windowsでは生成物に.exeを付ける。上記のHub試験は起動stub/httptestを使う既存testを選んでおり、実機一往復の代替ではない。フロント生成・Build・provider起動は本人側が行う。

次のCLIは**追加実装するインターフェース案**であり、今は実行不能。実装後のhelpと照合して確定する。設定fileは秘密の値ではなく本人管理のsecret参照を持つ。

```sh
./out/dots-bridge-c2 relay --config ./c2/relay.json
./out/dots-bridge-c2 local --config ./c2/local.json --max-jobs 2 --job-timeout 10m
./out/dots-bridge-c2 trial --config ./c2/trial.json --path mcp --case color
./out/dots-bridge-c2 trial --config ./c2/trial.json --path socket --case color
./out/dots-bridge-c2 trial --config ./c2/trial.json --path mcp --case recovery
./out/dots-bridge-c2 trial --config ./c2/trial.json --path socket --case recovery
./out/dots-bridge-c2 stop --config ./c2/local.json --drain
```

localのmax-jobsはcampaign内の候補単位で累積2件の上限。件数をDBで管理し、worker再起動や経路変更で予算をリセットしない。候補変更前にdrainと台帳照合を行う。relay/localの別terminal・別hostを混同しない。trialはlive設定と外部送信許可がない限り拒否する契約。trialがjob IDを記録したら、失敗しても別jobを勝手に追加しない。

ローカル検収結果は、許可済みGitHub試験記録に固定SHA・短い結果・redactedログを保存し、そのURLを同じ#5 Slackへ既存認可経路で返す。自動通知がない間は本人がdotsへ「検収結果を見て」と伝え、dotsがURLとheadを読み合わせる。第三者送信や未承認の新通知を追加しない。

## 9 予算 停止 撤収

- 基準は**2合成job／候補、各10分、同eventの重複は各job1回**。MとSは計4job、H追加時は計6jobまで。比較を終えるための二件目であり、成功するまでの無制限再試行枠ではない。
- 2fixtureの本文は候補間で同じ。recovery jobに二経路重複・主経路停止/復帰を割り当てる。順序逆転・全クラッシュ点・共有障害はまずfakeとローカル内部testで網羅する。実機で10分に収まらない検査は未検証を残し、追加job/時間を理由とともに本人へ提案する。予算を黙って増やさない。
- transport再送はjob期限内の最大3回、指数backoffとRetry-Afterを優先。通信再送とAI再起動を区別する。本人の金銭上限は未設定で、実契約・費用見積りとともに本人が決める。
- 停止：新規claimを止め、outboxを凍結し、running/unknown jobをneeds_reconcileとして残す。起動済みAIを自動killしない。継続/取消はローカル指揮者と本人が元runを確認して決める。
- M撤収：試験購読をunsubscribe、event配送停止を確認。一時plugin接続の解除は本人が必要性を確認して行う。
- S/H撤収：専用receiverと投稿workerを停止。変更前のSocket Mode、Event Subscriptions、Request URL、scope/会話参加の記録へ戻す。一時権限だけを対象にし、既存dot連携を削除しない。app/credential削除は本人の別操作。
- queue撤収：全jobの終端/保留理由をexportし、secretを含まない結果を固定保存。未配送outboxを隠して捨てない。試験DBの保持期間と削除は本人が判断する。

## 10 採用判断と引き継ぎ

| 判断対象 | 暫定候補／役割 | 共通障害・残る実装 | 本人が決める事項 |
|---|---|---|---|
| 主経路 | MかS。両者の同課題結果・UXで選ぶ。公式根拠上はMが有力だが未決定 | queue、PC/Hub、auth、dotsの同job続行。bridgeとlocal workerが必要 | 設定負担、待ち時間、会話UX、費用を受け入れるか |
| 補助経路 | 主経路以外のM/S、必要ならH。部分成立なら手動回収用に限定 | 別入口でも共有queue/認証/PC/dots障害を代替しない。route_epochとexecution_epochの分離/状態照合が必要 | 自動切替を許可するか、手動で止めて回収するか |
| GitHub記録 | 指示・成果・レビュー・検収の固定記録 | 原則起動を担当しない。Webhook/poll追加は実益と起動根拠がある場合だけ | 記録先と公開可能な証跡範囲 |
| 追加実装 | §3の新規bridge、adapter、fake tests、operator guide | Go基準と稼働版差、認証・保存、単一writer。製品内部APIを作った前提にしない | 実装対象branch/範囲、設定/送信/Build/実機試験の許可 |
| 全候補未成立 | route固有とqueue/auth/PC/dotsの共通原因を分離 | 条件待ちはfailedと区別。入口を増やして共通障害を隠さない | 条件整備、設計変更、暫定手動運用の継続 |

合格候補が複数でも、同一案件が別dots会話へ切り替わって安全に続くか未検証なら自動failoverを採用しない。queueの結果を人が照合する補助経路として残せる。一往復の成功だけで常設運用・C3着手を自動許可しない。

次の担当が読む順序は、PROGRESSの現行方針 → 固定d533a72 → 本書 → FINDINGSの基準ソース → PUBLIC_RESEARCHのR1/R2/R3/R6/R8 → 固定成果headのdiff。最初の操作は**ローカル指揮者が対象headと稼働版を照合し、本人の追加許可範囲を記録すること**。許可前に設定や試験を始めない。

暫定運用は本人がdotsの返信通知に気付き、ローカルAIへ「#5の進捗を見て」と伝える。本文転記は不要で、ローカルAIが認可済みSlack/GitHubを取得する。このセッションに自動通知・定期監視・自動起動はまだない。新しいローカルセッションへ渡す際は固定head、candidate/job台帳、未確認条件、次の操作、許可範囲を引き継ぐ。

[比較HTML](comparison.html) / [FINDINGS](FINDINGS.md) / [公開実践例](PUBLIC_RESEARCH.md) / [PROGRESS](PROGRESS.md) / [Draft PR #8](https://github.com/ishizakahiroshi/many-ai-cli/pull/8)
