# many-ai-cli 開発ガイド

> 最終更新: 2026-08-15(土) 23:40:00 — v0.7.0 リリースの学びから 2 節追加（本家との差分で slash-commands を削除しない・版数を手で直す場所は無い）

> 詳細は `CLAUDE/*.md` を参照。このファイルは常時ロード分のみ。

> **On translations / 翻訳について**
>
> This development guide is maintained in **Japanese only** — the Japanese text above and below is the authoritative version. There is no English edition of this file. (The user-facing README *is* available in English: [README.md](README.md), with [README.ja.md](README.ja.md) and [README.vi.md](README.vi.md).)
>
> [CLAUDE.vi.md](CLAUDE.vi.md), [docs/README.vi.md](docs/README.vi.md) and the `docs/manual_*.vi.md` files are a **point-in-time snapshot (2026-08-15)** kindly contributed by a community translator. They are **no longer kept in sync** with this guide, so please read them as background rather than as current rules, and check the Japanese original before acting on anything. This does **not** apply to the Vietnamese **UI** locale (`web/src/i18n/vi.json`), which is a shipped feature and is maintained normally.
>
> If you work in another language, we're sorry to ask — please translate as needed on your side (a machine translation of this file is usually enough). Translation contributions are genuinely welcome; we just can't promise to keep them in step with the Japanese original, so anything merged will be treated the same way: a dated snapshot.

## プロジェクト概要

**many-ai-cli** — 複数のAIコーディングCLI（Claude Code / Codex CLI）を並列で動かすときの **承認操作・進捗監視を 1 画面の Web ダッシュボードで一元管理** するツール。単一 Go バイナリ（Hub 常駐 + ラッパー機能）+ ブラウザ UI（xterm.js / TypeScript）。

> **Gemini CLI は wrap 対象外**（2026-05-06 決定 / 利用規約上の制約）。詳細は [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md) 「2. 公開スコープ」参照。
> 2026-08-13、Spotify Xirp の「浅いホスト方式」を踏まえて再検討したが **方針は維持**（保留記録: `docs/local/pending_gemini-shallow-host-option.md`）。他社が Gemini を扱っている事実・対応プロバイダ数の見劣りを根拠に着手を提案しないこと。

**現状**: v0.6.0 までリリース済み（v0.1.1 が初回正式リリース、v0.1.0 は試験扱い。未リリースだった `[0.5.2]` を統合したため v0.5.2 は欠番）。v0.1.2 でバージョン文字列を ldflags + `/api/info` 経由の single source of truth に再設計、v0.2.0 で WSL ランチャー・Files/Git/Chat/Split/Multi・Commit all・Ollama routing・サーバ側ユーザー設定を追加。v0.3.0 で Workbench（SQLite セッション履歴）・PWA/Web Push・統合ランチャーのクロスプラットフォーム化（SSH は全 OS、WSL は Windows 専用）・リモートサーバー/Docker デプロイ資産・npm 配布を追加し、プロジェクト名を any-ai-cli から many-ai-cli へリネーム。**v0.4.0 で Workbench 機能と Hub 内蔵チャットプロキシ（`internal/proxy/`・`chat_proxy`）を撤去済み**（Sonnet 5 以降のデフォルト 1M コンテキストが Hub 経由でも回復する副次効果あり）。設計書はソースコードを正本として更新済み。

**設計書（正本）**: [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md)

## 新規 provider を増やす提案をしない（2026-08-13 制定）

2026-08-13 に AI コーディング CLI 20 製品を調査し **全件見送りで確定**（資料: `docs/local/design_cli-agent-provider-candidates_2026-08-13.html`）。理由は候補側の優劣ではなく、**本ツールの規模（スター 6 / npm 週次 11）では候補の利用者数を根拠に採否を決める枠組みが成立しない**ため。

- **候補の人気・伸び・他社の対応状況を根拠に provider 追加を提案しない**（Gemini の節と同系統）
- 検討の起点は「作者がその CLI を素で使い、日常の並列運用に入っているか」。使う前に実装しない
- 追加コストは opencode 実績で 18 ファイル 180 行。本当のコストは鮮度チェックの無い `resources/approval-patterns/` と `resources/models/` の追従先が 1 本増えること

## デスクトップアプリ化・Microsoft Store 提出は提案しない（2026-08-14 制定）

