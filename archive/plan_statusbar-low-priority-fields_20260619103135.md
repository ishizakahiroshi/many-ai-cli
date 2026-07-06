# [完了] ステータスバーに残りの低優先 statusLine フィールドと Codex 対称情報を引き継ぐ

> 最終更新: 2026-06-19(金) 11:43:02

## context配分

| C | 種別 | 内容 | 並列 |
|---|---|---|---|
| C1 | fix | 共有型に低優先フィールド追加（proto/messages.go + web/src/types/proto.ts）— version / output_style / vim_mode / agent_name / repo_owner / repo_name / remaining_pct | — |
| C2 | fix | relay→Hub 中継（usagerelay.go パース+payload / usage_stat.go req・struct・clamp・broadcast / server.go snapshot）+ テスト | — |
| C3 | fix | Web 表示（token-statusbar.ts: ctx tooltip に残り% / agent セグメントに agent.name・version / project に repo owner・name + i18n + DETAIL_SEGMENTS + TOGGLEABLE_SEGMENTS） | — |
| C4 | fix | Codex 対称情報の調査と引き継ぎ（rollout JSONL の token_count info を精査し、引ける有用値があれば Codex 経路で中継・表示） | — |

実行順序: `C1 → C2 → C3 → C4`（C1〜C3 は同一共有ファイル群を触るため直列。C4 は Codex 経路調査主体だが usagerelay.go を C2 と共有するため C2 完了後）

---

## 概要

直前の `plan_statusbar-native-fields-relay`（[様子見]・実装済み）で、Claude Code statusLine の主要算出値（rate_limits / lines / effort / thinking / exceeds_200k / durations）をステータスバーへ引き継いだ。その際 **△低優先としてスコープ外** にした残りのフィールドを、同じ relay→Hub→Web 直結パイプに乗せて引き継ぐための将来計画。

引き継ぎ候補（公式 statusLine ドキュメント `https://code.claude.com/docs/en/statusline.md` で裏取り済みのフィールド）:

- `version` — Claude Code バージョン。承認パターン検出が CLI バージョン依存のため、どの版で動いているかはデバッグ価値あり（常時表示は過剰 → tooltip / 詳細モーダル向き）。
- `output_style.name` — 現在の出力スタイル。ニッチ。
- `vim.mode` — vim モード（NORMAL/INSERT/…）。Web 操作とは無関係だが statusLine 経由で取得可。優先度低。
- `agent.name` — `--agent` 実行時のエージェント名。サブエージェント運用時に有用。
- `workspace.repo.{host,owner,name}` — git origin から解決済みのリポジトリ identity。既存 project セグメントの補強（owner/name 表示）。
- `context_window.remaining_percentage` — 残り%（`100 − used` で導出可だが、本体算出値を直結すれば端数まで一致）。ctx tooltip 向き。

加えて、Codex 側の対称情報（rollout JSONL の `token_count.info` に含まれる reasoning tokens 等）を精査し、引き継げる有用値があれば Codex 経路でも中継する（C4）。

**スコープ外**: 上記以外の statusLine フィールド（session_id / transcript_path / cwd 等、既に別経路で取得済み or 表示価値が薄いもの）。新規セグメントの大量追加（横幅圧迫を避け、原則 **既存セグメントの tooltip / 補強** で吸収する方針）。

## 現状と問題

- 上記フィールドは relay のパース構造体（`claudeStatusLineInput`）・`hubUsagePayload`・Hub の usage_stat 経路・`proto.Message`・Web の `UsageCacheEntry` のいずれにも乗っていない。
- ctx セグメントの tooltip は「used / limit tokens」のみで、本体算出の残り%を出していない（自前 `100−used` で近似は可能だが未実装）。
- project セグメントは folder 名のみで、リポジトリ owner/name（モノレポや同名 folder の識別）を出していない。
- Codex セッションは token_count しか引いておらず、reasoning tokens 等の Codex 固有メタを取りこぼしている可能性がある（要調査）。

## 方針

1. C1〜C3 は直前 plan と同型のレイヤー別実装。relay がフィールドを追加パースし、Hub が中継、Web は **新規セグメントを増やさず既存セグメントの tooltip / 補強** で表示する（横幅圧迫の回避を優先）。
   - `remaining_pct` → ctx セグメント tooltip に「残り X%」を追記。
   - `agent_name` → agent セグメントに `[agent名]` を併記（存在時のみ）。`version` は agent セグメント tooltip に。
   - `repo_owner` / `repo_name` → project セグメント tooltip（または `owner/name` の小表示）に。
   - `vim_mode` / `output_style` → 当面は詳細モーダル（DETAIL_SEGMENTS）止まり、もしくは保留（表示価値を見て判断）。
2. Claude のみ供給。Codex / 未取得は従来どおりフォールバック（値なしで非表示・tooltip 省略）。クランプ方針は直前 plan に倣う。
3. C4 は調査主体。まず Codex の rollout JSONL を実データで精査し、引き継ぐ価値のあるフィールドを確定してから中継・表示を実装する（価値が無ければ「対象なし」と結論し C4 を畳む）。
4. 表示を増やす項目は、直前 plan で実装した **セグメント表示/非表示トグル（`TOGGLEABLE_SEGMENTS`）** にも必要に応じて追加し、デフォルト表示は厳選する。

