# リモートサーバー SSH トンネル運用手順

> 最終更新: 2026-06-14(日) 00:15:54 — 読みやすさ改善・pnpm 導入を反映・旧名 any-ai-cli を many-ai-cli に統一

## このドキュメントは何か

リモートサーバー上で動く `many-ai-cli` の Hub を、**手元 PC のブラウザから SSH トンネル越しに安全に開く**ための手順です。

`many-ai-cli` の Hub は設計上ローカル専用で、`127.0.0.1` にしか bind しません。だから「リモートサーバーで動いている Hub」を手元から見るには、SSH のローカルフォワード（`-L`）で *手元の `127.0.0.1:<port>`* と *リモートの `127.0.0.1:<port>`* を 1 対 1 でつなぎます。グローバル IP 公開・`0.0.0.0` bind・nginx / Caddy / Cloudflare Tunnel などのリバースプロキシは**一切使いません**。

対象は「SSH でログインできる自分専用のリモートサーバーに、手元 PC から一時的につなぐ確認用途」だけです。

## 一番大事なルール（先に結論）

**経路上のポート番号は、手元側もリモート側も全部同じにする。**

Hub は `Host` / `Origin` を `127.0.0.1:<Hub のポート>`（または `localhost:<Hub のポート>`）で検証します。手元とリモートでポートがズレると、HTML は開けても WebSocket / API が `host not allowed` で落ちます。`-L` は必ず左右同番号で書いてください。

```text
手元 PC のブラウザ
  http://127.0.0.1:47777/?token=<token>
        │
        │ ssh -L 127.0.0.1:47777:127.0.0.1:47777   ← 左右が同じ 47777
        ▼
リモートサーバー上の many-ai-cli Hub
  http://127.0.0.1:47777/?token=<token>
```

## 最短手順（まずこれを試す）

1. **リモートに many-ai-cli を入れる（pnpm）**
   ```bash
   curl -fsSL https://get.pnpm.io/install.sh | sh -   # pnpm が無ければ
   pnpm env use --global lts                          # Node が無ければ
   pnpm add -g many-ai-cli
   many-ai-cli --version
   ```
2. **リモートで Hub を起動**
   ```bash
   cd ~/work && many-ai-cli serve --port 47777
   ```
   起動ログの `Open: http://127.0.0.1:47777/?token=<token>` から **ポートと token を控える**。
3. **手元 PC でトンネルを張る**（PowerShell の例）
   ```powershell
   ssh -N -T -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 `
     -L 127.0.0.1:47777:127.0.0.1:47777 user@remote.example.com
   ```
4. **手元のブラウザで開く**: `http://127.0.0.1:47777/?token=<token>`

> 毎回手で張るのが面倒なら、後述の **launcher による自動接続**（プロファイル 1 つで 1〜4 を自動化）を使ってください。AI エージェントに丸ごと設定させたい場合は文末の「関連」にある設定用 manual を参照。

## 前提

- リモートサーバーに SSH ログインできる
- リモートに `many-ai-cli` と、使う provider CLI（`claude` / `codex` / `copilot` / `cursor-agent`）が入っている
- リモートの firewall / security group で Hub ポートを外部公開しない（開けるのは SSH のみ）
- 手元 PC に OpenSSH client がある
- Hub URL の `?token=...` は誰にも共有しない

## launcher による自動接続（推奨）

`many-ai-cli-launcher`（Windows / Linux / macOS 対応。Windows は `many-ai-cli-launcher.exe`）を使うと、SSH トンネル確立・Hub 起動・ブラウザ起動を 1 ステップで自動化できます。SSH の `serve` / `tunnel` プロファイルは全 OS で動作します（`wsl` プロファイルのみ Windows 専用）。下の「手動手順」は launcher が使えないときの代替です。

プロファイルは `~/.many-ai-cli/launcher-profiles.yaml` に書きます。接続方法は 2 モード:

| モード | 使う場面 |
|---|---|
| `serve` | 接続のたびにリモートで `many-ai-cli serve` を起動する（常駐させない単一サーバー） |
| `tunnel` | すでに常駐している Hub（Docker compose など）にトンネルだけ張る |

### serve モード（接続時にリモートで Hub を起動）

```yaml
version: 1
profiles:
  - name: my-remote
    type: ssh
    mode: serve           # 接続時に many-ai-cli serve をリモートで起動
    host: remote.example.com
    user: your-user
    hub_port: 47777       # 0 にするとポート自動選択
    cwd: /home/your-user/projects
```