2026-08-14 に Tauri での Windows アプリ化と Microsoft Store 提出を検討し、**1 行も書かないまま全件見送りで確定**（経緯と全根拠: `docs/local/archive/declined/plan_ms-store-tauri-windows-app.md`・H1 は `[廃止]`。子 6 本も同ディレクトリ）。

- **UI は既定ブラウザのタブで開く形を維持する。** 独立した窓（Tauri / Electron / PWA いずれも）を提案しない。作者は複数のブラウザタブを行き来しながら使うため、Hub がタブの 1 つであること自体が利点。窓は画面の枠を 1 つ占有し、他のウィンドウと場所を取り合うので**後退になる**
- `web/src/manifest.webmanifest` は `display: standalone` で PWA 導入可能な状態にあるが、上記の理由で**使われていない**。「PWA を入れれば窓になる」と勧めない
- **OS のネイティブ通知（トースト）を実装しない。** 作者が好まない。承認待ちの知らせは既存の 3 点（タイトル点滅 `web/src/app/session-list.ts:1060` / favicon の件数バッジ 同 1039-1080 / 通知音 `user_prefs.notify_sound`）で成立している
- **配布チャネルを増やす提案をしない。** 既に winget / npm / Homebrew / Docker の 4 つある。実測（2026-08-14）で star 6・リポジトリのページに 1 日ユニーク 2〜3 人・npm 週次 11。記事も 2 か月で 132 本、製品紹介は v0.3.2 / v0.4.0 / v0.5.0 の 3 世代ぶん出している。**チャネルも記事も打ち手は尽きており、天井は需要側（複数の AI CLI を同時に走らせ承認が詰まっている人という狭い積集合）で決まっている**
- 例外は 1 つだけ。**作者自身が窓の形を使いたくなったとき**。人気・他社の対応状況・利用者増の見込みを根拠にしない（Gemini・新規 provider の節と同系統）

残った実課題（起動と停止がデスクトップのアイコン 2 個に分かれている）だけを、窓を作らないトレイ常駐として `docs/local/plan_tray-resident-hub-lifecycle.md` へ切り出した。

**見送った方針は `docs/local/reference_declined-directions.md` に台帳としてまとめてある。** Gemini の wrap・新規 provider・セッションカンバン・本節の 4 件が入っており、各項目に「再検討してよい条件」と「再検討の根拠にしてはいけないもの」を書いてある。**同種の提案を出す前に必ずここを読む。** 見送りは永久否定ではないので、条件が満たされているなら再検討してよい。

## 現在の実装状態

v0.6.0 までに以下がすべて実装済み（v0.4.0 で Workbench / chat_proxy を撤去し opencode / Grok Build CLI provider・Ollama `base_url` 設定を追加、v0.5.0 で `setup` / `doctor` サブコマンド・autoapproval・ターン単位 diff を追加）。v0.6.0 では transcript ベースのチャット本文・Workflow 進捗の Hub 権威化・OpenCode 全許可起動・古いビルド警告を追加し、リリース前監査 A-01/A-02/A-03/A-05 に対応した：

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
| Provider | `claude` / `codex` / `copilot` / `cursor-agent`（v0.4.0 で `opencode` / `grok` 追加。`gemini` は対象外、上記スコープ更新参照） |

> md 内の参照は `many-ai-cli` に統一（旧名 `any-ai-cli` は履歴記述を除き使わない）。ローカルのプロジェクト配置パスは `CLAUDE.local.md` に記載する。
>
> **grep 注意**: 旧名 `any-ai-cli` は新名 `many-ai-cli` の部分文字列（`m` + `any-ai-cli`）。旧名マーカー（`<!-- any-ai-cli:approval-rules -->` 等）の残骸を新名パターンで grep すると **0 件に見える**。旧名側のパターンで grep すれば新旧どちらにも当たる。

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

### 本家との差分を根拠に `slash-commands/*.md` から削除しない（2026-08-15 制定）

鮮度チェックは本家の一覧と突き合わせて「追加候補」と「削除候補」を出すが、**削除候補は原則そのまま採用しない**。

