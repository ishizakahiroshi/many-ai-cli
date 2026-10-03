# dots とローカルAIの連携方式 調査結果

> 最終更新: 2026-10-03(土) 18:57:54 JST

## 結論

**C2はMCP tools＋MCP EventsとSlack Socket Modeの両方を同じ二つの合成課題で比較する。MCPが成功してもSocketの比較を終了しない。** PC停止中の受付またはSocket固有制約が重要ならSlack HTTP＋中継を加え、主経路と補助／手動復旧経路を選ぶ。全7方式の実装・常設は求めない。

OpenAIの公開仕様がdots対応を明記するMCP Eventsは調査上の有力候補。Socketには公開ローカルbridgeの実例があるが、別Botの投稿をdotsが受理し同jobを続行するかは未確認。どちらも実環境の合格・採用済みではない。通知・保存・AI起動を分け、二入口は共通job/inbox/outboxへ集約する。

現在の選定方針と具体的な試験手順は [C2_TEST_PLAN](C2_TEST_PLAN.md)。[固定追加指示d533a72](https://github.com/ishizakahiroshi/many-ai-cli/blob/d533a72f66752d1f9e97a40d4ccba468c65e4f6f/docs/bot/dots-local-ai-bridge/SUPPLEMENT_MULTI_PATH_PLAN.md)が、旧「順位順に試して最初の成功で採用」の方針を置き換える。以下の1〜7位は調査上の評価優先度で、試験終了条件や全方式の実装命令ではない。GitHubはまず指示・成果・レビューの記録を担う。

## 公開実践例を追加した判断

2026-10-03にX・Zenn・Qiita・note・技術ブログ・公開GitHubを日本語と英語で調査し、本文・コードへ辿った8事例を [PUBLIC_RESEARCH](PUBLIC_RESEARCH.md) にまとめた。追加指示は [固定0f14ba5](https://github.com/ishizakahiroshi/many-ai-cli/blob/0f14ba574aaf6e00bacaa92744f2db1e55845345/docs/bot/dots-local-ai-bridge/SUPPLEMENT_PUBLIC_RESEARCH.md)。検索日・検索語・公開日・版・認証・費用の不明点・採否を同資料に記録する。

**2026-10-03の公開調査時点では評価順位を維持。現在のC2選定手順は上記の複数候補比較へ更新。** ローカルClaudeでのSocket→resumeには著者報告と公開コードがあるが、通常messageでBotを除外する実装もあり、dotsとの無人一往復を証明しない（R1/R2）。MCP Eventsはdotsへの公開製品適合が第一位の理由で、導入実績の量による一位ではない。PC停止中の受付を優先する場合は、引き続きSlack内で3位HTTP＋queueを2位Socket localより先に評価する。

採用を決める前に保存の粒度を見る。SQLiteにthreadとsessionを残しても待ちjobがメモリ内なら再起動で失われ得る（R2）。R2のSlack返信はfresh headless resumeであり、実行中TUIへ注入できない。TUIとSlackの同時書込みは会話を分岐させるため、既存sessionを使う設計では書込み担当を一つにする。MCPツールのsocket待ちは動作中呼出しへの返却であり、プロセス終了後の起動機構ではない（R3）。n8nはDB待機・認証付き再開URLを提供する中継部品候補（R6）だが、dots起動・PCへの取得・Hub起動adapterは別途必要。Claude Channels（R5）もローカル入力adapterの別案として扱い、7候補へ混ぜて順位を水増ししない。

GitHubの過去の再帰課金報告（R7）は現行のbot制御と照合し、現行版の再現性は未確認とした。MCP Eventsの公開demo（R8）は実験版・メモリ保持で、dots製品統合の実証ではない。QiitaのClaude Slack実測（R4）はクラウドの別製品。Xに有力原典を見つけられず、noteの2記事は取得エラーだったため、全媒体を網羅したとはしない。

## 範囲と証拠

- 確認日：2026-10-03。指示commit：`c2822a8f39a18809bca8fe9484f1c84bd387f64b`。ソース基準：`21d0bc7935a2c4696fb89ccff2e324157a528c2d`。
- 固定指示commitは上記基準に文書と索引だけを追加。ソース比較は基準SHAの固定リンクで示す。別件Rust版との統合は未確認。
- **公式仕様**＝公開文書に存在する機能。**ソース確認**＝読んだコードの経路。**実測**＝実行結果。本C1の通信実測は0件。**未確認**＝環境・権限・製品動作のゲート。
- 対象端末のファイル、非公開docs、秘密は取得しない。新しいapp／scope／webhook、外部試験送信、AI・Hub起動、実装・配備は行わない。

## 既存Hubで確認できたこと

| 論点 | ソース確認とC2への含意 |
|---|---|
| 新しいAIを起動 | routine runは定義済prompt・cwd・provider・modelをコピーし、spawnRoutineSession→spawnWrappedSessionへ進む。新規session。既存sessionの会話再開APIと同一ではない。 |
| 手動起動口 | `POST /api/routines/<id>/runs` は request_id 必須（最大128 bytes）、triggerはmanual。外部event専用endpointではない。bridgeが認証して変換する実装は未作成。 |
| 認証とbind | guardはtoken・method・Host・変更系Origin確認、必要時remote PIN。listenはloopback。Hub公開やtoken権限モデルの変更をせず、ローカルbridgeから呼ぶ構成を提案する。tokenはOSユーザー相当として扱い、中継に転送しない。 |
| 認可とcwd | validateRoutineは絶対プロジェクトcwd・directory実在・provider/model/promptを検証。spawn側でcmd.Dirに反映。既存CLI認証・permission設定と承認待ちは別途実機確認。無人完走や承認迂回は保証しない。 |
| 重複 | routine ID＋request_id、および同routineのactive runをチェック。active時は別requestでも既存runを返し、新しいrequest_idをそのrunへのaliasとして永続保存する。そのIDの再試行は終了後も同じrunへ戻る。bridgeは専用routineがactiveの間はdispatchしない。衝突して無関係なrunが返ったら対応不一致として停止・照合し、同じIDの再試行や無条件のID再発行で解決しない。 |
| 保存と結果 | routines.jsonへatomic保存してから実行を開始。run履歴・結果がある。finishedは終了信号で、要求全成功ではない。承認待ち等はwaiting。 |
| 停止と再起動 | 起動時・遅れた時刻のscheduleをskip。失われたsessionのpromptを再実行しない。labelで生存sessionを再関連付けするのは観測復元で、終了AIの再開ではない。 |
| 既存sessionへの回答 | routine経路に任意job回答を同じAI会話へ戻す専用契約は見当たらない。cross_session_message.goは観測記録でroutingではない。他の入力・provider再開経路まで含む「Hub全体で不可能」とは断定しない。C2ではまず新規の合成runを使う。 |

根拠（すべて基準SHA、読取のみ。テストは今回実行していない）：
- [README routines](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/README.md#shared-instructions-and-routines)
- [routine_handlers.go L22–42](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/routine_handlers.go#L22-L42)
- [routine_runner.go L12–73 / L118–190](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/routine_runner.go#L12-L190)
- [routine_runner.go L224–314](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/routine_runner.go#L224-L314)
- [routine_store.go L123–207](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/routine_store.go#L123-L207)
- [http_helpers.go L435–493](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/http_helpers.go#L435-L493)
- [server.go L1617–1644](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/server.go#L1617-L1644)
- [spawn_handler.go L176–241 / L298–334](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/spawn_handler.go#L176-L334)
- [routine_test.go](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/routine_test.go)（重複、保存失敗、skip、再関連付け、認証、結果未確定等の意図を確認）
- [cross_session_message.go L11–56](https://github.com/ishizakahiroshi/many-ai-cli/blob/21d0bc7935a2c4696fb89ccff2e324157a528c2d/internal/hub/cross_session_message.go#L11-L56)

## 通知と起動と状態保存

1. 受信成功やHTTP 2xxは、AI起動・回答・再開・完了の証拠ではない。
2. 常駐するのは軽い受信プログラム。新しい許可済jobがある時だけAIを起動し、空振り巡回で毎回AIを起こさない。
3. queueは受付済jobを保存し、実行lease・request_id・返信IDを追跡する。Slack/GitHub履歴は人が読む記録として有用だが、実行キューのtransaction保証にはならない。
4. PC電源断ではどの方式もそのPCのAIを実行できない。常時中継が残せるのは依頼と状態。復帰後の再処理は重複除去・期限・認証の確認を通す。
5. 「停止」は業務状態（blocked/cancelled/needs_approval）通知と、プロセス強制終了を区別する。C2の停止通知試験は状態通知だけ。終了操作は別承認・別設計。

## 方式比較 調査上の評価優先度

### 1位 専用HTTP / MCP 通知口

推奨は MCP tools ＋ MCP Events ＋ 永続キュー。**公式仕様あり・一往復未実測**。

- 往路：dots の質問・停止・完了 → MCP tool → 専用中継で保存 → PC受信プログラムが取得 → Hub認証 → ローカルAIを必要時だけ起動
- 復路：ローカルAIの依頼・回答 → 中継が保存 → MCP Events callback → dotsの購読チャットで処理・再開 → 完了をtoolで中継 → PCが受信
- 利点：dotsでのイベント反応に公開製品根拠がある。Slack bot同士の受信条件に依存せず、構造化IDと状態を扱いやすい。
- 弱点：新しいplugin・認証・常時中継・購読管理が必要。一般のHTTP POSTやMCP通知だけではこの経路にならない。
- 設定・権限・常駐：認証付きMCP 2.0 endpoint、toolsとevents/list・subscribe・unsubscribe、callback検証、購読永続化。公開口は中継側だけ。本人のplugin接続・イベントtask許可をC2前に確認。
- 切断・PC停止：中継が受付済みの依頼はPC停止中も保持可能。PC再接続までローカルAIは起動しない。期限切れ・順序逆転・重複はアプリ側で処理。
- 費用・負荷：導入・運用は高め。中継compute/storageとAIの実行分を分離。通知を受けただけで追加AIを無制限に起こさない。
- 条件・除外：公開仕様は対応を確認。本アカウントでのplugin登録可否、実際の購読、質問後の同じjobへの再開、Hubとの往復は未確認。
- 根拠：[O1](#o1), [O2](#o2), [O3](#o3), [O4](#o4)

### 2位 Slack Events API / Socket Mode

PCから外向きWebSocketで受ける。**配送仕様あり・dots受信ゲート未確認**。

- 往路：dots の質問・停止・完了 → 専用Slack会話に保存 → Socket受信プログラムがACK・保存 → Hub認証 → ローカルAIを起動
- 復路：ローカルAIの依頼・回答 → chat.postMessage → 同一thread → dotsの受信・再開条件は未確認 → 完了 → Slack → Socket受信
- 利点：公開HTTP入口不要。PCが稼働する小さな試作に向く。Slack会話を人も読める。
- 弱点：別Bot投稿でdotsが起動するかが最大のゲート。接続refreshと欠落補完が必要。Socketは保存装置ではない。
- 設定・権限・常駐：bridge app、app-level connections:write、参加会話のhistory scope、chat:write。botの会話参加とdotsとの共存。ローカル受信プロセスは常駐。
- 切断・PC停止：PC上だけのreceiverは停止中受信不可。再接続＝全件replayの保証は確認できず、履歴との差分照合が必要。cloud receiver＋queueなら停止耐性を追加できる。
- 費用・負荷：初期は中、常駐PCと再接続運用が必要。Slack契約・app枠・LLM枠は別。API無料／LLM無制限とは断定しない。
- 条件・除外：Slackのbot event表現は確認。dotsが別Botの本文を依頼として扱うか、mentionなしthread返信の回収、補完可能期間は未確認。
- 根拠：[S1](#s1), [S2](#s2), [S4](#s4), [S5](#s5), [S6](#s6), [S7](#s7), [S8](#s8)

### 3位 Slack HTTP Events ＋中継

常時受信と永続化をPCから切り離す。**配送仕様あり・中継未構築**。

- 往路：dots の質問・停止・完了 → Slack保存 → HTTPS Events → 中継が署名検証・queue保存・ACK → PC受信 → Hub認証 → ローカルAI
- 復路：ローカルAIの依頼・回答 → chat.postMessage → 同一thread → dots再開はSocket案と同じ未確認 → 完了 → 中継保存 → PC受信
- 利点：PC停止中の受付を維持しやすい。永続queueで受理後の再試行・監査・dead-letterを設計できる。
- 弱点：HTTPS公開口、署名、ストレージ、監視が増える。Socketから変えてもdotsのbot拒否条件は解決しない。
- 設定・権限・常駐：Slack app/event設定、signing secret管理、中継HTTPS、queueとPC outbound取得。scopeはSocket案と会話種別に応じた同等範囲。
- 切断・PC停止：中継受理分を保持し、PC復帰時に取得。Slackの未配送分は別問題。PCへのtunnelだけでは停止耐性は得られない。
- 費用・負荷：導入・運用は高。queue保持量・egress・監視費とSlack/AI契約が別。低遅延候補だが実測値なし。
- 条件・除外：HTTPは3秒以内ACK、通常最大3回retry。Delayed Eventsの24時間毎時retryもbest effortで永続保証ではない。
- 根拠：[S2](#s2), [S3](#s3), [S7](#s7), [S8](#s8), [S12](#s12)

### 4位 GitHub Webhook

Issue / PR会話を記録の正本にする。**配送仕様あり・dotsのevent購読未確認**。

- 往路：dots が質問・停止・完了をcomment → GitHubが保存 → issue_comment → HTTPS中継が検証・queue保存 → PC受信 → Hub認証 → ローカルAI
- 復路：ローカルAIが依頼・回答comment → GitHub保存 → dots側のevent購読等 → dotsの起動・再開は別途必要 → 完了comment → webhook → PC
- 利点：コメントURL・IDが証跡になる。Slackの会話範囲に左右されず、コード案件と対応づけやすい。
- 弱点：GitHub webhook自体はdotsを起こさない。接続済pluginが対象commentを起動条件にできるか未確認。新規受信口と管理権限も必要。
- 設定・権限・常駐：repo webhook管理権限、またはGitHub App Issues:read購読。comment読書きはIssuesまたはPull requests権限。HTTPS中継、署名secret、10秒以内2xx。
- 切断・PC停止：受理済queueとGitHubコメントから復旧。GitHubは失敗配信を自動再送しないため、再配信・差分照合を設計する。
- 費用・負荷：中～高。中継運用とAPI上限あり。Actionsを中継にする場合は別の実行枠・設計が必要で本案に暗黙追加しない。
- 条件・除外：issue_commentはPR会話コメントも対象。inline review commentは別event。gh webhook forwardは開発・試験専用でproduction非対応。
- 根拠：[G1](#g1), [G2](#g2), [G3](#g3), [G4](#g4), [G5](#g5), [G8](#g8)

### 5位 GitHub 軽量ポーリング

AIを起こさない小さな差分取得プロセス。**差分取得仕様あり・自動往復未確認**。

- 往路：dots が質問・停止・完了comment → GitHubに保存 → PCが対象Issueの差分を定期GET → 新しいjobのみ保存 → Hub → AI
- 復路：ローカルAIが依頼・回答comment → GitHubに保存 → dots側の許可済起動経路が必要 → 完了commentを次回GETで回収
- 利点：公開受信口不要。GitHubに残る履歴からPC復帰後に追いつける。限定された案件では実装が小さい。
- 弱点：検知は巡回間隔分遅れる。PC側pollだけではdotsを起こせない。削除・非公開化・権限変更で復元できない。
- 設定・権限・常駐：単一Issue/PRのcomments API、最小read/write権限、ETag/If-Modified-Since、ページング、comment IDのwatermark、429/403 backoff。
- 切断・PC停止：停止中はGETも実行も不可。復帰後の可視コメントを照合する。編集の扱いと古い未処理jobのTTLを決める。
- 費用・負荷：初期低、運用中。通常認証5000 requests/h。認証付き304はprimary枠を消費しないがsecondary制限等は残る。
- 条件・除外：GitHub公式はwebhook優先。これは公開口等を使えない場合の代替。dots側を毎回AI巡回させる設計は採用しない。
- 根拠：[G5](#g5), [G6](#g6), [G7](#g7)

### 6位 Slack 軽量ポーリング

小さな専用会話の補完・代替。**取得仕様あり・対象権限とdots受信未確認**。

- 往路：dots の質問・停止・完了 → Slack会話とthreadに保存 → PCがhistory＋対象repliesをGET → 新しいjobのみ保存 → Hub → AI
- 復路：ローカルAIが依頼・回答を投稿 → Slack同一threadに保存 → dotsの別Bot受信条件は共通 → 完了をhistory/repliesで回収
- 利点：公開受信口やSocket接続維持が不要。既存会話の補完には使いやすい。
- 弱点：historyだけでは古い親への返信を拾えない。保持期限・app分類・会話数で復旧と遅延が変わる。bot拒否を回避できない。
- 設定・権限・常駐：historyとrepliesの対象を固定し、会話種別のhistory scope＋chat:write。thread_ts・ts・pagination・取得watermarkを保存。
- 切断・PC停止：復帰後にアクセス可能で保持中の履歴のみ補完。削除済／保持期限外は復元保証なし。本人とdotsの既存DMを第三Botが読めると仮定しない。
- 費用・負荷：内部専用appは現行Tier 3（50+回/分）。Marketplace外の新規商用配布等は1回/分・15件の制限対象。method×workspace×appで評価。
- 条件・除外：対象app分類、tokenでの実際のthread可視性を確認。一律user token必須／全app1回毎分とは断定しない。
- 根拠：[S7](#s7), [S8](#s8), [S9](#s9), [S10](#s10), [S11](#s11), [S13](#s13)

### 7位 A2A エージェント間通信

タスク状態の意味を標準化する候補。**標準仕様のみ・製品対応未確認**。

- 往路：dots側A2A agent ※未確認 → A2A task / stream / push → 受信bridgeがtask状態を保存 → Hub認証 → ローカルAIを起動
- 復路：ローカルAIの回答・状態 → 同じtaskId / contextIdへmessage → dots側agentの処理・再開 ※未確認 → completed等 → 受信bridge
- 利点：質問待ち・継続・完了をtaskとして表しやすい。複数実装の相互運用が目的なら有用。
- 弱点：両端のAgent Card・対応binding・認証が必要。dotsや現HubがA2A対応と示す根拠を確認できず、今回の最短経路ではない。
- 設定・権限・常駐：A2A 1.0対応両端、Agent Card、認証、task store。pushNotifications capabilityが必要。本提案は公開HTTPS callbackを採用条件とする（仕様のHTTPSはSHOULD）。
- 切断・PC停止：Get Taskによる照合や再購読は可能な設計だが、保持・replay・PC復帰時の処理は実装依存。規格だけでPCは起きない。
- 費用・負荷：導入・運用は高。protocol自体の利用枠とLLM／hosting料金は別。接続先製品の料金は未確定。
- 条件・除外：現行release 1.0.0を確認。input-requiredへの追加入力とtask相関は規格。dotsのendpoint・Agent Card・サポートは未確認。
- 根拠：[A1](#a1)

### 専用HTTPと通常MCP通知の境界

第一候補のMCP Eventsは、通常MCPの進捗／resource変更通知とは別の製品統合である。現行MCP 2026-07-28のStreamable HTTPはrequest-scoped SSEとsubscriptions/listenを使い、旧版のGET stream・protocol session・Last-Event-ID再開は現行版にない。旧2025-11-25前提の説明を流用しない。MCPのメッセージを届ける仕様だけから、休止中のdotsが任意の外部通知で起動するとはいえない [O2]。

OpenAIのMCP Eventsはplugin登録・認証・購読が前提で、webhookとcallback verificationをサポートする。draftにあるpoll/stream/gap/terminatedの全てが製品対応ではない。HTTP 2xx後は非同期処理であり、特定job再開・承認なしの実行を保証しない [O1/O3]。

任意URLへの直接POSTはdots側の公開された送受信契約が確認できた場合のみ候補。localhost URLはクラウドからそのPCを指さない。Workspace Agentsの公開trigger APIは別製品のpublished agent向けで、dots本人のendpointと読み替えない（[公式製品ページ](https://developers.openai.com/workspace-agents)）。

### Slackの落とし穴

- 専用の共有channelを想定し、既知channel ID・thread_tsを固定する。本人とdotsの既存DMに第三Botを入れられるとは仮定しない。
- app_mentionはDMで配送されない。bridge自身のmentionしか購読しないと、dotsの普通の返信を逃す。会話message eventと参加条件を確認する [S4/S5]。
- private channelならgroups:history、bot宛DMならim:history、MPDMならmpim:history等、対象に対応する最小scopeを選ぶ。不要な全channel投稿権限や本人tokenへの置換は提案しない。
- bot_message subtypeだけに依存せず、現行appのbot_id等も識別する。LLM由来への応答を避けるSlack推奨は配送禁止の証明ではないが、dotsのbot受信を未確認とする理由になる [S6/S8]。
- 現行history/repliesのinternal appはTier 3。Marketplace外の新規商用配布app等に1回/分・15件制限がある。app分類を確認する。最新repliesのBot token欄にhistory scopesがあり、古い知識だけでuser token必須としない。個別tokenの取得可否はC2 [S9/S10/S11]。

<a id="c2-一往復の試行計画-未実施"></a>

## C2 複数経路の比較と検収 未実施

正本は [C2_TEST_PLAN](C2_TEST_PLAN.md)。C1文書の完了は実装・接続実験・設定・Build・起動・外部試験送信の許可ではない。次の担当はPROGRESS、固定d533a72、本試験計画、基準ソースの順で読み、稼働版と追加許可を確認する。

| 段階 | 実施する比較・合格条件 | 担当 |
|---|---|---|
| 条件確認 | M/Sのplugin/app利用可否、sender・会話・認証、PC/Hub/CLI/cwd、費用上限。未提供はcondition_wait | 本人＋ローカル指揮者 |
| 同課題比較 | color/recoveryの2fixtureをMとSへそれぞれ投入。依頼→質問→保存→Hub新規AI→回答→dots同job続行→完了回収。先に成功しても比較を続ける | ローカル指揮者が実機検収、dotsが許可後の実装とmock tests |
| 複数入口 | 同一報告、順序逆転、自己返信、主経路停止、復帰、途中crash。共通job/logical_message、inbox/outbox、launch intentを照合 | dotsの自動test＋ローカル指揮者の内部test/実機確認 |
| 採用判断 | 主経路、補助/手動回収、GitHub記録を決める。queue/auth/PC/dots共通障害では別入口も代替にならない | 本人。独立レビュー済み証跡を参照 |

Hub既存POSTはrequest_idだけを受け、動的promptは渡せない。新規提案のlocal workerが専用cwd内の合成mailboxを用意し、固定manual routineが読む契約とする。既存GETのrun一覧/詳細で相関する。active中はdispatchを止め、別runへのrequest_id aliasや応答喪失はneeds_reconcile。別IDを増やして再起動しない。AIのauth/connector/cwdを現在の会話から継承する前提にも立たない。

live tool待ち、既存session再開、fresh run、dots同job続行は別の証跡。最小ローカル試験はfresh run。既存session案を後で採用する場合はTUI/bridgeの単一writerを別検証する。session対応表が残るだけでは未処理job復旧の合格にしない。

予算は2合成job/候補・各10分・同event重複1回を基準とし、M/Sで計4件、H追加時は計6件まで。網羅的障害検査はまずfakeで行い、実機で収まらない項目は未検証を記録して追加許可を得る。本人の金銭上限は勝手に決めない。停止時は新規claimとoutboxを止め、起動済みrunを照合し、購読・一時設定を対象範囲だけ撤収する。

暫定運用は本人が返信通知を見てローカルAIへ「#5の進捗を見て」と伝える。ローカルAIが認可済みSlack/GitHubを取得し、本文転記は不要。自動通知・定期監視・自動起動は未導入。検収結果は固定head・候補別ログ・未確認条件を許可済み経路でdotsへ返す。

## 費用の比較方法

通信回数、常時receiver/queue、保存量と保持日数、PC稼働、dots/CLIのAI実行を分けて見積もる。具体的な月額はprovider・プラン・event量未指定のため未算定。pushでも無関係なeventでAIを起こせば高くなる。pollでも軽い差分GETだけならAI枠を消費しない設計にできる。SlackやGitHubの上限は「無料で無制限」の根拠にはならない。dotsの利用枠は現在の契約条件をC2時に確認する [O4]。

## 公式資料一覧

全件2026-10-03に公式Web資料を確認。版のないWeb文書は同日の記述。動的ページは将来変わるため、C2開始前に対象version・scope・制限を再確認する。

<a id="o1"></a>
- **O1** [OpenAI MCP Events 製品仕様](https://developers.openai.com/plugins/build/mcp-events) — MCP 2.0 / 2026-07-28。dots対応を明記。Events拡張自体はdraft。
<a id="o2"></a>
- **O2** [MCP Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http) — 2026-07-28。通常通知とEvents webhookを混同しない。
<a id="o3"></a>
- **O3** [MCP Events draft](https://github.com/modelcontextprotocol/experimental-ext-triggers-events/blob/main/docs/design-sketch-proposal.md) — Draft proposal。全モードがChatGPT製品で使えるわけではない。
<a id="o4"></a>
- **O4** [dots 公開ヘルプ](https://help.openai.com/en/articles/20001530-getting-started-with-your-dot) — Slack接続・ローカルPC・使用枠。個別環境の許可を意味しない。
<a id="s1"></a>
- **S1** [Slack Socket Mode](https://docs.slack.dev/apis/events-api/using-socket-mode/) — 現行Web仕様。公開受信URL不要、ACK・接続refresh。
<a id="s2"></a>
- **S2** [Slack HTTP と Socket の比較](https://docs.slack.dev/apis/events-api/comparing-http-socket-mode/) — 現行Web仕様。local/productionの選定。
<a id="s3"></a>
- **S3** [Slack Events API](https://docs.slack.dev/apis/events-api/) — 現行Web仕様。3秒ACK、retry、Delayed Events。
<a id="s4"></a>
- **S4** [Slack app_mention](https://docs.slack.dev/reference/events/app_mention/) — DM対象外、参加条件。
<a id="s5"></a>
- **S5** [Slack message.channels](https://docs.slack.dev/reference/events/message.channels/) — 会話への参加とchannels:history。
<a id="s6"></a>
- **S6** [Slack bot_message](https://docs.slack.dev/reference/events/message/bot_message/) — granular botのbot_id等の識別。
<a id="s7"></a>
- **S7** [Slack chat.postMessage](https://docs.slack.dev/reference/methods/chat.postMessage/) — chat:write、thread_ts、投稿制限。
<a id="s8"></a>
- **S8** [Slack security best practices](https://docs.slack.dev/concepts/security/#validate-message-source) — LLM/bot由来への処理・返信を避ける推奨。配送禁止とは区別。
<a id="s9"></a>
- **S9** [Slack conversations.history](https://docs.slack.dev/reference/methods/conversations.history/) — token可視性、app分類別rate limits。
<a id="s10"></a>
- **S10** [Slack conversations.replies](https://docs.slack.dev/reference/methods/conversations.replies/) — thread取得、app分類別rate limits。
<a id="s11"></a>
- **S11** [Slack rate limits](https://docs.slack.dev/apis/web-api/rate-limits/) — method・workspace・app単位、429。
<a id="s12"></a>
- **S12** [Slack request verification](https://docs.slack.dev/authentication/verifying-requests-from-slack/) — 署名、raw body、timestamp。
<a id="s13"></a>
- **S13** [Slack Free 制限](https://slack.com/help/articles/115002422943-Usage-limits-for-free-workspaces) — 履歴・app枠。永続キューとは別。
<a id="s14"></a>
- **S14** [Slack app approval](https://slack.com/help/articles/222386767-Manage-app-approval-for-your-workspace-Manage-app-installation-settings-for-your-workspace) — workspace承認要件。
<a id="g1"></a>
- **G1** [GitHub webhook受信](https://docs.github.com/en/webhooks/using-webhooks/handling-webhook-deliveries) — HTTP受信口・10秒以内応答。
<a id="g2"></a>
- **G2** [GitHub webhook best practices](https://docs.github.com/en/webhooks/using-webhooks/best-practices-for-using-webhooks) — HTTPS、secret、delivery ID、queue。
<a id="g3"></a>
- **G3** [GitHub 配送失敗](https://docs.github.com/en/webhooks/using-webhooks/handling-failed-webhook-deliveries) — 自動redeliveryなし。
<a id="g4"></a>
- **G4** [GitHub CLI転送の用途制限](https://docs.github.com/en/webhooks/testing-and-troubleshooting-webhooks/using-the-github-cli-to-forward-webhooks-for-testing) — 開発・試験専用。production非対応、repo/orgのみ。
<a id="g5"></a>
- **G5** [GitHub issue comments REST](https://docs.github.com/en/rest/issues/comments) — 現行例のAPI version 2026-03-10。read/write権限。
<a id="g6"></a>
- **G6** [GitHub REST best practices](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api) — 条件付きGET、304、backoff。
<a id="g7"></a>
- **G7** [GitHub REST rate limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api) — 認証通常5000/h、secondary制限。
<a id="g8"></a>
- **G8** [GitHub issue_comment event](https://docs.github.com/en/webhooks/webhook-events-and-payloads#issue_comment) — PR会話コメントも含む。inline reviewは別event。
<a id="a1"></a>
- **A1** [A2A 仕様](https://a2a-protocol.org/v1.0.0/specification/) — latestが示したrelease 1.0.0。Agent Card・task・push capability。

## 成果物と検証

- [比較HTML](https://github.com/ishizakahiroshi/many-ai-cli/blob/docs/dots-bridge-c1-comparison-20261003/docs/bot/dots-local-ai-bridge/comparison.html)（取得してブラウザで開く単体HTML。GitHub blobはソース表示）
- [最新PROGRESS](https://github.com/ishizakahiroshi/many-ai-cli/blob/docs/dots-bridge-c1-comparison-20261003/docs/bot/dots-local-ai-bridge/PROGRESS.md)
- [固定指示](https://github.com/ishizakahiroshi/many-ai-cli/blob/c2822a8f39a18809bca8fe9484f1c84bd387f64b/docs/bot/dots-local-ai-bridge/INSTRUCTIONS.md) / [固定Review](https://github.com/ishizakahiroshi/many-ai-cli/blob/c2822a8f39a18809bca8fe9484f1c84bd387f64b/docs/bot/dots-local-ai-bridge/REVIEW.md)
- 文書の静的検証・表示QA・レビューSHA・未実施範囲の最新結果はPROGRESSに記録する。通信実測は実施していない。
