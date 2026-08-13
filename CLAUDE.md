# many-ai-cli 開発ガイド

> 最終更新: 2026-08-11(火) — 実行時配信リソース節を追加（リリース不要で更新できるものの正典化）

> 詳細は `CLAUDE/*.md` を参照。このファイルは常時ロード分のみ。  
> **Vietnamese translation (team docs):** [CLAUDE.vi.md](CLAUDE.vi.md) · [README.vi.md](README.vi.md) · [docs/README.vi.md](docs/README.vi.md)

## プロジェクト概要

**many-ai-cli** — 複数のAIコーディングCLI（Claude Code / Codex CLI）を並列で動かすときの **承認操作・進捗監視を 1 画面の Web ダッシュボードで一元管理** するツール。単一 Go バイナリ（Hub 常駐 + ラッパー機能）+ ブラウザ UI（xterm.js / TypeScript）。

> **Gemini CLI は wrap 対象外**（2026-05-06 決定 / 利用規約上の制約）。詳細は [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md) 「2. 公開スコープ」参照。
> 2026-08-13、Spotify Xirp の「浅いホスト方式」を踏まえて再検討したが **方針は維持**（保留記録: `docs/local/pending_gemini-shallow-host-option.md`）。他社が Gemini を扱っている事実・対応プロバイダ数の見劣りを根拠に着手を提案しないこと。

**現状**: v0.5.1 までリリース済み（v0.1.1 が初回正式リリース、v0.1.0 は試験扱い）。**v0.6.0 が次のリリース対象**（CHANGELOG の `[0.6.0]` 節。未リリースだった `[0.5.2]` を統合したため v0.5.2 は欠番）。v0.1.2 でバージョン文字列を ldflags + `/api/info` 経由の single source of truth に再設計、v0.2.0 で WSL ランチャー・Files/Git/Chat/Split/Multi・Commit all・Ollama routing・サーバ側ユーザー設定を追加。v0.3.0 で Workbench（SQLite セッション履歴）・PWA/Web Push・統合ランチャーのクロスプラットフォーム化（SSH は全 OS、WSL は Windows 専用）・リモートサーバー/Docker デプロイ資産・npm 配布を追加し、プロジェクト名を any-ai-cli から many-ai-cli へリネーム。**v0.4.0（Unreleased）で Workbench 機能と Hub 内蔵チャットプロキシ（`internal/proxy/`・`chat_proxy`）を撤去済み**（Sonnet 5 以降のデフォルト 1M コンテキストが Hub 経由でも回復する副次効果あり）。設計書はソースコードを正本として更新済み。

**設計書（正本）**: [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md)

## 新規 provider を増やす提案をしない（2026-08-13 制定）

2026-08-13 に AI コーディング CLI 20 製品を調査し **全件見送りで確定**（資料: `docs/local/design_cli-agent-provider-candidates_2026-08-13.html`）。理由は候補側の優劣ではなく、**本ツールの規模（スター 6 / npm 週次 11）では候補の利用者数を根拠に採否を決める枠組みが成立しない**ため。

- **候補の人気・伸び・他社の対応状況を根拠に provider 追加を提案しない**（Gemini の節と同系統）
- 検討の起点は「作者がその CLI を素で使い、日常の並列運用に入っているか」。使う前に実装しない
- 追加コストは opencode 実績で 18 ファイル 180 行。本当のコストは鮮度チェックの無い `resources/approval-patterns/` と `resources/models/` の追従先が 1 本増えること

## 現在の実装状態

v0.5.1 までに以下がすべて実装済み（v0.4.0 で Workbench / chat_proxy を撤去し opencode / Grok Build CLI provider・Ollama `base_url` 設定を追加、v0.5.0 で `setup` / `doctor` サブコマンド・autoapproval・ターン単位 diff を追加）。v0.6.0 では transcript ベースのチャット本文・Workflow 進捗の Hub 権威化・OpenCode 全許可起動・古いビルド警告を追加し、リリース前監査 A-01/A-02/A-03/A-05 に対応した：