- **公式 docs は built-in コマンドの完全一覧ではない。** 2026-08-15 の照合で、codex の docs に `/model` `/status` `/usage` `/resume` `/logout` `/mcp` が載っていなかった。docs との差分をそのまま削除すると、実在する基本コマンドがピッカーから消える
- **`/orchestrate` は many-ai-cli 自身が提供する Hub の機能**で、本家 CLI には無い（`claude` / `codex` / `copilot` / `cursor-agent` の 4 本に同一行）。**本家と比べる限り毎回「削除候補」として出続けるのが正常**。`freshness-report.ps1` の `$script:HouseCommands` で除外してある
- **差分は追加候補から先に数える。** 「本家にあって md に無い」が 0 件なら、逆向きが何件でも利用者への実害は無い（ピッカーから欠けているものが無い）。件数の多い削除候補側から見ると判断を誤る
- 削除してよいのは **実機の `/help` で「無い」ことを確かめたとき**だけ

## 版数を手で直す場所は無い（2026-08-15 制定）

**タグが単一ソース。** リポジトリ内に版数を持つファイルがいくつかあるが、**どれも CI がタグから上書きするので、古いままなのが正常**。

| ファイル | 誰が焼くか |
|---|---|
| `npm/*/package.json` の `version` | `release.yml` が `scripts/sync-npm-version.mjs "${RELEASE_TAG}"` |
| `winres/*.json` の `file_version` / `product_version` | `.goreleaser.yaml` の before hook が `go-winres make --file-version={{ .Version }}` |
| バイナリの `many-ai-cli version` | goreleaser の ldflags `-X main.version={{ .Version }}` |

2026-08-15 の v0.7.0 リリース時点で、`npm/many-ai-cli/package.json` は `0.3.1`、`winres/winres.json` は `0.3.1.0` のままだが、**配布物はすべて 0.7.0 になっている**。手で直すと二重管理になり、タグとズレたときに気づけなくなる。

> `docs/manual_release.md` の「リリース前に手動確認・更新するもの」に winres が挙がっていたが、**版数については誤り**だったので 2026-08-15 に訂正済み（アイコン・製品名・manifest identity は手動確認の対象のまま）。

## 調査用の観測コードを入れるときのルール（必須）

バグ調査のために一時的なログ・ダンプ・debug エンドポイントを仕込むときは、**同じコミットで `instrumentation.json` へ登録する**。登録の無い観測コードは `scripts/check-instrumentation.mjs` がブロックする（Validate の `instrumentation` ジョブと release の前提チェックで走る）。

- `gate` は必ず埋める。**`always-on` は原則禁止**。とくに入力由来のバイトを保存するものは既存の opt-in（`log.session_enabled`）に従わせる
- `due` を必ず書く。過ぎたらブロックされる。延ばすなら `due` を更新して `reason` に「なぜまだ要るか」を書き足す（黙って延ばさない）
- 撤去したら `status: removed` にする。実体が残っていれば検査で落ちる

**「原因が確定したら消す」とコメントに書くだけでは消えない。** v0.5.1 の `/api/debug/batch-log`、v0.5.x の入力トレース（監査 A-01）、v0.6.0 の `/api/debug/ui-log` と `ui_input_trace` と、3 回続けて出荷直前まで残った。とくに `ui_input_trace` は専用ファイルを持たず共有ファイル 3 つに分散していたため、同種の撤去作業から取り残された。

## 利用者のファイルへ書く機能は「次回起動時の回収」まで設計する（2026-08-14 制定）

セッション中だけ書き換えて終了時に戻す作りは、後始末が `defer` や graceful shutdown に載っている限り kill で飛ぶ。しかも**次の実行が残骸を「オリジナル」として採用して書き戻すため、一度置き去りになると自己修復せず恒久化する**。`opencode.json`（`internal/wrapper/opencode_config.go`）と AGENTS.md の承認ルールブロック（`internal/hub/approval_rules_state.go`）の 2 経路で、公開リポジトリへ commit されるところまで進んだ。

- **後始末の到達率を上げる方向で考えない**（kill は防げない）。次回起動時に置き去りを回収する経路を必ず持たせる
- 回収するには「自分が何を書いたか」を残す必要がある。**現物がそれと違えば他者が触った後なので戻さない**
- 生成物のファイル名は `.gitignore` にも入れる。ただし **gitignore は tracked file には効かない**ので、既に commit されたものは `git rm` でしか消えない
- **回収より前に commit されてしまったものは、回収経路に乗らない。** そちらは `many-ai-cli doctor` の置き去り検査（`internal/doctor/residue.go`）が検出する。設計原則と検出手段は対で、片方だけ直しても利用者には届かない

