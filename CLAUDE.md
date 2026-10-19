# many-ai-cli 開発ガイド

> 最終更新: 2026-10-19(月) 23:34:48 — 本線を Rust 版（develop）へ移したことに合わせて概要・構成・作業ルールを書き換えた
> 2026-10-16(金) 11:56:03 — 利用者指定で見送り台帳 D-01〜D-13 と custom_providers の制限を全件撤廃し、判断基準を利用者の使いやすさに改めた
> 2026-10-11(日) 20:36:31 — 共通UI部品・確認文言と機械検査の索引を追加
> 2026-10-09(金) 19:36:22 — 全修正の普段のビルド元への反映を明文化
> 2026-09-29(火) 21:14:45 — prompt-audit の指摘で、古くなった版数の段落を削り、サブコマンド表を現行の分岐に合わせ、`internal/` の列挙を索引への参照に替えた
> 2026-09-28(月) 19:34:08 — 設計原則の索引「引き継ぎ看板と…」の行に、作業メモ（`memo_store.go`）に書かれたパスも `isPathMentionedInMemos` 経由で読み取り専用に開ける旨を追記（plan_memo-panel.md C4）
> 2026-09-26(土) 01:08:47 — 引き継ぎメモの表示を、記録済みパスに対する明示的な Markdown プレビューとして設計原則の索引へ反映。索引化の経緯（2026-08-19）: 常時ロード分が 5 週間で 129 → 269 行に倍増したため「制定」節の本文を正本へ移した。再肥大は `scripts/check-claude-md.mjs` が CI で止める

> **このファイルは索引であって本文ではない。** 全 AI セッションで全文がロードされるので、本文を置くと全員のコンテキストを毎回消費する。詳細は各行が指す正本を読む。タスク別の詳細は `CLAUDE/*.md`。

> **On translations / 翻訳について**
>
> This development guide is maintained in **Japanese only** — the Japanese text above and below is the authoritative version. There is no English edition of this file. (The user-facing README *is* available in English: [README.md](README.md), with [README.ja.md](README.ja.md) and [README.vi.md](README.vi.md).)
>
> [CLAUDE.vi.md](CLAUDE.vi.md), [docs/vi/README.md](docs/vi/README.md) and the `docs/vi/manual_*.vi.md` files are a **point-in-time snapshot (2026-08-15)** kindly contributed by a community translator. They are **no longer kept in sync** with this guide, so please read them as background rather than as current rules, and check the Japanese original before acting on anything. This does **not** apply to the Vietnamese **UI** locale (`web/src/i18n/vi.json`), which is a shipped feature and is maintained normally.
>
> If you work in another language, we're sorry to ask — please translate as needed on your side (a machine translation of this file is usually enough). Translation contributions are genuinely welcome; we just can't promise to keep them in step with the Japanese original, so anything merged will be treated the same way: a dated snapshot.

## プロジェクト概要

**開発方針（2026-10-09 利用者指定、2026-10-19 に本線を移行）:** 本線は Rust 版で、`develop` で開発し v1.0.0 でリリースする予定。全ての修正は普段のビルド元への反映まで行う（`CLAUDE/development.md`）。Go 版のソース（`cmd/` `internal/` `go.mod`）は凍結してあり、ビルド・改修・追加調査・回帰確認は行わない（Rust のテストが固定基準として読むだけ。`go/` へ移す予定）。

**many-ai-cli** — 複数のAIコーディングCLI（Claude Code / Codex CLI など。対応一覧は下の用語表の Provider 行）を並列で動かすときの **承認操作・進捗監視を 1 画面の Web ダッシュボードで一元管理** するツール。Rust の実行ファイル 2 本（Hub 常駐 + ラッパー機能の `many-ai-cli`、ランチャーの `many-ai-cli-launcher`）+ ブラウザ UI（xterm.js / TypeScript）。

**仕様の正本**は README（公開仕様）とソースコード（実装仕様）。v0.3.x の設計書は [docs/local/archive/v0.3.x/v0.3.x-many-ai-cli-design.md](docs/local/archive/v0.3.x/v0.3.x-many-ai-cli-design.md) へ退避した（非公開・履歴）。リリースごとの変更は [CHANGELOG.md](CHANGELOG.md)。**実装状況をここに書き写さない**（すぐ古くなり、二重管理になる）。