`host` は `~/.ssh/config` のホストエイリアスでも、`user@host` 形式でも可。

起動:

```powershell
many-ai-cli-launcher.exe --profile my-remote
```

launcher の動作:

1. `ssh.exe -t -L 47777:127.0.0.1:47777 your-user@remote.example.com -- bash -ilc "many-ai-cli serve --port 47777"` を起動
2. リモートの起動バナーから Hub URL を検出（ポート衝突時は +100 刻みで最大 5 回再試行）
3. Windows の既定ブラウザで Hub UI を開く
4. Ctrl+C または終了時にリモートの serve プロセスを cleanup

### tunnel モード（常駐 Hub にトンネルだけ張る）

リモートで Docker compose 構成が常駐している（`restart: unless-stopped`・固定ポート）場合に使います。リモートでの `serve` 起動はせず、トンネルを張って常駐 Hub につなぎます。

```yaml
version: 1
profiles:
  - name: remote-docker
    type: ssh
    mode: tunnel          # 常駐 Hub へのトンネルのみ
    host: remote.example.com
    user: your-user
    hub_port: 47801       # 常駐 Hub が listen しているポートと同番号
    token_command: "docker exec aac-user1 sh -c 'grep ^token ~/.many-ai-cli/config.yaml | cut -d\" \" -f2'"
```

`token_command` はリモートで実行され、その標準出力（トリム済み）が token になります。コンテナ名・config.yaml パスは実環境に合わせてください。

起動:

```powershell
many-ai-cli-launcher.exe --profile remote-docker
```

launcher の動作:

1. `ssh.exe -N -L 47801:127.0.0.1:47801 your-user@remote.example.com` でトンネル確立
2. `token_command` を SSH 経由で実行して token 取得
3. `/api/info?token=<token>` で疎通確認
4. `/api/net-hint` に `ssh=true`・`host_label`・`env_kind=remote-tunnel` を登録
5. 既定ブラウザで `http://127.0.0.1:47801/?token=<token>&via=ssh&host_label=<host>&env_kind=remote-tunnel` を開く
6. Ctrl+C でトンネルを閉じる（リモート Hub は止めない）

### 複数プロファイルがある場合

```powershell
many-ai-cli-launcher.exe           # 2 件以上あれば選択画面を表示
many-ai-cli-launcher.exe --last    # 前回のプロファイルで接続
many-ai-cli-launcher.exe --ui      # 常に選択画面を表示
```

### 環境識別表示（操作対象を取り違えないために）

Hub UI は `/api/info` の `env_kind` で、ブラウザタブ title・favicon・ヘッダー badge・Settings/About 表示を切り替えます。複数 Hub を同時に開くときは、タブの色と badge で操作対象を確認してください。

| env_kind | 表示 | favicon | 主なケース |
|---|---|---|---|
| `local` | Local | 緑 + `L` | 手元 PC のローカル Hub |
| `wsl` | WSL | 青 + `W` | WSL Hub / Windows launcher 経由の WSL Hub |
| `remote` | リモートサーバー | オレンジ + `R` | SSH で起動したリモートサーバー上の Hub |
| `remote-tunnel` | リモートサーバー（トンネル） | 赤 + `T` | tunnel モードで常駐 Hub に接続する場合 |

URL が `127.0.0.1` でも、`リモートサーバー（トンネル）` 表示なら、操作対象の filesystem / Git / ログは**すべてリモート側**です。

---

## 手動手順（launcher が使えないとき）

### 1. リモートに many-ai-cli を入れる（pnpm）

```bash
curl -fsSL https://get.pnpm.io/install.sh | sh -   # pnpm が無ければ
pnpm env use --global lts                          # Node が無ければ
pnpm add -g many-ai-cli
many-ai-cli --version
```

使う provider CLI もリモートに入れてログインしておく（セッションはリモートで動くため）。多くは npm パッケージなので pnpm で入る:

```bash
pnpm add -g @anthropic-ai/claude-code @openai/codex @github/copilot
# cursor-agent は公式インストーラ（pnpm では入らない・任意）
```

### 2. リモートで Hub を起動

```bash
cd ~/work
many-ai-cli serve --port 47777
```

起動ログの URL からポートと token を控える:

```text
Open: http://127.0.0.1:47777/?token=<token>
```

ポート衝突で `47778` 以降に移った場合は、以降の `47777` を実際のポートに読み替える。常駐させたいなら `tmux` / `screen` / `systemd` を使う。

### 3. 手元 PC からトンネルを張る

PowerShell:

