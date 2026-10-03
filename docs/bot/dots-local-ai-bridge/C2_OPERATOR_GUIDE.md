# C2 offline prototype 実装契約

> 最終更新: 2026-10-03(土) 19:39:36

## 固定基準と範囲

コード基準: `21d0bc7935a2c4696fb89ccff2e324157a528c2d`。
実装指示: [6698089](https://github.com/ishizakahiroshi/many-ai-cli/blob/66980896680bdab7d5a746ceae7f3f379a4143b8/docs/bot/dots-local-ai-bridge/IMPLEMENTATION_OFFLINE.md)。
試験設計: [d5e0d02](https://github.com/ishizakahiroshi/many-ai-cli/blob/d5e0d022490d55ef26b446f5317bba06be5798b4/docs/bot/dots-local-ai-bridge/C2_TEST_PLAN.md)。
設計PR #8を変更せず、コード基準から独立branchを作成。既存Hub/API/認証/DB、provider、Rust移植、Makefile、workflowを変更しない。
本書初版は実装前契約。現状は [実装看板](OFFLINE_PROGRESS.md) を参照。

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