## 用語・名称

| 項目 | 値 |
|------|------|
| プロダクト名 / バイナリ名 | `many-ai-cli`（Windows: `many-ai-cli.exe`） |
| サブコマンド | `serve` / `connect` / `setup` / `doctor` / `issue` / `wrap <provider>`（`<provider>` 単体でも同じ）/ `shell-init` / `stop` / `status` / `tray` / `profile-export` / `log-clean` / `uninstall` / `version`。ほかに `provider` と隠しサブコマンド `usage-relay` / `orchestrate`（正本は `rust/src/cli.rs` のサブコマンド分岐） |
| Hub URL | `http://127.0.0.1:47777/?token=<random>` |
| 設定ファイル | `~/.many-ai-cli/config.yaml`（Win: `%USERPROFILE%\.many-ai-cli\config.yaml`） |
| ログ | `~/.many-ai-cli/logs/sessions/<provider>_<日時>_<folder>_s<id>.log/.jsonl/.txt` |
| 透過化環境変数 | `MANY_AI_CLI_AUTO=1` |
| Provider | `claude` / `codex` / `copilot` / `cursor-agent` / `opencode` / `grok` / `command-code`（`command-code` は承認検出が未検証・`gemini` は未対応・任意の CLI は `config.yaml` の `custom_providers`） |

> **grep 注意**: 旧名 `any-ai-cli` は新名 `many-ai-cli` の部分文字列（`m` + `any-ai-cli`）。旧名マーカーの残骸を新名パターンで grep すると **0 件に見える**。旧名側のパターンで grep すれば新旧どちらにも当たる。また `docs/local` と `docs/obsidian` はジャンクションで、AI の Glob / Grep ツールは中を辿らず同じく **0 件に見える**。この 2 つの列挙・検索は PowerShell で行う。

## 技術スタック・ディレクトリ構成

Rust 1.90.0（`rust/rust-toolchain.toml`。Cargo は `rust/Cargo.toml` の単一 package で、依存の版はそこに固定）/ 非同期は `tokio` / HTTP+WS は `axum`・`hyper` / PTY は `portable-pty` / 保存は `rusqlite`。フロントは静的 HTML/CSS + TypeScript(ESM) + esbuild + vendored xterm.js を `web/dist/` へ出し、`rust/build.rs` が検査して同梱する（`web/dist` が無いと cargo が止まるので、画面資材を先にビルドする）。

```
rust/src/bin/      実行ファイル 2 本（many-ai-cli / many-ai-cli-launcher）のエントリ
rust/src/hub/      HTTP+WS / セッション管理 / attach / spawn
rust/src/wrapper/  PTY ラッパー / 承認ルールの注入
rust/src/          ほか機能別モジュール（一覧と役割は .omitnix/index.json）
web/src/           フロントソース（web/dist/ は生成物・gitignore）
cmd/ internal/     凍結した Go 版（ビルド・改修しない。go/ へ移す予定）
docs/local/        設計書・plan 等（非公開）
```

## クロスプラットフォーム原則

- **OS固有コードは `#[cfg(windows)]` / `#[cfg(unix)]` で分離**する
- **パス操作は `Path` / `PathBuf::join`**（`/` ハードコード禁止）
- **改行・PTY 動作の差異**は `rust/src/wrapper/` と `rust/src/terminal/` で吸収し、上位層は OS 非依存に保つ
- **設定・ログのデフォルトディレクトリ**は全 OS 共通の `~/.many-ai-cli/`

## ローカルサーバの設計上の制約

- **バインドは `127.0.0.1` 固定**（外部公開しない）。デフォルトポート 47777、衝突時は 47778, 47779… と自動探索
- **ランダムトークンを起動時生成し URL に付与**（`?token=xxx`）
- テレメトリは送信しない。ただしスラッシュコマンド一覧取得等で GitHub へ HTTPS 通信する（README のセキュリティ節参照）
- **`.bashrc` 等への永続書き込みなし**（透過化は環境変数 + `eval "$(many-ai-cli shell-init)"` のオプトインのみ）

## 設計原則の索引（本文は正本にある）

事故から生まれた設計ルール。**本文はここに書かず、破る人が必ず開く場所に置いてある。** 機械検査があるものは、それが最終的な歯止め。`internal/` のパスと `Test...` は凍結した Go 版の正本で、Rust 版の対応箇所への付け替えは [pending](docs/local/pending_rust-design-index-remap.md) にある（ルール自体は Rust 版にもそのまま効く）。

