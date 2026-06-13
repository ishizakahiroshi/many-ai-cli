# any-ai-cli Docker マルチユーザー運用 manual（admin リモートサーバー）

> 最終更新: 2026-06-04(木) 22:44:40

XServer リモートサーバー（サーバー名 `admin`）上で「ユーザー 1 人 = Docker コンテナ 1 つ」の分離方式で `any-ai-cli` を複数人運用するための手順書。設計の経緯・決定ログは [plan_docker-multiuser-isolation.md](plan_docker-multiuser-isolation.md) を参照。

**接続情報（サーバー IP・秘密鍵パス・token 等）の正本は管理者ローカルの認証情報ファイル（Git 管理外）に置く。本書には実値を書かない。**

## 全体像

```text
メンバーの PC ブラウザ
  -> http://127.0.0.1:478NN/?token=<本人専用 token>
  -> SSH local forward（本人の OS アカウント + 公開鍵）
  -> リモートサーバー host 127.0.0.1:478NN
  -> (docker publish) コンテナ aac-<user> の :48000
  -> (socat 中継) コンテナ内 127.0.0.1:478NN = Hub
```

- Hub はコンテナ内で **本人の割当ポート 478NN** で listen する（Hub の Host/Origin 検証がポート完全一致を要求するため、経路上のすべてのポート番号を 478NN に揃える。コンテナ内 socat の受け口 48000 だけは内部固定値）
- host 側 publish は `127.0.0.1` 限定。リモートサーバーのグローバル IP には一切露出しない。外からの経路は SSH トンネルのみ
- token はコンテナ初回起動時に自動生成され、`aac-home-<user>` volume 内に永続化される

## サーバー側レイアウト

```text
/opt/any-ai-cli/                 # root 所有・管理者のみ操作
├─ compose.yaml                  # include で users/*.yaml を束ねる
├─ users/<user>.yaml             # ユーザー別サービス定義（ポート・volume・専用ネットワーク）
├─ templates/sshd-member.conf.template   # メンバー用 sshd 設定の雛形
├─ assign.md                     # ユーザー ⇔ ポート ⇔ volume 割当表（本書の表と同期）
└─ src/                          # リポジトリソース（イメージビルド用。deploy/docker/ が正本）

/srv/any-ai-cli/work/<user>/     # 作業リポジトリ置き場（uid 1000 所有で bind mount）
named volume: aac-home-<user>    # コンテナ内ホーム（AI CLI 認証・~/.any-ai-cli）
```

