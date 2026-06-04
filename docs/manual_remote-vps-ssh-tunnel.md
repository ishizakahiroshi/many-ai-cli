# リモート VPS SSH トンネル運用手順

> 最終更新: 2026-06-04(木) 12:59:29

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

## 手順

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