| ルール | 正本（本文はここ） | 機械検査 |
|---|---|---|
| 承認の同一性は 1 本（`candidateKey` + `sourceEpoch`）だけ。誤表示を踏んでも抑止を足さない | `internal/hub/approval_identity.go` / `web/src/app/approval-answered.ts` | `TestApprovalSuppressionStateIsSingleSource` |
| 承認の表示は Hub が持つ保留中の記録を描くだけ。画面は端末の文字から承認を作らず、「保留中」も記録からだけ出す。承認マーカーの供給元はセッションに 1 つ（claude / codex は CLI のトランスクリプト、他は VT ミラー） | `internal/hub/approval_record.go` の冒頭 / `internal/hub/approval_marker_transcript.go` | `scripts/check-approval-display-source.mjs`（Validate CI） / `internal/hub/approval_display_guard_test.go` / `TestReplayApprovalSkipsVTMarkerWhenTranscriptIsSource` |
| 複数サブスクリプションは設定ディレクトリを env で切るだけ。token を持たない | `internal/subscription/adapter.go` のパッケージ doc | `TestLiveSessionAuthIsNeverSwapped` ほか 2 件 |
| auto 選択は spawn 時の round-robin だけ。残量を見て自動で別契約へ乗り換えない | `internal/hub/subscription.go` の `pickAutoSubscription` 冒頭 | `TestAutoSubscriptionNeverConsultsUsage` |
| 利用者のファイルへ書く機能は「次回起動時の回収」まで設計する | `internal/doctor/residue.go` の冒頭 | `many-ai-cli doctor` の置き去り検査 / `scripts/check-approval-rules-residue.mjs`（commit 混入） |
| 調査用の観測コードは同じコミットで `instrumentation.json` へ登録し、リリース成果物には入れない（build tag オプトイン＋成果物検査） | `scripts/check-instrumentation.mjs` の冒頭 / `web/src/debug/probe.ts` / `docs/local/archive/v0.8.x/plan_instrumentation-probe-lifecycle.md` | `scripts/check-instrumentation.mjs`（Validate CI） / `scripts/check-artifact-clean.mjs` |
| 版数を手で直す場所は無い（タグが単一ソース。古いままが正常） | `scripts/check-version-sources.mjs` の冒頭 | 同スクリプト（Validate CI） |
| `resources/` は `main` へ push した時点で全ユーザーへ live 配信される | [`resources/README.md`](resources/README.md) | `scripts/check-slash-commands.mjs` |
| profile へ持ち込む設定は additive のみ。credential は運ばず、書くのは自分のツリーの中だけ | `internal/subscription/seed.go` のパッケージ doc | `TestSeedIsAdditiveOnly` / `TestSeedNeverCarriesCredentials` |
| 本ファイルを索引のまま保つ（本文を書き戻さない） | `scripts/check-claude-md.mjs` の冒頭 | 同スクリプト（Validate CI） |
| 代替画面を CLI が全面管理している端末へこちらのテキストを差し込まない（消せない残骸になる）／マーカーは端末の折り返しで分断される前提で照合する（連続バイト完全一致にしない）／端末出力を溜めて捨てる状態機械を置かない（再描画フレームに CLOSE が描かれないことがある） | `web/src/app/hub-marker-filter.ts` の案 H・案 I・案 J | `hub-marker-filter-fixtures.ts` の案 H 5 件・案 I 8 件・案 J 6 件 |
| 全画面オーバーレイには wheel 除外クラス `.aac-wheel-overlay` を付ける（端末に重なるだけのポップオーバーは `data-wheel-native`） | `web/src/app/terminal.ts` の `isModalOverlayOpen()` | `scripts/check-wheel-overlays.mjs`（Validate CI） |
| 画面の JS を読み込んだ瞬間に、循環 import の相手の変数を同期で読まない（TDZ で評価が止まり「読み込み中...」のまま固まる。遅らせるなら `queueMicrotask`）。止まったときは黙らず失敗の帯を出す | `web/src/app/boot-guard.ts` の冒頭 / `scripts/check-web-module-init.mjs` の冒頭 | `scripts/check-web-module-init.mjs`（Validate CI） / `web/tests/boot-guard.test.ts` |
| relay は Hub の状態機械で回し、AI conductor に判断させない。利用者ブランチへは触らない | `internal/hub/relay.go` / `internal/hub/relay_worktree.go` | `TestRelay_*`（`internal/hub/`） |
| サイドバーの配置は 1 本の木から導く（兄弟順と折りたたみ以外でノードが器を越えない） | `web/src/app/sidebar-tree.ts` | `sidebar-tree-fixtures.ts` |
| 引き継ぎ看板とサブエージェントの木は、入れてよいものだけを型で受け、それ以外は入れる口を作らない。メモ本文は Record に入れず、記録済みパスを利用者が明示して開いた場合だけ Hub の読み取り専用 Markdown ビューアが表示できる。会話ログは引き続き後継のツールだけが開く。利用者の作業メモ（`internal/hub/memo_store.go`）に書かれたパスも、論理リモートからは `isPathMentionedInMemos`（`files_content.go`）経由で同様に読み取り専用で開ける（秘密 denylist は変わらず先に適用） | `internal/handoff/handoff.go` の冒頭コメント / `internal/proto/messages.go` の `SubagentNode` / `internal/hub/files_content.go` の `isPathMentionedInMemos` | `internal/handoff/handoff_test.go` の allowlist フィールド固定テスト / `TestSubagentNodeFieldsAreTheAllowlist` / `TestHandleFilesContent_MemoMentionedOutsidePathReadOnly` ほか `files_content_test.go` の Memo 系 |
| 残量ソースは provider 直書きの switch/slice ではなく 1 本の表で持つ。問い合わせて取る経路は作らない（`ReadUsage` を Adapter に足さない） | `internal/subscription/usage_source.go` | `internal/subscription/usage_source_test.go` |

