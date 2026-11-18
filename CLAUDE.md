# many-ai-cli 開発ガイド

> 最終更新: 2026-11-17(火) 20:20:29 — v0.10.0 の公開後も「予定」のままだった記述と、切れていた参照 3 件を実物に合わせた
> 2026-11-17(火) 17:09:36 — 設計原則の索引の表を `CLAUDE/design-principles.md` へ、過去の更新履歴を `CLAUDE/history.md` へ移した（理由と上限は `scripts/check-claude-md.mjs` の冒頭）
> 2026-11-03(火) 09:16:20 — 次のRust版をv0.10.0へ変更（これより前は `CLAUDE/history.md`）

> **このファイルは索引であって本文ではない。** 全 AI セッションで全文がロードされるので、本文を置くと全員のコンテキストを毎回消費する。詳細は各行が指す正本を読む。タスク別の詳細は `CLAUDE/*.md`。

> **On translations / 翻訳について**
>
> This development guide is maintained in **Japanese only** — the Japanese text above and below is the authoritative version. There is no English edition of this file. (The user-facing README *is* available in English: [README.md](README.md), with [README.ja.md](README.ja.md) and [README.vi.md](README.vi.md).)
>
> [CLAUDE.vi.md](CLAUDE.vi.md), [docs/vi/README.md](docs/vi/README.md) and the `docs/vi/manual_*.vi.md` files are a **point-in-time snapshot (2026-08-15)** kindly contributed by a community translator. They are **no longer kept in sync** with this guide, so please read them as background rather than as current rules, and check the Japanese original before acting on anything. This does **not** apply to the Vietnamese **UI** locale (`web/src/i18n/vi.json`), which is a shipped feature and is maintained normally.
>
> If you work in another language, we're sorry to ask — please translate as needed on your side (a machine translation of this file is usually enough). Translation contributions are genuinely welcome; we just can't promise to keep them in step with the Japanese original, so anything merged will be treated the same way: a dated snapshot.

## プロジェクト概要

**開発方針（2026-10-09 利用者指定、2026-10-19 に本線を移行）:** 本線は Rust 版で、`develop` で開発する。v0.10.0（2026-11-16 公開）から Rust 版を配布している。全ての修正は普段のビルド元への反映まで行う（`CLAUDE/development.md`）。Go 版のソースは `go/` に凍結してあり、ビルド・改修・追加調査・回帰確認は行わない（Rust のテストが固定基準として読むだけ）。

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
| Provider | `claude` / `codex` / `copilot` / `cursor-agent` / `opencode` / `grok` / `command-code`（`command-code` は承認検出が未検証・`gemini` は未対応・任意の CLI は `many-ai-cli provider add`・画面の「AI CLI を追加」、旧方式は `config.yaml` の `custom_providers`） |

> **grep 注意**: 旧名 `any-ai-cli` は新名 `many-ai-cli` の部分文字列（`m` + `any-ai-cli`）。旧名マーカーの残骸を新名パターンで grep すると **0 件に見える**。旧名側のパターンで grep すれば新旧どちらにも当たる。また `docs/local` と `docs/obsidian` はジャンクションで、AI の Glob / Grep ツールは中を辿らず同じく **0 件に見える**。この 2 つの列挙・検索は PowerShell で行う。

## 技術スタック・ディレクトリ構成

Rust 1.90.0（`rust/rust-toolchain.toml`。Cargo は `rust/Cargo.toml` の単一 package で、依存の版はそこに固定）/ 非同期は `tokio` / HTTP+WS は `axum`・`hyper` / PTY は `portable-pty` / 保存は `rusqlite`。フロントは静的 HTML/CSS + TypeScript(ESM) + esbuild + vendored xterm.js を `web/dist/` へ出し、`rust/build.rs` が検査して同梱する（`web/dist` が無いと cargo が止まるので、画面資材を先にビルドする）。

