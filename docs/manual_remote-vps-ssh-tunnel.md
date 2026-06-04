# リモート VPS SSH トンネル運用手順

> 最終更新: 2026-06-04(木) 23:18:51

## 位置づけ

このドキュメントは、VPS 上で動く `any-ai-cli` Hub を、手元 PC のブラウザから SSH local forwarding 経由で確認するための手順メモ。

`any-ai-cli` の公開仕様はローカル実行限定であり、Hub は `127.0.0.1` にのみ bind する。VPS の public IP、`0.0.0.0` bind、nginx / Caddy / Cloudflare Tunnel などの reverse proxy、トークン付き URL の共有は対象外。

この手順で許容するのは、SSH でログインできる自分専用の VPS に対して、手元 PC から一時的に `127.0.0.1:<port>` 同士をつなぐ確認用途のみ。

## 接続イメージ

```text
手元 PC のブラウザ
  http://127.0.0.1:47777/?token=<remote-token>
        |
        | ssh -L 127.0.0.1:47777:127.0.0.1:47777
        |
VPS 上の any-ai-cli Hub
  http://127.0.0.1:47777/?token=<remote-token>
```

Hub 側の `Host` / `Origin` 検証は `127.0.0.1:<Hub port>` または `localhost:<Hub port>` を前提にしている。そのため、local forwarding のローカル側ポートと VPS 側 Hub ポートは同じ番号にする。

## 前提

- VPS に SSH ログインできること
- VPS 上に `any-ai-cli` と wrap 対象 CLI（`claude` / `codex` / `copilot` / `cursor-agent`）が導入済みであること
- VPS の firewall / security group で Hub ポートを外部公開しないこと
- 手元 PC に OpenSSH client があること
- Hub URL の `?token=...` を他人に共有しないこと

## launcher での自動接続

`any-ai-cli-launcher.exe`（Windows 専用）を使うと、VPS への SSH トンネル確立・Hub 起動・ブラウザ起動を 1 ステップで自動化できます。

下記の手動手順はトラブル時の代替として残しています。通常は launcher を使ってください。

### serve モード（VPS 上で Hub を都度起動する場合）

`~/.any-ai-cli/launcher-profiles.yaml` に以下を追記します。

```yaml
version: 1
profiles:
  - name: my-vps
    type: ssh
    mode: serve           # 接続時に any-ai-cli serve をリモートで起動する
    host: vps.example.com
    user: your-user
    hub_port: 47777       # 0 にするとポートを自動選択
    cwd: /home/your-user/projects
```

`host` は `~/.ssh/config` のホストエイリアスも使えます（`user` の代わりに `user@host` 形式も可）。

起動:

```powershell
any-ai-cli-launcher.exe --profile my-vps
```

launcher は以下を自動で行います。

1. `ssh.exe -t -L 47777:127.0.0.1:47777 your-user@vps.example.com -- bash -ilc "any-ai-cli serve --port 47777"` を起動
2. リモートの起動バナーから Hub URL を検出（ポートが衝突した場合は +100 刻みで最大 5 回再試行）
3. Windows の既定ブラウザで Hub UI を開く
4. Ctrl+C または終了時にリモートの serve プロセスを cleanup

### tunnel モード（VPS 上で Docker 常駐 Hub を使う場合）

VPS で `deploy/docker/` の compose 構成が常駐している（`restart: unless-stopped`・固定ポート）場合は `mode: tunnel` を使います。リモート側での `serve` 起動は行わず、トンネルを確立して常駐 Hub に接続します。

```yaml
version: 1
profiles:
  - name: vps-docker
    type: ssh
    mode: tunnel          # 常駐 Hub へのトンネルのみ確立する
    host: vps.example.com
    user: your-user
    hub_port: 47801       # 常駐 Hub が listen しているポートと同じ番号にする
    token_command: "docker exec any-ai-cli-user1 sh -c 'grep ^token ~/.any-ai-cli/config.yaml | cut -d\" \" -f2'"
```

`token_command` はリモートで実行され、その標準出力（トリム済み）がトークンとして使われます。Docker コンテナ名と config.yaml のパスは実際の環境に合わせてください。

起動:

```powershell
any-ai-cli-launcher.exe --profile vps-docker
```

launcher は以下を自動で行います。

1. `ssh.exe -N -L 47801:127.0.0.1:47801 your-user@vps.example.com` でトンネルを確立
2. `token_command` を SSH 経由でリモート実行してトークンを取得
3. `/api/info?token=<token>` で Hub の疎通を確認
4. Windows の既定ブラウザで `http://127.0.0.1:47801/?token=<token>` を開く
5. Ctrl+C でトンネルを閉じる（リモート Hub は停止しない）

#### tunnel モードのポートについて

Hub の `Host` / `Origin` 検証は `127.0.0.1:<Hub port>` 同士の一致を前提にしています。そのため `hub_port` の値はローカル側とリモート側で必ず同じ番号にしてください。

#### プロファイル一覧が複数ある場合

プロファイルが 2 件以上ある場合は、起動時に接続先選択画面がブラウザで開きます。

```powershell
any-ai-cli-launcher.exe           # 複数あれば選択画面を表示
any-ai-cli-launcher.exe --last    # 前回使ったプロファイルで接続
any-ai-cli-launcher.exe --ui      # 常に選択画面を表示
```

---

