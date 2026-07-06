---
type: bugfix
status: done
tags: []
owner: 
review_status: draft
related: []
last_reviewed: 2026-07-04
---
# [完了] 障害対応記録: LM Studio/Ollama + Qwen2.5-7B-InstructでClaude Codeが無反応・使い物にならない

## 症状

Hyper-Vホスト(192.168.11.50, RTX 4060 / 8GB VRAM)上のLM Studioで動かしたQwen2.5-7B-Instruct(Q4_K_M, context上限32768)に、many-ai-cli経由でClaude Codeを接続。セッション開始直後に「今日は何日」と送っても応答が返らず、画面上は「\*Tomfoolering…」等のスピナーが回り続けるだけに見えた。

再現手順:
1. many-ai-cli で claude セッションを spawn し、`ANTHROPIC_BASE_URL` をLM Studio(`http://192.168.11.50:1234`)に向ける
2. モデル `qwen2.5-7b-instruct` で何かメッセージを送る
3. スピナーが回り続け、いつまで待っても応答が返らない(実際は指数バックオフでリトライを繰り返している)

影響: many-ai-cli 経由でローカル小型モデル(Qwen2.5-7B級)をClaude Codeのバックエンドとして使う構成全般。

## 根本原因（root cause）

セッションログ (`~/.many-ai-cli/logs/sessions/claude_2026-07-03_195654_many-ai-cli_s1.log`) に実エラーが記録されていた。

```
500 The number of tokens to keep from the initial prompt is greater than the context length (n_keep: 38369 >= n_ctx: 32768). Try to load the model with a larger context length, or provide a shorter input.
```

(後日、CLAUDE.md無しプロジェクトでの再現時に上記の完全なエラー文言を確認。実際に送られたプロンプトは約38,369トークンだったことが確定した。)

Claude Code が送るシステムプロンプト(ツール定義・Skill一覧・CLAUDE.md群・memory files等)が、LM Studio側にロードされたQwen2.5-7B-Instructのcontext長を超えており、llama.cppサーバーが `n_keep`(オーバーフロー時に保持する初期プロンプト量)がcontext長を超えるとして500を返し、Claude Codeがそれを一時エラーとみなして最大10回まで指数バックオフでリトライし続けていた(1s→3s→9s→27s…)。これが「スピナーが回ったまま無反応」に見えた正体。

切り分けで判明した点:
1. LM Studio単体チャット(短文)は正常応答 → LM Studio/モデル自体は健全
2. サーバーロード設定の `Parallel`(Max Concurrent Predictions)を4→1に変更しても同じ500が再発 → 「contextがParallel数で分割される」という仮説は誤りと判明(未検証のまま提示してしまった点は反省)
3. Sonnet 5(Anthropic本家)セッションの `/context` 実測: ロード時点(メッセージ0件)で System prompt 8.9k + System tools 14.1k + Memory files 19.5k + Skills 4.8k = 合計47.3kトークン。ただしこれは **many-ai-cliプロジェクト(巨大CLAUDE.md込み)** での実測値であり、後述の通り条件の異なるqwenセッションにそのまま適用するのは早計だった
4. CLAUDE.md無しの別プロジェクト(`C:\dev\nursery\code`, no git)でも同じ500が再発 → CLAUDE.mdの分量が主因ではなく、Claude Code本体のtools/skills一覧などの固定オーバーヘッドが相応に効いている
5. MCPを切っても改善せず(LM Studio側では)

追試: CLAUDE.md無しプロジェクトのSonnet 5(本家Anthropic)セッションで `/context` を再実測したところ、やはり合計47.3kトークン(内訳も System prompt 8.9k / System tools 14.1k / Memory files 19.5k / Skills 4.8kと完全一致)。「Memory files」の内訳は `~/.claude/CLAUDE.md`(11.9k)等の**グローバル設定のみ**で、プロジェクト固有のCLAUDE.mdは0件。つまり47.3kという床はプロジェクトのCLAUDE.mdに依存せず、グローバル設定だけで決まる**プロジェクト非依存の固定値**と確認できた。同条件(CLAUDE.md無しプロジェクト)でLM Studio + Qwen2.5-7B-Instructを再度動かしても同じ500エラーが再発することも確認済み。