- `many-ai-cli serve` で Hub が起動する
- `many-ai-cli claude` / `codex` / `copilot` / `cursor-agent` が Hub 未起動時に自動起動し接続する
- Hub UI に xterm.js でPTY出力がリアルタイム表示される
- xterm.js バッファスキャンで承認待ちを検出し action-bar を表示する
- 承認マーカー指示を Claude / Codex / Copilot / Cursor Agent の instruction file へ冪等注入し、active session 参照が0になったファイルから削除する
- Hub UI の選択結果を PTY へ返送する
- ターミナル直接入力で承認が解決された場合、action-bar を消す
- Claude Code の折りたたみ展開キャプチャ（ctrl+o）が動作する
- 画像添付（paste/D&D → ローカル保存 → PTY inject）が動作する
- `/api/spawn` でUI からセッションをspawnできる

## 用語・名称

| 項目 | 値 |
|------|------|
| プロダクト名 | `many-ai-cli` |
| バイナリ名 | `many-ai-cli`（Windows: `many-ai-cli.exe`） |
| サブコマンド | `serve` / `wrap <provider>` / `shell-init` / `stop` / `status` / `uninstall` / `version` |
| Hub URL | `http://127.0.0.1:47777/?token=<random>` |
| 設定ファイル | `~/.many-ai-cli/config.yaml`（Win: `%USERPROFILE%\.many-ai-cli\config.yaml`） |
| ログ | `~/.many-ai-cli/logs/sessions/<provider>_<日時>_<folder>_s<id>.log/.jsonl/.txt`（PTY生ログ + イベント履歴JSONL + クリーンテキスト） |
| 透過化環境変数 | `MANY_AI_CLI_AUTO=1` |
| Provider | `claude` / `codex` / `copilot` / `cursor-agent`（v0.4.0 Unreleased で `opencode` / `grok` 追加。`gemini` は対象外、上記スコープ更新参照） |

> md 内の参照は `many-ai-cli` に統一（旧名 `any-ai-cli` は履歴記述を除き使わない）。ローカルのプロジェクト配置パスは `CLAUDE.local.md` に記載する。

## 技術スタック

| レイヤ | 採用 |
|------|------|
| 言語 | Go（クロスコンパイルで Win/Mac/Linux 単一バイナリ生成） |
| PTY | `creack/pty`（Unix）+ `aymanbagabas/go-pty`（Windows / ConPTY） |
| HTTP | `net/http` 標準 |
| WebSocket | `golang.org/x/net/websocket` |
| フロント | 静的HTML/CSS + TypeScript（ESM）+ esbuild ファイル単位トランスパイル + vendored xterm.js（`web/dist/` を `go:embed` でバイナリ同梱） |
| 設定 | YAML (`gopkg.in/yaml.v3`) |
| ログ | `log/slog` 標準 |

## ディレクトリ構成（実際）

設計書 `docs/v0.3.x-many-ai-cli-design.md` を参照。

```
many-ai-cli/
├─ cmd/many-ai-cli/main.go    # 単一バイナリのエントリポイント
├─ internal/
│  ├─ hub/        # HTTP+WS / セッション管理 / attach処理 / spawn
│  ├─ wrapper/    # PTYラッパー / PTY実装（OS別）/ attach inject
│  ├─ shell/      # shell-init 出力（bash/zsh）
│  ├─ proto/      # WSメッセージ定義
│  ├─ attach/     # 画像保存・inject生成
│  ├─ config/
│  ├─ log/        # slog ファイルロガー（lumberjack ローテーション）
│  └─ ...         # ほか launcher / notify / orchestrate / sessionlog / sessionstore / uninstall / usagerelay / whisperruntime / wslutil
├─ web/src/       # 静的HTML/CSS/TypeScript + vendored xterm.js（フロントソース）
├─ web/dist/      # bun run build の生成物（go:embed対象 / gitignore）
└─ docs/local/    # 設計書・ロードマップ等（非公開）
```

