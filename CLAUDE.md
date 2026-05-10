# ai-cli-hub 開発ガイド

> 最終更新: 2026-05-11(月) 03:07:52

> 詳細は `CLAUDE/*.md` を参照。このファイルは常時ロード分のみ。

## プロジェクト概要

**ai-cli-hub** — 複数のAIコーディングCLI（Claude Code / Codex CLI）を並列で動かすときの **承認操作・進捗監視を 1 画面の Web ダッシュボードで一元管理** するツール。単一 Go バイナリ（Hub 常駐 + ラッパー機能）+ ブラウザ UI（xterm.js / Vanilla JS）。

> **Gemini CLI は wrap 対象外**（2026-05-06 決定 / 利用規約上の制約）。詳細は [docs/v0.1.x-ai-cli-hub-design.md](docs/v0.1.x-ai-cli-hub-design.md) 冒頭「スコープ更新ログ」参照。

**現状**: v0.1.1 を初回正式リリースとして準備中。v0.1.0 は試験リリース扱い。設計書はソースコードを正本として更新済み。

**設計書（正本）**: [docs/v0.1.x-ai-cli-hub-design.md](docs/v0.1.x-ai-cli-hub-design.md)

> 全AI共通ルール（言語・確認・質問フォーマット・ターン終端の出力ルール・スクリーンショット規約等）は `C:\Users\admin\.claude\CLAUDE.md` を正本とする。Claude Code は自動ロード、Codex 等他AIは `AGENTS.md` 経由で参照。

## 現在の実装状態（v0.1.1）

v0.1.1 初回正式リリースとして以下がすべて実装済み：

- `ai-cli-hub serve` で Hub が起動する
- `ai-cli-hub claude` / `codex` が Hub 未起動時に自動起動し接続する
- Hub UI に xterm.js でPTY出力がリアルタイム表示される
- xterm.js バッファスキャンで承認待ちを検出し action-bar を表示する
- Hub UI の選択結果を PTY へ返送する
- ターミナル直接入力で承認が解決された場合、action-bar を消す
- Claude Code の折りたたみ展開キャプチャ（ctrl+o）が動作する
- 画像添付（paste/D&D → ローカル保存 → PTY inject）が動作する
- `/api/spawn` でUI からセッションをspawnできる

## 用語・名称

| 項目 | 値 |
|------|------|
| プロダクト名 | `ai-cli-hub` |
| バイナリ名 | `ai-cli-hub`（Windows: `ai-cli-hub.exe`） |
| サブコマンド | `serve` / `wrap <provider>` / `shell-init` / `stop` / `status` |
| Hub URL | `http://127.0.0.1:47777/?token=<random>` |
| 設定ファイル | `~/.ai-cli-hub/config.yaml`（Win: `%USERPROFILE%\.ai-cli-hub\config.yaml`） |
| ログ | `~/.ai-cli-hub/logs/sessions/<provider>_<日時>_<folder>_s<id>.log/.jsonl`（PTY生ログ + イベント履歴JSONL） |
| 透過化環境変数 | `AI_CLI_HUB_AUTO=1` |
| Provider | `claude` / `codex`（`gemini` は対象外、上記スコープ更新参照） |