一方、Ollamaデスクトップアプリで同モデル(`qwen2.5:7b`, num_ctx=32768に明示設定)を使うと500エラーは解消した。ホスト側 `server.log` を実測したところ、実際に送られたプロンプトは4リクエストとも `truncated = 0`(切り詰めなし)、トークン数は3580/9080/11533/535と、いずれも32768に対して余裕で収まっていた。このOllamaテストは「CLAUDE.md無し」かつ「直前にMCPを切った」状態で行っており、LM Studioの500エラー時(CLAUDE.md無しだがMCP有効)とは条件が完全には揃っていない。

**有力な手がかり**: LM Studioのサーバーログ(`C:\Users\ishiz\.lmstudio\server-logs\2026-07\2026-07-03.1.log`)に以下の行があった。

```
[DEBUG] Received request: POST to /v1/messages/count_tokens with body {
[ERROR] Unexpected endpoint or method. (POST /v1/messages/count_tokens?beta=true). Returning 200 anyway
```

Claude Codeは本来、実送信前にAnthropic本家の `/v1/messages/count_tokens` エンドポイントで事前にトークン数を把握し、必要に応じて会話の圧縮や `n_keep` の調整を行っていると考えられる。しかしLM StudioのAnthropic互換レイヤーはこのエンドポイントを実装しておらず、未知のエンドポイントとして扱いつつ200を返してしまう(＝正しいトークン数が返っていない可能性が高い)。これによりClaude Code側が正確な事前トークン数を把握できないまま送信し、結果的に38,369トークンというオーバーフローに至った、という説明が成り立つ。Ollama側でこの `count_tokens` エンドポイントがどう扱われているか(実装されている／Claude Codeが接続先をOllamaと認識して呼ばないだけ等)は未検証で、**LM StudioとOllamaで実際に送信されたプロンプトサイズがここまで違う理由の核心は依然未確定**。

**追加確認(同日)**: Ollamaホスト側 `server.log`(`$env:LOCALAPPDATA\Ollama\server.log`)を `count_tokens|/v1/messages|404|unexpected|unknown endpoint` でgrepしたところ、`/v1/messages?beta=true` への通常のPOSTのみが記録されており、**`/v1/messages/count_tokens` へのリクエストは一件も存在しなかった**。LM Studio側では明確に `count_tokens` 呼び出しが記録されていたのと対照的。これにより「Claude CodeはOllamaに対してはcount_tokens事前チェックを呼ばず、LM Studioに対しては呼んでいる」という非対称性が実測で裏付けられた。

**決定的な確認**: 両サーバーへ直接同じダミーリクエスト(`POST /v1/messages/count_tokens?beta=true`)を投げて比較した。

```powershell
Invoke-WebRequest -Uri "http://192.168.11.50:11434/v1/messages/count_tokens?beta=true" -Method POST -ContentType "application/json" -Body '{"model":"qwen2.5:7b","messages":[{"role":"user","content":"hi"}]}'
# => StatusCode 404, Content: "404 page not found"

Invoke-WebRequest -Uri "http://192.168.11.50:1234/v1/messages/count_tokens?beta=true" -Method POST -ContentType "application/json" -Body '{"model":"qwen2.5-7b-instruct","messages":[{"role":"user","content":"hi"}]}'
# => StatusCode 200, Content: {"error":"Unexpected endpoint or method. (POST /v1/messages/count_tokens?beta=true)"}
```

**Ollamaは素直にHTTP 404を返すのに対し、LM StudioはHTTP 200(成功扱い)のままボディに `{"error": ...}` を入れて返す**、という食い違いが確定した。Claude CodeはおそらくHTTPステータスコードだけを見て「200=成功」と判断し、本来 `{"input_tokens": N}` が返るべきところに `{"error": ...}` が返っても気づかず、正確な事前トークン数を把握できないまま送信を続け、最終的に38,369トークンのオーバーフローに至った、という説明で一貫する。Ollamaは404を受けて「このエンドポイントは無い」と正しく認識し、count_tokensに頼らず自前の見積もりだけで動作したため、実際のプロンプトサイズが32768に収まる範囲で済んだと考えられる。

