# 設計原則の索引（表）

> 最終更新: 2026-11-24(火) — provider の id の直書きを止める検査の行を足した（宣言化の作業ブランチの取り込み）
> 2026-11-17(火) 20:20:29 — 廃止した pending への参照を外し、CLAUDE.md と同じ文だった末尾の 1 段落を消した
> 2026-11-17(火) 17:09:27 — `CLAUDE.md` の「設計原則の索引」の表を、行を変えずにここへ移した。常時読ませる量に上限のある AI CLI にも `CLAUDE.md` が全部届くようにするため（上限は `scripts/check-claude-md.mjs` の冒頭）

**いつ読むか**: 実装・修正・設計の提案に入る前に、毎回読む。`@` を展開する AI CLI（Claude Code）には、`CLAUDE.md` の取り込みで常時届く。

事故から生まれた設計ルール。**本文はここに書かず、破る人が必ず開く場所に置いてある。** 機械検査があるものは、それが最終的な歯止め。`go/internal/` のパスと `Test...` は凍結した Go 版の正本で、Rust 版の対応箇所への付け替えはしていない（付け替えの pending は廃止した。対応する Rust の箇所は推測で書かず、実物を読んで確かめる。ルール自体は Rust 版にもそのまま効く）。

| ルール | 正本（本文はここ） | 機械検査 |
|---|---|---|
| 承認の同一性は 1 本（`candidateKey` + `sourceEpoch`）だけ。誤表示を踏んでも抑止を足さない | `go/internal/hub/approval_identity.go` / `web/src/app/approval-answered.ts` | `TestApprovalSuppressionStateIsSingleSource` |
| 承認の表示は Hub が持つ保留中の記録を描くだけ。画面は端末の文字から承認を作らず、「保留中」も記録からだけ出す。承認マーカーの供給元はセッションに 1 つ（claude / codex は CLI のトランスクリプト、他は VT ミラー） | `go/internal/hub/approval_record.go` の冒頭 / `go/internal/hub/approval_marker_transcript.go` | `scripts/check-approval-display-source.mjs`（Validate CI） / `go/internal/hub/approval_display_guard_test.go` / `TestReplayApprovalSkipsVTMarkerWhenTranscriptIsSource` |
| 複数サブスクリプションは設定ディレクトリを env で切るだけ。token を持たない | `go/internal/subscription/adapter.go` のパッケージ doc | `TestLiveSessionAuthIsNeverSwapped` ほか 2 件 |
| auto 選択は spawn 時の round-robin だけ。残量を見て自動で別契約へ乗り換えない | `go/internal/hub/subscription.go` の `pickAutoSubscription` 冒頭 | `TestAutoSubscriptionNeverConsultsUsage` |
| 利用者のファイルへ書く機能は「次回起動時の回収」まで設計する | `go/internal/doctor/residue.go` の冒頭 | `many-ai-cli doctor` の置き去り検査 / `scripts/check-approval-rules-residue.mjs`（commit 混入） |
| 調査用の観測コードは同じコミットで `instrumentation.json` へ登録し、リリース成果物には入れない（build tag オプトイン＋成果物検査） | `scripts/check-instrumentation.mjs` の冒頭 / `web/src/debug/probe.ts` / `docs/local/archive/v0.8.x/plan_instrumentation-probe-lifecycle.md` | `scripts/check-instrumentation.mjs`（Validate CI） / `scripts/check-artifact-clean.mjs` |
| 版数を手で直す場所は無い（タグが単一ソース。古いままが正常） | `scripts/check-version-sources.mjs` の冒頭 | 同スクリプト（Validate CI） |
| `resources/` は `main` へ push した時点で全ユーザーへ live 配信される | [`resources/README.md`](../resources/README.md) | `scripts/check-slash-commands.mjs` |
| profile へ持ち込む設定は additive のみ。credential は運ばず、書くのは自分のツリーの中だけ | `go/internal/subscription/seed.go` のパッケージ doc | `TestSeedIsAdditiveOnly` / `TestSeedNeverCarriesCredentials` |
| 本ファイルを索引のまま保つ（本文を書き戻さない） | `scripts/check-claude-md.mjs` の冒頭 | 同スクリプト（Validate CI） |
| provider の id（同梱 7 本）をコードに文字列で直接書かない。固定が要るときは定義の宣言（`rust/assets/provider/declarations.json`）へ出す | `scripts/check-provider-hardcode.mjs` の冒頭 / `scripts/provider-hardcode-baseline.json` | 同スクリプト（Validate CI）・`scripts/check-provider-hardcode.test.mjs` |
| 代替画面を CLI が全面管理している端末へこちらのテキストを差し込まない（消せない残骸になる）／マーカーは端末の折り返しで分断される前提で照合する（連続バイト完全一致にしない）／端末出力を溜めて捨てる状態機械を置かない（再描画フレームに CLOSE が描かれないことがある） | `web/src/app/hub-marker-filter.ts` の案 H・案 I・案 J | `hub-marker-filter-fixtures.ts` の案 H 5 件・案 I 8 件・案 J 6 件 |
| 全画面オーバーレイには wheel 除外クラス `.aac-wheel-overlay` を付ける（端末に重なるだけのポップオーバーは `data-wheel-native`） | `web/src/app/terminal.ts` の `isModalOverlayOpen()` | `scripts/check-wheel-overlays.mjs`（Validate CI） |
| 画面の JS を読み込んだ瞬間に、循環 import の相手の変数を同期で読まない（TDZ で評価が止まり「読み込み中...」のまま固まる。遅らせるなら `queueMicrotask`）。止まったときは黙らず失敗の帯を出す | `web/src/app/boot-guard.ts` の冒頭 / `scripts/check-web-module-init.mjs` の冒頭 | `scripts/check-web-module-init.mjs`（Validate CI） / `web/tests/boot-guard.test.ts` |
| relay は Hub の状態機械で回し、AI conductor に判断させない。利用者ブランチへは触らない | `go/internal/hub/relay.go` / `go/internal/hub/relay_worktree.go` | `TestRelay_*`（`go/internal/hub/`） |
| サイドバーの配置は 1 本の木から導く（兄弟順と折りたたみ以外でノードが器を越えない） | `web/src/app/sidebar-tree.ts` | `sidebar-tree-fixtures.ts` |
| 引き継ぎ看板とサブエージェントの木は、入れてよいものだけを型で受け、それ以外は入れる口を作らない。メモ本文は Record に入れず、記録済みパスを利用者が明示して開いた場合だけ Hub の読み取り専用 Markdown ビューアが表示できる。会話ログは引き続き後継のツールだけが開く。利用者の作業メモ（`go/internal/hub/memo_store.go`）に書かれたパスも、論理リモートからは `isPathMentionedInMemos`（`files_content.go`）経由で同様に読み取り専用で開ける（秘密 denylist は変わらず先に適用） | `go/internal/handoff/handoff.go` の冒頭コメント / `go/internal/proto/messages.go` の `SubagentNode` / `go/internal/hub/files_content.go` の `isPathMentionedInMemos` | `go/internal/handoff/handoff_test.go` の allowlist フィールド固定テスト / `TestSubagentNodeFieldsAreTheAllowlist` / `TestHandleFilesContent_MemoMentionedOutsidePathReadOnly` ほか `files_content_test.go` の Memo 系 |
| 残量ソースは provider 直書きの switch/slice ではなく 1 本の表で持つ。問い合わせて取る経路は作らない（`ReadUsage` を Adapter に足さない） | `go/internal/subscription/usage_source.go` | `go/internal/subscription/usage_source_test.go` |
