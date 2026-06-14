# AI エージェント設定タスク: リモートサーバー（Docker / マルチユーザー・隔離向け）

> 最終更新: 2026-06-14(日) 21:09:45 — 禁止事項に Tailscale Funnel 公開禁止を追記・コンテナ内 serve 不可とモバイル接続経路を注記

このファイルは **そのまま AI エージェント（手元 PC の Claude Code / Codex CLI 等）に貼り付けて使う設定タスク指示書** です。エージェントは手元 PC で動き、SSH 越しにリモートサーバーへ `many-ai-cli` の Docker compose 構成を配置し、自動再起動付きで常駐起動し、最後に手元の launcher プロファイル（tunnel モード）を作成します。

単一サーバー版（pnpm + `serve`）との違い:

| | 単一サーバー版 | Docker 版（このファイル） |
|---|---|---|
| リモート導入 | pnpm でグローバル install | GHCR のイメージを compose で pull |
| Hub の起動 | `serve` を手動 or systemd/tmux 等で起動（都度・常駐どちらも可） | `restart: unless-stopped` で自動再起動付き常駐 |
| launcher モード | `serve` | `tunnel`（トンネルだけ張る） |
| 向く用途 | 手軽に始めたい・単一ユーザー・隔離不要（常駐も可） | 複数ユーザーを隔離・自動再起動/自動更新を仕組み化したい |

※ `serve` モードでもリモート側 `serve` を常駐させ、`tunnel` モードで繋ぐ運用は可能。モード=常駐可否ではない。

使い方:

1. 下の「変数」を埋める。
2. このファイルを丸ごと AI エージェントに貼り付け、「この手順を実行して」と指示する。
3. エージェントが配置・起動・検証・プロファイル作成まで自動で行う。