```
rust/src/bin/      実行ファイル 2 本（many-ai-cli / many-ai-cli-launcher）のエントリ
rust/src/hub/      HTTP+WS / セッション管理 / attach / spawn
rust/src/wrapper/  PTY ラッパー / 承認ルールの注入
rust/src/          ほか機能別モジュール（一覧と役割は .omitnix/index.json）
rust/assets/       Rust ビルドが同梱する資材（ランチャー画面・provider 定義・Windows ランタイム）
web/src/           フロントソース（web/dist/ は生成物・gitignore）
go/                凍結した Go 版（ビルド・改修しない）
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

事故から生まれた設計ルール。**本文はここに書かず、破る人が必ず開く場所に置いてある。** 機械検査があるものは、それが最終的な歯止め。表は [`CLAUDE/design-principles.md`](CLAUDE/design-principles.md) にある（承認・サブスクリプション・利用者のファイルへの書き込み・観測コード・版数・`resources/`・端末と画面・relay・サイドバー・引き継ぎ看板・残量 の 17 行）。

**実装・修正・設計の提案に入る前に、必ずその表を読む。** `@` を展開する AI CLI には、次の行で常時届く。

@CLAUDE/design-principles.md

**新しいルールを足したくなったら、まずその表に 1 行足せる形にできないかを考える。** できないもの（機械検査も、決まったファイルも無いもの）だけが本文を持ってよい。

## 機能の採否は利用者の使いやすさで決める（2026-10-16 改定）

**提案・採否の基準は「利用者が使いやすくなるか」だけにする。** 2026-10-16 の利用者指定で、見送り台帳 D-01〜D-13 と `custom_providers` の制限（玄人設定・手書きのみ・UI から追加させない・フックは built-in のみ・サンプル限定。2026-08-31 決定）を全件撤廃した。[`docs/local/reference/reference_declined-directions.md`](docs/local/reference/reference_declined-directions.md) は過去の調査記録として読むだけにし、提案を止める根拠には使わない。「作者が使うまで作らない」「規模が小さいので採否を決められない」も根拠にしない。

作る前に確かめるのは、利用者に害が出るかどうかだけ。

- 規約上、利用者のアカウントが止められるおそれがある連携（旧 D-01 Gemini・Antigravity / D-07 NVIDIA NIM / D-09 ChatGPT Web の自動操作など）は、作る前に現行の規約を一次資料で確かめる
- 公開版に作者の個人パス・秘密を入れない（旧 D-08）。Hub token を画面・ログ・会話へ出さない

上の「設計原則の索引」は不具合の再発を止める実装上の約束で、撤廃の対象外。

## AI 作業共通ルール

ビルドと Git 操作の承認範囲、secrets-scan 責務、plan/bugfix/pending md の作成ルール等は、各利用者のグローバル AI 設定に従う（作者環境の例: `~/.claude/CLAUDE.md` と `~/.claude/guides/`）。本ファイルはプロジェクト固有ルールだけを扱う。

- ビルドだけでなく **実行・Hub 起動・ブラウザリロードも全てユーザーが行う**（`cargo run` / `serve` / `stop` / 再起動 / ブラウザリロード等）
- **AI がコードを編集したら、ビルドせず構文だけ確認してよい**（Rust は `cargo +1.90.0 fmt --manifest-path rust/Cargo.toml -- --check`、TS は **`web` ディレクトリで** `bun run check`。リポジトリ直下に `package.json` は無く、直下で叩くと `Script not found` になる）。ただし**構文が通ることは動作の証明ではない**。AI 側で保証できるのは「parse が通る」までと明示する
- `many-ai-cli` 自身のセッションログは情報密度が低い。承認検出のバグ調査では、実際に動いている AI CLI 本体が書き出す生ログを見ること

## 監査・リリース証拠の境界

- 静的確認、ローカルテスト、CI 実行、release artifact、実機確認は**別の証拠**として報告する。未実行の CI・artifact・実機確認を完了扱いにしない
- Agent Chat の cursor 変更は parser 単体で完了扱いにせず、caller の offset 採用と次 poll の再開まで追跡・検証する
- リリースに同梱する資材は clean release job で取得・検証・artifact 化する。ローカルにファイルがあることを release の証拠にしない
- release workflow の手順・SHA・secret scope は [`.github/workflows/release.yml`](.github/workflows/release.yml) が正本。**ここに手順を複製しない**

## 詳細ガイド（タスク種別ベース）

タスクに該当する md だけ Read すること。**該当しない md は読み込まない**（Context 節約）。

| タスク種別 | 読むファイル |
|---|---|
| 調査・読み取り・質問応答 | （本ファイルのみ。`CLAUDE/*` は読まない） |
| Rust移行の看板・現在地・次の1個・再開 | [マスター計画](docs/local/plan_rust-migration-execution.md#看板の現在地) → [台帳・看板運用](docs/local/reference/manual_rust-migration-board.md)（ローカル資料） |
| 実装・コーディング（Rust / TypeScript）、共通UI部品・確認文言 | `CLAUDE/design-principles.md` / `CLAUDE/coding.md` / [docs/ui-components.md](docs/ui-components.md) / `scripts/check-ui-components.mjs`（Validate CI） |
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
| 設計書 v0.2.0 / v1（履歴） | [docs/local/archive/v0.2.x/v0.2.x-any-ai-cli-design.md](docs/local/archive/v0.2.x/v0.2.x-any-ai-cli-design.md) / [docs/local/archive/v0.1.x/v0.1.x-any-ai-cli-design.md](docs/local/archive/v0.1.x/v0.1.x-any-ai-cli-design.md) |
| Codex 用補足 | [AGENTS.md](AGENTS.md)（ローカル補足があれば `AGENTS.local.md`） |
| Gemini 用補足 | [GEMINI.md](GEMINI.md)（**wrap 対象外**。本リポジトリで Gemini CLI を開発補助に使う場合の手引き） |

<!-- OPENWIKI:START -->

## OpenWiki

See [AGENTS.md](AGENTS.md) for OpenWiki agent instructions.

<!-- OPENWIKI:END -->
