# ai-cli-hub

複数の AI コーディング CLI（Claude Code / Codex CLI）を並列で動かすときの、**承認操作・進捗監視を 1 画面の Web ダッシュボードで一元管理する**ツール。

> **ローカル優先設計** — Hub UI は `127.0.0.1` のみにバインド。`ai-cli-hub` 自身はテレメトリを送信しません（slash command 一覧の GitHub 取得・wrap 対象 CLI のベンダー API 通信については「[セキュリティ](#セキュリティ)」セクション参照）。
> セッションログにはユーザー入力・AI出力が保存されます。機密情報として扱ってください。

---

## 概要

複数のターミナルで AI コーディング CLI を並列実行していると、承認待ちがどこで発生しているか把握しづらくなります。`ai-cli-hub` は各 CLI を PTY でラップし、ブラウザの Hub UI で一元監視・承認操作を行えるようにします。CLI 本体の機能はそのままで、承認 GUI だけを追加する設計です。

```
Terminal pane #1              Terminal pane #2
┌────────────────────┐        ┌────────────────────┐
│ ai-cli-hub claude  │        │ ai-cli-hub codex   │
│  (PTY 素通し)       │        │  (PTY 素通し)       │
└────────┬───────────┘        └────────┬───────────┘
         │ WebSocket                   │ WebSocket
         └─────────────┬───────────────┘
                       ▼
            ┌──────────────────┐
            │ ai-cli-hub serve │  http://127.0.0.1:47777
            │  (Hub 常駐)       │
            └────────┬─────────┘
                     │
                     ▼
            ┌──────────────────┐
            │  ブラウザ Hub UI  │
            │  承認ポップオーバー│
            │  セッション一覧  │
            └──────────────────┘
```

---

## 動作要件

| 項目 | 要件 |
|---|---|
| Go | 1.25 以上（ビルド時） |
| OS | Windows 10/11、macOS、Linux |
| ブラウザ | Chrome / Edge / Firefox / Safari |
| AI CLI | Claude Code、Codex CLI（別途インストール済みであること） |

### v0.1.3 の検証状況

- 実機で動作検証済み: Windows
- 実機で未検証: Linux / macOS

Linux / macOS でも動作する想定ですが、v0.1.3 時点では実機での十分な検証は未実施です。
ご利用の際はご了承ください。問題があれば Issue で報告してください。

---

## インストール

### ビルド済みバイナリを使う場合

リリースページからバイナリをダウンロードして、`PATH` の通った場所に置きます。

#### リリース成果物の検証（チェックサム + 署名）

`v0.1.2` 以降の正式リリースには以下が含まれます。

- `SHA256SUMS.txt`
- `SHA256SUMS.txt.sig`
- `SHA256SUMS.txt.pem`

1. `SHA256SUMS.txt` の署名を検証:

```bash
cosign verify-blob \
  --certificate SHA256SUMS.txt.pem \
  --signature SHA256SUMS.txt.sig \
  --certificate-identity-regexp "https://github.com/ishizakahiroshi/ai-cli-hub/.github/workflows/release.yml@refs/tags/v.*" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  SHA256SUMS.txt
```

2. ダウンロードしたバイナリをチェックサムで検証:

```bash
sha256sum -c SHA256SUMS.txt
```

### ソースからビルドする場合

```powershell
# リポジトリをクローン
git clone https://github.com/ishizakahiroshi/ai-cli-hub
cd ai-cli-hub

# 現在の OS 向けにビルド
go build -o ai-cli-hub.exe ./cmd/ai-cli-hub   # Windows
go build -o ai-cli-hub ./cmd/ai-cli-hub        # macOS / Linux
```

#### クロスコンパイル

```bash
GOOS=windows GOARCH=amd64 go build -o dist/win/ai-cli-hub.exe   ./cmd/ai-cli-hub
GOOS=darwin  GOARCH=amd64 go build -o dist/mac/ai-cli-hub        ./cmd/ai-cli-hub
GOOS=darwin  GOARCH=arm64 go build -o dist/mac-arm/ai-cli-hub    ./cmd/ai-cli-hub
GOOS=linux   GOARCH=amd64 go build -o dist/linux/ai-cli-hub      ./cmd/ai-cli-hub
```

---

## クイックスタート（推奨）

通常はこれだけです。CLI を直接叩く必要はありません。

1. リリースページから `ai-cli-hub.exe`（macOS / Linux はバイナリ）をダウンロードする
2. **`ai-cli-hub.exe` をダブルクリックして起動する**（または引数なしで `ai-cli-hub` を実行）
   - Hub が起動し、ブラウザが自動で開きます（`http://127.0.0.1:47777/?token=<token>`）
   - すでに Hub が起動済みの場合は、ブラウザを開くだけで終了します
3. ブラウザの Hub UI 左下の **「+ 新しいセッション」** をクリックし、Claude Code / Codex CLI のセッションを起動する
4. セッションが UI に表示されれば運用開始。承認待ちが発生すると入力欄の下にアクションバーが出るので、クリックまたはキーボードで操作する

ターミナルを別途開かなくても、セッションの起動・操作・承認はすべて Hub UI から行えます。

> **⚠ コンソールウィンドウについて**
> `.exe` を起動すると黒いコンソールウィンドウがブラウザと一緒に開きますが、**これが Hub サーバの実体プロセスです**。ウィンドウを `×` で閉じると Hub が終了します（邪魔な場合は閉じずに **最小化** してください）。
> なお Hub が落ちた場合でも、走行中の AI セッションはデフォルトで **60 分間 Hub の復帰を待つ**ようになっています（`config.yaml` で 0–86400 秒 = 最大 24 時間まで変更可、長時間タスクを走らせる場合は伸ばせます）。間に合わなければ自動で終了するので、Web UI 側のバグや再起動で AI 作業が即座に道連れにはなりません。詳細は「[Shutdown, zombie protection & Hub crash resilience](#shutdown-zombie-protection--hub-crash-resilience)」相当のセクション「[ゾンビセッション対策と Hub クラッシュ耐性](#ゾンビセッション対策と-hub-クラッシュ耐性)」参照。
> Hub を意図的に停止するときは、Hub UI 右上の `⏻` ボタン、または別ターミナルで `ai-cli-hub stop` を使ってください。

---

## ターミナル / シェルから直接起動したい場合

CLI 派の人や、シェル統合・自動化を組みたい人向けの代替手段です。Hub UI の「+ 新しいセッション」と機能的には同等で、好みで使い分けてください。

### 方法 A: provider 直指定

```powershell
ai-cli-hub claude      # Hub 未起動なら自動でバックグラウンド起動してから Claude を起動
ai-cli-hub codex       # 同上
```

`ai-cli-hub serve` を事前に実行しておく必要はありません。

### 方法 B: wrap サブコマンド（デバッグ用）

```powershell
ai-cli-hub wrap claude
ai-cli-hub wrap codex
```

方法 A と機能は同じですが、内部実装の確認やデバッグ用途に使います。

### 方法 C: 透過モード（`AI_CLI_HUB_AUTO`）

シェルで一度だけ初期化しておくと、普段の `claude` / `codex` コマンドがそのままラッパー経由で起動されるようになります。

> `ai-cli-hub shell-init` は **POSIX シェル（bash / zsh）専用** の関数定義を出力します。PowerShell 用のスニペットは出力しません（後述の代替手順を参照）。

```bash
# シェル起動時に 1 回だけ実行（bash / zsh）
eval "$(ai-cli-hub shell-init)"

# 監視したいセッションだけ環境変数を ON にする
export AI_CLI_HUB_AUTO=1
claude    # ← 自動でラッパー経由・Hub 未起動なら自動起動
codex     # ← 同上
```

`AI_CLI_HUB_AUTO=1` が設定されていないシェルでは、`claude` / `codex` はそのまま元のコマンドとして動作します。グローバルな `.bashrc` 等は改変しません。

#### OS 別の自動化設定例

**PowerShell（Windows）**

`$PROFILE` に以下を追記してください（`shell-init` は PowerShell 非対応のため、関数を直接定義します）。

```powershell
if ($env:AI_CLI_HUB_AUTO -eq '1') {
    function claude { ai-cli-hub claude @args }
    function codex  { ai-cli-hub codex  @args }
}
```

Windows Terminal のプロファイル側で `AI_CLI_HUB_AUTO=1` をセットしておけば、そのタブだけ透過モードになります。

```jsonc
{
  "name": "AI Watch",
  "commandline": "pwsh.exe -NoExit",
  "environment": { "AI_CLI_HUB_AUTO": "1" }
}
```

**iTerm2（macOS）**

- Profiles → Environment → Variables: `AI_CLI_HUB_AUTO=1`
- Profiles → General → Send text at start: `eval "$(ai-cli-hub shell-init)"`

**tmux（全 OS 共通）**

```bash
# ~/.tmux.conf
set-option -g default-command "AI_CLI_HUB_AUTO=1 bash -c 'eval \"$(ai-cli-hub shell-init)\"; exec bash'"
```

---

## サブコマンド一覧

| コマンド | 説明 |
|---|---|
| `serve [--open] [--port N]` | Hub を起動。`--open` でブラウザを自動で開く |
| `claude [args...]` | Claude Code を Hub 経由で起動 |
| `codex [args...]` | Codex CLI を Hub 経由で起動 |
| `wrap <provider> [args...]` | 任意 provider をラップ（デバッグ用） |
| `shell-init` | 透過モード用のシェル関数スニペットを出力 |
| `status` | Hub の起動状態を表示 |
| `stop` | Hub を停止 |

---

## Hub UI

ブラウザで `http://127.0.0.1:47777/?token=<token>` を開きます。

```
┌─ AI-CLI-HUB  [1][0][6] │ ● Claude:2  ● Codex:5            [⏻] [設定] ─┐
├──────────────────────────┬──────────────────────────────────────────────┤
│ [+ 新しいセッション]     │ ● Codex  cwd: C:\dev\ai-cli-hub   [↑最上部へ]│
│ 📁 ai-cli-hub  [1][0][6] │ ターミナル出力 — Windows PowerShell          │
│ ─────────────────────── │                                              │
│ ★ #7 ● Codex  実行中  × │   (xterm.js のターミナル出力)               │
│    最終応答: 00:11:57   │                                              │
│    docs/local/plan_…    │                                              │
│                         │                                              │
│ ☆ #6 ● Codex スタンバイ ×│                                              │
│    最終応答: 00:05:48   │                                              │
│    docs/local/plan_…    │   ┌─ 承認（waiting 時のみ表示）──────┐     │
│                         │   │ Command: npm install axios          │     │
│ ☆ #4 ● Claude スタンバイ │   │ Risk: MEDIUM                        │     │
│    最終応答: 23:00:38   │   │ [YES (y)] [NO (n)]                  │     │
│    基本的にローカル実…  │   └─────────────────────────────────────┘     │
│                         │ ─────────────────────────────────────────── │
│   …(以下省略)…          │ [📎] 入力欄  auto mode on (shift+tab)        │
│                         │      [送信] [🪄] [/clear] [/model] [/]       │
└──────────────────────────┴──────────────────────────────────────────────┘
  ヘッダーの [1][0][6] は左から「実行中 / 承認待ち / スタンバイ」のセッション数
```

### 画面構成

- **ヘッダー**
  - 状態サマリチップ `[実行中][承認待ち][スタンバイ]`（承認待ち > 0 のときは点滅）と、プロバイダ別接続数 `Claude:N / Codex:N`
  - 右端: `⏻`（Hub 停止）、`設定`（言語・テーマ・タイムアウト等の設定パネル）
- **左サイドバー（セッション一覧）**
  - 上部: `+ 新しいセッション` ボタン（クリックで spawn ダイアログを開く）
  - 起動 cwd 直下の **プロジェクトフォルダ単位**でグルーピング表示。フォルダ名横にもセッション数チップ
  - 各セッションカード: `★`（お気に入り）／ `×`（閉じる）／ プロバイダ色のドット ＋ 番号 ＋ 状態バッジ（実行中 / スタンバイ / 待機中 / 完了 / エラー / 切断）／ 最終応答時刻 ／ 直近の出力プレビュー
  - 完了・エラーのセッションも一覧に残ります（手動で `×` を押すまで保持）
- **右ペイン（ターミナル + 入力）**
  - 上部バー: アクティブセッションのプロバイダ・cwd、`↑最上部へ`（PTY バッファの先頭にスクロール）
  - 中央: xterm.js でリアルタイム描画される PTY 出力
  - 下部: 入力欄（複数行可）、添付・送信・スラッシュコマンドピッカー（`/clear`, `/model`, `/`）、auto mode 切替ヒント `shift+tab`
- **承認アクションバー**: 承認待ちが発生すると入力欄の上に表示。クリック、または `←` / `→` でフォーカス移動 → `Enter` で確定
- **ターミナル直接入力との同期**: ターミナル側で `y` / `n` 等を直接タイプして承認を解決した場合、アクションバーは自動で消えます
- **画像添付**: ペースト・D&D で添付エリアに置くと、送信時にローカルファイル化して PTY に inject されます

---

## 音声入力

Hub UI の入力欄に音声でテキストを入力できます。

### 使い方

1. 🎤 ボタンをクリック、または `Alt+V`（macOS: `Option+V`）で録音開始
2. マイクに向かって話す
3. 認識されたテキストが入力欄に随時挿入される
4. 再度 `Alt+V` またはボタンをクリックで録音停止 → 入力欄の内容を確認して `Enter` で送信

> **対応ブラウザ**: Chrome / Edge（ブラウザ内蔵の Web Speech API を使用）  
> 初回使用時にマイクへのアクセス許可が必要です。

### 自動送信トリガー

設定パネル → **自動送信トリガー** を ON にして送信フレーズを設定すると、音声認識または手入力でフレーズが末尾に検出されたとき自動的に送信されます。

**例**: フレーズを `送信実行` に設定した場合
- 「バグを修正して**送信実行**」と発話 → 「バグを修正して」が自動送信される
- 入力欄に `バグを修正して送信実行` と入力 → 「バグを修正して」が自動送信される

フレーズ自体は PTY・AI には送られません。

---

## キーボードショートカット

| キー | 操作 |
|------|------|
| `Enter` | メッセージを送信 |
| `Shift+Enter` | 入力欄で改行 |
| `Tab` / `Shift+Tab` | 次 / 前のセッションへ切り替え |
| `←` / `→` | アクションバーのボタン間でフォーカス移動（アクションバー表示中・入力が空のとき） |
| `Enter` | フォーカス中のアクションバーボタンを実行 |
| `Alt+V` | 音声入力のON/OFFを切り替え |
| `Ctrl+V` | 画像を添付エリアにペースト |
| `Ctrl+C` | PTY に SIGINT を送信（テキスト選択中はコピー） |
| `Ctrl+D` | PTY に EOF を送信 |
| `Ctrl+O` | Claude Code の折りたたみ内容を展開 |

---

## 設定ファイル

初回起動時に自動生成されます。

| OS | パス |
|---|---|
| Windows | `%USERPROFILE%\.ai-cli-hub\config.yaml` |
| macOS / Linux | `~/.ai-cli-hub/config.yaml` |

```yaml
hub:
  port: 47777               # デフォルトポート（衝突時は 47778, 47779... と自動探索）
  open_browser: false       # true にすると serve 起動時にブラウザを自動で開く
  auto_shutdown: true       # 全ラッパーが終了したら Hub も自動停止
  log_dir: ""               # 空 = ~/.ai-cli-hub/logs
  idle_timeout_min: 60      # アイドル状態のセッションを自動切断するまでの分数（0 = 無効）

log:                        # hub.log のローテーション設定（lumberjack）
  enabled: true
  max_size_mb: 10           # 1 ファイルの上限サイズ
  max_backups: 3            # 保持するローテーション後ファイル数
  compress: false           # ローテーション後に gzip 圧縮するか

token: ""                   # 空 = 起動時にランダム生成（再起動しても URL は変わらない）
```

`token` をリセットしたい場合は `token:` 行を削除して Hub を再起動してください。

> このほか `approval` / `spawn` / `slash_cmd_sources` セクションが UI 操作によって自動追記されることがあります（手書き不要）。

---

## 終了パターン

| 操作 | 結果 |
|---|---|
| ブラウザのタブを閉じる | Hub もセッションも継続（一定時間放置すると後述のアイドルタイムアウトで切断） |
| AI CLI が終了する | そのセッションが「完了 / エラー / 異常」状態になる |
| シェルを閉じる | ラッパーが終了 → セッション切断 |
| 全ラッパー終了 | Hub も自動停止（`auto_shutdown: true` のとき） |
| Hub UI の「Hub 停止」ボタン | Hub 停止 |
| `ai-cli-hub stop` | Hub 停止 |

### ゾンビセッション対策と Hub クラッシュ耐性

子の AI セッション（Claude Code / Codex CLI のプロセス）が宙に浮いたまま走り続けて API 課金が止まらないこと、および Web UI のバグや手動再起動で進行中の AI 作業が道連れになることを避けるため、wrapper 側に「Hub が落ちたら一定時間は復帰を待つ」ロジックが入っています。

WebSocket が切れたとき、wrapper はまず Hub の HTTP エンドポイントを probe して**意図的な切断か Hub クラッシュかを判別**します。

| 切断の種類 | wrapper 側の挙動 |
|---|---|
| **意図的な切断**（Hub UI の `×` で個別 dismiss、すべて停止、idle timeout など）<br>= Hub HTTP が正常応答する | 配下の PTY（claude / codex プロセス）を**即座に**終了させて exit。猶予なし |
| **Hub クラッシュ / `.exe` コンソール `×` 閉じ**<br>= Hub HTTP に到達できない | `wrapper_reconnect_grace_sec` 秒間（デフォルト **3600 秒 = 60 分**）、2 秒間隔で Hub への再接続を試みる。<br>　• 復帰したら新しいセッションとして再登録し、PTY をそのまま継続。直近 64KB の出力を UI に再生<br>　• 猶予が切れても復帰しなければ PTY を kill |
| **ブラウザだけ閉じて Hub は生存**（UI 接続が 0 のまま） | Hub 側で `idle_timeout_min` 分（デフォルト 60 分）経過後に全 wrapper を強制切断（→ 上記「意図的な切断」扱いになる） |

> **意図**: Web UI／Hub サーバ側のバグで Hub が落ちた／再起動が必要になった場合に、走行中の AI セッションを猶予時間内に復帰させれば作業が失われないようにするための仕組みです。長時間タスク（数時間スケールの自走）を扱う場合は `wrapper_reconnect_grace_sec` を 12 時間（43200 秒）など長めに設定しておくと安全です。「ブラウザを閉じて忘れる」「dismiss を押す」など**ユーザが明示的に止めた**経路は従来どおり即座にセッションを終了させます。

設定箇所:

- `~/.ai-cli-hub/config.yaml`
  - `hub.wrapper_reconnect_grace_sec`: 0 で再接続を無効化（旧来の即 kill 動作）。0–86400 秒（最大 24 時間）。デフォルト 3600 秒 = 60 分（設定パネルからも分単位で変更可。**新しく起動するセッションのみが対象**で、すでに走行中のセッションは spawn 時の値のまま）
  - `hub.idle_timeout_min`: UI 切断後のセーフティネット。0–1440 分、0 で無効化（設定パネルからも変更可）

---

## 画像転送

Hub UI から wrap セッションへ画像ファイルを送信できます。

### 操作手順

1. `ai-cli-hub serve` を起動
2. ブラウザで Hub UI を開く
3. セッションカードを選択した状態で、画像を以下のいずれかの方法で送信:
   - **ペースト**: `Ctrl+V`
   - **ドラッグ&ドロップ**: サイドバー下部の枠にドロップ
   - **クリック選択**: 枠をクリックしてファイルダイアログを開く
4. Hub が `~/.ai-cli-hub/attachments/<session-id>/` に保存し、PTY へパスを注入
   - Claude: `@<保存パス>` 形式
   - Codex: `<保存パス>` 形式

### 動作確認スクリプト（Windows / PowerShell 7）

```powershell
pwsh scripts/test_attach.ps1          # テスト実行（Hub 自動起動 → WS 接続 → PNG 送信）
pwsh scripts/test_attach.ps1 -KeepHub # Hub を起動したままにする
```

---

## ログ

| 種類 | パス | 内容 |
|---|---|---|
| Hub ログ | `~/.ai-cli-hub/logs/hub.log` | Hub サーバの動作ログ（lumberjack でローテーション。設定は `log:` セクション参照） |
| セッション生ログ | `~/.ai-cli-hub/logs/sessions/<provider>_<YYYY-MM-DD_HHMMSS>_<folder>_s<id>.log` | 各 wrap セッションの PTY 生ログ（ANSI 含む） |
| セッション履歴 | `~/.ai-cli-hub/logs/sessions/<provider>_<YYYY-MM-DD_HHMMSS>_<folder>_s<id>.jsonl` | セッションイベント履歴（`session_start` / `user_input` / `pty_output` / `attach` / `session_end` / `session_dismiss`） |

Hub UI のログパスボタンでログディレクトリのパスをクリップボードにコピーできます。

---

## セキュリティ

- Hub の HTTP / WebSocket サーバは `127.0.0.1` のみにバインドし、外部ホストから直接アクセスすることはできません
- ランダムトークンを起動時に生成し、URL に付与します（`?token=xxx`）
- `ai-cli-hub` 自身はテレメトリ・利用状況の送信を一切行いません

### 外部への通信について

`ai-cli-hub` 自体はローカル動作を前提としていますが、以下の外部 HTTPS 通信が発生し得ます。

- **スラッシュコマンド一覧の取得（Hub 本体の通信）**: スラッシュコマンドピッカーを開くと、Hub は `https://raw.githubusercontent.com/ishizakahiroshi/ai-cli-hub/main/resources/slash-commands/{claude,codex}.md` を取得し、24 時間キャッシュします。取得元 URL は設定パネルの **スラッシュコマンドソース** から変更可能で、ローカルファイルパスを指定することもできます。
- **wrap 対象 CLI の API 通信（CLI 自身の通信）**: ラップ対象である Claude Code / Codex CLI 自身は、それぞれのベンダー API（Anthropic / OpenAI）と HTTPS で直接通信します。`ai-cli-hub` は PTY の入出力をローカル WebSocket で中継するだけで、これらの API 通信を傍受・記録・プロキシすることはありません。元の CLI のネットワーク挙動がそのまま適用されます。

### ⚠️ 重要: ローカル実行限定

`ai-cli-hub` は **同一マシン上での利用** を前提に設計されています。以下は絶対に行わないでください。

- リモートサーバー（VPS / クラウド）で `serve` を起動して外部から接続する
- `127.0.0.1` 以外のアドレス（`0.0.0.0` / LAN IP 等）にバインドするよう改造する
- Hub UI をリバースプロキシ（nginx / Caddy 等）で外部公開する
- Hub URL（トークン付き）を他人と共有する

Hub UI には「ログフォルダを OS のファイルマネージャで開く」など、ホストマシンに対する操作 API（`/api/open-dir` 等）が含まれます。これらはローカル前提だから安全な設計であり、外部公開すると **任意のフォルダ操作・情報漏洩** につながる可能性があります。

---

## ライセンス

MIT

第三者依存の通知は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)、vendored/browser 側のライセンス本文は [web/src/vendor/THIRD_PARTY_LICENSES.txt](web/src/vendor/THIRD_PARTY_LICENSES.txt) に記載しています。