これより先(Claude Code自身がHTTPステータスをどう解釈しているかの内部実装)はクライアントがクローズドソースのため検証手段が無く、深追いの限界点とする。

ただし、500エラーの有無に関わらず、Qwen2.5-7B-Instructの応答品質そのものが実用に耐えなかった。Ollama経由で「今日は何日？」と聞いても質問に答えず、`name: japanese-context / description: 日本語での応答の設定` のようなシステムプロンプト内の設定断片をそのまま復唱し、さらに中国語が混入した支離滅裂な文章を54秒かけて生成した。これはinstruction-following能力そのものの不足であり、context長の問題とは別次元の限界。

## 修正内容

コード修正ではなく運用判断。ローカル小型モデル(Qwen2.5-7B級、32k context級)でのClaude Code運用を諦め、cloudモデル(Claude本家 / Ollama Cloudのgpt-oss:120b-cloud等)に一本化する。

## 変更ファイル

| ファイル | 内容 |
|---|---|
| `C:\Users\ishiz\.claude\projects\C--dev-github-public-many-ai-cli\memory\local-llm-8gb-claude-code-unusable.md` | 既存memoryに今回の検証結果(LM Studio context超過の経緯)を追記 |

## 検証

- LM Studio単体チャットで短文は正常応答することを確認
- LM Studioの `Parallel` を4→1に変更しても500エラーが再発することを確認(仮説の反証)
- Sonnet 5セッションの `/context` でシステムプロンプトの内訳(System prompt/System tools/Memory files/Skills)を実測
- CLAUDE.md無しプロジェクトでも同じ500エラーが再発することを確認
- Ollama(num_ctx=32768)で同モデルを実行し、GPU使用率97%まで上がり実際に推論が完走することを確認
- Ollamaホスト側 `server.log` を実測し、`truncated = 0` および実際のプロンプトトークン数(3580/9080/11533/535)を確認
- Ollama経由の応答内容を確認し、質問に答えずシステムプロンプト断片を復唱・多言語混入する不具合を確認
- CLAUDE.md無しプロジェクトのSonnet 5セッションで `/context` を再実測し、47.3kの床がプロジェクト非依存(グローバル設定のみに起因)であることを確認
- 同条件(CLAUDE.md無し)でLM Studio + Qwen2.5-7B-Instructを再実行し、同じ500エラーが再発することを確認(完全なエラー文言 `n_keep: 38369 >= n_ctx: 32768` を確認)
- LM Studioのサーバーログ(`C:\Users\ishiz\.lmstudio\server-logs\2026-07\2026-07-03.1.log`)を実測し、`/v1/messages/count_tokens` エンドポイントが未実装で200を誤返却している行を確認
- Ollamaのサーバーログ(`$env:LOCALAPPDATA\Ollama\server.log`)を同条件でgrepし、`count_tokens` 呼び出しが1件も無く `/v1/messages?beta=true` のみであることを確認(LM Studioとの非対称性を裏付け)
- 両サーバーへ直接 `POST /v1/messages/count_tokens?beta=true` を投げ、Ollamaは404・LM Studioは200+エラー本文、という食い違いを確認(決定的な証拠)

## 備忘

- LM Studio と Ollama で、llama.cppサーバーの `Parallel`/`num_parallel` 設定の意味が逆になる点に注意。LM StudioはParallel数でcontextを分割するイメージだったが実測で否定された。Ollamaは `OLLAMA_NUM_PARALLEL` を上げてもnum_ctxは分割されず、各リクエストがフルnum_ctxを使う(その代わりメモリ使用量が並列数倍になる)
- Ollamaのデスクトップアプリはcontext lengthのデフォルトが4096(または相当に小さい値)なので、比較実験の際は明示的に上げないとLM Studioより早く詰まって不公正な比較になる
- 「なぜLM StudioとOllamaで実際に送られたプロンプトサイズがここまで違ったか」は確定: `POST /v1/messages/count_tokens` に対し、Ollamaは素直にHTTP 404、LM StudioはHTTP 200(成功扱い)+ボディに`{"error":...}`という食い違った応答を返す。Claude CodeはHTTPステータスだけを見て200を成功と誤認し、LM Studio相手には正確な事前トークン数を把握できないまま送信し続けて38k超に至った可能性が高い。ただしClaude Code自身がHTTPステータスをどう解釈しているかの内部実装はクローズドソースのため検証不可。これが今回の深追いの限界点
- 関連memory: `local-llm-8gb-claude-code-unusable.md`(8GB VRAM機での8B級モデル検証。今回はその続きとして32k context級モデルでも実用不可という結論を補強)