## 手動手順（トラブル時の代替）

以下の手順は、launcher が使えない場合や接続の仕組みを確認したい場合の代替です。

### 1. VPS 側で Hub を起動する

VPS に SSH ログインし、Hub を明示ポートで起動する。

```bash
any-ai-cli serve --port 47777
```

起動ログに表示される URL から、実際のポートと token を控える。

```text
Open: http://127.0.0.1:47777/?token=<remote-token>
```

ポート衝突で `47778` 以降に移った場合は、以降の `47777` を実際に表示されたポートへ置き換える。

別 SSH shell から `any-ai-cli claude` / `any-ai-cli codex` などの wrapper command を使う場合は、Hub を起動したポートと設定上の Hub port が一致している前提になる。ポート衝突で自動移動した場合は、Hub UI の spawn を使うか、空いているポートを選んで明示起動し直す。

### 2. 手元 PC から SSH tunnel を張る

PowerShell:

```powershell
ssh -N -T `
  -o ExitOnForwardFailure=yes `
  -o ServerAliveInterval=30 `
  -L 127.0.0.1:47777:127.0.0.1:47777 `
  user@vps.example.com
```

Git Bash / WSL / Linux / macOS:

```bash
ssh -N -T \
  -o ExitOnForwardFailure=yes \
  -o ServerAliveInterval=30 \
  -L 127.0.0.1:47777:127.0.0.1:47777 \
  user@vps.example.com
```

`user@vps.example.com` は実際の SSH 接続先に置き換える。

### 3. 手元ブラウザで開く

手元 PC のブラウザで、VPS 側の起動ログに出た token を付けて開く。

```text
http://127.0.0.1:47777/?token=<remote-token>
```

`localhost` でも動くが、同じ確認中は `127.0.0.1` に統一する。

### 4. セッションを起動する

Hub UI の spawn から provider を起動するか、VPS 側の別 SSH shell で `any-ai-cli claude` / `any-ai-cli codex` などを起動する。

この場合、操作対象の filesystem、Git repository、ログ、attach 保存先はすべて VPS 側になる。手元 PC のファイルを操作しているわけではない。

### 5. 終了する

作業後は以下を順に閉じる。

1. Hub UI のセッション
2. SSH tunnel terminal（`Ctrl+C`）
3. VPS 側 Hub（`Ctrl+C` または別 shell から `any-ai-cli stop`）

## ポートを変える場合

local port と remote Hub port は同じ番号にする。

例: `47777` が手元 PC で使用中なら、VPS 側 Hub も同じ番号で起動する。

```bash
any-ai-cli serve --port 47877
```

PowerShell:

```powershell
ssh -N -T `
  -o ExitOnForwardFailure=yes `
  -L 127.0.0.1:47877:127.0.0.1:47877 `
  user@vps.example.com
```

ブラウザ:

```text
http://127.0.0.1:47877/?token=<remote-token>
```

`127.0.0.1:47778:127.0.0.1:47777` のように local / remote で別ポートにすると、HTML は取れても WebSocket や POST API が `host not allowed` / `origin not allowed` で失敗する。

## 禁止事項

- VPS の `0.0.0.0:<port>` / public IP / LAN IP で Hub を listen させる
- `ssh -L 0.0.0.0:<port>:...` や `ssh -g` で手元 PC 側の tunnel を LAN 公開する
- `GatewayPorts yes` と `ssh -R` を組み合わせて VPS 側に公開 port を作る
- nginx / Caddy / Cloudflare Tunnel / Tailscale Funnel などで Hub UI を公開する
- token 付き URL をチャット、Issue、ログ、スクリーンショットで共有する
- 複数人で同じ Hub を同時操作する

## トラブル切り分け

| 症状 | 主な原因 | 対処 |
|---|---|---|
| `ssh: bind: Address already in use` | 手元 PC の同ポートが使用中 | VPS 側 Hub も同じ別ポートで起動し、同じ番号で tunnel を張る |
| HTML は開くがターミナルが接続しない | local / remote の port 不一致 | `-L 127.0.0.1:<p>:127.0.0.1:<p>` の形に直す |
| `host not allowed` / `origin not allowed` | ブラウザ URL が public host、別ポート、または proxy 経由 | `http://127.0.0.1:<Hub port>/?token=...` で開く |
| `404` / `403` / blank | token 誤り、古い token、Hub 再起動後の URL 使用 | VPS 側の起動ログから最新 token を使う |
| 操作対象が想定と違う | Hub は VPS 上で動いている | cwd、Git branch、ログパスが VPS 側であることを確認する |
| tunnel が途中で切れる | SSH 接続断 | `ServerAliveInterval=30` を付け、必要なら再接続する |

## セキュリティ確認

作業前後に以下を確認する。

```bash
ss -ltnp | grep ':47777'
```

`47777` は実際の Hub port に置き換える。期待値は `127.0.0.1:<Hub port>` の listen のみ。`0.0.0.0:<Hub port>` や VPS の public IP で listen している場合は停止する。

VPS の firewall / cloud security group で `47777` などの Hub ポートを inbound 許可しない。SSH の `22/tcp` または運用中の SSH port だけで接続する。

## 関連

- [v0.2.0 設計書: Security / Privacy](v0.2.0-any-ai-cli-design.md#17-security--privacy)
- [README.ja.md: セキュリティ](../README.ja.md#セキュリティ)
