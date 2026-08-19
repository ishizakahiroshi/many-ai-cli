# many-ai-cli 開発ガイド

> 最終更新: 2026-08-19(水) — **本ファイルを索引へ再編した。** 常時ロード分が 5 週間で 129 → 269 行に倍増していたため、日付入りの「制定」節 7 本の本文を正本（コード・検査スクリプト・台帳）へ移し、ここには索引の 1 行ずつだけを残した。再肥大は `scripts/check-claude-md.mjs` が CI で止める

> **このファイルは索引であって本文ではない。** 全 AI セッションで全文がロードされるので、本文を置くと全員のコンテキストを毎回消費する。詳細は各行が指す正本を読む。タスク別の詳細は `CLAUDE/*.md`。

> **On translations / 翻訳について**
>
> This development guide is maintained in **Japanese only** — the Japanese text above and below is the authoritative version. There is no English edition of this file. (The user-facing README *is* available in English: [README.md](README.md), with [README.ja.md](README.ja.md) and [README.vi.md](README.vi.md).)
>
> [CLAUDE.vi.md](CLAUDE.vi.md), [docs/README.vi.md](docs/README.vi.md) and the `docs/manual_*.vi.md` files are a **point-in-time snapshot (2026-08-15)** kindly contributed by a community translator. They are **no longer kept in sync** with this guide, so please read them as background rather than as current rules, and check the Japanese original before acting on anything. This does **not** apply to the Vietnamese **UI** locale (`web/src/i18n/vi.json`), which is a shipped feature and is maintained normally.
>
> If you work in another language, we're sorry to ask — please translate as needed on your side (a machine translation of this file is usually enough). Translation contributions are genuinely welcome; we just can't promise to keep them in step with the Japanese original, so anything merged will be treated the same way: a dated snapshot.

## プロジェクト概要

**many-ai-cli** — 複数のAIコーディングCLI（Claude Code / Codex CLI）を並列で動かすときの **承認操作・進捗監視を 1 画面の Web ダッシュボードで一元管理** するツール。単一 Go バイナリ（Hub 常駐 + ラッパー機能）+ ブラウザ UI（xterm.js / TypeScript）。

**設計書（正本）**: [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md)。リリースごとの変更は [CHANGELOG.md](CHANGELOG.md)。**実装状況をここに書き写さない**（すぐ古くなり、二重管理になる）。

v0.7.0 まで出荷済み。v0.4.0 で Workbench と Hub 内蔵チャットプロキシを撤去、v0.5.0 で `setup` / `doctor` / autoapproval、v0.6.0 で transcript ベースのチャット本文、v0.7.0 で承認同一性の一本化とトレイ常駐を追加した。

## 用語・名称

| 項目 | 値 |
|------|------|
| プロダクト名 / バイナリ名 | `many-ai-cli`（Windows: `many-ai-cli.exe`） |
| サブコマンド | `serve` / `wrap <provider>` / `shell-init` / `setup` / `doctor` / `tray` / `stop` / `status` / `uninstall` / `version` |
| Hub URL | `http://127.0.0.1:47777/?token=<random>` |
| 設定ファイル | `~/.many-ai-cli/config.yaml`（Win: `%USERPROFILE%\.many-ai-cli\config.yaml`） |
| ログ | `~/.many-ai-cli/logs/sessions/<provider>_<日時>_<folder>_s<id>.log/.jsonl/.txt` |
| 透過化環境変数 | `MANY_AI_CLI_AUTO=1` |
| Provider | `claude` / `codex` / `copilot` / `cursor-agent` / `opencode` / `grok`（`gemini` は対象外・見送り台帳 D-01） |

> **grep 注意**: 旧名 `any-ai-cli` は新名 `many-ai-cli` の部分文字列（`m` + `any-ai-cli`）。旧名マーカーの残骸を新名パターンで grep すると **0 件に見える**。旧名側のパターンで grep すれば新旧どちらにも当たる。