不採用: 低優先フィールドごとに新規セグメントを増やす案。横幅が逼迫し UX が劣化するため、原則 tooltip / 既存セグメント補強で吸収する。

---

## C1: 共有型に低優先フィールドを追加

### 作業内容

`internal/proto/messages.go` の usage_stat 系 Message と `web/src/types/proto.ts` の `Message` に以下を追加（全て omitempty / optional、Claude のみ供給）。直前 plan の statusbar 追加メタ群の近傍に並べる。

- `version` (string) / `output_style` (string) / `vim_mode` (string) / `agent_name` (string)
- `repo_host` (string) / `repo_owner` (string) / `repo_name` (string)
- `remaining_pct` (float64)

### 変更予定ファイル

- `internal/proto/messages.go` — Message 構造体に追加（struct タグ整列は LF 正規化 gofmt で確認）。
- `web/src/types/proto.ts` — `Message` 型に snake_case で追加。

### 完了条件

- `go build ./...` と `bun run check`（tsc）が型エラーなく通る。
- Go/TS のミラーが一致し、JSON タグが snake_case 規則と整合。

---

## C2: relay→Hub 中継経路で低優先フィールドを伝搬

### 作業内容

- `claudeStatusLineInput` に対応パース（`Version` / `OutputStyle.Name` / `Vim.Mode` / `Agent.Name` / `Workspace.Repo.{Host,Owner,Name}` / `ContextWindow.RemainingPercentage`）を追加し `hubUsagePayload` へ。
- `usage_stat.go`: req 構造体・内部 `usageStat`・broadcast・クランプ（remaining_pct は 0–100、文字列はサニタイズ + 長さ制限）。
- `server.go`: 接続時 snapshot にも反映。
- `usagerelay_test.go`: 低優先フィールドを含む statusLine JSON で payload まで伝わることを検証。

### 変更予定ファイル

- `internal/usagerelay/usagerelay.go` — パース構造体・payload・`runClaude` 詰め込み。
- `internal/usagerelay/usagerelay_test.go` — 検証ケース追加。
- `internal/hub/usage_stat.go` — req・struct・クランプ・broadcast。
- `internal/hub/server.go` — snapshot 送信箇所。

### 完了条件

- `go test -count=1 ./internal/usagerelay/... ./internal/hub/...` が通る。
- 低優先フィールドが payload→Message まで欠落なく伝わる（テストで確認）。
- Codex 経路は 0/空のまま流れる。

---

## C3: Web 表示（既存セグメントの tooltip / 補強で吸収）

### 作業内容

- `UsageCacheEntry` に新フィールドを追加し `handleUsageStatMessage` で格納。
- ctx セグメント tooltip に「残り {remaining_pct}%」を追記（Claude かつ値有効時）。
- agent セグメントに `agent_name`（存在時）を併記、tooltip に `version`。
- project セグメント tooltip に `repo_owner/repo_name`。
- `vim_mode` / `output_style` は当面 DETAIL_SEGMENTS（詳細モーダル）止まり、または保留判断。
- 必要なら `TOGGLEABLE_SEGMENTS` / i18n(ja/en) / css を追補。

### 変更予定ファイル

- `web/src/app/token-statusbar.ts` — `UsageCacheEntry` / `handleUsageStatMessage` / 各セグメントの tooltip・補強。
- `web/src/i18n/ja.json` / `web/src/i18n/en.json` — 追加キー（必要時）。
- `web/src/styles/token-statusbar.css` — 補強表示のスタイル（必要時）。

### 完了条件

- `bun run check`（tsc）が通る。
- Claude セッションで残り%・agent名・version・repo identity が確認でき、本体 statusLine と整合。
- Codex / 未取得で破綻しない。

---

## C4: Codex 対称情報の調査と引き継ぎ

### 作業内容

- Codex の rollout JSONL（`token_count.info`）を実データで精査し、`reasoning_output_tokens` 等の Codex 固有メタのうち表示価値のあるものを洗い出す（`scanLastTokenCount` 近傍）。
- 価値があれば Codex 経路（`runCodex`）で中継し、Web の該当セグメント tooltip / 補強に反映。
- 価値が無ければ「対象なし」と結論し、本 C を畳む（その旨を本 plan に追記）。

### 変更予定ファイル

- `internal/usagerelay/usagerelay.go` — `runCodex` / `tokenCountEvent` の拡張（調査結果次第）。
- `internal/usagerelay/usagerelay_test.go` — Codex サンプルの検証（調査結果次第）。
- `web/src/app/token-statusbar.ts` — 表示反映（調査結果次第）。

### 完了条件

- Codex token_count info に引き継ぐ価値があるか調査結果を本 plan に記録。
- 引き継ぐ場合は payload→表示まで通し、`go test` / `bun run check` が通る。
- Codex rollout JSONL の実データ精査で対象フィールドを確定済み。

### 実行結果

- 既存テストに含まれていた Codex rollout JSONL 実データの `token_count.info.total_token_usage.reasoning_output_tokens` を有用値として採用。
- `reasoning_output_tokens` を Codex relay → Hub `usage_stat` → Web `UsageCacheEntry` へ中継し、tok セグメント tooltip に表示する実装を追加。
- 検証: `go test -count=1 ./internal/usagerelay/... ./internal/hub/...`、`web` 配下で `bun run check` が通過。