```powershell
ssh -N -T `
  -o ExitOnForwardFailure=yes `
  -o ServerAliveInterval=30 `
  -L 127.0.0.1:47777:127.0.0.1:47777 `
  user@remote.example.com
```

Git Bash / WSL / Linux / macOS:

```bash
ssh -N -T \
  -o ExitOnForwardFailure=yes \
  -o ServerAliveInterval=30 \
  -L 127.0.0.1:47777:127.0.0.1:47777 \
  user@remote.example.com
```

`user@remote.example.com` は実際の接続先に置き換える。

### 4. 手元ブラウザで開く

```text
http://127.0.0.1:47777/?token=<token>
```

`localhost` でも動くが、混乱を避けるため `127.0.0.1` に統一する。

### 5. セッションを起動

Hub UI の spawn から provider を起動するか、リモートの別 SSH shell で `many-ai-cli claude` / `many-ai-cli codex` などを起動する。操作対象の filesystem・Git・ログ・添付保存先は**すべてリモート側**になる（手元 PC のファイルではない）。

### 6. 終了

1. Hub UI のセッション
2. SSH トンネルの端末（`Ctrl+C`）
3. リモートの Hub（`Ctrl+C` または別 shell で `many-ai-cli stop`）

## ポートを変える場合

手元側ポートとリモート Hub ポートは**必ず同番号**にする。例: `47777` が手元で使用中なら、リモート Hub も別の同じ番号で起動する。

```bash
many-ai-cli serve --port 47877
```

```powershell
ssh -N -T -o ExitOnForwardFailure=yes `
  -L 127.0.0.1:47877:127.0.0.1:47877 user@remote.example.com
```

```text
http://127.0.0.1:47877/?token=<token>
```

`-L 127.0.0.1:47778:127.0.0.1:47777` のように左右でポートを変えると、HTML は取れても WebSocket / POST API が `host not allowed` / `origin not allowed` で失敗する。

## 禁止事項

- リモートの `0.0.0.0:<port>` / public IP / LAN IP で Hub を listen させる
- `ssh -L 0.0.0.0:<port>:...` や `ssh -g` で手元側トンネルを LAN 公開する
- `GatewayPorts yes` + `ssh -R` でリモート側に公開ポートを作る
- nginx / Caddy / Cloudflare Tunnel / Tailscale Funnel などで Hub UI を公開する
- token 付き URL をチャット・Issue・ログ・スクリーンショットで共有する
- 複数人で同じ Hub を同時操作する

## トラブル切り分け

| 症状 | 主な原因 | 対処 |
|---|---|---|
| `ssh: bind: Address already in use` | 手元の同ポートが使用中 | リモート Hub も別の同番号で起動し、同番号でトンネルを張る |
| HTML は開くがターミナルが繋がらない | 手元 / リモートのポート不一致 | `-L 127.0.0.1:<p>:127.0.0.1:<p>` の形に直す |
| `host not allowed` / `origin not allowed` | URL が public host・別ポート・proxy 経由 | `http://127.0.0.1:<Hub port>/?token=...` で開く |
| `404` / `403` / 空白 | token 誤り・古い token・Hub 再起動後の URL | リモートの起動ログから最新 token を使う |
| 操作対象が想定と違う | Hub はリモートで動いている | cwd・Git branch・ログパスがリモート側か確認 |
| トンネルが途中で切れる | SSH 接続断 | `ServerAliveInterval=30` を付け、必要なら再接続 |

## セキュリティ確認

作業前後にリモートで listen を確認する:

```bash
ss -ltnp | grep ':47777'
```

`47777` は実際の Hub ポートに置き換える。期待値は `127.0.0.1:<Hub port>` の listen のみ。`0.0.0.0:<Hub port>` や public IP で listen していたら止める。

リモートの firewall / cloud security group で Hub ポートを inbound 許可しない。接続は SSH（`22/tcp` または運用中の SSH ポート）だけで行う。

## 関連

- AI エージェントに丸ごと設定させたい場合（手元 PC の AI に貼り付けて SSH 越しに設定）:
  - 単一サーバー版（pnpm）: [manual_remote-server-agent-single.md](manual_remote-server-agent-single.md)
  - Docker 版: [manual_remote-server-agent-docker.md](manual_remote-server-agent-docker.md)
- Docker マルチユーザー運用の詳細: [manual_docker-multiuser.md](manual_docker-multiuser.md)
- [v0.2.0 設計書: Security / Privacy](v0.2.0-any-ai-cli-design.md#17-security--privacy)
- [README.ja.md: セキュリティ](../README.ja.md#セキュリティ)