## 技術スタック・ディレクトリ構成

Go（クロスコンパイルで Win/Mac/Linux 単一バイナリ）/ PTY は `creack/pty`（Unix）+ `aymanbagabas/go-pty`（Windows ConPTY）/ HTTP は `net/http` / WS は `golang.org/x/net/websocket` / 設定は `gopkg.in/yaml.v3` / ログは `log/slog`。フロントは静的 HTML/CSS + TypeScript(ESM) + esbuild + vendored xterm.js を `web/dist/` へ出し `go:embed` で同梱。

```
cmd/many-ai-cli/main.go   単一バイナリのエントリポイント
internal/hub/             HTTP+WS / セッション管理 / attach / spawn
internal/wrapper/         PTY ラッパー（OS 別実装）/ attach inject
internal/                 ほか approval / attach / config / doctor / hubruntime / launcher /
                          log / notify / orchestrate / proto / sessionlog / sessionstore /
                          shell / subscription / tray / uninstall / usagerelay / wslutil
web/src/                  フロントソース（web/dist/ は生成物・gitignore）
docs/local/               設計書・plan 等（非公開）
```

## クロスプラットフォーム原則

- **OS固有コードは build tag で分離**（例: `pty_unix.go` / `pty_windows.go`）
- **パス操作は `filepath.Join` / `os.UserHomeDir`**（`/` ハードコード禁止）
- **改行・PTY 動作の差異**は `internal/wrapper/` で吸収し、上位層は OS 非依存に保つ
- **設定・ログのデフォルトディレクトリ**は全 OS 共通の `~/.many-ai-cli/`

## ローカルサーバの設計上の制約

- **バインドは `127.0.0.1` 固定**（外部公開しない）。デフォルトポート 47777、衝突時は 47778, 47779… と自動探索
- **ランダムトークンを起動時生成し URL に付与**（`?token=xxx`）
- テレメトリは送信しない。ただしスラッシュコマンド一覧取得等で GitHub へ HTTPS 通信する（README のセキュリティ節参照）
- **`.bashrc` 等への永続書き込みなし**（透過化は環境変数 + `eval "$(many-ai-cli shell-init)"` のオプトインのみ）

## 設計原則の索引（本文は正本にある）

事故から生まれた設計ルール。**本文はここに書かず、破る人が必ず開く場所に置いてある。** 機械検査があるものは、それが最終的な歯止め。

| ルール | 正本（本文はここ） | 機械検査 |
|---|---|---|
| 承認の同一性は 1 本（`candidateKey` + `sourceEpoch`）だけ。誤表示を踏んでも抑止を足さない | `internal/hub/approval_identity.go` / `web/src/app/approval-answered.ts` | `TestApprovalSuppressionStateIsSingleSource` |
| 複数サブスクリプションは設定ディレクトリを env で切るだけ。token を持たない | `internal/subscription/adapter.go` のパッケージ doc | `TestLiveSessionAuthIsNeverSwapped` ほか 2 件 |
| 利用者のファイルへ書く機能は「次回起動時の回収」まで設計する | `internal/doctor/residue.go` の冒頭 | `many-ai-cli doctor` の置き去り検査 |
| 調査用の観測コードは同じコミットで `instrumentation.json` へ登録する | `scripts/check-instrumentation.mjs` の冒頭 | 同スクリプト（Validate CI） |
| 版数を手で直す場所は無い（タグが単一ソース。古いままが正常） | `scripts/check-version-sources.mjs` の冒頭 | 同スクリプト（Validate CI） |
| `resources/` は `main` へ push した時点で全ユーザーへ live 配信される | [`resources/README.md`](resources/README.md) | `scripts/check-slash-commands.mjs` |
| 本ファイルを索引のまま保つ（本文を書き戻さない） | `scripts/check-claude-md.mjs` の冒頭 | 同スクリプト（Validate CI） |