## クロスプラットフォーム原則

- **OS固有コードは build tag で分離**（例: `pty_unix.go` / `pty_windows.go`）
- **パス操作は `filepath.Join` / `os.UserHomeDir`**（`/` ハードコード禁止）
- **改行・PTY 動作の差異**は `internal/wrapper/` で吸収し、上位層は OS 非依存に保つ
- **設定・ログのデフォルトディレクトリ**は全 OS 共通の `~/.many-ai-cli/`（Windows でも `%USERPROFILE%\.many-ai-cli\` で同じ意味）

## ローカルサーバの設計上の制約

- **バインドは `127.0.0.1` 固定**（外部公開しない）
- **デフォルトポート 47777**（衝突時は 47778, 47779… と自動探索）
- **ランダムトークンを起動時生成し URL に付与**（`?token=xxx`）
- **外部公開しない**（`127.0.0.1` 固定）。`many-ai-cli` 自身はテレメトリを送信しないが、スラッシュコマンド一覧取得で GitHub へ HTTPS 通信する場合がある（README のセキュリティ節参照）
- **`.bashrc` 等への永続書き込みなし**（透過化は環境変数 + `eval "$(many-ai-cli shell-init)"` のオプトイン方式のみ）

## 実行時配信リソース（リリース不要で更新できるもの）

`resources/` 配下の 4 ディレクトリは **バイナリに焼き込まれず、`main` の raw URL から実行時に fetch される**（URL 定数は `internal/config/config.go` の `Default*Source`）。

| ディレクトリ | 中身 |
|---|---|
| `resources/approval-patterns/` | 承認 trigger phrase |
| `resources/models/` | spawn パネルのモデル候補（`defaults.json`） |
| `resources/slash-commands/` | スラッシュコマンドピッカー |
| `resources/usage-links/` | 利用状況リンク |

**帰結（毎回調べ直さない）**:

- 内容を直すのに **リビルド・タグ・リリースは一切不要**。`main` へ push した時点で全ユーザーへ反映される
- 逆に **`develop` に置いても誰にも届かない**。配信元は `main` だけ
- 全ユーザーへ即時 live 配信されるため、**誤りもそのまま公開される**。自動反映せず採否は人間が決める
- 形式制約を破るとパーサが壊れる（`internal/hub/slash_cmd_fetch.go` ほか）

**鮮度チェックの仕組みがあるのは `slash-commands` だけ**（`.claude/skills/slash-commands-update` の report / apply / preflight。release の前提チェックから preflight が呼ばれる）。`models` / `usage-links` / `approval-patterns` には無く、**陳腐化しても誰も気づかない**（2026-08-11、モデル一覧が Claude 5 世代を丸ごと欠いたまま放置されていたのを発見）。

詳細は設計書 [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md) の承認パターン / モデル / スラッシュコマンド各節と、[docs/manual_slash_commands_update.md](docs/manual_slash_commands_update.md)。

## 調査用の観測コードを入れるときのルール（必須）

バグ調査のために一時的なログ・ダンプ・debug エンドポイントを仕込むときは、**同じコミットで `instrumentation.json` へ登録する**。登録の無い観測コードは `scripts/check-instrumentation.mjs` がブロックする（Validate の `instrumentation` ジョブと release の前提チェックで走る）。

- `gate` は必ず埋める。**`always-on` は原則禁止**。とくに入力由来のバイトを保存するものは既存の opt-in（`log.session_enabled`）に従わせる
- `due` を必ず書く。過ぎたらブロックされる。延ばすなら `due` を更新して `reason` に「なぜまだ要るか」を書き足す（黙って延ばさない）
- 撤去したら `status: removed` にする。実体が残っていれば検査で落ちる

**「原因が確定したら消す」とコメントに書くだけでは消えない。** v0.5.1 の `/api/debug/batch-log`、v0.5.x の入力トレース（監査 A-01）、v0.6.0 の `/api/debug/ui-log` と `ui_input_trace` と、3 回続けて出荷直前まで残った。とくに `ui_input_trace` は専用ファイルを持たず共有ファイル 3 つに分散していたため、同種の撤去作業から取り残された。

## AI 作業共通ルール

ビルド・コミット禁止、secrets-scan 責務、plan/bugfix/pending md の作成ルール等の AI 作業共通ルールは、各利用者のグローバル AI 設定に従う（作者環境の例: `~/.claude/CLAUDE.md` および `~/.claude/guides/`）。AI の個人グローバルルール（言語・確認・質問フォーマット等）も各利用者のグローバル設定に置き、本ファイルはプロジェクト固有ルールだけを扱う。

プロジェクト固有の追加ルール:

- ビルドだけでなく **実行・Hub 起動・ブラウザリロードも全てユーザーが行う**（`go run` / `many-ai-cli serve` / `many-ai-cli stop` / Hub プロセスの起動・終了・再起動・ブラウザリロード等も対象）。
- **ビルドコマンドは `make build` が基本**。`bun run build` 単体は使わない（ユーザーへの案内でも `make build` を示す）。`make build` が web ビルド（bun install + bun run build）〜 Windows/Linux バイナリ生成 〜 WSL 配備まで一括で行う。ユーザーの「ビルドして」という指示は **`make build` の実行指示** を意味する。
- `many-ai-cli` 自身のセッションログ保持機能（`~/.many-ai-cli/logs/sessions/`）は非推奨の機能。承認検出まわりのバグ調査でも、このログではなく実際に動いている AI エージェント本体（Claude Code / Codex CLI 等）が書き出す生ログを見ること。ラッパー層のログは情報密度が低く、原因特定に向かない。

## 詳細ガイド（タスク種別ベース）

タスクに該当する md だけ Read すること。**該当しない md は読み込まない**（Context 節約）。

| タスク種別 | 読むファイル |
|---|---|
| 調査・読み取り・質問応答 | （`CLAUDE.md` root のみ。`CLAUDE/*` は読まない） |
| 実装・コーディング（Go / TypeScript） | `CLAUDE/coding.md` |
| ビルド・配布・クロスコンパイル | `CLAUDE/deployment.md` |
| context分割・docs命名・AI作業モデル・plan自走/停止条件 | `CLAUDE/development.md` |
| Git・コミット・出力ルール | `CLAUDE/operations.md` |
| Windows 開発環境固有設定 | `CLAUDE/windows_setup.md` |

## plan・docs 作業ルール（必須トリガー）

`plan_*.md` を**作成・実行**する作業、`docs/` 配下の `.md` を**新規作成・更新**する作業に着手する前に、必ず `CLAUDE/development.md` を Read すること（context分割・自走条件・停止条件・最終更新日時記載 等の正本）。

## 参照リンク

| 項目 | パス |
|------|------|
| 設計書 v0.3.0（現行・正本） | [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md) |
| 設計書 v0.2.0（履歴） | [docs/v0.2.x-any-ai-cli-design.md](docs/v0.2.x-any-ai-cli-design.md) |
| 設計書 v1（履歴） | [docs/local/archive/v0.1.3/cli-popup-design-v1.md](docs/local/archive/v0.1.3/cli-popup-design-v1.md) |
| Codex 用補足 | [AGENTS.md](AGENTS.md)（ローカル補足があれば `AGENTS.local.md`） |
| Gemini 用補足 | [GEMINI.md](GEMINI.md)（**many-ai-cli の wrap 対象外**。本リポジトリで Gemini CLI を開発補助に使う場合の手引きとして残置） |