リポジトリ側の正本は `deploy/docker/`（Dockerfile / entrypoint.sh / compose.yaml / users/*.yaml）。マルチユーザー運用の詳細手順は [manual_docker-multiuser.md](manual_docker-multiuser.md) を参照（このタスクは「1 ユーザー分を立てて手元からつなぐ」最小構成）。

---

## エージェントへの指示（ここから下をそのまま実行させる）

あなたは手元 PC 上で動く AI エージェントです。以下の手順で、リモートサーバーに Docker で `many-ai-cli` Hub を 1 つ常駐させ、手元 PC から SSH トンネルで開けるようにしてください。**破壊的操作・外部公開は行わないこと。** 各ステップは verify を満たしてから次へ進み、満たさなければ原因を報告して停止してください。

### 変数（実行前に確定する）

| 変数 | 説明 | 例 |
|---|---|---|
| `SSH_TARGET` | SSH 接続先（`user@host` またはエイリアス）。docker を叩ける権限が要る | `your-user@remote.example.com` |
| `SSH_KEY` | 秘密鍵パス（config で指定済みなら空でよい） | `C:\dev\.ssh\id_ed25519` |
| `USER_TAG` | サービス/コンテナ/volume を区別する名前 | `user1` |
| `HUB_PORT` | host 側 publish ポート（手元と同番号にする） | `47801` |
| `AAC_TAG` | 追従するイメージタグ（`latest`=main / `develop`=検証） | `latest` |
| `REMOTE_BASE` | サーバー側 compose 配置先 | `/opt/many-ai-cli` |
| `WORK_DIR` | コンテナにマウントする作業ディレクトリ（host 側） | `/srv/many-ai-cli/work/user1` |
| `PROFILE_NAME` | 手元 launcher プロファイル名 | `remote-docker` |

> 値が未確定なら、ここで 1 度だけまとめてユーザーに確認してから進む。
> コンテナ内では Hub が `HUB_PORT` で listen し、socat の受け口 `48000`（内部固定値）経由で host 側に `127.0.0.1:HUB_PORT:48000` として publish される設計。Hub の Host/Origin 検証がポート完全一致を要求するため、`HUB_PORT`（env・host publish・手元側トンネル）は必ず全部同番号にする。

### 前提チェック（C1）

SSH 疎通・OS・Docker / compose の有無を確認する。

```bash
ssh SSH_TARGET 'bash -lc "uname -a; docker version --format \"{{.Server.Version}}\"; docker compose version" && echo OK-PREREQ'
```

verify: `OK-PREREQ` と Docker / compose のバージョンが出る。Docker が無ければ**ここで停止**し、導入方法（[manual_docker-multiuser.md](manual_docker-multiuser.md) の「サーバー初期整備の記録」）をユーザーに案内する（このタスクでは Docker 自体の導入は行わない）。

### compose 構成の配置（C2）

リポジトリの `deploy/docker/` をリモートの `REMOTE_BASE/src` へ転送し、`REMOTE_BASE` に `compose.yaml` / `.env` / `users/USER_TAG.yaml` を用意する。**既存ファイルがあれば上書きせず差分を確認**する。

手元 PC のリポジトリルートから（コミット済み HEAD を転送）:

```bash
git archive HEAD deploy | ssh SSH_TARGET 'bash -lc "
  set -e
  mkdir -p REMOTE_BASE/src REMOTE_BASE/users WORK_DIR
  tar -xf - -C REMOTE_BASE/src
  echo OK-SYNC
"'
```

`.env`（追従タグ）と user 定義を生成する。user 定義は `deploy/docker/users/example.yaml` をテンプレートに、`example`→`USER_TAG`・`47801`→`HUB_PORT` を置換して作る:

```bash
ssh SSH_TARGET 'bash -lc "
  set -e
  cd REMOTE_BASE
  printf \"AAC_TAG=AAC_TAG\n\" > .env
  sed -e \"s/example/USER_TAG/g\" -e \"s/47801/HUB_PORT/g\" \
    src/deploy/docker/users/example.yaml > users/USER_TAG.yaml
  # work dir をコンテナ内 uid 1000 (ubuntu) に合わせる
  chown 1000:1000 WORK_DIR || sudo chown 1000:1000 WORK_DIR
  echo OK-RENDER
"'
```

`compose.yaml` の `include` に `users/USER_TAG.yaml` を含める（無ければ `deploy/docker/compose.yaml` を雛形に作成）。`name: many-ai-cli` と `include:` を持つ最小形:

```yaml
name: many-ai-cli
include:
  - users/USER_TAG.yaml
```

verify: `ssh SSH_TARGET 'cd REMOTE_BASE && docker compose config --quiet && echo OK-CONFIG'` が `OK-CONFIG` を返す（compose の構文・参照が妥当）。

### イメージ取得と起動（C3）

```bash
ssh SSH_TARGET 'bash -lc "
  set -e
  cd REMOTE_BASE
  docker compose pull --quiet aac-USER_TAG
  docker compose up -d aac-USER_TAG
  sleep 5
  docker ps --filter name=aac-USER_TAG --format \"{{.Names}} {{.Status}}\"
"'
```

verify: `aac-USER_TAG` が `Up`（やがて `healthy`）。healthcheck は 60s 間隔なので `healthy` 確定まで 1〜2 分かかることがある。`Restarting` ループなら `docker logs aac-USER_TAG` を確認して停止（`HUB_PORT` 未設定で entrypoint 即終了が典型）。

### セキュリティ確認（C4）

host 側 publish が loopback 限定であることを確認する。

```bash
ssh SSH_TARGET 'bash -lc "ss -ltnp | grep :HUB_PORT || true"'
```

verify: `127.0.0.1:HUB_PORT` のみ。`0.0.0.0:HUB_PORT` や public IP が出たら**設定を中止し警告**する（compose の `ports:` が `127.0.0.1:HUB_PORT:48000` になっているか確認）。あわせて firewall / security group で `HUB_PORT` を inbound 許可していないことを確認するようユーザーに促す（開けてよいのは SSH のみ）。

### token の取得（C5）

token はコンテナ初回起動時に自動生成され volume に永続化される。

```bash
ssh SSH_TARGET 'bash -lc "docker exec aac-USER_TAG sh -c \"grep ^token: /home/ubuntu/.many-ai-cli/config.yaml\""'
```

verify: `token: ...` が取れる。**token はログ・チャットに残さない**（launcher は接続のたびに `token_command` で取り直すので、ここで控えた値はファイルに保存しない）。

> **モバイル接続（📱）の経路**: コンテナ内 Hub には `tailscale` CLI が無いため `tailscale serve` は使えない。スマホから繋ぐ場合は **SSH トンネル / launcher 経由**（モバイル接続ウィザードは degrade して SSH トンネルへ誘導する）。詳細は [local/plan_mobile-connect-flow-redesign.md](local/plan_mobile-connect-flow-redesign.md)。
> **token モデルの差異**: launcher は接続のたびに `token_command` で token を取り直し**ファイルに保存しない**。一方モバイル接続（📱）は token を QR でスマホ端末に**永続化**する（紛失対策は別プラン `plan_remote-auth-hardening-future.md` の領分）。モバイル接続をリモート Hub 上で使う場合も当該 Hub 自身の token を読むため、上記の `token_command` 運用と矛盾しない。

### 手元 launcher プロファイルの作成（C6）

手元 PC の `~/.many-ai-cli/launcher-profiles.yaml`（Windows: `%USERPROFILE%\.many-ai-cli\launcher-profiles.yaml`）に **tunnel モード**のプロファイルを追記する（バックアップを取ってから）。

```yaml
version: 1
profiles:
  - name: PROFILE_NAME
    type: ssh
    mode: tunnel
    host: <SSH_TARGET の host 部>
    user: <SSH_TARGET の user 部>
    identity_file: <SSH_KEY の値。空なら行ごと省略>
    hub_port: HUB_PORT
    token_command: "docker exec aac-USER_TAG sh -c 'grep ^token ~/.many-ai-cli/config.yaml | cut -d\" \" -f2'"
```

このプロファイルの値はエージェントが SSH 接続時に把握済み（host/user は `SSH_TARGET`、port は `HUB_PORT`）で、ユーザー確認が要るのは実質 **鍵（`SSH_KEY`）だけ**：

- `SSH_KEY` を指定されていれば `identity_file:` にその値をそのまま書く。空（ssh-agent / config 任せ）なら行ごと省略。
- `host:` は **SSH 接続先（VPS の DNS 名 / 公開 IP / config エイリアス）** にする。tunnel モードはこの host へ SSH してコンテナの `127.0.0.1:48000` を転送するため、**コンテナ内 IP は書かない**（手元からは到達できない）。
- `SSH_TARGET` がエイリアスなら `host:` にエイリアスを書き `user:` は省略してよい。

verify: YAML が妥当で、既存プロファイルを壊していないこと。

### 起動確認（C7）

```powershell
many-ai-cli-launcher.exe --profile PROFILE_NAME
```

verify: 既定ブラウザで `http://127.0.0.1:HUB_PORT/?token=...&env_kind=remote-tunnel` が開き、Hub UI が表示される。badge が `リモートサーバー（トンネル）`（赤 + `T`）であること。

> `many-ai-cli-launcher.exe` が PATH に無い場合はリリース zip から取り出して PATH に置くようユーザーに案内する。

### 完了報告（C8）

以下を 1 つにまとめて報告する:

- 配置した `REMOTE_BASE` のパスと、起動した `aac-USER_TAG` の status（`healthy` 化を待っているなら明記）
- host 側 listen が `127.0.0.1:HUB_PORT` のみだった確認結果
- 作成した launcher プロファイル名とファイルパス
- **ユーザーが次に手で行うこと**: provider CLI のログイン（`docker exec -it aac-USER_TAG bash` 内で本人認可。手順は [manual_docker-multiuser.md](manual_docker-multiuser.md) の「AI CLI 初回ログイン」）、必要なら追従タグ更新（`docker compose pull && docker compose up -d`）

## やってはいけないこと（厳守）

- `ports:` を `0.0.0.0:...` / public IP にする、firewall で Hub ポートを開ける、リバースプロキシ公開
- **Tailscale Funnel での公開**（全世界公開で方針に反する。スマホ接続は SSH トンネル / launcher 経由で行う）
- 引数なしの `docker compose down`（**全ユーザーのコンテナを巻き込む**）。個別操作は必ずサービス名を付けるか `stop` / `rm` を使う
- token 付き URL や token をログ・チャット・コミットに残す
- リモートの既存 compose / 他ユーザー定義 / volume の破壊的変更
- コンテナ内で provider CLI を self-update する（バージョンはイメージ再ビルドで揃える方針）

## 関連

- 入口（どれを入れて、どの手順を使うか）: [manual_remote-server-overview.md](manual_remote-server-overview.md)
- 仕組み・トラブル対処: [manual_remote-server-ssh-tunnel.md](manual_remote-server-ssh-tunnel.md)
- Docker マルチユーザー運用の正本: [manual_docker-multiuser.md](manual_docker-multiuser.md)
- 単一サーバー版（pnpm）の設定タスク: [manual_remote-server-agent-single.md](manual_remote-server-agent-single.md)
