# AI エージェント設定タスク: リモートサーバー（単一サーバー / pnpm 版）

> 最終更新: 2026-06-14(日) 21:09:45 — 禁止事項に Tailscale Funnel 公開禁止を追記

このファイルは **そのまま AI エージェント（手元 PC の Claude Code / Codex CLI 等）に貼り付けて使う設定タスク指示書** です。エージェントは手元 PC で動き、SSH 越しにリモートサーバーを設定し、最後に手元の launcher プロファイルを作成します。Docker は使いません（リモートに pnpm で `many-ai-cli` を入れ、`serve` で Hub を起動する単一サーバー構成）。`serve` は使うときだけ起動しても、systemd / tmux / nohup 等で常駐させてもよい（常駐は Docker 版だけの役割ではない）。

使い方:

1. 下の「変数」を自分の値で埋める（または貼り付け時にエージェントへ口頭で渡す）。
2. このファイルの内容を丸ごと AI エージェントに貼り付け、「この手順を実行して」と指示する。
3. エージェントが SSH 接続・導入・検証・プロファイル作成まで自動で行う。

---

## エージェントへの指示（ここから下をそのまま実行させる）

あなたは手元 PC 上で動く AI エージェントです。以下の手順で、リモートサーバーに `many-ai-cli` を pnpm で導入し、手元 PC から SSH トンネルで Hub を開けるようにしてください。**破壊的操作・外部公開は行わないこと。** 各ステップは verify を満たしてから次へ進み、満たさなければ原因を報告して停止してください。

### 変数（実行前に確定する）

| 変数 | 説明 | 例 |
|---|---|---|
| `SSH_TARGET` | SSH 接続先（`user@host` または `~/.ssh/config` のエイリアス） | `your-user@remote.example.com` |
| `SSH_KEY` | 秘密鍵パス（config で指定済みなら空でよい） | `C:\dev\.ssh\id_ed25519` |
| `HUB_PORT` | Hub のポート（手元・リモートで同番号にする） | `47777` |
| `REMOTE_CWD` | リモートでセッションを動かす作業ディレクトリ | `~/work` |
| `PROVIDERS` | 入れる provider CLI | `claude codex` |
| `PROFILE_NAME` | 手元 launcher プロファイル名 | `my-remote` |

> 値が未確定なら、ここで 1 度だけまとめてユーザーに確認してから進む。

### 前提チェック（C1）

SSH 疎通と OS を確認する。

```bash
ssh SSH_TARGET 'uname -a && echo OK-SSH'
```

verify: `OK-SSH` が返る。返らなければ接続情報を見直して停止。

### pnpm / Node の用意（C2）

リモートに pnpm が無ければ入れ、Node も pnpm 管理に切り替える。**冪等**に書く（既に入っていれば何もしない）。

```bash
ssh SSH_TARGET 'bash -lc "
  set -e
  command -v pnpm >/dev/null || curl -fsSL https://get.pnpm.io/install.sh | sh -
  export PNPM_HOME=\"\$HOME/.local/share/pnpm\"; export PATH=\"\$PNPM_HOME:\$PATH\"
  command -v node >/dev/null || pnpm env use --global lts
  echo PNPM=\$(pnpm -v) NODE=\$(node -v)
"'
```

verify: `PNPM=...` と `NODE=...` が両方表示される。

> 注意: pnpm インストーラは `~/.bashrc` に PATH を追記する。以降のステップは `bash -lc`（ログインシェル）で実行し、PATH が読まれるようにする。

### many-ai-cli と provider CLI の導入（C3）

```bash
ssh SSH_TARGET 'bash -lc "
  set -e
  pnpm add -g many-ai-cli
  pnpm add -g @anthropic-ai/claude-code @openai/codex @github/copilot
  many-ai-cli --version
"'
```

`PROVIDERS` に応じて不要なパッケージは省いてよい。`cursor-agent` は pnpm では入らないため、必要な場合のみ公式インストーラを別途案内する（このタスクでは扱わない）。

verify: `many-ai-cli --version` がバージョンを表示する。

> provider CLI のログイン（`claude` / `codex login` 等）は対話・本人認可が必要なため、このタスクでは行わない。導入後にユーザー本人がリモート shell または Hub UI から実行する旨を報告する。

### 起動テストとセキュリティ確認（C4）

Hub を一時起動して loopback 限定で listen しているか確認し、すぐ止める。

```bash
ssh SSH_TARGET 'bash -lc "
  set -e
  mkdir -p REMOTE_CWD
  cd REMOTE_CWD
  nohup many-ai-cli serve --port HUB_PORT >/tmp/aac-serve.log 2>&1 &
  sleep 4
  echo === listen ===; ss -ltnp | grep :HUB_PORT || true
  echo === banner ===; grep -m1 Open: /tmp/aac-serve.log || true
  many-ai-cli stop || pkill -f \"many-ai-cli serve\" || true
"'
```

verify（両方を満たすこと。満たさなければ停止して報告）:

- listen 行が `127.0.0.1:HUB_PORT` のみであること。`0.0.0.0:HUB_PORT` や public IP が出たら**設定を中止し警告**する。
- banner に `Open: http://127.0.0.1:HUB_PORT/?token=...` が出ること。

