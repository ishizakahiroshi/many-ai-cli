# dots とローカルAIの連携を補強する公開実践例

> 最終更新: 2026-10-03(土) 17:51:00 JST

## 判断に効いたこと

**検証順位は維持する。1 MCP Events、2 Slack Socket、3 Slack HTTP＋中継、4 GitHub Webhook、5 GitHub poll、6 Slack poll、7 A2A。** 8事例を本文・公開コードまで確認した。ローカルClaudeの再開や待機を実装する部品は見つかったが、別Botからdotsを起こし、質問への回答で同じjobを再開する一往復の公開実証は、今回の範囲では見つからなかった。

追加の判断は三つ。セッション対応表の永続化と未処理jobの永続化は別である。動作中のMCPツールを待たせる実装は、終了したAIを起こす実装ではない。既製の中継は受信・保存・再開を減らせるが、dotsの起動口とmany-ai-cliのローカル起動adapterは残る。第一候補は[OpenAIのdots対応仕様](https://developers.openai.com/plugins/build/mcp-events)によるもので、公開導入実績の多さによる順位ではない。

固定追加指示は [0f14ba5 の SUPPLEMENT_PUBLIC_RESEARCH.md](https://github.com/ishizakahiroshi/many-ai-cli/blob/0f14ba574aaf6e00bacaa92744f2db1e55845345/docs/bot/dots-local-ai-bridge/SUPPLEMENT_PUBLIC_RESEARCH.md)。元のソース基準21d0bc7とC1文書限定を維持する。ここにある実験・設定は提案であり、C2は未着手。

## 読み方と調査範囲

- 調査日は全件 **2026-10-03 UTC**。公開日と調査日を分けた。Web仕様に更新日や版がなければ不明とする。
- **公式仕様**は提供能力、**公開コード**は読んだ実装、**著者報告**は第三者の経験、**提案**は本件への適用案。こちらでの接続・コード実行・費用実測は全件未確認。
- 8件は異なる仕組み・障害条件を選んだ事例群。1記事の転載、同一製品の公式説明と紹介記事を独立した成功例として水増ししない。
- 費用は導入前の確認項目を示す。金額や無料枠が資料にない場合は不明。OSSのライセンスとAI・サーバ・Slack等の利用料を分ける。

| ID | 事例 | 証拠 | 本件で使える部分 | 残るゲート |
|---|---|---|---|---|
| R1 | WindowsのSlack→Claude relay | 著者報告＋記事内コード | スレッドとsessionの対応、失敗時の扱い | Bot除外、文脈消失、PC停止 |
| R2 | Rust製claude-slack-bridge | 固定SHAの公開コード | SQLite対応表、同threadの直列処理 | 待ちjobはメモリ、dots非対応 |
| R3 | ask_on_slackと復旧処理 | 固定SHAの公開コード | 実行中ツール待ち、再起動中断の区別 | 生きた接続が必要、再送が残る |
| R4 | QiitaのClaude Slack導入 | 著者報告＋現行公式 | Slack→クラウドAIの実例 | ローカルCLIでもdotsでもない |
| R5 | Claude Code Channels | 公式仕様・公式demo | 開いているCLI会話へイベント投入 | preview、常駐、dotsとは別契約 |
| R6 | n8n WaitとSlack承認 | 公式仕様＋障害報告 | DB待機・認証付き再開URL | URL設定、ローカル配送adapter |
| R7 | Claude Code GitHub Action | 公式コード＋費用障害報告 | comment起動、実行制限の設計 | bot制御、別製品、ローカル配送 |
| R8 | mcpkit Events demo | 固定SHAの公開サンプル | webhook購読・署名・期限の参考 | 実験版・メモリ保持・製品互換 |

## R1 WindowsのSlack relay

出典：[kentaroのZenn記事](https://zenn.dev/kentaro_tak/articles/claude-code-headless-slack-relay)。公開2026-08-23、更新日不明。Windows・Python・Claude Code、版とClaudeの契約／認証方式・月額は不明。Bot OAuthとSocket app token、connections:writeを記載。完全なscope一覧はない。

**著者報告**では、人のDM→Socket常駐→claude -p→同thread返信。thread_tsとsession_idをファイル保存し、追加入力をresumeする。Windowsの複数行引数欠落をstdin渡しで修正し、消えたresume先には文脈なし再試行を採用している。bot_id付き入力は除外する。監視によるプロセス再起動はあるが、PC停止中の未受信job回収は立証していない。

対応候補2。本件ではthread対応・複数行伝達の確認に使う。**文脈なし再試行は同じjob再開の合格にしない**。Bot除外やホームcwdの設定をコピーせず、専用cwd・認可済sender・Hub adapterが必要。[Claudeの公式CLI再開仕様](https://code.claude.com/docs/en/headless)も確認したが、Hub routineがその機能を提供するという意味ではない。

## R2 SQLiteの対応表とメモリの待ちjob

出典：[pomcho555/claude-slack-bridge](https://github.com/pomcho555/claude-slack-bridge/tree/279bf95228c282be0859e416ee9fd1fa0f49dca4)。確認commit 279bf952（2026-09-12）、Cargo版0.4.0、初回公開日不明。Rust・SQLite・ローカルClaude CLI。Slack app/bot token、READMEのhistory・chat:write等のscope、Claude CLI認証が必要。MIT、AI契約／実費は不明。

**公開コード確認**：[store.rs](https://github.com/pomcho555/claude-slack-bridge/blob/279bf95228c282be0859e416ee9fd1fa0f49dca4/src/store.rs)はthread→sessionと状態をSQLiteへ保存。[app.rs](https://github.com/pomcho555/claude-slack-bridge/blob/279bf95228c282be0859e416ee9fd1fa0f49dca4/src/app.rs)は返信をresumeし、同threadを直列処理するが、待ちjobはメモリ内channelであり永続inboxではない。通常message経路はbot_id/subtypeを除外する。PC停止中の配送保証や再起動後の待ちjob復元は、この二つの機構からは証明できない。

[固定READMEのHandoff model](https://github.com/pomcho555/claude-slack-bridge/blob/279bf95228c282be0859e416ee9fd1fa0f49dca4/README.md#handoff-model--read-this)も確認した。Slack返信は新しいheadlessプロセスによるresumeで、実行中TUIへの入力ではない。同じsessionへTUIとSlackの双方が書くと会話が分岐し、thread内の直列workerでもプロセス間の競合は防げない。

対応候補2、候補3のローカルadapter参考。対応表＋未処理inbox＋完了outboxを別々に検証する。コードを読めたことは稼働確認ではなく、dots投稿を受ける既製品とも扱わない。

## R3 質問を待つMCPツールと中断復旧

出典：[tomeraitz/claude-slack-bridge](https://github.com/tomeraitz/claude-slack-bridge/tree/003b0ae8e2a64f49d6d9d563583779b109fedb62)。確認commit 003b0ae8（2026-09-17）、リリース版・初回公開日不明。Python・Docker常駐・Claude Code。Bot/app token、chat/history等、MCP接続が必要。MIT、Claude利用枠／ホスト費不明。

**公開コード確認**：[SessionBroker](https://github.com/tomeraitz/claude-slack-bridge/blob/003b0ae8e2a64f49d6d9d563583779b109fedb62/src/session_broker.py)は質問投稿後、Unix socketのreadで人の回答を待つ。notifyは待たない。[session_store](https://github.com/tomeraitz/claude-slack-bridge/blob/003b0ae8e2a64f49d6d9d563583779b109fedb62/src/session_store.py)は対応表をJSON保存する一方、[crash_recovery](https://github.com/tomeraitz/claude-slack-bridge/blob/003b0ae8e2a64f49d6d9d563583779b109fedb62/src/crash_recovery.py)は中断runを通知して再送を求める。生きた呼出しへの返却と、死んだAIの自動再開は別である。

候補2の質問adapter参考。候補1のOpenAI MCP Events製品統合とは異なる。[security.md](https://github.com/tomeraitz/claude-slack-bridge/blob/003b0ae8e2a64f49d6d9d563583779b109fedb62/docs/security.md)の既定は広い受信許可なので、そのまま導入せず許可済sender/channelと期限を要件にする。PC停止・MCPクライアント終了を越える質問待ちの復旧は未実証。

## R4 Slackで動いたClaudeはクラウドの別製品

出典：[ore88ore / Yusuke SakaiのQiita記事](https://qiita.com/ore88ore/items/ee3dd6af834ab475be22)。公開2026-02-09、更新日・CLI版不明。著者はSlack Free、Claude Pro、GitHub Free、アカウント連携とクラウド環境設定を報告。正確なscope一覧・実費不明。

**著者報告**は人のSlackメンション→Claude Code on the web→結果・PR作成画面という流れ。PC内のCLI起動、別Botへの反応、切断後の同job復旧を報告していない。[現行公式Slack資料](https://code.claude.com/docs/en/slack)もクラウドsession起動を説明し、Team/EnterpriseはClaude Tagへ移行、Pro/Maxは従来経路と区別している（公式ページの更新日・版不明）。

SlackからAIを起こすUXの参考に限定する。7候補の代替として採用しない。dotsを別製品で置換し、ローカル実行要件まで変えるためである。PCが不要なクラウド実行を「PC停止中もローカルAIが動く」と読み替えない。

## R5 Claude Code Channelsは開いている会話への入力

出典：[Anthropic公式Channels](https://code.claude.com/docs/en/channels)。公開・更新日、最小CLI版は当該ページで不明。2026-10-03時点はresearch preview。claude.aiまたはConsole API認証、既製pluginはBun、組織設定とsender許可が必要。Bedrock等は対象外。認証方式に応じたAI枠／API料金とPC費用がかかり、総額は未算定。

**公式仕様・demo**では、MCP channel→開いているClaude Code session→reply toolで返す。fakechatによる確認経路がある。セッションを開いている間だけ届くため常駐が要る。通常のMCP登録だけでは有効にならずchannels指定が必要。PC停止を越えるdurable job処理はこの説明から保証されない。

候補1/2の**ローカル入力adapterの別案**として記録し、独立した8位にはしない。dots向けMCP Eventsとは製品契約が異なる。many-ai-cli経由の起動・入力・完了相関も未確認。C2の最小一往復へ常時Claudeを追加せず、将来、既存会話の継続が必須になった場合の別設計判断とする。

## R6 n8nによるDB待機と再開URL

出典：[n8n / Elvis Saraviaの公式実践ガイド](https://blog.n8n.io/production-ai-playbook-human-oversight/)（2026-03-09）、[Wait公式仕様](https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.wait/)（更新日・対象版不明）。n8n Cloudまたは自前サーバ＋DBが常駐。Slack接続権限と、再開URLのBasic/Header/JWT等の認証を選ぶ。ホスト／Cloudプラン・実行回数・AI費用の見積りが必要で、固定価格は本資料では未算定。

**公式仕様**は待機時に実行データをDBへ退避し、固有URLの呼出しで続行する。時間待ち65秒未満はDB退避しない例外がある。これはn8n workflowの続行であり、dotsやローカルClaudeの同session続行ではない。PCと分離したサーバ／DBならPC停止中の受付・保存部品に使えるが、受理前の欠落やlocal claimは別設計。

**障害報告**：[Matt_Stefanの公開スレッド](https://community.n8n.io/t/human-in-the-loop-slack-send-wait-node-broken-on-self-hosted-n8n-server/135013)は2025-06-19、n8n 1.97.1・Docker・macOS。Slack待機返信URLの接続エラーを報告し、翌日にport設定の修正で解消したと本人が追記。現在版共通の不具合とはしない。[公式reverse proxy設定](https://docs.n8n.io/hosting/configuration/configuration-examples/webhook-url/)へ照合し、実行固有IDを削る修正を採用しない。

候補3、1/4のqueue補助候補。受信・DB待機・返信を減らせる一方、dots起動口、PCからの取得、Hub起動、冪等キーを接続するadapterは残る。既製品を追加すれば最小構成になるとは限らない。

## R7 GitHub Actionと再帰起動の報告

出典：[Anthropic公式Action](https://github.com/anthropics/claude-code-action/tree/ed670b4cf9de2a5a570d130d2f6197b9e543cd64)の確認commit ed670b4（2026-10-02）。GitHub runner上のClaude、GitHub認可とAnthropic等のAI認証が必要。権限・runner費・AI料金はworkflowと認証方式依存。個人PC停止はクラウドrunnerを止めないが、ローカルファイル操作を代行するものではない。

**失敗の著者報告**：[issue #1445](https://github.com/anthropics/claude-code-action/issues/1445)、Avkroken、2026-06-29公開・同日更新、Action v1。メンションが再帰的にAIを起こして課金が増え、workflowを撤去したと報告。調査時open。こちらで再現しておらず、当時の完全な実行条件や現行版の再現性は未確認。

**現行公式との照合**：[security.md](https://github.com/anthropics/claude-code-action/blob/ed670b4cf9de2a5a570d130d2f6197b9e543cd64/docs/security.md)は既定でbotを拒否し、allowed_botsを別扱いする。[GitHubのtrigger仕様](https://docs.github.com/en/actions/how-tos/writing-workflows/choosing-when-your-workflow-runs/triggering-a-workflow)にもGITHUB_TOKEN由来イベントの再起動抑制がある。過去報告を「現行版は既定で必ずループする」と一般化しない。

候補4/5のループ防止の根拠。thread・comment・sender・起動予算の相関をC2へ追加する。Actionを導入してもdotsが起きる証拠にはならず、本件ではworkflow追加・token切替・bot制限解除をしない。

## R8 MCP Eventsの公開サンプルは互換性と保存を別確認

出典：[panyam/mcpkit のDiscord Events sample](https://github.com/panyam/mcpkit/tree/4311b527dbd7b808dbd8e41bc63864dce33cbc0f/examples/events/discord)。確認commit 4311b527（2026-10-01）、sample server version 0.1.0、初回公開日不明。Go、実験的Events拡張。test modeはDiscord token不要、実接続はDiscord bot権限、運用認証はOIDC等。Apache-2.0、サーバ・認証基盤・AI等の実費は不明。

**公開コード／サンプル**：[README](https://github.com/panyam/mcpkit/blob/4311b527dbd7b808dbd8e41bc63864dce33cbc0f/examples/events/discord/README.md)は購読・署名webhook・期限更新を実演し、イベント保持は上限1000件のring。[main.go](https://github.com/panyam/mcpkit/blob/4311b527dbd7b808dbd8e41bc63864dce33cbc0f/examples/events/discord/main.go)はメモリstoreとdemo用匿名認証経路を持つ。再起動を越えるdurable queueの完成例ではない。

候補1の参考部品。[OpenAI製品仕様](https://developers.openai.com/plugins/build/mcp-events)が対応するwebhook・callback検証と突き合わせ、sampleのpoll/SSE等をdots対応と見なさない。認証、MCP 2.0、購読永続化、未配送event保持、実際のdots応答を別に確認する。sampleのdemo用設定を運用構成として採用しない。

## 検索記録

全行2026-10-03 UTCに実施。引用符を含む検索語は検索時のもの。通常検索とドメイン限定検索を用い、採用は検索断片ではなく本文・原典の確認後に決めた。

| 媒体・目的 | 検索語 | 結果と採否 |
|---|---|---|
| Zenn 日本語 | site:zenn.dev Claude Code Slack Socket Mode 質問 再開 | R1採用。別記事は重複する機構として補助扱い |
| Qiita 日本語 | site:qiita.com Claude Code Slack 質問 回答 セッション | R4採用、クラウド製品の区別 |
| note 日本語 | site:note.com Claude Code Slack スマホ 質問 / site:note.com Claude Code Slack Socket Mode 再開 停止 | 候補記事を発見したが下記2件は本文openがInternal Error、成功証拠に不採用 |
| X 英語 | site:x.com "Claude Code" "Slack" "resume" / "Claude Code" "Slack" "resume"（x.com限定） | 採用できるX投稿本文・原典の組を取得できず。GitHub原典のR2/R3は別途採用 |
| X dots | site:x.com "dots" "MCP Events" / "dots" "MCP" "Events"（x.com限定） / site:x.com "dots" "Slack" OpenAI | 対象往復を証明する公開投稿を確認できず |
| X 日本語 | site:x.com Claude Code Slack 質問 再開 | 採用可能な原典なし |
| GitHub bridge | "Claude Code" "Slack" bridge SQLite GitHub session | R2/R3を固定SHAまで確認 |
| MCP Events | "MCP Events" dots example plugin | 公式OpenAI仕様とR8。名前だけ一致する別packageは除外 |
| 中継・技術ブログ | AI agent Slack n8n human in the loop wait webhook persistence blog | R6公式ガイド・仕様・障害報告を確認 |
| GitHub失敗例 | Claude Code GitHub Actions bot comments not trigger infinite loop official | R7と現行公式を照合 |
| ローカル入力 | site:code.claude.com docs slack channels preview resume remote control disconnect | R5とR4の公式根拠 |
| 保存条件 | site:docs.n8n.io wait node database 65 seconds resume webhook authentication | R6のDB退避例外・認証条件 |
| A2A | "A2A" "input-required" langgraph sample persistence | 古い仕様・派生tutorialが中心。dots対応を増やす根拠なし、深掘り対象に追加せず |
| poll | "GitHub" "polling" "Claude" Slack bridge lightweight | 巡回AIや別製品の例。対象7候補の不足条件を解消せず、数合わせで採用しない |
| dots国内記事 | site:qiita.com "dots" "MCP" / site:zenn.dev "dots" "MCP Events" / site:note.com "dots" "Slack" 連携 | 対象一往復の実践例を確認できず |
| dots英語記事 | OpenAI dots webhook local agent practical implementation | 技術解説・利用感想を発見。原典を辿り、下記理由で実証例に数えない |

## 不採用と残る穴

- [ZennのMac bridge記事](https://zenn.dev/shiro_kuma_san/articles/ef1abad11c0682)（shiro_kuma_san、2026-03-13）は本文を確認。Socket・thread mapping・PC sleep制約はR1/R2と重なり、独立した追加事例には数えない。
- [ForgeNexusWorksのnote](https://note.com/forge_nexusworks/n/na2daac28896a)と[たつまるのnote](https://note.com/tatsumaru_note/n/n71018e6e627e)は検索で発見、本文openがInternal Error。公開日・内容は検索断片だけに依存するため事例判断に使わない。ログイン・有料契約・迂回取得はしていない。
- [endueのdots技術解説](https://endue.ai/blog/openai-dots-how-it-works/)（2026-10-02）は公式資料と利用感想の整理。MCP Eventsの主張はOpenAI原典へ戻した。ローカルAIとの無人一往復の実測コード・ログは当該記事で確認できず、成功事例には数えない。
- [Redditのdots利用報告](https://www.reddit.com/r/OpenAI/comments/1wuznlr/i_spent_a_day_poking_dots_with_sticks_heres_what/)は本文を確認したが、対象bridgeの質問・回答・再開・完了を通した証跡ではない。製品一般の感想から通信可否を推測しない。
- PyPI/Packagistの同名mcp-eventsは、ツール処理の警告・ライフサイクルを扱う別物が検索に混在した。OpenAIの購読webhook統合と同一視しない。
- Xは閲覧可能な原典を十分得られず、noteは取得エラーが残る。「事例が存在しない」「全媒体を網羅した」とは言わない。ログイン要求や有料壁を越えた取得もない。
- 最終追加検索はMCP/dots、poll、A2Aまで広げたが、7候補の順位を変えるdots側の適合証拠は増えなかったため終了。実装差分・運用年数・障害率・費用の網羅比較は行っていない。

## C2へ渡す変更

R1〜R8を採用品の推薦リストにはしない。[FINDINGSのC2計画](FINDINGS.md#c2-一往復の試行計画-未実施)へ次を反映した。

1. session対応表・未処理inbox・完了outboxを別証跡にする。restartで対応表が残るだけではPC停止耐性合格にしない。
2. ツールを待たせる経路、既存sessionへの入力、fresh run、dotsのjob再開をログで分ける。resume失敗を新規会話で隠さない。
3. Bot除外・sender許可・workflow起動条件を設定前に確認する。本人投稿で代替した試験は無人往復の成功にしない。
4. callback成功と受信AIの処理完了を別に確認し、再開URLの認証・job相関を検査する。
5. 有効な回答を待つ間にAIを巡回起動しない。最大2job・各10分・同event再配送1回の予算を維持し、停止・撤収を行う。

[比較HTML](comparison.html) / [FINDINGS](FINDINGS.md) / [PROGRESS](PROGRESS.md) / [draft PR #8](https://github.com/ishizakahiroshi/many-ai-cli/pull/8)
