# C2 offline prototype 実装契約

> 最終更新: 2026-10-03(土) 20:02:48 JST

## 固定基準と範囲

コード基準: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`。
実装指示: [6698089](https://github.com/ishizakahiroshi/many-ai-cli/blob/66980896680bdab7d5a746ceae7f3f379a4143b8/docs/bot/dots-local-ai-bridge/IMPLEMENTATION_OFFLINE.md)。
試験設計: [d5e0d02](https://github.com/ishizakahiroshi/many-ai-cli/blob/d5e0d022490d55ef26b446f5317bba06be5798b4/docs/bot/dots-local-ai-bridge/C2_TEST_PLAN.md)。
設計PR #8を変更せず、コード基準から独立branchを作成。既存Hub/API/認証/DB、provider、Rust移植、Makefile、workflowを変更しない。
本書の契約をoffline prototypeへ実装した。現状は [実装看板](OFFLINE_PROGRESS.md) を参照。

## 永続化と状態契約

専用SQLiteにjobs、inbox、logical_events、launch_intents、routine_locks、outbox、quarantine、budgetsを保持する。状態変更と意味イベント/論理outboxは同一transaction。受信ACKはcommit後だけ。source/eventの再送とjob/logical_messageの意味重複を別々に記録し、同じIDの異なるsemantic payload hashはjobをneeds_reconcileへ止める。

job期限は作成から最大10分。campaign/candidateごとの累積job上限2、transport試行最大3。新規claim/launch/deliveryは期限・停止フラグ・完全一致leaseを確認する。lease期限だけで別ownerへ移譲しない。再起動で予算やunknown runは消えない。

route_epochは配送先の世代、execution_epochはworker所有権の世代。明示route切替はrun/request/execution epochを保ち、元runの正当な結果を現routeの同じlogical outboxへ一度だけ載せる。送信前にattemptを永続化し、crash/応答喪失はneeds_reconcile、別routeへ自動再送しない。物理配送のexactly-onceを主張しない。

## mailbox取得の厳密な契約

1. workerがexact lease、期限、専用routine lockを確認する。launch intent、累積起動予算、安定request_id（job/question/execution世代由来）、32-byte乱数dispatch nonce、staged inputをPOSTより前にcommitする。固定input.jsonは公開しない。
2. Hub POSTには既存契約のrequest_idだけを送る。応答またはGET一覧/詳細からHub instance、routine、canonical request、run、sessionを照合する。active alias、異なるrequest、session不足、不明runはneeds_reconcile。応答喪失時はGETでのみ回収し、再POSTしない。
3. staging取得には、nonceだけでなく、trusted consumer verifierが認証した同じHub instance/request/run/sessionのconsumer capabilityが必要。nonceはそのverified runに結び付けて一度だけ原子的に消費する。結果もjob/question/execution/request/run/session/instance/nonceの全てを照合する。
4. offline版はfake Hub内部で発行するopaque consumer capabilityをmock CLIだけに渡す。任意の自己申告session/run/nonce文字列は認可にならない。実Hub既存HTTPにはこのconsumer attestation APIを足さない。将来live adapterには既存wrapper sessionを実runへ確実に結び付けるローカルbroker検証が必要で、その実装・実機検証が終わるまでlive拒否を維持する。
5. finishedかつresult_available=trueでも、古い/別run、前job、旧execution owner、部分/過大JSON、期限後の結果は配送しない。bounded quarantineへ証跡を保存し、期限後の正当な回答も照合待ちに保持する。起動済みAIをkillする機能は持たない。

## 実装段階

queue/state → fake Hub/worker/mailbox → fake M/S/CLI → crash/negative tests →独立固定SHAレビュー。
新依存は追加しない。fake transportは実MCP/Slack protocol適合やdots同job続行を証明しない。実機UX、課金、購読、設定、公開relay、常設監視、実AIは未検証・未実施。

## 実装済みの範囲と意図的な制約

- `cmd/dots-bridge-c2` は独立binary。`help`、`trial --mock`、`status`、`stop`、`export`だけを提供する。既定実行はhelpで、`live`/`relay`/`local`/`--live`はDBを開く前に拒否する。秘密やURLを受け取る設定、provider実行、socket/HTTP clientは無い。
- 共通状態機械と専用SQLiteは実装済み。ファイルのapplication_idがbridge専用であることを既存DBのschema変更前に確認し、無関係なSQLiteを拒否する。HubのDBを指定しない。
- 一つのjobに一つの固定color question/run/answerを扱う最小prototype。campaign/candidateごとにcolor/recoveryの2jobまで。キャンペーンを指定するのはoperatorで、受信eventから新campaignや任意taskを作らない。途中のtrialを同じCLIで再実行してもAIを再起動しない。状態不明はexportして保留する。
- 初回のquestionと相関するblocked/completedのみ適用する。受信の順序逆転は隔離し、自動replayしない。完了には論理answerの配送確定が必要。
- `HubClient`は既存POST/list/detailを表すinterface。live HTTP実装は無い。`ConsumerVerifier`は追加Hub APIではなく将来のローカルbroker境界。fakeではHub内部発行のopaque capabilityをmock CLIに渡し、GET再照合後にstaged dispatch nonceを原子的に一回消費する。session文字列やnonceだけを申告する取得経路は無い。
- routine lockは専用DBで保持。lease失効による自動奪取/再POSTなし。terminal runをGET照合した明示fencingは旧execution ownerを無効にするが、jobは保留のまま。自動再投入/実AI停止を実装しない。
- outboxは送信intentのcommit後だけfake送信を行う。確実に未送信のときだけ最大3attempt、指数backoffとRetry-Afterを適用。ACK喪失・DB ACK保存失敗・途中crashはneeds_reconcile。異経路への盲目的な再送なし。
- quarantineの本文は専用DBに最大8192 bytesずつ保存する。exportはhash/理由だけを出し、raw sample、lease token、dispatch nonce、staged inputを除く。実トークンを入れる入力欄/認証設定は無い。既存同一OSユーザーの信頼境界は変更していない。

## 正確な再現コマンド

Go 1.26.8。repo root、未使用の専用DBで行う。既存依存以外は不要。通常製品のBuildは従来どおり `make build`。以下は今回個別許可されたprototypeのみで、web/distは参照しない。

```sh
go test ./internal/dotsbridge/... -count=1
go test -race ./internal/dotsbridge/... -count=1
go test ./cmd/dots-bridge-c2 -count=1
go vet ./internal/dotsbridge/... ./cmd/dots-bridge-c2
mkdir -p out
go build -o ./out/dots-bridge-c2 ./cmd/dots-bridge-c2
./out/dots-bridge-c2 help
./out/dots-bridge-c2 trial --mock --db ./out/comparison.db --campaign offline-20261003 --path mcp --case color
./out/dots-bridge-c2 trial --mock --db ./out/comparison.db --campaign offline-20261003 --path socket --case color
./out/dots-bridge-c2 trial --mock --db ./out/comparison.db --campaign offline-20261003 --path mcp --case recovery
./out/dots-bridge-c2 trial --mock --db ./out/comparison.db --campaign offline-20261003 --path socket --case recovery
./out/dots-bridge-c2 status --db ./out/comparison.db
./out/dots-bridge-c2 export --db ./out/comparison.db > ./out/comparison-export.json
./out/dots-bridge-c2 stop --db ./out/comparison.db
./out/dots-bridge-c2 live
# 最後だけ想定exit 2: offline_only。それ以外はexit 0。
```

Windowsではbinary名に`.exe`を付け、出力directoryを対応shellで作る。停止はDBに永続し再開コマンドは無い。比較を新たに始める場合は、まず既存DBをexportして照合し、operatorが別の専用DB/試行を決める。未知runを捨てて新DBに切り替える復旧として使わない。実AI費用の上限はこの10分deadlineでは保証されない。

## 負例・クラッシュの証拠

`bridge_test.go` は固定10テストに加えて、保存失敗POSTゼロ、ACK前DB失敗/ACK喪失、同logical ID異hash、部分/過大JSON、manual run割込み中の未認可取得ゼロ、session再利用、POST応答喪失GET、期限後の遅い回答保持、DB再open、二つのDB接続からのowner競合、input一回取得競合、配送ACK保存失敗、3attempt/backoff、stopped永続性を検査する。
障害はtransaction commit直前とPOST/resultの前後に明示注入し、専用DBをclose/openする。実PC電源断、実Hub、実CLIの強制終了試験ではない。mock CLIはGo関数として同一processで実行し、実provider subprocessは起動しない。

## 次のliveゲート

1. ローカル指揮者が対象SHA、稼働版、Windows/Go条件と内部テスト/Buildを照合する。
2. 実consumerを既存wrapper session＋Hub instance/run/requestへ結び付けるbrokerを、同一OSユーザー境界内で設計・実装・検証する。自己申告IDだけでliveを有効にしない。
3. 実MCP EventsとSlack Socket Modeのadapter、アカウント/app/購読、対象会話、秘密管理、配置先、費用上限を本人が別途承認する。Hは必要性が明らかな場合だけ。
4. M/S同じ合成課題の実同job続行、7区間時刻、UX操作数、停止/再接続、費用を別々に測る。fakeの合格から本番対応/主経路採用/常設監視を宣言しない。

`.omitnix/index.json` は基準branchの生成物（omitnix 0.1.3、生成commit 51f37cb）を保持。検証済みgeneratorがこの環境に無いため未再生成。手書きで生成完了に見せず、ローカル指揮者が通常の `omitnix` と `omitnix --check` で確認する。