**新しいルールを足したくなったら、まずこの表に 1 行足せる形にできないかを考える。** できないもの（機械検査も、決まったファイルも無いもの）だけが本文を持ってよい。

## 機能の採否は利用者の使いやすさで決める（2026-10-16 改定）

**提案・採否の基準は「利用者が使いやすくなるか」だけにする。** 2026-10-16 の利用者指定で、見送り台帳 D-01〜D-13 と `custom_providers` の制限（玄人設定・手書きのみ・UI から追加させない・フックは built-in のみ・サンプル限定。2026-08-31 決定）を全件撤廃した。[`docs/local/reference/reference_declined-directions.md`](docs/local/reference/reference_declined-directions.md) は過去の調査記録として読むだけにし、提案を止める根拠には使わない。「作者が使うまで作らない」「規模が小さいので採否を決められない」も根拠にしない。

作る前に確かめるのは、利用者に害が出るかどうかだけ。

- 規約上、利用者のアカウントが止められるおそれがある連携（旧 D-01 Gemini・Antigravity / D-07 NVIDIA NIM / D-09 ChatGPT Web の自動操作など）は、作る前に現行の規約を一次資料で確かめる
- 公開版に作者の個人パス・秘密を入れない（旧 D-08）。Hub token を画面・ログ・会話へ出さない

上の「設計原則の索引」は不具合の再発を止める実装上の約束で、撤廃の対象外。進行中の作業は [`docs/local/plan_user-first-config-and-providers.md`](docs/local/plan_user-first-config-and-providers.md)。

## AI 作業共通ルール

ビルド・コミット禁止、secrets-scan 責務、plan/bugfix/pending md の作成ルール等は、各利用者のグローバル AI 設定に従う（作者環境の例: `~/.claude/CLAUDE.md` と `~/.claude/guides/`）。本ファイルはプロジェクト固有ルールだけを扱う。

- ビルドだけでなく **実行・Hub 起動・ブラウザリロードも全てユーザーが行う**（`cargo run` / `serve` / `stop` / 再起動 / ブラウザリロード等）
- **ビルドコマンドは `make build` が基本**（画面資材 → `cargo build` の debug。毎日の試用は `scripts/rust-local.ps1`）。`bun run build` 単体は使わない。ユーザーの「ビルドして」は `make build` の実行指示を意味する
- **AI がコードを編集したら、ビルドせず構文だけ確認してよい**（Rust は `cargo +1.90.0 fmt --manifest-path rust/Cargo.toml -- --check`、TS は **`web` ディレクトリで** `bun run check`。リポジトリ直下に `package.json` は無く、直下で叩くと `Script not found` になる）。ただし**構文が通ることは動作の証明ではない**。AI 側で保証できるのは「parse が通る」までと明示する
- `many-ai-cli` 自身のセッションログは情報密度が低い。承認検出のバグ調査では、実際に動いている AI CLI 本体が書き出す生ログを見ること