リポジトリ側の正本は `deploy/docker/`（Dockerfile / entrypoint.sh / compose.yaml / users/*.yaml / sshd-member.conf.template）。サーバーの `/opt/any-ai-cli/compose.yaml`・`users/`・`templates/` は配置コピーであり、変更時はリポジトリ側を直してから再配置する。

設定の要点（entrypoint が初回起動時に config.yaml へ事前生成）:

- `idle_timeout_min: 0` — 既定 60 分の「UI 切断後に全 PTY セッションを kill」を無効化（リモートはトンネル切断が常態のため。UI 設定から変更可能）
- `auto_shutdown: false` / `open_browser: false` — Hub 常駐・headless 前提
- コンテナはユーザー専用ネットワーク（`aac-net-<user>`）に置き、コンテナ間の直接到達を遮断
- `memswap_limit` を `mem_limit` と同値にし、host swap の奪い合いを防止（超過時は当該コンテナのみ OOM）

## 割当表

| ユーザー | ポート | コンテナ | volume | work dir | SSH 権限 |
|---|---|---|---|---|---|
| admin | 47801 | aac-admin | aac-home-admin | /srv/any-ai-cli/work/admin | 管理者（通常シェル + docker グループ） |

- ポートは 47801 から連番で採番する
- 変更したら `/opt/any-ai-cli/assign.md` と本書の両方を更新する

## メンバー追加手順（管理者作業）

前提: 新メンバーから SSH 公開鍵（`ssh-ed25519 ...` など 1 行）を受け取っていること。秘密鍵は受け取らない。

以下 `<user>` = 新メンバー名、`<port>` = 割当表で採番した次のポート（478NN）。

### 1. OS アカウント作成（トンネル専用）

```bash
adduser --disabled-password --gecos "" <user>
install -d -m 700 -o <user> -g <user> /home/<user>/.ssh
echo '<公開鍵 1 行>' > /home/<user>/.ssh/authorized_keys
chmod 600 /home/<user>/.ssh/authorized_keys
chown <user>:<user> /home/<user>/.ssh/authorized_keys
```

docker グループ・sudo は **付与しない**。

### 2. sshd トンネル専用設定の適用

```bash
sed -e 's/__USER__/<user>/g' -e 's/__PORT__/<port>/g' \
  /opt/any-ai-cli/templates/sshd-member.conf.template \
  > /etc/ssh/sshd_config.d/aac-member-<user>.conf
sshd -t                      # 構文エラーがないこと（エラー時は適用しない）
systemctl reload ssh
```

これでメンバーは「シェルなし・自分のポートのみ local forward 可」になる（`ForceCommand /usr/sbin/nologin` + `PermitOpen 127.0.0.1:<port>`）。

### 3. コンテナ定義の追加と起動

```bash
# work dir（コンテナ内 uid 1000 = ubuntu に合わせて chown）
mkdir -p /srv/any-ai-cli/work/<user>
chown 1000:1000 /srv/any-ai-cli/work/<user>

# ユーザー定義（admin.yaml をテンプレートに複製・置換。
# service 名・volume 名・専用ネットワーク名・ポートがまとめて置換される）
cd /opt/any-ai-cli
sed -e 's/admin/<user>/g' -e 's/47801/<port>/g' users/admin.yaml > users/<user>.yaml

# compose.yaml の include に追記してから構文検証 → 起動
sed -i 's|^\(  - users/admin.yaml\)$|\1\n  - users/<user>.yaml|' compose.yaml
docker compose config --quiet && docker compose up -d aac-<user>
```

> リポジトリ側 `deploy/docker/users/` にも同じ `<user>.yaml` を追加して同期させること。

### 4. AI CLI 初回ログイン（本人のアカウントで）

メンバー本人はリモートサーバーに入れないため、**コマンドは管理者のシェルで・認可はメンバー本人のブラウザで**という分担にする。チャットで URL / コードを往復するだけで済み、画面共有もメンバーへのリモートサーバーアクセス付与も不要。

```bash
docker exec -it aac-<user> bash   # 以降このシェルで（管理者が実行）
```

**claude（コード貼り戻し方式）**

1. 管理者: `command claude` → ログインを選択 → 表示された URL をメンバーへチャットで送る
2. メンバー: 自分のブラウザで開き、自分の Anthropic アカウントで認可 → 表示されたコードを管理者へ返送
3. 管理者: ターミナルへ貼り付け → 完了

**codex（device code 方式・`--device-auth` 必須）**

1. 管理者: `codex login --device-auth` → URL（`https://auth.openai.com/codex/device`）とワンタイムコード（15 分有効）をメンバーへ送る
2. メンバー: 自分のブラウザで開いてコード入力・自分の OpenAI アカウントで認可 → ターミナル側が自動で完了を検知（コードの返送は不要）
3. 管理者: `codex login status` で「Logged in」を確認

**copilot（GitHub device flow）**

1. 管理者: `command copilot` → 信頼確認（Yes）→ `/login` → 表示された device code と `https://github.com/login/device` をメンバーへ送る
2. メンバー: 自分のブラウザで開いてコード入力・自分の GitHub アカウント（Copilot サブスクリプション必須）で認可 → ターミナル側が自動で完了を検知

**cursor-agent（URL ポーリング方式）**

1. 管理者: `NO_OPEN_BROWSER=1 command cursor-agent login` → 表示された URL（`https://cursor.com/loginDeepControl?...`）をメンバーへ送る
2. メンバー: 自分のブラウザで開き、自分の Cursor アカウントで承認 → CLI が自動で完了を検知（localhost コールバックなし。コンテナで完結する）

注意:

- `command claude` の `command` は透過 wrap（シェル関数）を回避する指定。ログインに wrap は不要で、VT 対応の弱い端末（旧 conhost 等）だと wrap 経由で画面が出ないことがある（2026-06-04 実例）
- codex で素の `codex login` を使うとコンテナ内 `localhost:1455` でコールバックを待ち、ブラウザのリダイレクトは認可した PC の localhost へ行くため**必ず失敗する**（2026-06-04 実例）。誤って始めてしまった場合は、ブラウザに表示された失敗 URL のクエリを中継すれば救済できる: `docker exec aac-<user> curl "http://127.0.0.1:1455/auth/callback?code=...&scope=...&state=..."`（`codex login` のプロセスが待機中のうちに実行）
- **信頼モデル**: 完了後の OAuth トークンは `aac-home-<user>` volume に保存され、管理者は技術的にアクセス可能な位置にある。メンバー追加時に「管理者を信頼する運用」であることを一言伝えること

認証情報は `aac-home-<user>` volume に保存され、コンテナ再起動後も保持される。

### 5. token の確認と本人への引き渡し

```bash
docker exec aac-<user> grep '^token:' /home/ubuntu/.any-ai-cli/config.yaml
# または起動 banner から:
docker logs aac-<user> 2>&1 | grep -m1 'Open:'
```

本人へ伝えるもの: ①割当ポート `<port>` ②token（URL 形式 `http://127.0.0.1:<port>/?token=...` で渡すと楽）。
**token は他人に共有しない**よう本人に伝える（漏れた場合は管理者が volume 内 config.yaml の `token:` を削除してコンテナ再起動 → 再生成）。

## メンバーの接続手順（各自の PC）

割当ポート `<port>`・token は管理者から受け取る。

```powershell
# Windows (PowerShell) — つなぎっぱなしにするウィンドウで実行
ssh -N -L <port>:127.0.0.1:<port> <user>@<server-ip> -i <受領した秘密鍵のパス>
```

```bash
# macOS / Linux
ssh -N -L <port>:127.0.0.1:<port> <user>@<server-ip> -i <秘密鍵パス>
```

ブラウザで `http://127.0.0.1:<port>/?token=<token>` を開く。

- **左右のポートは必ず同じ番号**にする（Hub の Host 検証がポート一致を要求）
- 接続時にシェルは開けない（`-N` 必須。シェルを試みても nologin で切断される）
- 他人のポートへの forward は `PermitOpen` で拒否される

## 管理者向け運用

すべて `admin`（docker グループ）または root で、`/opt/any-ai-cli` で実行。

| 操作 | コマンド |
|---|---|
| 全コンテナ起動 | `docker compose up -d` |
| 個別再起動 | `docker compose restart aac-<user>`（即死ループに入った場合は下記トラブルシュート） |
| 停止 | `docker compose stop aac-<user>` |
| ログ確認 | `docker logs -f aac-<user>` |
| リソース実測 | `docker stats` |
| コンテナ内シェル | `docker exec -it aac-<user> bash` |
| イメージ更新 | 手元からソース再転送（下記）→ `docker compose build && docker compose up -d` |
| メンバー削除 | `docker compose stop aac-<user> && docker compose rm -f aac-<user>` → `docker volume rm any-ai-cli_aac-home-<user>` → users/<user>.yaml と include 行を削除 → 本人のトンネル切断後 `userdel -r <user>` → `/etc/ssh/sshd_config.d/aac-member-<user>.conf` 削除 → `sshd -t && systemctl reload ssh` → `/srv/any-ai-cli/work/<user>` 退避または削除 → assign.md 更新 |

> ⚠️ 引数なしの `docker compose down` は **全ユーザーのコンテナを停止・削除** する。個別操作は必ずサービス名を付けるか stop/rm を使う。

### 停止手順

通常停止は `docker compose stop aac-<user>` を使う。可能なら先に Hub UI で当該ユーザーのセッションを終了してから停止する。

停止時は Docker から entrypoint へ SIGTERM が届き、entrypoint が `any-ai-cli wrap` / `any-ai-cli claude` / `any-ai-cli codex` / `any-ai-cli copilot` / `any-ai-cli cursor-agent` の wrapper 群へ SIGTERM を送る。wrapper は既存実装で子プロセスの AI CLI へ SIGTERM を転送するため、ファイル書き込みや git 操作が途中の場合でも通常の終了猶予を得られる。

猶予は entrypoint 側で wrapper 最大 20 秒、compose 側でコンテナ全体 `stop_grace_period: 40s`。40 秒を超えると Docker が SIGKILL するため、緊急時以外は `docker kill` や `docker compose stop -t 0` を使わない。停止確認は `docker logs aac-<user>` で `sending TERM to wrapper processes` → `wrapper processes exited` → `sending TERM to Hub` → `Hub exited` の順に出ているかを見る。

ソース再転送（手元 PC のリポジトリルートから）:

```bash
# コミット済みの安定状態（HEAD）を転送する。working tree を tar すると
# 並行編集中（別 AI セッション・エディタ）の中途半端なファイルを拾って
# ビルドが壊れることがある（2026-06-04 に実際に発生）
git archive HEAD | gzip | \
  ssh root@<server-ip> 'rm -rf /opt/any-ai-cli/src && mkdir -p /opt/any-ai-cli/src && tar -xzf - -C /opt/any-ai-cli/src'

# deploy/ が未コミットの間だけ、作業ツリーから上書き転送する（コミット後は不要）。
# entrypoint.sh の CR 除去は git archive の autocrlf 変換対策として常に実行してよい
tar -czf - deploy | \
  ssh root@<server-ip> "tar -xzf - -C /opt/any-ai-cli/src && sed -i 's/\r\$//' /opt/any-ai-cli/src/deploy/docker/entrypoint.sh"
```

### provider CLI のバージョン管理ポリシー

**バージョンは Dockerfile で pin 固定し、更新はイメージ再ビルドの一斉ロールアウトだけで行う。コンテナ内での self-update / 手動 update（`claude update` / `copilot update` 等）は使わない。**

更新手順: Dockerfile の pin を上げる → 再ビルド → **承認 action-bar の検出が壊れていないか確認**（検出は CLI の画面文字列に依存するため、CLI の UI 変更で壊れうる）→ `docker compose up -d` で全コンテナ再作成（volume は維持されるためログインは無傷）。

コンテナ内更新を禁止する理由:

- イメージ内 CLI は root 所有（/usr/lib, /opt）のため非 root の更新は大半失敗するが、self-updater が `~/.local` 側へインストールするタイプだと **volume に残留して PATH を奪い、以後イメージを更新してもそのユーザーだけ古い自前版が使われ続ける**（亡霊バージョン）
- ユーザー間でバージョンが揃わなくなり、不具合の再現性が失われる
- 防御済み: claude は `DISABLE_AUTOUPDATER=1`、npm 3 種 + cursor-agent は root 所有領域で非 root から更新不可
- cursor-agent は公式インストーラではなく versioned package URL を Dockerfile の `CURSOR_AGENT_VERSION` で直接取得する。`cursor-agent --version` が pin と合わない場合は build を fail させる
- 万一「亡霊バージョン」を疑う場合: `docker exec aac-<user> sh -c 'which claude; claude --version'` でパスと版数を確認し、`~/.local/bin` 配下なら削除する

## セキュリティ注意

- **Hub 系ポート（478NN）を XServer パケットフィルターで開けない**。開けてよいのは SSH(22) のみ。host 側 publish が `127.0.0.1` 限定のため開けても直接は露出しないが、多層防御として閉じておく（XServer 管理パネルで要確認）
- メンバーはトンネル専用（`ForceCommand /usr/sbin/nologin` + `PermitOpen 127.0.0.1:<自ポート>` + `PermitListen none`）。シェル・SFTP・他ポート到達は不可
- メンバーはサーバー上での作業が一切できないため、コンテナ再起動・work dir のホスト側操作はすべて管理者経由
- 各コンテナの token は本人だけが知る。URL（token 付き）をチャット等に貼らない
- AI CLI のログイン情報はコンテナ別 volume に閉じており、他メンバーからは到達不能（コンテナ境界 + ポート分離 + token の三層）

## サーバー初期整備の記録（C1 実施内容・再構築用）

2026-06-04 実施。まっさらな Ubuntu 24.04（root のみ・Docker/Node なし）からの手順。

```bash
# 1. Docker Engine + compose plugin（公式 apt リポジトリ）
apt-get update && apt-get install -y ca-certificates curl
install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu noble stable" > /etc/apt/sources.list.d/docker.list
apt-get update && apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
docker run --rm hello-world

# 2. 管理者ユーザー admin（既存の管理者秘密鍵 <admin-key>.pem の公開鍵を使い回し）
adduser --disabled-password --gecos "" admin
usermod -aG docker admin
install -d -m 700 -o admin -g admin /home/admin/.ssh
# ssh-keygen -y -f <admin-key>.pem の出力を authorized_keys へ
chmod 600 /home/admin/.ssh/authorized_keys && chown admin:admin /home/admin/.ssh/authorized_keys

# 3. ディレクトリ規約
mkdir -p /opt/any-ai-cli/users /opt/any-ai-cli/templates /srv/any-ai-cli/work/admin
chown admin:admin /srv/any-ai-cli/work/admin   # uid 1000（コンテナ内 ubuntu と一致）

# 4. sshd メンバー用テンプレート → /opt/any-ai-cli/templates/sshd-member.conf.template
#    （本書「メンバー追加手順 2」参照。実メンバー追加時に適用する）
```

確認済みの初期状態: listen は 22 のみ / swap 2 GiB 有効 / `unattended-upgrades` enabled / ufw 非アクティブ（フィルターは XServer パネル側で管理）。

導入時バージョン: Docker 29.5.3 / Docker Compose v5.1.4 / イメージ: Ubuntu 24.04 + Node.js 22 + `@anthropic-ai/claude-code` 2.1.162 + `@openai/codex` 0.137.0 + `@github/copilot` 1.0.59 + cursor-agent 2026.06.03-0bbb28e（versioned package URL から `/opt/cursor-agent` へ配置 — ホーム配下は volume で覆われるため）。CLI の更新はすべてイメージ再ビルドで行う。

## トラブルシュート

| 症状 | 確認・対処 |
|---|---|
| ブラウザで「アクセスできません」 | SSH トンネルが生きているか / 左右ポートが同一か / コンテナが Up か（`docker ps`） |
| 401 が返る | token の打ち間違い・別ユーザーの token を使っている |
| 403 / API が失敗する | URL のポートと割当ポートの不一致（Host/Origin 検証はポート完全一致を要求） |
| UI の「フォルダを開く」「ピッカー」が反応しない | コンテナ内（headless）には GUI 実行系がなく非対応。パスは手入力する。フォルダの新規作成は Files タブの「新規フォルダ」（📁+）で可能 |
| トンネルが張れない | `PermitOpen` の対象ポートか（自分の割当ポート以外は拒否される）/ 公開鍵が登録されているか |
| コンテナが再起動を繰り返す | `docker logs aac-<user>`。`HUB_PORT` 未設定だと entrypoint が即終了する |
| restart 後に exit 137 の即死ループ（ログ出力なし・0.2 秒で die を繰り返す） | docker 側の単発レース（2026-06-04 に 1 回観測、Docker 29.5.3。再現条件不明・通常の restart は 0.5 秒で正常）。**復旧: `docker compose up -d --force-recreate aac-<user>`**。volume は維持されるため token・認証情報は失われない |
| メモリ逼迫 | `docker stats` で実測 → `users/<user>.yaml` の `mem_limit` を調整（収容人数の目安は plan の C5 結果参照） |