**新しいルールを足したくなったら、まずこの表に 1 行足せる形にできないかを考える。** できないもの（機械検査も、決まったファイルも無いもの）だけが本文を持ってよい。

## 見送った方針は台帳で管理する（提案の前に読む）

**台帳: [`docs/local/reference_declined-directions.md`](docs/local/reference_declined-directions.md)。** 各項目に「再検討してよい条件」と「再検討の根拠にしてはいけないもの」が書いてある。見送りは永久否定ではないので、条件が満たされているなら再検討してよい。**新しい見送りは台帳に行を足す。ここに節を作らない。**

| ID | 見送った方針 |
|---|---|
| D-01 | Gemini CLI を wrap 対象に入れる（利用規約上の制約・2026-05-06 決定） |
| D-02 | 新規 provider を増やす（20 製品を調査し全件見送り） |
| D-03 | セッション カンバンビュー |
| D-04 | Windows デスクトップアプリ化（Tauri）と Microsoft Store 提出 |
| D-05 | Copilot / Cursor Agent の複数サブスクリプション対応 |
| D-06 | OpenRouter を route（接続先）として載せる |

**共通する却下理由**（個別に蒸し返さないため 1 度だけ書く）: 候補の人気・伸び・他社の対応状況・利用者増の見込みを根拠にしない。本ツールの規模（star 6 / npm 週次 11・2026-08-14 実測）では、それらを根拠に採否を決める枠組みが成立しない。起点は「作者が実際に使っていて、日常の並列運用に入っているか」。

## AI 作業共通ルール

ビルド・コミット禁止、secrets-scan 責務、plan/bugfix/pending md の作成ルール等は、各利用者のグローバル AI 設定に従う（作者環境の例: `~/.claude/CLAUDE.md` と `~/.claude/guides/`）。本ファイルはプロジェクト固有ルールだけを扱う。

- ビルドだけでなく **実行・Hub 起動・ブラウザリロードも全てユーザーが行う**（`go run` / `serve` / `stop` / 再起動 / ブラウザリロード等）
- **ビルドコマンドは `make build` が基本**。`bun run build` 単体は使わない。ユーザーの「ビルドして」は `make build` の実行指示を意味する
- **AI がコードを編集したら、ビルドせず構文だけ確認してよい**（Go は `gofmt -e`、TS は `bun run check`）。ただし**構文が通ることは動作の証明ではない**。AI 側で保証できるのは「parse が通る」までと明示する
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
| 実装・コーディング（Go / TypeScript） | `CLAUDE/coding.md` |
| ビルド・配布・クロスコンパイル | `CLAUDE/deployment.md` |
| context分割・docs命名・AI作業モデル・plan自走/停止条件 | `CLAUDE/development.md` |
| Git・コミット・出力ルール | `CLAUDE/operations.md` |
| Windows 開発環境固有設定 | `CLAUDE/windows_setup.md` |

**`plan_*.md` の作成・実行、`docs/` 配下の `.md` の新規作成・更新に着手する前は、必ず `CLAUDE/development.md` を Read すること**（context分割・自走条件・停止条件・最終更新日時記載 等の正本）。

## 参照リンク

| 項目 | パス |
|------|------|
| 設計書 v0.3.0（現行・正本） | [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md) |
| 設計書 v0.2.0 / v1（履歴） | [docs/v0.2.x-any-ai-cli-design.md](docs/v0.2.x-any-ai-cli-design.md) / [docs/local/archive/v0.1.3/cli-popup-design-v1.md](docs/local/archive/v0.1.3/cli-popup-design-v1.md) |
| Codex 用補足 | [AGENTS.md](AGENTS.md)（ローカル補足があれば `AGENTS.local.md`） |
| Gemini 用補足 | [GEMINI.md](GEMINI.md)（**wrap 対象外**。本リポジトリで Gemini CLI を開発補助に使う場合の手引き） |
