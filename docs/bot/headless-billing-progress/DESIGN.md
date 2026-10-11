# #P-20261010-003 many-ai-cli：画面なし実行の払い方と進みの設計

> 最終更新: 2026-10-11(日) 00:08:54 JST
> 状態: 設計受け入れ待ち。文書のみ。実装 commit、Draft PR、実キーでの実行はまだ無い。

## context配分

| C | 種別 | 内容 | 備考/注意点 |
|---|---|---|---|
| C1 | planned | 子ごとの払い方と resolver の境界 | 同じ会話で設計受け入れ後。CI 経路の判断も必要 |
| C2 | planned | 構造化された進みを wrapper から Hub へ | C1 後。既存の process 所有と session incarnation を使う |
| C3 | planned | 開いたカードと左一覧への同一データ表示 | C2 後。headless の入力禁止を維持 |
| C4 | planned | 合成試験、別担当レビュー、Draft PR と対象 SHA の CI | 実キー、有料 API、稼働中 Hub、Windows 実画面は使わない |

実行順序: C1 → C2 → C3 → C4。今は C1 に着手しない。

## 1. 出典と起点

- 管理 Issue: https://github.com/ishizakahiroshi/many-ai-cli/issues/14
- 固定指示: agent-board の `0fe1c55f7d641585f885802d09c516ae675f234c` にある README / FACTS / REVIEW / PROGRESS。初回公開 `6b3d447dda72f27500cf1bb095a9886252ea3391` とは別。
- 挙動の基準: `e1ab622aa7708ae883a3cdd83d042fbf2cf31e38`
- 着手時に GitHub API と `git ls-remote` で取得した remote base tip: `e1ab622aa7708ae883a3cdd83d042fbf2cf31e38`。同一。fresh clone の祖先検査も成功。
- 作業 branch: `dots/P-20261010-003-headless-billing-progress`
- PR base: `dots/rust-recovery-resume-3`
- ローカルにしか無い 2 commit は clone に含めていない。この文書の commit は実装 code SHA と呼ばない。
- `.omitnix/index.json` は `a114727651bdca2884c3d4b92262bbae240f6b2c` 時点で古い。索引だけで判断せず、以下の source を基準 SHA で読み直した。

## 2. 基準 source に既にあるもの / 足りないもの

- `rust/src/config/model.rs`: Claude は `-p --output-format stream-json --verbose`。ほか 5 provider は text 系。Codex は既存の権限引数と exec の不整合により意図的に除外。
- `rust/src/application/orchestrate_cli.rs`: `--execution-mode` と `--effort` は既存。別名を増やさず、そのまま使う。
- `rust/src/orchestration/headless_formats.rs`: 開始、発言、tool の開始/終了、result を読むが、主に端末用文字列へ変換する。turn 数、usage、result の費用を構造化して保持しない。
- `rust/src/wrapper/entry.rs`: headless の出力は `pty_data` と `session_end`。現在は設定に応じて stdout/stderr の raw log を作る。この経路をキー課金でそのまま使わない。
- `rust/src/application/session_usage/`: cost 表と usage 経路はあるが、既存表は cache write の内訳などを持たない。今回の精密な見込み額としてそのまま流用しない。
- `web/src/app/session-list.ts`: headless の chip、既存の最終メッセージ/タイトル、ブラウザ内で測った running 経過はある。要求された 5 項目を同じ実行の構造化データで出す機能は無い。`last_message` はユーザー入力や transcript 観測にも使われるので、最後の assistant 発言の正本に流用しない。
- `web/src/app.ts`: headless への送信を止める `isSessionHeadless` / `sendText` がある。これを維持する。

## 3. 初回の対応範囲

初回は Claude のプランログイン / キー課金切替と構造化された 5 項目を実装する。他の既存 headless 起動は保ち、確認できない値は「未対応 / 未取得」と表示する。対応していない provider へキー課金を指定した要求は、プランへ落とさず起動前にエラーにする。