### 手元 launcher プロファイルの作成（C5）

手元 PC の `~/.many-ai-cli/launcher-profiles.yaml`（Windows: `%USERPROFILE%\.many-ai-cli\launcher-profiles.yaml`）に serve モードのプロファイルを**追記**する（既存 `profiles:` があれば配列に足す。無ければ新規作成）。

```yaml
version: 1
profiles:
  - name: PROFILE_NAME
    type: ssh
    mode: serve
    host: <SSH_TARGET の host 部>
    user: <SSH_TARGET の user 部>
    identity_file: <SSH_KEY の値。空なら行ごと省略>
    hub_port: HUB_PORT
    cwd: REMOTE_CWD
```

このプロファイルの値はすべて、エージェントが SSH 接続時に既に把握している（host/user は `SSH_TARGET`、port は `HUB_PORT`、cwd は `REMOTE_CWD`）。ユーザーへ確認が要るのは実質 **鍵（`SSH_KEY`）だけ**：

- `SSH_KEY` を指定されていれば `identity_file:` にその値をそのまま書く。
- `SSH_KEY` が空（ssh-agent / `~/.ssh/config` の鍵指定に任せる運用）なら `identity_file:` 行ごと省略する。
- `SSH_TARGET` がエイリアスなら `host:` にエイリアスを書き `user:` は省略してよい（鍵も config 側に任せられる）。
- `host:` は **手元から到達できる名前**（VPS の DNS 名 / 公開 IP / config エイリアス）にする。サーバー上でしか見えない LAN/コンテナ内 IP は書かない。

verify: YAML が妥当で、既存プロファイルを壊していないこと（追記前にバックアップを取る）。

### 起動確認（C6）

```powershell
many-ai-cli-launcher.exe --profile PROFILE_NAME
```

verify: 既定ブラウザで `http://127.0.0.1:HUB_PORT/?token=...` が開き、Hub UI が表示される。env_kind badge が `リモートサーバー` 系であること。

> `many-ai-cli-launcher.exe` が PATH に無い場合は、リリース zip から取り出して PATH に置くようユーザーに案内する（README「Launcher」節）。

### 完了報告（C7）

以下を 1 つにまとめて報告する:

- 導入した `many-ai-cli` / provider のバージョン
- リモートで listen が `127.0.0.1:HUB_PORT` のみだった確認結果
- 作成した launcher プロファイル名とファイルパス
- **ユーザーが次に手で行うこと**: provider CLI のログイン（本人認可）、Hub UI からのセッション起動

## やってはいけないこと（厳守）

- `0.0.0.0` / public IP / LAN IP での bind、`ssh -g` / `ssh -R` / `GatewayPorts`、リバースプロキシ公開
- **Tailscale Funnel での公開**（全世界公開で方針に反する。スマホ接続は `tailscale serve` の tailnet 内限定 HTTPS か SSH トンネルで行う）
- token 付き URL をログ・チャットへ貼る、token をファイルにコミットする
- リモートの既存ファイル・既存 SSH 設定・他プロファイルの破壊的変更
- provider のログイン情報を勝手に操作する

## 運用形態（都度起動 / 常駐）

C4 の一時起動はあくまで listen 確認用ですが、本番運用では (a) 使うときだけ `serve` を起動する、(b) systemd service 化 / tmux・screen で残す / nohup で常駐させる、のどちらも選べます。常駐は Docker 版だけの役割ではなく、単一サーバー構成でも `serve` を立てっぱなしにできます。(b) の常駐 Hub には、launcher を tunnel モードのプロファイルに切り替えれば、毎回 `serve` を起動し直さずに再接続できます。

### 常駐 Hub に tunnel で繋ぐ場合のプロファイル例（物理1台・Docker 不使用）

物理サーバーで `serve` を常駐させ、launcher を `serve` → `tunnel` に切り替える場合の例です。**トークンの値は控えず**、SSH 越しに `config.yaml` から取り出すコマンドを `token_command` に設定します（Docker 版と違い `docker exec` でラップしない素の形）。

```yaml
profiles:
  - name: PROFILE_NAME
    type: ssh
    mode: tunnel
    host: <SSH_TARGET の host 部>
    user: <SSH_TARGET の user 部>
    identity_file: <SSH_KEY の値。空なら行ごと省略>
    hub_port: HUB_PORT          # serve --port と同番号・固定（tunnel は auto 不可）
    token_command: "grep ^token ~/.many-ai-cli/config.yaml | cut -d' ' -f2"
```

注意:

- `serve` を一度も起動していないとリモートの `~/.many-ai-cli/config.yaml` にトークンが無く、`token_command` が空を返して接続に失敗します。**先に最低1回 `serve` を起動**してトークンを生成しておくこと。
- `serve` を `--port HUB_PORT` 固定で常駐させる（tunnel モードは `hub_port` の auto=0 を許さないため、ポートを固定する）。

## 関連

- 入口（どれを入れて、どの手順を使うか）: [manual_remote-server-overview.md](manual_remote-server-overview.md)
- 仕組み・トラブル対処: [manual_remote-server-ssh-tunnel.md](manual_remote-server-ssh-tunnel.md)
- Docker 版の設定タスク: [manual_remote-server-agent-docker.md](manual_remote-server-agent-docker.md)