## 追記(同日): gpt-oss:20bの再試行を検討中(進行中メモ)

Qwen2.5-7B系の検証完了後、ユーザーがホスト側のシステムRAMを32GB→**64GBに増設**したとのことで、`gpt-oss:20b`(Ollama)を同ホストで再度試す予定。

過去の記録(`local-llm-8gb-claude-code-unusable.md`)では、同一ホスト(RTX 4060 / 8GB VRAM)でRAMが32GBだった時に `gpt-oss:20b`(Q4で約13GB)をロードしただけで**物理メモリ完全枯渇→スワップ地獄→ホストOS全体フリーズ・リモート接続断線**という実害が出ている。今回はRAM増設によりこの根本原因(RAM不足)が解消されている前提での再試行。

これは検証結果が出る前の中間チェックポイントとして記録している(ホストがフリーズした場合に備え、ここまでの経緯を失わないため)。結果が出たら本セクションを更新するか、新たな `bugfix_*.md` に切り出す。

### 結果

RAM増設(32GB→64GB以上、実測ではホスト総量95.7GB)によりホストフリーズは発生せず、システムメモリ使用率は最大でも74%程度(70.8/95.7GB)で危険域(前回クラッシュ時の87%)には達しなかった。ただし32GBはHyper-Vゲストへの固定割当のため、ホストが自由に使える実質的な枠はもっと狭い点に注意。

Context lengthを32k→256kまで上げて`今日は何日？`を再送したところ、今度は正しく応答した:「今日の日付は 2026年7月3日 です。」(`Thought for 10m 0s`, `Cooked for 20m 47s` = 応答まで**約21分**)。

Qwen2.5-7B-Instructとの違い:
- 応答品質: gpt-oss:20bは質問に正しく答え、instruction-followingも機能した(Qwen2.5-7Bはシステムプロンプト断片の復唱・多言語混入で的外れだった)
- 実用性: 21分という所要時間は、Claude Codeの実際のコーディング作業(何度もtool callが往復する)には非現実的に遅い。8GB VRAMに20Bモデル+256k contextを載せると大半がCPU/共有メモリに溢れ、GPU本来の速度が出ない

**未解明の副現象**: 応答が画面上「終わった」ように見えた後も、Ollamaホスト側で`llama-server.exe`(PID 5768)がGPU使用率100%のまま数分間動き続けた。`ollama ps`は一時「Stopping...」と表示していたが実際のプロセスは止まっておらず、サーバーログ実測で最初のリクエスト(task 236, 3分48秒)完了の直後に**別の大きなリクエスト(task 258, プロンプト9276トークン)が新規に処理され始めていた**ことを確認した。ただしmany-ai-cli画面上はどのセッションも「実行中」表示が無く、どこから送られたリクエストかは特定できなかった。最終的にはGPU使用率5%まで自然に収束し、実害(フリーズ等)は無かった。この「正体不明の追加リクエスト」の発生源は未解明のまま。

**追加検証: Ollama単体(Claude Code経由なし)での速度比較**。同じ`gpt-oss:20b`に、Claude Codeを介さずOllama自身のチャットUIで直接「今日は何日ですか?」と聞いたところ、`Thought for 9.3 seconds`で正答(「今日は2026年7月3日(日本時間)です。」)が返った。Claude Code経由の21分と比べて圧倒的に速い。

これにより、21分という所要時間の正体がほぼ確定した。以前のサーバーログ実測(`prompt processing`が78〜100トークン/秒程度)から逆算すると、Claude Codeが送る4.7万トークン超のシステムプロンプトの前処理だけで9〜10分程度かかる計算になり、実際に観測された`Thought for 10m 0s`とほぼ一致する。**gpt-oss:20b自体の応答能力・速度に問題は無く、遅かったのはモデルの「思考」ではなくClaude Codeの巨大なシステムプロンプトを毎回丸ごと処理する「前処理」の時間**だったと結論づけられる。モデルを変えてもClaude Code側のプロンプトサイズが変わらない限り、8GB VRAM機ではこの前処理コストは避けられない。