## 監査・リリース証拠の境界

- 静的確認、ローカルテスト、CI 実行、release artifact、実機確認は**別の証拠**として報告する。未実行の CI・artifact・実機確認を完了扱いにしない
- Agent Chat の cursor 変更は parser 単体で完了扱いにせず、caller の offset 採用と次 poll の再開まで追跡・検証する
- `go:embed` 対象の Gitignored runtime は clean release job で取得・検証・artifact 化する。ローカルにファイルがあることを release の証拠にしない
- release workflow の手順・SHA・secret scope は [`.github/workflows/release.yml`](.github/workflows/release.yml) が正本。**ここに手順を複製しない**

## 詳細ガイド（タスク種別ベース）

タスクに該当する md だけ Read すること。**該当しない md は読み込まない**（Context 節約）。

| タスク種別 | 読むファイル |
|---|---|
| 調査・読み取り・質問応答 | （本ファイルのみ。`CLAUDE/*` は読まない） |
| Rust移行の看板・現在地・次の1個・再開 | [マスター計画](docs/local/plan_rust-migration-execution.md#看板の現在地) → [台帳・看板運用](docs/local/reference/manual_rust-migration-board.md)（ローカル資料） |
| 実装・コーディング（Rust / TypeScript）、共通UI部品・確認文言 | `CLAUDE/coding.md` / [docs/ui-components.md](docs/ui-components.md) / `scripts/check-ui-components.mjs`（Validate CI） |
| Rust版のローカルビルド・起動・停止（手元の手順） | [reference_rust-local-build.md](docs/local/reference/reference_rust-local-build.md)（ローカルに存在する場合） |
| ビルド・配布・クロスコンパイル | `CLAUDE/deployment.md` |
| 全修正のビルド元への反映・context分割・docs命名・AI作業モデル・plan自走/停止条件 | `CLAUDE/development.md` |
| Git・コミット・出力ルール | `CLAUDE/operations.md` |
| Windows 開発環境固有設定 | `CLAUDE/windows_setup.md` |

**`plan_*.md` の作成・実行、`docs/` 配下の `.md` の新規作成・更新に着手する前は、必ず `CLAUDE/development.md` を Read すること**（context分割・自走条件・停止条件・最終更新日時記載 等の正本）。

## 参照リンク

| 項目 | パス |
|------|------|
| **どのファイルが何をして、どのテーブルを読み書きするか**（探す前にここ） | `.omitnix/index.json`。全ファイルの索引とテーブル逆引き。**解析できなかったファイルも名前と理由付きで載る**ので「索引に無い」と「読めなかった」を取り違えない。参照 0 件は「未使用」ではない。**`generated.commit` が HEAD と違えば索引はその commit 時点のもの**なので、古いまま断定せず `omitnix` で作り直すか、古いことを添えて答える |
| 設計書 v0.3.x（履歴） | [docs/local/archive/v0.3.x/v0.3.x-many-ai-cli-design.md](docs/local/archive/v0.3.x/v0.3.x-many-ai-cli-design.md) |
| 設計書 v0.2.0 / v1（履歴） | [docs/local/archive/v0.2.2/v0.2.x-any-ai-cli-design.md](docs/local/archive/v0.2.2/v0.2.x-any-ai-cli-design.md) / [docs/local/archive/v0.1.3/v0.1.x-any-ai-cli-design.md](docs/local/archive/v0.1.3/v0.1.x-any-ai-cli-design.md) |
| Codex 用補足 | [AGENTS.md](AGENTS.md)（ローカル補足があれば `AGENTS.local.md`） |
| Gemini 用補足 | [GEMINI.md](GEMINI.md)（**wrap 対象外**。本リポジトリで Gemini CLI を開発補助に使う場合の手引き） |

<!-- OPENWIKI:START -->

## OpenWiki

See [AGENTS.md](AGENTS.md) for OpenWiki agent instructions.

<!-- OPENWIKI:END -->