## 承認の同一性は 1 本しか持たない（2026-08-15 制定）

**「この承認はもう回答済みか」を決める state は `candidateKey + sourceEpoch` の 1 本だけ**（`web/src/app/approval-answered.ts`）。

以前はこの役目が 3 つに分かれていて、**それぞれ「同じ質問とは何か」の定義が違った**。

| かつての state | 同一性の定義 | 失効 |
|---|---|---|
| `approvalConsumedSig` | 選択肢の署名 | 5〜10 秒のタイマー |
| `answeredMarkerSigs` | マーカーブロック全文のハッシュ | 失効しない（セッション恒久） |
| `approvalQuestionKey` | 質問文のハッシュ | 手動 dismiss の間 |

**3 者の食い違いそのものが症状だった。** タイマー方式は TUI の再描画が続いている最中に失効して回答済みを再表示し、ブロック全文方式は逆に「エージェントが本当に出し直した同じ質問」まで永久に埋めた。v0.7 で 3 本とも撤去して 1 本へ寄せている。

- **承認の誤表示を踏んでも、抑止をもう 1 本足さない。** まず既存の 1 本で説明できない症状かを確かめる。説明できるなら直すのは `candidateKey` の作り方か `sourceEpoch` の進み方であって、新しい state ではない
- `candidateKey` は provider・承認種別・正規化した質問・選択肢番号・送信文字列から作る。**ラベル・空白・罫線・折返しを含めない**（含めると再描画のたびに別候補になる）
- `sourceEpoch` は live prompt の境界でのみ進む。**replay と reflow では進めない**（進めると復元しただけで新しい承認に見える）
- 世代が進めば同じ質問文でも新しい候補として表示するのが**仕様**。「同じ質問を二度と出さない」方向の永久抑止に戻さない

## AI 作業共通ルール

ビルド・コミット禁止、secrets-scan 責務、plan/bugfix/pending md の作成ルール等の AI 作業共通ルールは、各利用者のグローバル AI 設定に従う（作者環境の例: `~/.claude/CLAUDE.md` および `~/.claude/guides/`）。AI の個人グローバルルール（言語・確認・質問フォーマット等）も各利用者のグローバル設定に置き、本ファイルはプロジェクト固有ルールだけを扱う。

プロジェクト固有の追加ルール:

- ビルドだけでなく **実行・Hub 起動・ブラウザリロードも全てユーザーが行う**（`go run` / `many-ai-cli serve` / `many-ai-cli stop` / Hub プロセスの起動・終了・再起動・ブラウザリロード等も対象）。
- **ビルドコマンドは `make build` が基本**。`bun run build` 単体は使わない（ユーザーへの案内でも `make build` を示す）。`make build` が web ビルド（bun install + bun run build）〜 Windows/Linux バイナリ生成 〜 WSL 配備まで一括で行う。ユーザーの「ビルドして」という指示は **`make build` の実行指示** を意味する。
- `many-ai-cli` 自身のセッションログ保持機能（`~/.many-ai-cli/logs/sessions/`）は非推奨の機能。承認検出まわりのバグ調査でも、このログではなく実際に動いている AI エージェント本体（Claude Code / Codex CLI 等）が書き出す生ログを見ること。ラッパー層のログは情報密度が低く、原因特定に向かない。

## 監査・リリース証拠の境界

- Agent Chat の cursor 変更は parser 単体で完了扱いにせず、caller の offset 採用と次 poll の再開まで追跡・検証する。
- `go:embed` 対象の Gitignored runtime は、clean release job で取得・検証・artifact 化し、build 前の入力存在確認を必須にする。ローカルにファイルがあることだけでは release の証拠にしない。
- release workflow の詳細な手順・SHA・secret scope は [`.github/workflows/release.yml`](.github/workflows/release.yml) と [監査対応 plan](docs/local/archive/plan_security-vulnerability-quality-remediation-2026-08-13.md) を正本とする。CLAUDE.md に手順を複製しない。
- 静的確認、ローカルテスト、CI 実行、release artifact、実機確認は別の証拠として報告する。未実行の CI・artifact・実機確認を完了扱いにしない。

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