### 結論(gpt-oss:20b)

RAM増設によりホストフリーズという実害は再現しなくなったが、応答速度(約21分/質問)が実用に耐えず、cloud一本化の結論は変わらない。応答品質そのものはQwen2.5-7Bより明確に上(正しく質問に答え、指示追従も機能した)であり、「モデルサイズが大きくなればinstruction-followingは改善するが、8GB VRAM機ではその代償として速度が犠牲になり実用にならない」という知見に加え、「遅さの正体はモデルの生成速度ではなくClaude Code自身の巨大なシステムプロンプトの前処理コストである」ことがOllama単体比較で確定した。

### 追加検証: Qwen2.5-7Bも単体(Claude Code経由なし)で比較

gpt-oss:20bと同様、Qwen2.5-7B系もClaude Codeを介さず各サーバーソフト自身のチャットUIで直接「今日は何日ですか?」と聞いて比較した。

- **Ollama(`qwen2.5:7b`)単体・1回目**: 約10秒で応答。ただし「現在の日付は2023年10月25日です」と自信満々に誤った日付を返した(学習データの知識で止まっている、いわゆる通常のLLMの限界。真面目な誤答であり破綻はしていない)
- **LM Studio(`qwen2.5-7b-instruct`)単体**: 0.28秒(41.22トークン/秒、127トークン)で応答。「お答えできかねますが、現在の日付や曜日を知るためには、パソコンやスマートフォンなどのデバイスをチェックしてください」と正直に不明を認め、Pythonでの取得コードまで提案した。誠実で一貫した応答
- **Ollama(`qwen2.5:7b`)単体・2回目**: 1.7秒で応答。しかし「現在の日付は2023年10月25日です。お QUERY 的您想知道今天的日期是哪一天呢?答案就是2023年10月25日。如果您需要更具体的信息或其他帮助，请告诉我哦!」のように日本語と中国語が混在し、意味不明な断片(「お QUERY 的」)まで混ざる破綻した応答だった

**結論**: 「日本語のはずが中国語が混ざる」「文章が破綻する」という壊れ方は、Claude Codeのせいではなく、**Ollamaがこのモデルを読み込む際のチャットテンプレートの噛み合わせ自体が壊れている**ことに起因すると分かった。同じ系統のモデルでも、LM Studio経由は一貫して誠実(不明なら不明と答える)なのに対し、Ollama経由は自信満々に誤答したり多言語混入で破綻したりと不安定。サーバーソフトの実装差だけで、モデルの「性格」がここまで変わりうる。

Qwen2.5-7Bの全体像としては、(1) 単体でも真面目な誤答(学習データの限界)や言語混在(Ollama側の実装起因)という弱さがあり、(2) さらにClaude Codeの複雑なシステムプロンプトが乗ると、質問を無視してシステムプロンプト断片を復唱するというもっと深刻な壊れ方をする、という二重の弱さがあることが分かった。

### 根本的な前提の見直し(再追試・重要)

gpt-oss:20b単体を複数回試したところ、`Thought for 9.3 seconds`(1回目)、`Thought for 14.5 seconds`(2回目)と、実行のたびに10〜30秒程度の幅はあるが、いずれも正答かつ実用的な速度だった。

これは今回の調査全体の結論を見直す必要がある発見。**「ローカルモデルは遅くて実用にならない」という前提自体が誤りだった可能性が高い**。遅かったのはモデル単体の応答速度ではなく、あくまで「Claude Codeという、フロンティア級クラウドモデル前提で作られた重量級ハーネスに、ローカル小型モデルを繋いだ場合」に限った話だった。

次のアクション(次回の検証候補): Claude Code本体を使わず、より軽量なコーディングエージェントハーネス(例: opencode)経由でローカルモデルを使う、あるいはmany-ai-cli自身からサブエージェント的に直接Ollama/LM Studio APIを呼ぶ構成を試す。これが実現すれば、今回の根本原因(ハーネス側の4.7万トークン超の固定オーバーヘッド)を回避でき、ローカルモデルが実用になる可能性がある。