> プロジェクトディレクトリは `c:\dev\ai-cli-hub\`。md 内の参照は `ai-cli-hub` に統一。

## 技術スタック

| レイヤ | 採用 |
|------|------|
| 言語 | Go（クロスコンパイルで Win/Mac/Linux 単一バイナリ生成） |
| PTY | `creack/pty`（Unix）+ `aymanbagabas/go-pty`（Windows / ConPTY） |
| HTTP | `net/http` 標準 |
| WebSocket | `golang.org/x/net/websocket` |
| フロント | 静的HTML/CSS/Vanilla JS + vendored xterm.js（`go:embed` でバイナリ同梱） |
| 設定 | YAML (`gopkg.in/yaml.v3`) |
| ログ | `log/slog` 標準 |

## ディレクトリ構成（実際）

設計書 `docs/v0.1.x-ai-cli-hub-design.md` を参照。

```
ai-cli-hub/
├─ cmd/ai-cli-hub/main.go    # 単一バイナリのエントリポイント
├─ internal/
│  ├─ hub/        # HTTP+WS / セッション管理 / attach処理 / spawn
│  ├─ wrapper/    # PTYラッパー / PTY実装（OS別）/ attach inject
│  ├─ shell/      # shell-init 出力（bash/zsh）
│  ├─ proto/      # WSメッセージ定義
│  ├─ attach/     # 画像保存・inject生成
│  ├─ config/
│  └─ log/        # プレースホルダ（未実装）
├─ web/src/       # 静的HTML/CSS/JS + vendored xterm.js（go:embed対象）
└─ docs/local/    # 設計書・ロードマップ等（非公開）
```

## クロスプラットフォーム原則

- **OS固有コードは build tag で分離**（例: `pty_unix.go` / `pty_windows.go`）
- **パス操作は `filepath.Join` / `os.UserHomeDir`**（`/` ハードコード禁止）
- **改行・PTY 動作の差異**は `internal/wrapper/` で吸収し、上位層は OS 非依存に保つ
- **設定・ログのデフォルトディレクトリ**は全 OS 共通の `~/.ai-cli-hub/`（Windows でも `%USERPROFILE%\.ai-cli-hub\` で同じ意味）

## ローカルサーバの設計上の制約

- **バインドは `127.0.0.1` 固定**（外部公開しない）
- **デフォルトポート 47777**（衝突時は 47778, 47779… と自動探索）
- **ランダムトークンを起動時生成し URL に付与**（`?token=xxx`）
- **外部公開しない**（`127.0.0.1` 固定）。`ai-cli-hub` 自身はテレメトリを送信しないが、スラッシュコマンド一覧取得で GitHub へ HTTPS 通信する場合がある（README のセキュリティ節参照）
- **`.bashrc` 等への永続書き込みなし**（透過化は環境変数 + `eval "$(ai-cli-hub shell-init)"` のオプトイン方式のみ）

## 詳細ガイド（タスク種別ベース）

タスクに該当する md だけ Read すること。**該当しない md は読み込まない**（Context 節約）。

| タスク種別 | 読むファイル |
|---|---|
| 調査・読み取り・質問応答 | （`CLAUDE.md` root のみ。`CLAUDE/*` は読まない） |
| 実装・コーディング（Go / Vue） | `CLAUDE/coding.md` |
| ビルド・配布・クロスコンパイル | `CLAUDE/deployment.md` |
| context分割・docs命名・AI作業モデル・plan自走/停止条件 | `CLAUDE/development.md` |
| Git・コミット・出力ルール | `CLAUDE/operations.md` |
| Windows 開発環境固有設定 | `CLAUDE/windows_setup.md` |

## plan・docs 作業ルール（必須トリガー）

`plan_*.md` を**作成・実行**する作業、`docs/` 配下の `.md` を**新規作成・更新**する作業に着手する前に、必ず `CLAUDE/development.md` を Read すること（context分割・自走条件・停止条件・最終更新日時記載 等の正本）。

## 参照リンク

| 項目 | パス |
|------|------|
| 設計書 v0.1.x（現行・正本） | [docs/v0.1.x-ai-cli-hub-design.md](docs/v0.1.x-ai-cli-hub-design.md) |
| 設計書 v1（履歴） | [docs/local/archive/cli-popup-design-v1.md](docs/local/archive/cli-popup-design-v1.md) |
| Codex 用補足 | [AGENTS.md](AGENTS.md) / [AGENTS.local.md](AGENTS.local.md) |
| Gemini 用補足 | [GEMINI.md](GEMINI.md)（**ai-cli-hub の wrap 対象外**。本リポジトリで Gemini CLI を開発補助に使う場合の手引きとして残置） |