| Provider | 今回の扱い | 一次資料で確認したことと見送り理由 |
|---|---|---|
| claude | 初回対応 | 公式 headless、bare、apiKeyHelper、budget、stream と result がそろう |
| codex | headless 対象外を継続 | 基準の bounded/full はいずれも ask-for-approval=never を使う。exec の現行共有引数にこの option は無い。権限の弱体化や別起動方式で回避しない |
| copilot | recipe 機能は確認済み、今回の切替は見送り | API_KEY_COMMAND はある。ただし BYOK の endpoint / provider / model と providers.json の優先順位を別途固定する必要がある。helper だけでは現在の GitHub 側モデルの払い方は変わらない |
| grok | recipe 機能は確認済み、今回の切替は見送り | per-model auth_provider がある。モデルごとの接続先と env_key / api_key の優先順位、企業ポリシーを固定する必要がある |
| command-code | recipe 機能は確認済み、今回の切替は見送り | providers.json の command 参照があるが、保存済み接続情報が優先する。custom provider / model が必要。通常のキーだけで別の API 課金になるとは限らない |
| cursor-agent | recipe による切替は未確認 | 公式に確認できたのは値を env / argv へ渡す方式。Cursor のキーが直接モデル API 課金を意味するとも断定できない |
| opencode | recipe による切替は未確認 | env / file 参照はあるが、値を永続化せず CLI だけが command を実行する今回向けの経路を確認できていない。auth の command login は結果を保存するため採用しない |

「未確認」は機能が永久に存在しないという意味ではない。対応追加は、本設計の受け入れに含めず改めて根拠をそろえる。

