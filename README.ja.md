# any-ai-cli

![any-ai-cli ダッシュボード](assets/readme-dashboard.jpg)

複数の AI コーディング CLI（Claude Code / Codex CLI / GitHub Copilot CLI / Cursor Agent CLI）を並列で動かすときの、**承認操作・進捗監視を 1 画面の Web ダッシュボードで一元管理する**ツール。

> **ローカル優先設計** — Hub UI は `127.0.0.1` のみにバインド。`any-ai-cli` 自身はテレメトリを送信しません（slash command 一覧の GitHub 取得・wrap 対象 CLI のベンダー API 通信については「[セキュリティ](#セキュリティ)」セクション参照）。
> セッションログにはユーザー入力・AI出力が保存されます。機密情報として扱ってください。

---

## 概要

複数のターミナルで AI コーディング CLI を並列実行していると、承認待ちがどこで発生しているか把握しづらくなります。`any-ai-cli` は各 CLI を PTY でラップし、ブラウザの Hub UI で一元監視・承認操作を行えるようにします。CLI 本体の機能はそのままで、承認 GUI だけを追加する設計です。

```
Terminal pane #1              Terminal pane #2
┌────────────────────┐        ┌────────────────────┐
│ any-ai-cli claude  │        │ any-ai-cli codex   │
│  (PTY 素通し)       │        │  (PTY 素通し)       │
└────────┬───────────┘        └────────┬───────────┘
         │ WebSocket                   │ WebSocket
         └─────────────┬───────────────┘
                       ▼
            ┌──────────────────┐
            │ any-ai-cli serve │  http://127.0.0.1:47777
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

## 主な機能

- **承認パネル統合**: Claude Code / Codex CLI / GitHub Copilot CLI / Cursor Agent CLI の承認待ちをブラウザ上のアクションバーで処理
- **複数質問の一括承認**: 1つの承認ブロック内の番号付き質問を選択し、まとめて PTY へ送信
- **リアルタイム PTY 表示**: xterm.js + WebSocket で CLI 出力を表示
- **チャット履歴 / 分割表示**: 会話ログを吹き出し形式で読み、検索・フィルタし、ライブターミナルと並べて表示
- **マルチタブ**: 複数の実行中セッションをグリッドで同時監視
- **Files タブ**: プロジェクトファイルをツリー表示し、Markdown / コードのプレビュー、パスコピー、フォルダ作成、競合検出付き保存、リネーム、移動、空フォルダ削除を実行
- **Git ビュー**: ブランチ履歴、commit 詳細、変更ファイル、diff、fetch、`git pull --ff-only` を checkout なしで実行
- **Commit all**: 明示的な Review 後に working tree 全体を `git add -A` してローカル commit（push は実行しません）
- **Workbench タブ**: 保存済みセッション履歴、タイムライン、要約、redact 済み export、prompt template、task/policy メモ、diagnostics、usage 集計、stale session、worktree helper を扱う
- **ファイル / 画像添付**: ファイルや画像の paste / D&D からローカル保存し、セッションへパスを inject
- **音声入力**: Chrome / Edge の Speech Recognition API でマイクからプロンプトを入力
- **PWA + opt-in Web Push**: Hub をローカル Web アプリとしてインストールし、Settings で明示的に有効化した場合だけ承認待ち通知を受け取る
- **複数セッション管理**: 1つの Hub UI で複数 AI CLI セッションを切り替え
- **承認検出パターン profile**: GitHub から同期する公式 trigger phrase と、ユーザー編集用 custom profile を分離
- **サーバ側ユーザー設定**: 音声、通知音、お気に入り、セッション順、spawn 既定、アバター設定を `config.yaml` に保存
- **モデルピッカー + Ollama route 自動切替**: spawn フォームから Anthropic / OpenAI / Ollama Cloud / Ollama Local のモデルを選択でき、Hub が必要な `ANTHROPIC_*` / `OPENAI_*` 環境変数をセッションごとに自動注入（shell での事前設定不要）
- **Windows 統合ランチャー**: `any-ai-cli-launcher.exe` で WSL 内や VPS（SSH）の Hub へ接続プロファイルから接続し、Windows ブラウザから操作
- **VPS / Docker 運用資材**: GHCR image、ユーザー別コンテナ、loopback 限定公開、自動更新スクリプトでサーバー運用
- **クリーン transcript 生成**: 人間が読める `.txt` を自動生成し、`log-clean` で手動再生成も可能
- **ローカル限定**: Hub は `127.0.0.1` のみに bind。`any-ai-cli` 自身はテレメトリを送信しません

---

## 動作要件

| 項目 | 要件 |
|---|---|
| Go | 1.25 以上（ビルド時） |
| OS | Windows 10/11、macOS、Linux |
| ブラウザ | Chrome / Edge / Firefox / Safari |
| AI CLI | Claude Code、Codex CLI、GitHub Copilot CLI、Cursor Agent CLI（使う provider は別途インストール済みであること） |

### v0.3.0 の検証状況

- 実機で動作検証済み: Windows ローカル Hub、Windows 統合ランチャー（`wsl` / SSH tunnel profile）
- 実機で十分に未検証: ネイティブ Linux / ネイティブ macOS

Linux / macOS でも動作する想定ですが、v0.3.0 時点では実機での十分な検証は未実施です。
ご利用の際はご了承ください。問題があれば Issue で報告してください。

---

## インストール

### ビルド済みバイナリを使う場合

リリースページから自分の OS 向け zip をダウンロードして展開し、バイナリを `PATH` の通った場所に置きます。

#### Windows Smart App Control について

現在の Windows 向けリリースバイナリは Authenticode コード署名されていません。
Windows 11 の Smart App Control が有効な PC では、`any-ai-cli.exe` が信頼されていないアプリとしてブロックされる場合があります。
これはチェックサム検証とは別の仕組みです。`SHA256SUMS.txt` はリリース成果物の完全性確認用に署名されていますが、`.exe` 本体のコード署名ではありません。

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
  --certificate-identity-regexp "https://github.com/ishizakahiroshi/any-ai-cli/.github/workflows/release.yml@refs/tags/v.*" \
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
git clone https://github.com/ishizakahiroshi/any-ai-cli
cd any-ai-cli

# 現在の OS 向けにビルド
go build -o any-ai-cli.exe ./cmd/any-ai-cli   # Windows
go build -o any-ai-cli ./cmd/any-ai-cli        # macOS / Linux
```

#### クロスコンパイル

```bash
GOOS=windows GOARCH=amd64 go build -o dist/any-ai-cli-windows-x64.exe          ./cmd/any-ai-cli
GOOS=darwin  GOARCH=amd64 go build -o dist/any-ai-cli-macos-intel              ./cmd/any-ai-cli
GOOS=darwin  GOARCH=arm64 go build -o dist/any-ai-cli-macos-apple-silicon      ./cmd/any-ai-cli
GOOS=linux   GOARCH=amd64 go build -o dist/any-ai-cli-linux-x64                ./cmd/any-ai-cli
```

---

## アンインストール

`any-ai-cli` はインストーラなしの単体バイナリです。アンインストールは、バイナリを置いたフォルダで `uninstall` サブコマンドを実行します。

**Windows** — `any-ai-cli.exe` を置いたフォルダで実行:

```powershell
.\any-ai-cli.exe uninstall          # 設定・ログ（~/.any-ai-cli/）を削除
.\any-ai-cli.exe uninstall --purge  # 上記 + バイナリ本体も削除
```

**macOS / Linux / WSL** — `any-ai-cli` を置いたフォルダで実行:

```bash
./any-ai-cli uninstall          # 設定・ログ（~/.any-ai-cli/）を削除
./any-ai-cli uninstall --purge  # 上記 + バイナリ本体も削除
```

削除対象が表示され、確認後に実行されます。

| オプション | 削除されるもの |
|---|---|
| (なし) | `~/.any-ai-cli/`（設定・ログ・添付ファイル）。バイナリのパスを表示するので手動で削除してください |
| `--purge` | 上記 + バイナリ本体 |

**手動で削除する場合**

1. `~/.any-ai-cli/`（Windows: `%USERPROFILE%\.any-ai-cli\`）を削除
2. ダウンロードしたバイナリ（`any-ai-cli.exe` / `any-ai-cli`）を削除

> **ブラウザのデータは削除されません。** `uninstall` はブラウザのストレージに触れられません。大半の設定（テーマ・言語・フォントサイズ・お気に入り・クイックコマンド等）はサーバ側 `~/.any-ai-cli/` に保存されているため削除されますが、端末ごとの表示状態（ファイルツリーの開閉・ペインのレイアウト・スクロールバック量）は `localStorage` に残ります。消去するには Hub を開いていたタブで `F12` を押し、コンソールで `localStorage.clear()` を実行してください。

---

## クイックスタート（推奨）

通常はこれだけです。CLI を直接叩く必要はありません。

1. リリースページから自分の OS 向け zip をダウンロードして展開する
2. **`any-ai-cli.exe` をダブルクリックして起動する**（または引数なしで `any-ai-cli` を実行）
   - Hub が起動し、ブラウザが自動で開きます（`http://127.0.0.1:47777/?token=<token>`）
   - すでに Hub が起動済みの場合は、ブラウザを開くだけで終了します
3. ブラウザの Hub UI 左下の **「+ 新しいセッション」** をクリックし、使う AI CLI のセッションを起動する
4. セッションが UI に表示されれば運用開始。承認待ちが発生すると入力欄の下にアクションバーが出るので、クリックまたはキーボードで操作する

ターミナルを別途開かなくても、セッションの起動・操作・承認はすべて Hub UI から行えます。

> **⚠ コンソールウィンドウについて**
> `.exe` を起動すると黒いコンソールウィンドウがブラウザと一緒に開きますが、**これが Hub サーバの実体プロセスです**。ウィンドウを `×` で閉じると Hub が終了します（邪魔な場合は閉じずに **最小化** してください）。
> なお Hub が落ちた場合でも、走行中の AI セッションはデフォルトで **60 分間 Hub の復帰を待つ**ようになっています（`config.yaml` で 0–86400 秒 = 最大 24 時間まで変更可、長時間タスクを走らせる場合は伸ばせます）。間に合わなければ自動で終了するので、Web UI 側のバグや再起動で AI 作業が即座に道連れにはなりません。詳細は「[Shutdown, zombie protection & Hub crash resilience](#shutdown-zombie-protection--hub-crash-resilience)」相当のセクション「[ゾンビセッション対策と Hub クラッシュ耐性](#ゾンビセッション対策と-hub-クラッシュ耐性)」参照。
> Hub を意図的に停止するときは、Hub UI 右上の `⏻` ボタン、または別ターミナルで `any-ai-cli stop` を使ってください。

### Windows 統合ランチャー

`any-ai-cli-launcher.exe` は WSL と VPS SSH の両方を対象に接続プロファイルを管理できる統合ランチャーです。プロファイルは `~/.any-ai-cli/launcher-profiles.yaml` に保存されます。

#### 仕組み

保存したプロファイルをもとに接続先 Hub へ繋ぎ（必要な場合は Hub を起動し）、ブラウザを自動で開きます。プロファイルの種別は 2 つです。

| 種別 | 用途 |
|---|---|
| `wsl` | WSL 内で `any-ai-cli serve` を起動し、Windows ブラウザで開く |
| `ssh` | VPS 等のリモートマシンへ SSH 経由で接続する |

`ssh` 種別にはさらに 2 つのモードがあります。

| モード | 用途 |
|---|---|
| `serve` | VPS に SSH してリモート側で `any-ai-cli serve` を起動する |
| `tunnel` | リモート側ですでに起動中の Hub（Docker compose 常駐 Hub 等）へポートフォワードする |

どちらのモードでも、リモートの Hub は `127.0.0.1` にのみ bind したままです。SSH のローカルフォワード（`-L 127.0.0.1:<port>:127.0.0.1:<port>`）によって Hub をネットワークに公開することなく Windows ブラウザから到達可能にします。

`wsl` プロファイルは内部で `wsl.exe` を呼び出し、WSL 内の Linux バイナリ（`any-ai-cli serve`）を立ち上げます。Linux 側が Hub URL を標準出力へ出力した時点で Windows の既定ブラウザが自動的に開きます。WSL 内では `bash -ilc`（ログインシェル + インタラクティブ）で起動するため、`~/.bashrc` に書かれた `nvm` / `pnpm` / `cargo` 等の PATH 設定がそのまま有効になります。Windows 側でポートが衝突している場合（`any-ai-cli.exe` がすでに 47777 を使用中など）、ランチャーは空きポートを自動的に選択します。

#### セットアップ

`any-ai-cli-<version>-windows-x64.zip` を展開して `any-ai-cli-launcher.exe` を Windows の PATH が通った場所に置きます。

`~/.any-ai-cli/launcher-profiles.yaml` を作成します。

```yaml
version: 1
profiles:
  # WSL プロファイル — WSL 内で Hub を起動する
  - name: my-wsl
    type: wsl
    distro: Ubuntu-22.04  # 省略 = wsl.exe の既定ディストリビューション
    hub_port: 0           # 0 = Windows 側衝突を避けて自動選択

  # VPS プロファイル（serve モード）— SSH してリモートで any-ai-cli serve を起動
  - name: my-vps
    type: ssh
    mode: serve
    host: vps.example.com
    user: your-user
    hub_port: 47777

  # VPS プロファイル（tunnel モード）— Docker 常駐 Hub へポートフォワード
  - name: vps-docker
    type: ssh
    mode: tunnel
    host: vps.example.com
    user: your-user
    hub_port: 47801
    token_command: "docker exec any-ai-cli-user1 sh -c 'grep ^token ~/.any-ai-cli/config.yaml | cut -d\" \" -f2'"
```

#### WSL プロファイルの前提: WSL 側に Linux バイナリを配置

`wsl` プロファイルを使うには、WSL 内の PATH が通った場所に Linux 版 `any-ai-cli` バイナリが必要です。リリースページから `any-ai-cli-<version>-linux-x64.zip` をダウンロードして展開し、配置します。

```bash
unzip any-ai-cli-<version>-linux-x64.zip

# ~/.local/bin を使う場合（ユーザーローカル、sudo 不要）
mkdir -p ~/.local/bin
mv any-ai-cli ~/.local/bin/any-ai-cli
chmod +x ~/.local/bin/any-ai-cli

# ~/.local/bin が PATH に含まれているか確認
echo $PATH | grep -q "$HOME/.local/bin" && echo "OK" || echo "~/.local/bin を PATH に追加してください"
```

`~/.local/bin` が PATH に入っていない場合は `~/.bashrc` に追記します。

```bash
# ~/.bashrc に追記
export PATH="$HOME/.local/bin:$PATH"
```

システム全体に置く場合（sudo 必要）:

```bash
sudo mv any-ai-cli /usr/local/bin/any-ai-cli
sudo chmod +x /usr/local/bin/any-ai-cli
```

WSL 内で動作確認:

```bash
any-ai-cli --version
```

#### tunnel モード: 最初から最後までの流れ

`tunnel` モードは、リモートで動き続ける Hub に接続する方式です。ランチャーの窓を閉じても切れるのは SSH トンネルだけで、Hub と AI セッションは動き続けます。次回接続すれば前回の続きからそのまま再開できます。ゼロからの手順は以下のとおりです。

**A. リモート側の準備（最初に 1 回だけ）**

1. Linux 版 `any-ai-cli` バイナリをリモートマシンに配置し、実行権限を付けます。
2. **ポートを固定して** Hub を起動し、常駐させます（tunnel モードではポート自動選択は使えません）。常駐方法は systemd / tmux・screen / Docker のいずれでも構いません。

   ```bash
   any-ai-cli serve --port 47777
   ```

   初回起動時にアクセス用トークンが自動生成され、`~/.any-ai-cli/config.yaml` の `token:` に保存されます。
3. そのトークンを出力するコマンドを決めます（プロファイルの `token_command` になります）。例：

   ```bash
   awk '/^token:/{print $2}' ~/.any-ai-cli/config.yaml
   ```

   SSH 経由で一度実行し、トークン文字列が 1 行返ることを確認してください。

**B. Windows 側の準備（最初に 1 回だけ）**

4. SSH の**鍵認証**を設定します。ランチャーは `ssh.exe` を `-o BatchMode=yes`（対話入力禁止）で実行するため、パスワード認証は使えません。`ssh ユーザー名@ホスト` がパスワード入力なしで通る状態にしてください。
5. プロファイルを作成します。ランチャーの UI（種別: SSH / モード: tunnel）でも、`launcher-profiles.yaml` の直接編集でも構いません。

   | 項目 | 値 | 必須 |
   |---|---|---|
   | `name` | 任意の名前 | ○ |
   | `type` | `ssh` | ○ |
   | `mode` | `tunnel` | ○ |
   | `host` | リモートの IP / ホスト名 | ○ |
   | `user` | SSH ログインユーザー（空欄 = ssh 既定） | — |
   | `ssh_port` | 22 以外なら指定（0 = 既定） | — |
   | `identity_file` | 空欄 = 既定鍵 / agent | — |
   | `hub_port` | 手順 2 のポート番号（例: `47777`）。必ず一致させる | ○ |
   | `token_command` | 手順 3 のコマンド | ○ |

**C. 日常の利用（毎回）**

1. ランチャーを起動してプロファイルを選ぶと、トンネル確立 → `token_command` でトークン取得 → Hub の応答確認 → ブラウザ表示まで自動で進みます。
2. あとは Hub UI で普段どおり操作します（セッション spawn・承認など）。
3. 終わるときはランチャーの窓を閉じるだけ。切れるのはトンネルだけで、リモートのセッションは動き続けます。
4. 次回は同じプロファイルで接続すれば、前回の続きに入れます。

**つまずきやすい点**

- **ポート不一致** — リモートの `serve --port` とプロファイルの `hub_port` は同じ番号にする必要があります。
- **パスワードを聞かれる状態** — BatchMode で即失敗します。鍵認証が必須です。
- **`token_command` の出力が空** — リモートで Hub を一度も起動していないと `config.yaml` にトークンがありません。先に手順 2 を済ませてください。
- **Docker で動かす場合** — コンテナの Hub ポートをホスト側の `127.0.0.1` に公開しておく必要があります（トンネルの終点はリモートマシンの `127.0.0.1:<hub_port>` です）。

#### 起動

```powershell
any-ai-cli-launcher.exe                    # プロファイル 1 件なら即接続。複数なら選択画面を表示
any-ai-cli-launcher.exe --profile my-vps  # 指定プロファイルで接続
any-ai-cli-launcher.exe --last            # 前回使ったプロファイルで接続
any-ai-cli-launcher.exe --ui             # 常に選択画面を表示
```

#### セキュリティ前提

ランチャーを使っても Hub のセキュリティモデルは変わりません。

- Hub はリモート側でも `127.0.0.1` にのみ bind し続けます（`0.0.0.0` バインドやリバースプロキシ経由の公開はしません）
- SSH フォワードは `127.0.0.1` 同士のローカルフォワードのみ使用します（`-g` や `GatewayPorts` は不使用）
- パスワード・鍵のパスフレーズは保存しません。鍵認証が必要です（`-o BatchMode=yes` で対話を禁止）
- `token_command` で取得したトークンは現在のセッション中のみ使用し、`launcher-profiles.yaml` には書き込みません

プロファイルの全フィールドと接続フローの詳細は [docs/v0.2.0-any-ai-cli-design.md — §11b](docs/v0.2.0-any-ai-cli-design.md) を参照してください。

#### Windows がランチャーをブロックする場合: ローカル `.exe` なしで VPS に接続

Windows SmartScreen や会社 PC のポリシーで `any-ai-cli-launcher.exe` を実行できない場合でも、Windows 側で any-ai-cli の exe を一切動かさずに VPS 上の Hub へ接続できます。この導線で Windows 側が使うものは次の 2 つだけです。

- Windows 標準の OpenSSH client（`ssh.exe`）
- 普段使っているブラウザ

`any-ai-cli` 本体と provider CLI（`claude` / `codex` / `copilot` / `cursor-agent`）は VPS 側で動かします。ランチャーほど自動ではありませんが、SSH トンネル用のウィンドウを 1 つ開いたままにして、ブラウザで Hub URL を開くだけです。

**どの設定がどこに保存されるか**

| 項目 | 保存先 | 補足 |
|---|---|---|
| SSH 接続先・ユーザー・鍵パス | Windows の `%USERPROFILE%\.ssh\config` | 通常の SSH 設定として保存します |
| Hub token | VPS の `~/.any-ai-cli/config.yaml` | チャット、Issue、スクショに貼らないでください |
| Hub の UI 設定、お気に入り、spawn 既定値 | VPS の `~/.any-ai-cli/config.yaml` | Hub が VPS 側で動くため、再接続しても残ります |
| ログ・添付ファイル | VPS の `~/.any-ai-cli/logs/`, `~/.any-ai-cli/attachments/` | Windows PC には保存されません |
| 作業リポジトリ | VPS のファイルシステム | Hub が編集するのは VPS 側のファイルです |

**A. VPS を選んで準備する**

SSH ログインできる Linux VM であれば、特定の VPS 事業者に依存しません。Ubuntu 22.04 / 24.04 系の小さなインスタンスで始められます。最低ラインは 1 GB RAM 程度、provider CLI や長時間セッションを動かすなら 2 GB 以上あると余裕があります。無料枠を使う場合は、スリープするか、ディスクが永続化されるか、長時間 SSH が切られないかを確認してください。

VPS の firewall / security group はシンプルにします。

- SSH だけ許可します（`22/tcp`、または自分で変更した SSH port）
- `47777` / `47877` などの Hub port はインターネットに開けません
- nginx / Caddy / Cloudflare Tunnel などで Hub を外部公開しません

VPS に Linux 版 `any-ai-cli` を配置します。ユーザー単位で置くなら、たとえば次の形です。

```bash
mkdir -p ~/.local/bin
# GitHub Releases から any-ai-cli-<version>-linux-x64.zip をダウンロードして展開します。
mv any-ai-cli ~/.local/bin/any-ai-cli
chmod +x ~/.local/bin/any-ai-cli
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
any-ai-cli --version
```

使う provider CLI（`claude` / `codex` / `copilot` / `cursor-agent`）も VPS 側にインストールし、VPS 側でログインを済ませます。AI セッションは VPS 上で動くためです。

**B. Hub を固定 port の loopback で起動する**

最初の動作確認は、普通の SSH shell で構いません。

```bash
mkdir -p ~/work
cd ~/work
any-ai-cli serve --port 47777
```

普段使いでは `tmux` / `screen` / `systemd` / Docker のいずれかで常駐させます。手作業で一番分かりやすいのは `tmux` です。

```bash
tmux new -s any-ai-cli
cd ~/work
any-ai-cli serve --port 47777
```

`Ctrl+B` のあと `D` で tmux から抜けられます。あとで戻るときは:

```bash
tmux attach -t any-ai-cli
```

Hub が loopback にだけ bind していることを確認します。

```bash
ss -ltnp | grep ':47777'
```

期待値は `127.0.0.1:47777` です。`0.0.0.0:47777` や VPS の public IP が見えた場合は、そのまま接続せず設定を直してください。

token を確認します。

```bash
awk '/^token:/{print $2}' ~/.any-ai-cli/config.yaml
```

**C. Windows 側に SSH 接続先を保存する**

`%USERPROFILE%\.ssh\config` を作成または編集します。

```sshconfig
Host any-ai-vps
  HostName vps.example.com
  User ubuntu
  Port 22
  IdentityFile C:\Users\you\.ssh\id_ed25519
  ServerAliveInterval 30
```

PowerShell から接続確認します。

```powershell
ssh any-ai-vps
```

毎回パスワードを聞かれる場合は、先に SSH 鍵認証を設定してください。パスワード認証でもトンネル自体は張れますが、鍵認証の方が安定します。

**D. SSH トンネルを開く**

Windows PowerShell で次を実行します。

```powershell
ssh -N -T `
  -o ExitOnForwardFailure=yes `
  -o ServerAliveInterval=30 `
  -L 127.0.0.1:47777:127.0.0.1:47777 `
  any-ai-vps
```

このウィンドウは開いたままにします。手元ブラウザと VPS 上の Hub をつなぐ専用ケーブルだと思ってください。

Windows のブラウザで次を開きます。

```text
http://127.0.0.1:47777/?token=<VPSで確認したtoken>
```

`127.0.0.1` を VPS の IP アドレスに置き換えないでください。ブラウザは必ず手元 PC の転送 port に接続します。

**任意: ローカル `.cmd` でトンネル起動をショートカットする**

毎回 SSH コマンドを打ちたくない場合は、Windows 側に `connect-any-ai-cli.cmd` のようなファイルを自分で作れます。このファイルには token を保存せず、実行時に SSH 経由で VPS から token を読み取ってからブラウザを開きます。

```batch
@echo off
set HOST=any-ai-vps
set PORT=47777

for /f "tokens=2" %%T in ('ssh %HOST% "cat ~/.any-ai-cli/config.yaml" ^| findstr /b token:') do set TOKEN=%%T
if "%TOKEN%"=="" (
  echo Failed to read Hub token from %HOST%.
  pause
  exit /b 1
)

start "any-ai-cli tunnel" ssh -N -T -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -L 127.0.0.1:%PORT%:127.0.0.1:%PORT% %HOST%
timeout /t 2 >nul
start "" "http://127.0.0.1:%PORT%/?token=%TOKEN%"
```

切断するときは `any-ai-cli tunnel` のウィンドウを閉じます。Hub を `tmux` / `systemd` / Docker で常駐させていれば、切れるのは SSH トンネルだけで、VPS 側の Hub とセッションは残ります。

**launcher なし構成でつまずきやすい点**

- **ブラウザが 403 / 404 / blank になる** — token が違うか、Hub 再起動後の古い token を使っています。VPS 側で token を取り直してください。
- **HTML は出るがターミナルがつながらない** — local port と remote port は必ず同じ番号にします。例: `47777:127.0.0.1:47777`。
- **`ssh: bind: Address already in use`** — 手元 PC でその port が使用中です。VPS 側 Hub と SSH トンネルの両方を同じ別 port に変えてください。
- **ファイルが見つからない** — Hub は VPS 上で動いています。作業リポジトリは VPS 側に clone / 配置してください。
- **無料 VPS が切断された** — SSH をつなぎ直し、必要に応じて tmux / systemd / Docker の Hub を復帰してください。

---

## スマホから使う（iPhone / Android）

Hub UI はモバイル対応済みです（レスポンシブレイアウト・タッチ向けボタンサイズ・Esc/Ctrl/矢印のモバイルキーパネル・PWA 対応）。ただし Hub は `127.0.0.1` にのみ bind するため、**同一 Wi-Fi でも PC の LAN IP を開く方法では届きません**（これは設計どおりの挙動です）。スマホからはリモート PC アクセスと同じパターン、つまり **SSH ローカルフォワードで「スマホ自身の `127.0.0.1`」を Hub に向ける** 方法を使います。外部公開は不要です（そもそもサポート対象外です）。

**スマホ側に必要なもの**

- ローカルポートフォワード対応の SSH クライアントアプリ（例: [Termius](https://termius.com/) — 無料プランで十分）
- 通常のブラウザ（Safari / Chrome）

### A. 同一 Wi-Fi の自宅 PC に接続する

1. Hub を動かす PC 側で SSH サーバを有効化する
   - Windows: 設定 → システム → オプション機能 → **OpenSSH サーバー** を追加し、`sshd` サービスを開始
   - macOS: システム設定 → 一般 → 共有 → **リモートログイン**
   - Linux: `sshd` を導入・有効化
2. Termius に PC をホスト登録（LAN IP 例: `192.168.x.x`、PC のユーザー。鍵認証推奨）
3. **Port Forwarding** ルールを追加: 種別 **Local**、スマホ側 `127.0.0.1:47777` → 転送先 `127.0.0.1:47777`
4. トンネル接続後、スマホのブラウザで `http://127.0.0.1:47777/?token=<token>` を開く（token は PC 側の `serve` 出力か `~/.any-ai-cli/config.yaml` から取得）
5. 共有メニュー → **ホーム画面に追加** で PWA 化 — 以後はアプリ感覚でアイコンから起動できます

### B. VPS に接続する

A と同一手順で、Termius のホストを VPS にするだけです。自宅 PC の Hub と併用する場合は、接続先ごとにスマホ側ポートを分けてください（次節）。

### 複数 Hub を使うときのポート割り当て

トンネルはスマホ側の listen ポートを占有し、自身の Hub が動いている PC ではローカル `47777` がすでに使用中です。そこで **接続先ごとにスマホ側ポートを固定で割り当てます**:

| 接続先 | スマホ側 URL | Termius の Local Forward |
|---|---|---|
| 自宅 PC | `http://127.0.0.1:47777/?token=<PC側token>` | `47777` → `127.0.0.1:47777` |
| VPS | `http://127.0.0.1:47778/?token=<VPS側token>` | `47778` → VPS の `127.0.0.1:47777` |

Hub 自体はどこでも `47777` のままで構いません。変えるのはスマホ手元の listen ポートだけです。1つのスマホ側ポートを複数 Hub で使い回すことは推奨しません: ブラウザはポート番号込みでオリジンを区別するため、使い回すと別々の Hub が PWA・service worker・キャッシュ・`localStorage` を共有してしまい、トンネル切り替え後の token 不一致事故も起きます。ポートを分ければ「自宅」「VPS」2つの独立したホーム画面アイコンとして干渉なく併用できます。

### モバイル利用の注意点

- **iOS はバックグラウンドのアプリを凍結する** ため、Termius を裏に回すとしばらくしてトンネルが切れます。ホスト側のセッションは動き続けるので、Termius を開き直せば再接続され、PWA は続きから使えます。
- **Web Push**（設定で有効化・購読済みの場合）はトンネル切断中でも通知自体は届きますが、通知から Hub を開くにはトンネルの再接続が必要です。
- token は Hub 再起動で再生成されます。ブラウザが 403 になったら最新の token を取り直してください。

---

## ターミナル / シェルから直接起動したい場合

CLI 派の人や、シェル統合・自動化を組みたい人向けの代替手段です。Hub UI の「+ 新しいセッション」と機能的には同等で、好みで使い分けてください。

### 方法 A: provider 直指定

```powershell
any-ai-cli claude      # Hub 未起動なら自動でバックグラウンド起動してから Claude を起動
any-ai-cli codex       # 同上
any-ai-cli copilot     # 同上（インストール済み GitHub Copilot CLI を使用）
any-ai-cli cursor-agent # 同上（インストール済み Cursor Agent CLI を使用）
```

`any-ai-cli serve` を事前に実行しておく必要はありません。

### 方法 B: wrap サブコマンド（デバッグ用）

```powershell
any-ai-cli wrap claude
any-ai-cli wrap codex
any-ai-cli wrap copilot
any-ai-cli wrap cursor-agent
```

方法 A と機能は同じですが、内部実装の確認やデバッグ用途に使います。

### 方法 C: 透過モード（`ANY_AI_CLI_AUTO`）

シェルで一度だけ初期化しておくと、普段の `claude` / `codex` / `copilot` / `cursor-agent` コマンドがそのままラッパー経由で起動されるようになります。

> `any-ai-cli shell-init` は **POSIX シェル（bash / zsh）専用** の関数定義を出力します。PowerShell 用のスニペットは出力しません（後述の代替手順を参照）。

```bash
# シェル起動時に 1 回だけ実行（bash / zsh）
eval "$(any-ai-cli shell-init)"

# 監視したいセッションだけ環境変数を ON にする
export ANY_AI_CLI_AUTO=1
claude    # ← 自動でラッパー経由・Hub 未起動なら自動起動
codex     # ← 同上
copilot   # ← 同上
cursor-agent # ← 同上
```

`ANY_AI_CLI_AUTO=1` が設定されていないシェルでは、`claude` / `codex` / `copilot` / `cursor-agent` はそのまま元のコマンドとして動作します。グローバルな `.bashrc` 等は改変しません。

GitHub Copilot 対応は、公式 CLI を PTY 内で起動するだけです。`any-ai-cli` は GitHub OAuth token / PAT / Copilot credential を読み取り・保存・代理利用しません。

Cursor Agent 対応は、公式 `cursor-agent` CLI を PTY 内で起動するだけです（サインイン済みであることを前提とします）。`any-ai-cli` は Cursor のセッショントークンや認証情報を読み取り・保存・代理利用しません。

#### OS 別の自動化設定例

**PowerShell（Windows）**

`$PROFILE` に以下を追記してください（`shell-init` は PowerShell 非対応のため、関数を直接定義します）。

```powershell
if ($env:ANY_AI_CLI_AUTO -eq '1') {
    function claude { any-ai-cli claude @args }
    function codex  { any-ai-cli codex  @args }
    function copilot { any-ai-cli copilot @args }
    function cursor-agent { any-ai-cli cursor-agent @args }
}
```

Windows Terminal のプロファイル側で `ANY_AI_CLI_AUTO=1` をセットしておけば、そのタブだけ透過モードになります。

```jsonc
{
  "name": "AI Watch",
  "commandline": "pwsh.exe -NoExit",
  "environment": { "ANY_AI_CLI_AUTO": "1" }
}
```

**iTerm2（macOS）**

- Profiles → Environment → Variables: `ANY_AI_CLI_AUTO=1`
- Profiles → General → Send text at start: `eval "$(any-ai-cli shell-init)"`

**tmux（全 OS 共通）**

```bash
# ~/.tmux.conf
set-option -g default-command "ANY_AI_CLI_AUTO=1 bash -c 'eval \"$(any-ai-cli shell-init)\"; exec bash'"
```

---

## サブコマンド一覧

| コマンド | 説明 |
|---|---|
| `serve [--open] [--port N]` | Hub を起動。`--open` でブラウザを自動で開く |
| `claude [args...]` | Claude Code を Hub 経由で起動 |
| `codex [args...]` | Codex CLI を Hub 経由で起動 |
| `copilot [args...]` | GitHub Copilot CLI を Hub 経由で起動 |
| `cursor-agent [args...]` | Cursor Agent CLI を Hub 経由で起動 |
| `wrap <provider> [args...]` | 任意 provider をラップ（デバッグ用） |
| `shell-init` | 透過モード用のシェル関数スニペットを出力 |
| `status` | Hub の起動状態を表示 |
| `stop` | Hub を停止 |
| `log-clean <session.jsonl>` | セッション履歴からクリーン transcript を生成 |
| `uninstall [--purge]` | 設定・ログを削除してアンインストール。`--purge` でバイナリ本体も削除 |

---

## Hub UI

ブラウザで `http://127.0.0.1:47777/?token=<token>` を開きます。

```
┌─ ANY-AI-CLI  [1][0][6] │ ● Claude:2  ● Codex:5            [⏻] [設定] ─┐
├──────────────────────────┬──────────────────────────────────────────────┤
│ [+ 新しいセッション]     │ ● Codex  cwd: C:\dev\any-ai-cli   [↑最上部へ]│
│ 📁 any-ai-cli  [1][0][6] │ ターミナル出力 — Windows PowerShell          │
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
  - 起動 cwd 直下の **プロジェクトフォルダ単位**でグルーピング表示。フォルダ名横にもセッション数チップと Files 導線
  - 各セッションカード: `★`（お気に入り）／ `×`（閉じる）／ プロバイダ色のドット ＋ 番号 ＋ 状態バッジ（実行中 / スタンバイ / 待機中 / 完了 / エラー / 切断）／ Git ブランチバッジ（取得できる場合）／ 最終応答時刻 ／ 直近の出力プレビュー
  - カード右クリックで Git ビューを開く、Files タブを開く、セッションをアクティブ化、セッションIDコピーが可能
  - 完了・エラーのセッションも一覧に残ります（手動で `×` を押すまで保持）
- **右ペイン（ターミナル + 入力）**
  - 上部バー: アクティブセッションのプロバイダ・cwd、`↑最上部へ`（PTY バッファの先頭にスクロール）
  - 中央: xterm.js でリアルタイム描画される PTY 出力
  - 下部: 入力欄（複数行可）、添付・送信・スラッシュコマンドピッカー（`/clear`, `/model`, `/`）、auto mode 切替ヒント `shift+tab`
- **タブ**: Terminal / チャット履歴 / 分割 / マルチ / Files / Git を同じメイン領域で切り替え。Files / Git は遅延ロードされ、Hub 再起動後の復元にも対応
- **チャット履歴 / 分割**: ライブ PTY ストリームからユーザー入力、AI 出力、承認、添付を会話形式に整形。分割表示ではターミナルと履歴を並べて確認可能
- **マルチタブ**: 複数セッションをグリッドで表示し、フォーカス中ペインへ入力・リサイズ・承認 UI を連動
- **承認アクションバー**: 承認待ちが発生すると入力欄の上に表示。単一質問はボタン、複数質問は縦積みの選択肢と「Submit all」でまとめて送信
- **Files タブ**: 左にファイルツリー、右に Markdown / コードプレビュー。パスコピー、OS で開く、移動、リネームなどをコンテキストメニューから実行可能
- **Git タブ**: 読み取り専用の commit 履歴、ref 切替、commit 詳細、変更ファイル、diff プレビュー、コピー操作を提供。`Commit all` は Review 後にローカル commit のみ実行し、push はしない
- **ターミナル直接入力との同期**: ターミナル側で `y` / `n` 等を直接タイプして承認を解決した場合、アクションバーは自動で消えます
- **ファイル / 画像添付**: ペースト・D&D で添付エリアに置くと、送信時にローカルファイル化して PTY に inject されます

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

> ⚠️ **プライバシー注意**: ブラウザ内蔵認識のため、録音された音声は **ブラウザベンダー（Google / Microsoft）の音声認識サーバへ送信されます**。`any-ai-cli` のローカルファースト方針の例外となる機能なので、音声を外部に送りたくない場合は本機能を使用しないでください（ローカル推論の音声入力ツールを別途使う選択肢もあります）。詳細は「セキュリティ → 外部への通信について」を参照。

### 自動送信トリガー

設定パネル → **自動送信トリガー** を ON にして送信フレーズを設定すると、音声認識または手入力でフレーズが末尾に検出されたとき自動的に送信されます。

**例**: フレーズを `送信実行` に設定した場合
- 「バグを修正して**送信実行**」と発話 → 「バグを修正して」が自動送信される
- 入力欄に `バグを修正して送信実行` と入力 → 「バグを修正して」が自動送信される

フレーズ自体は PTY・AI には送られません。

### 終了検知の待ち時間

設定パネル → **音声入力** で「終了検知の待ち時間」を変更できます。Chrome の音声認識が無音で自動終了した後でも、直近の発話から指定秒数以内なら認識を再開します。

### トラブルシューティング

音声入力が動かない場合（ボタンを押しても反応しない・マイクは拾っているがテキストが出ない）は、以下の順で試してください。

1. **Chrome を完全に再起動する**（全ウィンドウを閉じて再起動）。Chrome 内部の音声認識状態が stuck することがあり、再起動で解消するケースが多いです。
2. 改善しない場合は、Chrome のアドレスバーに `chrome://settings/content/all?searchSubpage=127.0.0.1` を貼り付け、`127.0.0.1` のマイク権限をリセットして再度「許可」する。
3. それでも動かない場合は、同じ画面からサイトデータを全削除する。

> シークレットモードで同じ Hub URL を開いて音声入力が動く場合は、通常プロファイルの Chrome 内部状態が原因です。上記手順で復旧します。

設定パネル → **音声入力** の「診断」ボタンで症状の確認とログのコピーができます。

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
| `Ctrl+Shift+G` | 現在のセッションの Git タブを開く |
| `Ctrl+Shift+F` | 現在のセッションの Files タブを開く |
| `Ctrl+V` | 画像を添付エリアにペースト |
| `Ctrl+C` | PTY に SIGINT を送信（テキスト選択中はコピー） |
| `Ctrl+D` | PTY に EOF を送信 |
| `Ctrl+O` | Claude Code の折りたたみ内容を展開 |

---

## 設定ファイル

初回起動時に自動生成されます。

| OS | パス |
|---|---|
| Windows | `%USERPROFILE%\.any-ai-cli\config.yaml` |
| macOS / Linux | `~/.any-ai-cli/config.yaml` |

```yaml
hub:
  port: 47777               # デフォルトポート（衝突時は 47778, 47779... と自動探索）
  open_browser: false       # true にすると serve 起動時にブラウザを自動で開く
  auto_shutdown: true       # 全ラッパーが終了したら Hub も自動停止
  log_dir: ""               # 空 = ~/.any-ai-cli/logs
  idle_timeout_min: 60      # アイドル状態のセッションを自動切断するまでの分数（0 = 無効）

log:                        # hub.log のローテーション設定（lumberjack）
  enabled: true
  max_size_mb: 10           # 1 ファイルの上限サイズ
  max_backups: 3            # 保持するローテーション後ファイル数
  compress: false           # ローテーション後に gzip 圧縮するか

token: ""                   # 空 = 起動時にランダム生成（再起動しても URL は変わらない）
```

`token` をリセットしたい場合は `token:` 行を削除して Hub を再起動してください。

> このほか `approval` / `spawn` / `slash_cmd_sources` / `approval_pattern_sources` / `approval_profiles` / `user_prefs` セクションが UI 操作によって自動追記されることがあります（手書き不要）。

`user_prefs:` には音声入力、通知音、承認時の自動切替、クイックコマンド、利用リンク、お気に入り、セッション順、spawn 既定値などのユーザー機能設定が保存されます。ブラウザの localStorage だけに依存しないため、ポート変更や WSL ランチャー経由でも設定が維持されます。

承認検出パターンは provider ごとに `official` / `custom` プロファイルを持ちます。`official` は GitHub 上の `resources/approval-patterns/{claude,codex,copilot,cursor-agent,common}.md` から起動時に取得・キャッシュされ、`custom` はユーザー編集用です。

---

## 終了パターン

| 操作 | 結果 |
|---|---|
| ブラウザのタブを閉じる | Hub もセッションも継続（一定時間放置すると後述のアイドルタイムアウトで切断） |
| AI CLI が終了する | そのセッションが「完了 / エラー / 異常」状態になる |
| シェルを閉じる | ラッパーが終了 → セッション切断 |
| 全ラッパー終了 | Hub も自動停止（`auto_shutdown: true` のとき） |
| Hub UI の「Hub 停止」ボタン | Hub 停止 |
| `any-ai-cli stop` | Hub 停止 |

### ゾンビセッション対策と Hub クラッシュ耐性

子の AI セッション（Claude Code / Codex CLI / GitHub Copilot CLI / Cursor Agent CLI のプロセス）が宙に浮いたまま走り続けて API 課金が止まらないこと、および Web UI のバグや手動再起動で進行中の AI 作業が道連れになることを避けるため、wrapper 側に「Hub が落ちたら一定時間は復帰を待つ」ロジックが入っています。

WebSocket が切れたとき、wrapper はまず Hub の HTTP エンドポイントを probe して**意図的な切断か Hub クラッシュかを判別**します。

| 切断の種類 | wrapper 側の挙動 |
|---|---|
| **意図的な切断**（Hub UI の `×` で個別 dismiss、すべて停止、idle timeout など）<br>= Hub HTTP が正常応答する | 配下の PTY（claude / codex / copilot / cursor-agent プロセス）を**即座に**終了させて exit。猶予なし |
| **Hub クラッシュ / `.exe` コンソール `×` 閉じ**<br>= Hub HTTP に到達できない | `wrapper_reconnect_grace_sec` 秒間（デフォルト **3600 秒 = 60 分**）、2 秒間隔で Hub への再接続を試みる。<br>　• 復帰したら新しいセッションとして再登録し、PTY をそのまま継続。直近 64KB の出力を UI に再生<br>　• 猶予が切れても復帰しなければ PTY を kill |
| **ブラウザだけ閉じて Hub は生存**（UI 接続が 0 のまま） | Hub 側で `idle_timeout_min` 分（デフォルト 60 分）経過後に全 wrapper を強制切断（→ 上記「意図的な切断」扱いになる） |

> **意図**: Web UI／Hub サーバ側のバグで Hub が落ちた／再起動が必要になった場合に、走行中の AI セッションを猶予時間内に復帰させれば作業が失われないようにするための仕組みです。長時間タスク（数時間スケールの自走）を扱う場合は `wrapper_reconnect_grace_sec` を 12 時間（43200 秒）など長めに設定しておくと安全です。「ブラウザを閉じて忘れる」「dismiss を押す」など**ユーザが明示的に止めた**経路は従来どおり即座にセッションを終了させます。

設定箇所:

- `~/.any-ai-cli/config.yaml`
  - `hub.wrapper_reconnect_grace_sec`: 0 で再接続を無効化（旧来の即 kill 動作）。0–86400 秒（最大 24 時間）。デフォルト 3600 秒 = 60 分（設定パネルからも分単位で変更可。**新しく起動するセッションのみが対象**で、すでに走行中のセッションは spawn 時の値のまま）
  - `hub.idle_timeout_min`: UI 切断後のセーフティネット。0–1440 分、0 で無効化（設定パネルからも変更可）

---

## 画像転送

Hub UI から wrap セッションへ画像ファイルを送信できます。

### 操作手順

1. `any-ai-cli serve` を起動
2. ブラウザで Hub UI を開く
3. セッションカードを選択した状態で、画像を以下のいずれかの方法で送信:
   - **ペースト**: `Ctrl+V`
   - **ドラッグ&ドロップ**: サイドバー下部の枠にドロップ
   - **クリック選択**: 枠をクリックしてファイルダイアログを開く
4. Hub が `~/.any-ai-cli/attachments/<session-id>/` に保存し、PTY へパスを注入
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
| Hub ログ | `~/.any-ai-cli/logs/hub.log` | Hub サーバの動作ログ（lumberjack でローテーション。設定は `log:` セクション参照） |
| セッション生ログ | `~/.any-ai-cli/logs/sessions/<provider>_<YYYY-MM-DD_HHMMSS>_<folder>_s<id>.log` | 各 wrap セッションの PTY 生ログ（ANSI 含む） |
| セッション履歴 | `~/.any-ai-cli/logs/sessions/<provider>_<YYYY-MM-DD_HHMMSS>_<folder>_s<id>.jsonl` | セッションイベント履歴（`session_start` / `user_input` / `pty_output` / `attach` / `session_end` / `session_dismiss`） |
| クリーン transcript | `~/.any-ai-cli/logs/sessions/<provider>_<YYYY-MM-DD_HHMMSS>_<folder>_s<id>.txt` | 人間が読めるテキスト版（ANSI / スピナー / 制御コードを除去）。セッション終了時に自動生成。Hub クラッシュ等で生成漏れがあった場合は次回 `serve` 起動時に補完される |

Hub UI のログパスボタンでログディレクトリのパスをクリップボードにコピーできます。

手動でクリーン transcript を再生成することもできます。

```bash
any-ai-cli log-clean ~/.any-ai-cli/logs/sessions/<session>.jsonl -o transcript.txt
```

---

## トラブルシュート

### spawn 直後にセッションカードが `切断` 表示になる (Windows + pnpm 導入版 CLI)

`pnpm add -g` などのパッケージマネージャで Claude Code / Codex CLI / その他 wrap 対象 CLI を入れている場合、Hub UI からの spawn 直後にカードが `切断` 表示になり、PTY 生ログが 0 バイトのままになることがあります。カードには `理由: codex が PATH に見つかりません` のような短い理由表示も付きます。

Hub は起動時に親シェルの `PATH` スナップショットを継承します。Hub を立ち上げたシェルで `PNPM_HOME` が export されていなかった場合、永続 USER `Path` に書かれた `%PNPM_HOME%\bin` を Windows がプロセス起動時に展開できず、pnpm bin が PATH から事実上脱落します。これにより wrap サブプロセス内の `exec.LookPath("<provider>")` が失敗します。

**回復手順:**

1. `any-ai-cli stop` で Hub を停止
2. `$env:PNPM_HOME` が解決される対話 PowerShell を開く（`$env:PATH -split ';' | Select-String pnpm` で確認）
3. その PowerShell から `any-ai-cli claude` / `any-ai-cli codex` / `any-ai-cli copilot` / `any-ai-cli cursor-agent` のいずれかを実行 — Hub が新しい PATH スナップショットで再生成されます

各 spawn の診断情報は `~/.any-ai-cli/logs/spawn/<provider>-<timestamp>.log` に出力されます（解決後の PATH エントリ数・検出されたパッケージマネージャ一覧・`executable file not found` 検知時の対処ヒントを含む）。

> **v0.2.0 以降:** Hub は spawn 直前に USER `Path` の `%VAR%` 形式エントリを再展開します（`HKCU\Environment` を読み、`%LOCALAPPDATA%\pnpm` が実在する場合はそれを fallback として埋める）。通常はこの手動再起動は不要ですが、再展開でも解決できなかった場合の保険として上記手順を残しています。

---

## セキュリティ

- Hub の HTTP / WebSocket サーバは `127.0.0.1` のみにバインドし、外部ホストから直接アクセスすることはできません
- ランダムトークンを起動時に生成し、URL に付与します（`?token=xxx`）
- `any-ai-cli` 自身はテレメトリ・利用状況の送信を一切行いません

### ローカル instruction file への書き込み

**承認ボタン機能**を有効にすると、`any-ai-cli` は active な wrapped session が読む instruction file に、any-ai-cli のマーカー付き承認ルールブロックだけを追記します。Claude Code は `~/.claude/CLAUDE.md`、Codex / GitHub Copilot / Cursor Agent は project instruction root の `AGENTS.md` が対象です。ブロックは冪等に1つだけ入り、そのファイルを使う最後の active wrapped session が終了した時、承認ボタン機能を無効化した時、または Hub 停止時に削除されます。

### 外部への通信について

`any-ai-cli` 自体はローカル動作を前提としていますが、以下の外部 HTTPS 通信が発生し得ます。

- **スラッシュコマンド一覧の取得（Hub 本体の通信）**: スラッシュコマンドピッカーを開くと、Hub は `https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/{claude,codex,copilot,cursor-agent}.md` を取得し、24 時間キャッシュします。取得元 URL は設定パネルの **スラッシュコマンドソース** から変更可能で、ローカルファイルパスを指定することもできます。
- **承認検出パターンの取得（Hub 本体の通信）**: Hub 起動時に、公式の承認検出パターンを `https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/approval-patterns/{claude,codex,copilot,cursor-agent,common}.md` から取得し、24 時間キャッシュする場合があります。取得元 URL は config で上書きできます。
- **Web Push 通知（Hub 本体の通信 / opt-in のみ）**: プッシュ通知を有効にした場合、Hub は暗号化された Web Push request をブラウザベンダーの push サービスへ HTTPS 送信します。payload には OS 通知表示に必要なセッション ID / 名前、provider、承認質問・文脈の短い抜粋が含まれますが、Hub URL token は含めません。VAPID 鍵と購読情報は `~/.any-ai-cli/push_store.json` にローカル保存されます。SSH トンネルが切れていても通知配送自体は届く場合がありますが、通知から Hub を開くにはトンネルと Hub に到達できる必要があります。
- **音声入力（ブラウザ自身の通信 / 使用時のみ）**: 音声入力はブラウザ内蔵の Web Speech API を使用しており、Chrome / Edge では **マイク音声がブラウザベンダー（Google / Microsoft）の音声認識サーバへ送信されます**。送信するのは `any-ai-cli` ではなくブラウザ自身ですが、コーディング指示には固有名詞や未公開情報が混ざりやすい点に注意してください。これを避けたい場合は音声入力を使わない（マイクボタンを押さない）か、ローカル推論の音声入力ツールを別途利用してください。「音声入力」節の注意書きも参照。
- **wrap 対象 CLI の API 通信（CLI 自身の通信）**: ラップ対象である Claude Code / Codex CLI / GitHub Copilot CLI / Cursor Agent CLI 自身は、それぞれのベンダー API（Anthropic / OpenAI / GitHub / Cursor）と HTTPS で直接通信します。`any-ai-cli` は PTY の入出力をローカル WebSocket で中継するだけで、これらの API 通信を傍受・記録・プロキシすることはありません。元の CLI のネットワーク挙動がそのまま適用されます。

### ⚠️ wrap 対象 CLI のデータ保持について

`any-ai-cli` 自身はユーザーのデータを収集・送信しませんが、**wrap 対象 CLI は送信します**。Hub は PTY の入出力を中継するだけのため、各 CLI のデータ取り扱いポリシーがそのままユーザーに適用されます。立て付けはベンダーごとに異なります。

下表は 2026 年時点の各社方針の概要です。利用前に必ず最新規約を確認してください。

| CLI / バックエンド | デフォルトで学習に使われるか | opt-out / 制御 | 保持期間 |
|---|---|---|---|
| **Claude Code**（Anthropic 商用規約: API / Claude for Work / Enterprise / Education / Gov） | **使われない**（商用規約のデフォルトで除外） | opt-out 不要、エンタープライズ契約で Zero Data Retention 選択可 | API ログ最大 30 日、2025/9/14 以降は **7 日で自動削除** |
| **Codex CLI**（OpenAI: ChatGPT Plus / Pro / Business プラン経由） | **使われる可能性あり**（ChatGPT 個人プラン経由のコンテンツは学習対象になり得る） | プライバシーポータルで「Do not train on my content」、Codex Settings で「環境全体のトレーニング許可」を別途制御 | abuse 監視ログ最大 30 日、ZDR / Modified Abuse Monitoring で除外可 |
| **GitHub Copilot CLI**（GitHub: Product Specific Terms 2026/3 版） | **使われる**（プロンプトは保持され private モデルの fine-tune に利用） | 規約上の明示的な opt-out は不明（最新規約を要確認） | 明示なし |
| **Cursor Agent CLI**（Cursor） | 最新規約を要確認 | 最新規約を要確認 | 最新規約を要確認 |

### ⚠️ 規約変更リスクについて

wrap 対象 CLI のベンダーは、第三者ツール経由のアクセスや自動化を制限する方向に規約を変更する可能性があります。その場合、`any-ai-cli` 経由での利用が規約違反となる場合があります。

- 実例: Google は 2026 年に「Gemini Code Assist を第三者ツール経由で利用することは ToS 違反」とする運用を開始し、OpenClaw / OpenCode / Antigravity 等の wrapper 利用ユーザーに対して `403 ToS` アカウント停止が多発しました。この前例を踏まえ、本ツールでは **Gemini CLI は意図的に wrap 対象外** としています。
- 上表の wrap 対象 CLI についても同様のリスクがあり、ベンダーが第三者自動化を制限した場合は **予告なくサポートを終了する可能性があります**。各 CLI の最新規約はユーザー責任で確認してください。

### ⚠️ アカウントの複数人共有は禁止

`any-ai-cli` をサーバーに設置するなどして、**1 つの AI CLI アカウント（認証情報）を複数人で使い回すことは絶対にしないでください**。各ベンダーの利用規約に明確に違反します。

- **Claude Code（Anthropic）**: Consumer Terms によりアカウントは個人利用が前提で、認証情報（ログイン情報・OAuth トークン）の共有・譲渡は禁止されています。レート制限も個人利用を前提に設計されており、複数人での利用は異常な利用パターンとして検出・アカウント停止（返金なし）の対象になり得ます
- **Codex CLI（OpenAI）**: ChatGPT アカウントの共有は OpenAI の利用規約で同様に禁止されています
- **GitHub Copilot CLI / Cursor Agent CLI**: いずれもシート（個人ライセンス）単位の契約であり、共有は規約違反です

複数人で利用したい場合は、以下の正当な手段を使ってください。

- 各利用者が **自分のアカウントでログイン** する（サーバー上でも OS ユーザー / ホームディレクトリを分離し、各自の認証情報を使う）
- **API キー課金**（Anthropic API 等）に切り替え、組織契約の範囲内で利用する
- **Claude for Work（Team / Enterprise）** 等の組織向けプランでメンバーごとにシートを契約する

`any-ai-cli` 自体にもマルチユーザー対応機能はありません（次節「ローカル実行限定」参照）。

### ⚠️ 重要: ローカル実行限定

`any-ai-cli` はブラウザから **localhost として到達する** ことを前提に設計されています。リモート利用は、SSH ローカルフォワードでこの localhost 前提を保つ場合だけ許容します。以下は絶対に行わないでください。

- リモート Hub を他ホストから直接開ける形で公開する（必ず SSH ローカルフォワードを使ってください）
- `127.0.0.1` 以外のアドレス（`0.0.0.0` / LAN IP 等）にバインドするよう改造する
- Hub UI をリバースプロキシ（nginx / Caddy 等）で外部公開する
- Hub URL（トークン付き）を他人と共有する

Hub UI には「ログフォルダを OS のファイルマネージャで開く」など、ホストマシンに対する操作 API（`/api/open-dir` 等）が含まれます。これらはローカル前提だから安全な設計であり、外部公開すると **任意のフォルダ操作・情報漏洩** につながる可能性があります。

### 外部公開について（サポート対象外・自己責任）

`any-ai-cli` がサポートする構成は、前節の通り localhost 到達のみです。本ソフトウェアは MIT ライセンスで提供されており、リバースプロキシ等を前段に置いて外部公開する構成を技術的に妨げるものではありませんが、外部公開を選択した時点で以下に同意したものとみなします。

- **外部公開はサポート対象外です。** 公開構成に関する質問・不具合報告・セキュリティ相談には一切対応しません
- **Hub への到達は、そのホスト上での任意コマンド実行と同義です。** PTY への直接入力・承認の自動許可・新規セッションの起動がすべて可能であり、侵害された場合の被害は Web UI の乗っ取りではなくホストの乗っ取りに相当します
- 公開する場合、URL トークンのみを防御線とすることは想定されていません。TLS、独立した認証基盤（mTLS / SSO / IP 制限等）、レート制限を含む多段防御を、各技術の意味を理解した上でご自身で設計・運用・維持してください。これらを構成できない場合は公開しないでください
- 外部公開に起因するいかなる損害（ホストの侵害、データ・認証情報・API キーの漏洩、AI CLI アカウントの停止、第三者に生じた損害を含むがこれに限らない）についても、開発者は一切の責任を負いません。「[免責事項](#免責事項)」も併せて参照してください

---

## VPS / Docker 運用（自動更新）

コンテナ運用一式は [`deploy/docker/`](deploy/docker/) にあります（1 ユーザー = 1 コンテナ。Hub の公開は `127.0.0.1` 限定で、SSH トンネル等を介して到達する前提です。前節「ローカル実行限定」の通り、外部公開は行わないでください）。[`deploy/docker/users/example.yaml`](deploy/docker/users/example.yaml) を `users/<user>.yaml` にコピーし、ユーザー名とポートを置き換えてから `compose.yaml` に追加します。

`main` / `develop` への push をトリガーに GitHub Actions（[`docker-image.yml`](.github/workflows/docker-image.yml)）がコンテナイメージをビルドして GHCR へ publish します。サーバー側では一切ビルドしません:

```
ghcr.io/ishizakahiroshi/any-ai-cli:latest      # main 追従（通常運用）
ghcr.io/ishizakahiroshi/any-ai-cli:develop     # develop 追従（検証用）
ghcr.io/ishizakahiroshi/any-ai-cli:sha-<hash>  # コミット単位タグ（ロールバック用）
```

### 常に最新イメージで動かす

[`deploy/docker/aac-update.sh`](deploy/docker/aac-update.sh) を `compose.yaml` と同じディレクトリに置き、日次 cron に登録します。設定中のタグを pull し、**イメージが実際に変わったコンテナだけ** 再作成します（変化なしなら無停止）:

```cron
# root crontab — 毎日 04:30 に実行
30 4 * * * /opt/any-ai-cli/aac-update.sh >> /var/log/aac-update.log 2>&1
```

### 更新による再起動で消えるもの・残るもの

新しいイメージが無い日は cron は完全に何もしません（無停止）。イメージが**変わった**日は該当コンテナが再作成され、Hub が再起動します。各ユーザーへの影響は次のとおりです。

| | 項目 | 理由 |
|---|---|---|
| ❌ 消える | 動作中の AI セッション（claude / codex の PTY プロセス）と Hub UI 上のセッションカード | プロセスはコンテナと共に終了 |
| ✅ 残る | Hub のアクセストークン（`~/.any-ai-cli/config.yaml`） | ホーム volume で永続化 — **tunnel モードのランチャープロファイルはそのまま使い続けられる** |
| ✅ 残る | AI CLI のログイン状態（Claude 認証など） | 同上（ホーム配下） |
| ✅ 残る | 作業中のリポジトリ・ファイル | work ディレクトリを bind mount |
| ✅ 残る | セッションログ（`~/.any-ai-cli/logs/`） | 同上（ホーム配下） |
| △ 復元可 | AI との会話履歴 | provider CLI がホーム配下に履歴を保持。新しいセッションで `--resume` 系の再開が可能 |

終了は猶予付きの正常終了です（`stop_grace_period: 40s` + entrypoint が wrapper の終了を最大 20 秒待機）。

運用ティップス（特にマルチユーザー運用 — cron は**全ユーザー**のコンテナを一斉に再作成します）:

- **cron の時刻は慎重に選ぶ。** 深夜に長時間 AI タスクを走らせるユーザーがいると 04:30 で切られる可能性があります。誰も作業しない時間帯を選び、全ユーザーに周知してください。
- **大事な実行の前は凍結する。** `touch /opt/any-ai-cli/HOLD` で更新をスキップ（全ユーザー対象）。終わったら `rm HOLD` で再開。
- **タグ選択が再起動頻度を決める。** `AAC_TAG=develop` は develop への push のたびに再起動、`latest` は `main` へのリリース時のみ。

### 開発中のバイパス

イメージタグは compose プロジェクトの `.env` の `AAC_TAG` で切り替えます（未設定なら `latest`）。`compose.yaml` と同じディレクトリに `HOLD` ファイルを置くと自動更新 cron が凍結されます。

| モード | `.env` | 自動更新 cron |
|---|---|---|
| 通常運用（`main` 追従） | `AAC_TAG=latest` または未設定 | 動作する |
| `develop` 追従 | `AAC_TAG=develop` | 動作する（`develop` の最新を追従） |
| サーバー上でローカルビルド | `AAC_TAG=dev` | `touch HOLD` で凍結する |

ローカルビルドの例（GitHub を経由せずに変更を試したいとき）:

```bash
cd /opt/any-ai-cli
touch HOLD                            # 自動更新 cron を凍結
docker build -t ghcr.io/ishizakahiroshi/any-ai-cli:dev \
  -f src/deploy/docker/Dockerfile src # src/ = このリポジトリの checkout
# .env に AAC_TAG=dev を設定してから:
docker compose up -d

# 通常運用へ戻す:
# .env を AAC_TAG=latest に戻してから:
docker compose up -d && rm HOLD
```

---

## ライセンス

MIT

第三者依存の通知は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)、vendored/browser 側のライセンス本文は [web/src/vendor/THIRD_PARTY_LICENSES.txt](web/src/vendor/THIRD_PARTY_LICENSES.txt) に記載しています。

---

## 関連性について（非公式）

`any-ai-cli` は第三者によるコミュニティメンテナンスのツールです。**Anthropic / OpenAI / GitHub / Cursor / Ollama のいずれによっても公認・公式サポートされていません**。「Claude」「Claude Code」「Codex」「ChatGPT」「GitHub Copilot」「Cursor」「Cursor Agent」「Ollama」「Gemini」等の名称・商標は各社の所有物であり、本プロジェクトでは説明・相互運用の目的でのみ言及しています。

---

## 免責事項

本ツールはいかなる保証もなく「現状のまま」提供されます。利用にあたってはご自身の責任において行ってください。