一次資料:
- [Claude headless](https://code.claude.com/docs/en/headless)、[CLI](https://code.claude.com/docs/en/cli-reference)
- [Codex exec](https://github.com/openai/codex/blob/main/codex-rs/exec/src/cli.rs)、[共有引数](https://github.com/openai/codex/blob/main/codex-rs/utils/cli/src/shared_options.rs)
- [Copilot CLI](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference)、[BYOK](https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/use-byok-models)
- [Grok auth provider](https://github.com/xai-org/grok-build/blob/2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8/crates/codegen/xai-grok-shell/README.md#per-model-auth-providers)、[headless](https://docs.x.ai/build/cli/headless-scripting)
- [Command Code BYOK](https://commandcode.ai/docs/byok)、[provider billing](https://commandcode.ai/docs/provider)、[headless](https://commandcode.ai/docs/headless)
- [Cursor authentication](https://cursor.com/docs/cli/reference/authentication)、[output](https://cursor.com/docs/cli/reference/output-format)
- [OpenCode config](https://opencode.ai/docs/config/)、[CLI](https://opencode.ai/docs/cli/)、[login 実装](https://github.com/anomalyco/opencode/blob/055d95bb7e278c94baf06235a52cac79dd13ba67/packages/opencode/src/cli/cmd/providers.ts#L320-L350)

## 4. 払い方の契約

### 要求と確認画面

- 新規 spawn ごとの `billing_mode` は `plan-login` が既定。`api-key` は明示した子だけ。
- 新規 option は `--billing-mode`、`--billing-profile`、`--max-budget-usd`。既存の execution-mode、model、effort、permission はそのまま使う。
- API mode は headless + claude + ローカル登録済み profile ID + 正の有限な budget が必須。未知の mode / profile、provider 不一致、余分な API 設定を伴う plan mode、interactive + API mode は起動前に拒否。
- profile の表示名、provider、budget と「見込み額に基づく停止で、超過することがある」を既存の spawn 確認画面に表示する。recipe 本文は送らない。
- 払い方や budget を role の記憶 / 最近の設定 / permission の記憶へ保存しない。次の新規 spawn は必ず plan-login に戻る。
- recipe の revision を解決時に固定し、起動直前に一致を検査する。変更されていれば再確認待ちへ戻す。
- API mode の timeout 自動再起動は行わない。新しい起動は別の費用になるため、再度明示した spawn とする。プラン側の既存 retry は変更しない。

### recipe の置き場とデータの流れ

ローカルの `~/.many-ai-cli/headless-billing.json`（trial はその選択 root 内）に profile ID、表示名、provider、取り出し command の recipe だけを置く。キー値を受け取る欄は作らない。利用者が用意する既存の取り出し方を参照する設定であり、今回こちらが実環境に作成・登録・試用するものではない。

- このファイルは通常の public config、profile-export、Web のファイル表示、Issue、看板、診断出力へ含めない。API の一覧は ID / 表示名 / provider の許可した項目だけ。
- 所有者限定の既存 private I/O と safe filesystem を使う。パスは RuntimePaths から組み立て、symlink / trial 外参照を既存の境界で拒否する。
- Hub は recipe の構文と対象を検査するだけ。secret-store や取り出し command を実行しない。CLI のヘルパ出力を読み取るコードを Hub / wrapper に作らない。
- wrapper は既存の session-owned Claude settings に apiKeyHelper の recipe を合成し、`--settings <private-file>` を 1 本だけ渡す。argv は settings のパスと非秘密の選択項目だけ。キー値は argv / Hub の env / Hub WS / DB に置かない。
- 値の経路は「利用者の resolver → Claude が所有する helper の stdout → Claude の認証処理」。Hub へ折り返さない。プレビューや接続テストのためにも resolver を実行しない。
- 一時 settings は既存の SessionHooks の所有・終了時削除・次回の残骸回収に乗せる。共有の Claude settings を上書きしない。失敗、キャンセル、早期 return も削除対象。
- 型は recipe 本文を Debug に出さず、エラーには固定の分類だけを使う。recipe を含む設定の一般的な Serialize を公開経路へ流用しない。

### Claude の起動

共通: 既存 `-p --output-format stream-json --verbose`、stdin の prompt、既存 model / effort / permission に、部分的な数値観測のため `--include-partial-messages` を加える。

API mode のみ: `--bare`、上記の private settings、`--max-budget-usd` を加える。bare は自動発見する CLAUDE.md / skills / MCP などを省くので、この動作差を確認画面にも短く示す。権限フラグを弱めない。既存の必要な明示設定と apiKeyHelper は同じ settings に合成する。resume / continue による過去費用の混入は初回のこの API mode では許可しない。

Plan mode: bare と新しい helper / budget を付けない。API の失敗から plan へ、plan の失敗から API へ切り替える fallback を作らない。

認証には優先順位があるため、単に helper を省くだけで「プラン」と断定しない。Claude の cloud / gateway / API 環境変数、route、既存 helper 設定が選択と矛盾していないかを値を表示せず検査する。矛盾や不明な managed override がある場合は、利用者が整理するまで起動しない。設定や環境を黙って書き換えない。

プラン判定に用いる `claude auth status` は JSON が既定であり、authMethod など必要な非秘密項目だけを採用する。これは推論を行う接続テストにはしない。helper 実行や credential refresh が絶対に起きないという公式保証は無いので、API recipe を試す用途には使わない。認証状態が確認できなければプラン利用を断定せず停止する。ログイン/キー保存/認証の変更はしない。

実装時の合成試験と対応版確認が先。CLI の更新は行わず、未確認の版や非対応 option を推測で代替しない。`--help` に出ないだけで非対応と断定もしない。

出典: [認証の優先順位](https://code.claude.com/docs/en/authentication)、[CLI の auth status / budget / bare](https://code.claude.com/docs/en/cli-reference)。

## 5. ログと値の漏れを防ぐ境界

キー課金では従来の raw stdout / stderr ログを無効にする。ProcessPlan の出力保持量も 0 とし、構造化 parser が使う bounded buffer 以外に生出力を残さない。

- helper は「成功 stdout は値だけ、失敗は非ゼロ、stderr に秘密を出さない」契約。実際の command 本文・出力は fixture や文書に記録しない。
- provider stderr、未知/壊れた JSON、tool input、tool result、init の認証設定、error の生本文を Hub へ転送しない。キー課金では、現在の「壊れた JSON は text 出力に戻す」動作を使わない。
- 進捗の自由文は完結した assistant 発言の短い抜粋だけ。既存 `storage::mask_secrets` を完全な文字列へ適用してから制御文字除去・長さ制限・HTML escape を行う。部分 token を直接描画しない。tool は名前だけで、引数は出さない。
- 同じ安全な投影を端末表示、WS、replay、history、カードに使う。原文をログへ先に書いてから画面だけ伏せる形にしない。
- 認証失敗は固定の分類で示す。コマンド、環境全体、settings 内容、raw error をエラー文字列へ補間しない。
- 本物のキーは使わず、明らかな合成 sentinel で成功・失敗・stderr・壊れた JSON・chunk 分割・例外経路を検証する。

この境界は正常な公式 CLI と信頼した resolver を前提にする。任意の悪意ある command や CLI が別の場所へ秘密を書き出すことまで防げるとは主張しない。公式資料に helper stderr の安全保証は無いため、未確認の CLI 版は対応済みとしない。

## 6. 進みの正本と状態遷移

Rust-only の型 `HeadlessProgress` を追加する。正本は既存 SessionEngine の同じ session incarnation。別の session map、端末文字列の逆解析、外部 telemetry は作らない。

主な項目:
- trusted な run ID と単調増加 sequence
- billing mode（server が確認した spawn metadata からのみ）
- observed turns / result turns とそれぞれの出典
- active tool ID / 名前、複数なら件数
- started / elapsed / ended の値
- cost amount、currency=USD、kind=estimate/result/unavailable、coverage
- 最後の assistant 発言の安全な短文
- running / finishing / completed / failed / canceled / disconnected とデータ欠落の印

### wire と保持

- wrapper → Hub は、新しい `headless_progress` message と既存 `Message.data` の中の versioned JSON payload を使う。既存の base64 byte 契約を使い、固定 Go oracle 由来の `proto/generated.rs` と Go source は変更しない。
- Hub の wrapper socket は接続に束縛された SessionBinding、実行モード、run ID、sequence、数値の範囲、payload 上限を検証する。ブラウザや他 wrapper からの session ID 指定だけでは更新できない。
- `SessionSnapshot.headless_progress` と専用の Rust CoreEffect で配信する。既存の spawn correlation と同様に SessionCore の UI priming / live 配信順序を通し、直接 broadcast で追い越さない。
- 古い sequence、以前の実行、再接続前の binding は捨てる。費用と turns はスナップショットで置換し、同じイベントを足し込まない。
- ライブ中と終了カードに保持する。ブラウザの再読込・カード切替では snapshot から復元する。Hub 再起動で headless 子を reattach できない既存仕様は変更しない。再起動後に値を捏造して復元しない。
- 処理は bounded。burst は最新 snapshot にまとめ、終端前に必ず flush。出力欠落や channel lag は現在同様に停止・未完了として扱う。

### 5 項目の計算

1. ターン: message ID を使って主実行の応答を重複排除し、途中は「観測 N ターン」。複数 content block や同じ ID の再送を増分にしない。最後は正当な result.num_turns で置換する。
2. 今の道具: tool_use ID と tool_result の対応で active set を更新する。複数実行中は最新名 + 件数。終了後に古い道具を「実行中」のまま残さない。
3. 経過: wrapper の実起動に由来する monotonic elapsed を送る。画面を開いた時刻や最終出力時刻を起点にしない。双方の表示を共通 clock で更新し、終了で止める。
4. 費用: 次節の規則。数字が無いときは 0 にしない。
5. 最後の発言: 主実行の完結した assistant text の最後の安全な抜粋。tool の内容や最後のユーザー指示を代用しない。

provider result は会計値の出典であり、プロセスの正常終了を証明するものではない。先に成功 result が来ても終了までは finishing。非ゼロ exit、キャンセル、出力欠落なら成功扱いにしない。確かな result が無い終了は「最終額未取得」とする。

## 7. 見込み額と終了時の額

途中は「見込み（観測した主実行分）」と明記する。message_start / message_delta の usage は 1 応答内の累積なので置換し、message ID ごとの重複を避ける。assistant の output_tokens の仮値は確定出力数として使わない。

- API 料金表は正確な serving model ID と確認日を持つ最小の表にする。input / output / cache read / 5m cache write / 1h cache write を区別する。
- 未知モデル、cache の内訳不足、未対応の長文脈・fast・地域等の倍率は「部分見込み / 未取得」。既存の価格表へ雑に当てはめない。必要なら既知の観測額だけと除外範囲を表示する。
- 部分 stream は子 agent の token を網羅しない。見込みを実行全体の上限や請求額として見せない。料金をネットから実行中に取得する仕組みは作らない。
- 終了時は result.total_cost_usd を採用し「終了値（CLI 結果）」に置換する。累積 result を加算しない。subagent を含む合計との違いを保持する。
- 失敗終了でも有効な結果費用があれば表示し、成功状態とは分離する。欠落、異常値、crash による不自然なゼロは過去の見込みを消さず、最終額未取得を示す。
- プラン払いのドル表示は「API 換算の参考額」であり、追加請求やプラン残量と混同させない。
- `total_cost_usd` 自体も CLI の算出値である。「確定」はこの実行の最終結果という意味。請求書・Console の残高との一致を保証しない。`--max-budget-usd` も厳密なハードキャップではない。

出典: [stream](https://code.claude.com/docs/en/agent-sdk/streaming-output)、[API event](https://platform.claude.com/docs/en/build-with-claude/streaming#event-types)、[費用の扱い](https://code.claude.com/docs/en/agent-sdk/cost-tracking)、[価格](https://platform.claude.com/docs/en/about-claude/pricing)、[cache 内訳](https://platform.claude.com/docs/en/build-with-claude/prompt-caching#1-hour-cache-duration)。

## 8. カードと左一覧

`web/src/app/headless-progress.ts` に型の正規化、数値整形、共通 clock、表示用の安全な view model を置く。各表示が別々に費用やターンを計算しない。

- 開いている session card / pane に 5 項目と billing chip をまとめた読み取り専用の進捗帯を出す。単一表示・multi-pane で同じ session の model を参照する。
- 左一覧もターン、tool、経過、額、最後の発言を短く出す。長い発言は省略し、全文 tooltip ではなく同じ上限付きの安全な抜粋を使う。
- headless の既存 chip、状態、親子関係を維持する。progress の追加で入力欄や送信、paste、keyboard shortcut を有効にしない。
- 初期状態は「起動待ち / 未取得」、tool 終了後は「なし」、費用不明は「未取得」。通信切断は古い数字を進行中に見せず、その状態を添える。
- DOM へは textContent または既存 escape を通す。live region の過剰な読み上げを避ける。狭い幅で 5 項目が他カードの値に混ざらないよう合成 DOM で確認する。

主な変更予定:
- `rust/src/config/headless_billing.rs`（新規）、`config/paths.rs`、`files/scope.rs`: private recipe と公開除外
- `proto/core.rs`、`application/orchestrate_cli.rs`、`orchestration/child_options.rs`、`child_launch.rs`: 選択・検証・confirmation・trusted metadata
- `application/wrapped_spawn/launch.rs`、`wrapper/launch.rs`、`wrapper/hooks.rs`、`wrapper/entry.rs`: recipe 解決と設定の所有、出力境界
- `orchestration/headless.rs`、`headless_formats.rs`、新しい集計 module: 構造化観測
- `terminal/session/`、`hub/websocket.rs`、`hub/sockets.rs`: binding 検証、snapshot、順序付き配信
- `web/src/types/proto.ts`、`web/src/app/spawn-confirm*.ts`、`spawn-panel.ts`、`session-list.ts`、`session-strip.ts`、`multi-pane.ts`、WS 受信箇所、対応 CSS / locale / 合成 fixture

この一覧は責務の境界。実装前に actual caller を再確認し、関係ない refactor はしない。

## 9. 合成検証と独立レビュー

本物の CLI 推論、キー、有料 API は不要。fake provider / resolver と固定時刻の fixture を使う。

- plan 既定、API 明示、未対応 provider、未知 recipe、budget の不正値、interactive との併用、競合 route / auth の拒否
- execution-mode / effort / model の既存引数が二重化されない。Codex の既存除外と permission を保持
- recipe と settings の値が公開 config / profile export / エラー / WS / replay / history に出ない。helper の stdout を Hub が読まない
- 異常 stderr / 壊れた JSON / chunk 境界 / 認証エラー / 途中キャンセル / 一時 file 削除・残骸回収
- duplicate message、累積 token delta、複数 tool、result.num_turns、result.total_cost_usd、unknown price、cache 内訳、欠落と不自然なゼロ
- result と exit の順序違い、lag、古い binding / run ID / sequence、snapshot と live の競合
- 同時 2 子、カード切替、multi-pane、再読込、終了後の表示停止、XSS、headless 入力禁止
- 既存の Go oracle / generated DTO はそのまま。新しい Rust-only 契約に追加試験を置く

現時点で実施したのは source 読み取り、clone と祖先検査、dependency のサンプル取得、既存の text-hygiene / instrumentation 検査。後者 2 件は基準 SHA で成功し、この設計の実装試験の成功とは呼ばない。

受付時は cargo / rustc / bun が無かった。まず Cargo.lock の adler2 2.0.1 と bun.lock の TypeScript 6.0.3 を実取得し、それぞれ SHA256 / SHA512 が lock と一致した。

その後、同じ会話で toolchain 導入の明示許可を受けた。専用 cloud 作業領域へ公式配布物から Rust 1.90.0（cargo 1.90.0、rustfmt 1.8.0、Clippy 0.1.90）と Bun 1.3.14+0d9b296af を導入し、公式 SHA256 と実際の version 出力を確認した。shell profile と security 設定は変更していない。

既存依存の取得は cargo fetch --locked --manifest-path rust/Cargo.toml が exit 0。Bun は manifest / lock の隔離コピーで --frozen-lockfile --ignore-scripts が exit 0（10 packages）。元 checkout は clean、Cargo.lock / bun.lock 不変。製品依存の追加なし。build / Rust・Web tests / CI は未実行であり、取得成功をテスト成功とはしない。

実装後は別担当が固定 REVIEW.md の範囲を code SHA に対してレビューし、Issue 14 に結果を書く。修正後は該当範囲を新 SHA で読み直す。Windows 実画面、実キー、別製品による全行レビュー、merge は未実施として残す。

## 10. CI の不一致: 受け入れ時に判断が必要

基準 SHA の既存 workflow を読んだ結果:

- Validate は pull_request で動くが Go / Web 中心。secret-scan も PR / push にある。
- Rust Windows check は base branch push と手動起動用。job 自体が base ref のみの条件なので、今回の作業 branch を手動指定しても検査されない。
- Rust migration candidate は workflow_dispatch のみ。branch の github.sha を checkout し、Rust fmt / Clippy / tests / docs / build と receipt を持つので、コード上は任意 ref の検査候補になる。
- ただし default branch は main で、main 上の rust-migration.yml は 404。GitHub 公式の手動起動手順は workflow が default branch にあることを前提としている。このため現状のまま起動できるとは確認できない。

[手動起動の公式条件](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/manually-run-a-workflow)

CI workflow 変更と base / main push は禁止されている。こちらでは変更・起動しない。持ち主に「実装の exact SHA を検査できる既存 CI / runner の指定」または「別途 CI 経路を整備する判断」を求める。Go / Web の緑を Rust の合格にしない。

既存の Rust migration が持ち主側で起動可能と確認された場合のみ、手動で task branch を選ぶ候補とする。起動後は run の head_sha、checkout の HEAD、MANY_AI_REVIEW_HEAD_SHA、CANDIDATE-INPUT / BUILD-RECEIPT の source_sha が実装 code SHA と一致すること、該当検査が skip されず実行されたことを確認する。PR の merge SHA や古い code SHA の成功を取り違えない。CI の run URL と結果を Issue / 看板へ記録する。

## 11. ここで停止

受け入れてほしい内容:
1. 初回は Claude。新規 spawn はプランログイン既定、API mode だけ recipe と budget を使う。
2. Hub はキー値を持たず、CLI にだけ取得させる。競合や不明な認証は起動前に止める。
3. 途中額は観測した主実行分の見込み、終端は CLI 結果額。請求書の確定額や厳密な支出上限とは呼ばない。
4. Rust CI の実装 SHA を検査する経路を、持ち主が指定・判断する。

同じ案件の会話に設計受け入れが来るまで、実装 commit は行わない。CI 経路の未解決も明示したままにする。新規依存、課金、認証変更、禁止範囲の変更が必要になった場合は追加で停止する。Draft PR は受け入れ後の実装・レビュー工程で 1 件作成する。
