# many-ai-cli ビルド・配布・デプロイ

> 最終更新: 2026-06-13(土) 14:00:52

`many-ai-cli` は **Go 単一バイナリ + go:embed フロント** の構成。サーバーへのデプロイは無し（ユーザー PC にバイナリを置くだけ）。

設計書: [../docs/v0.2.0-any-ai-cli-design.md §4・§17](../docs/v0.2.0-any-ai-cli-design.md)

## ビルド前提

- Go 1.22+
- Node.js 20+（ビルドスクリプト `scripts/build.mjs` と `node --test` の実行用）
- Bun 1.3+（フロント `web/` の依存取得・スクリプト起動用。npm は使わない）
- フロントは事前に `web/dist/` をビルドし、Go の `//go:embed web/dist` で同梱

### フロントビルド

```bash
cd web
bun install         # bun.lock に固定された devDependencies を取得
bun run build       # web/dist/ が生成される
cd ..
```

依存の postinstall スクリプトはデフォルトでブロックされる（許可制 `trustedDependencies`）。
`web/bunfig.toml` の `minimumReleaseAge` で公開直後バージョンの取得も遅延させている。
CI / goreleaser では `bun install --frozen-lockfile` を使い lockfile との一致を強制する。

`web/dist/` が `internal/hub/` の `embed.FS` に取り込まれる前提で実装すること。
`git archive HEAD` などでソーススナップショットを展開した場合も、archive には `web/dist/` が含まれないため、展開後に必ず同じフロントビルドを実行してから Go ビルドする。

### Go バイナリビルド（クロスコンパイル）

```bash
# Windows (x86_64)
GOOS=windows GOARCH=amd64 go build -o dist/win/many-ai-cli.exe ./cmd/many-ai-cli

# macOS (Intel)
GOOS=darwin GOARCH=amd64 go build -o dist/mac/many-ai-cli ./cmd/many-ai-cli

# macOS (Apple Silicon)
GOOS=darwin GOARCH=arm64 go build -o dist/mac-arm/many-ai-cli ./cmd/many-ai-cli

# Linux (x86_64)
GOOS=linux GOARCH=amd64 go build -o dist/linux/many-ai-cli ./cmd/many-ai-cli
```

### CGO の扱い

- 標準は `CGO_ENABLED=0`（純 Go ビルド）を目指す
- PTY ライブラリが CGO 必須の場合は OS 別に build tag で分離し、Linux/Mac は CGO 有効・Windows は ConPTY API を使う純 Go 実装に倒す方針
- 詳細は実装時に決める。決まったら本ドキュメントに追記

### Windows での開発フロー

**ローカルビルドは原則 `make build` を使う**。`go build` を素で叩くと `go-winres` がスキップされて、`cmd/many-ai-cli/rsrc_windows_*.syso`（アプリアイコン等の Windows リソース）が古いまま `dist/many-ai-cli.exe` に embed される。

#### Makefile ターゲット一覧（v0.2.0 時点）

種類が増えてきたのでここに集約する。**他所に分散させない**。

| ターゲット | 出力物 / 動作 | 使う場面 |
|---|---|---|
| `make build` | 下 4 つ（windows + launcher + linux + deploy-wsl）を順に実行 | 通常はこれ 1 本。Windows.exe・統合ランチャー・Linux ELF を作って WSL 側へ自動転送まで完了する |
| `make build-windows` | `dist/many-ai-cli.exe` | Windows 本体だけ作り直したいとき（go-winres → go build） |
| `make build-launcher` | `dist/many-ai-cli-launcher.exe` | 統合ランチャー（`winres/winres-launcher.json` のアイコン付き）だけ作り直したいとき |
| `make build-linux` | `dist/linux/many-ai-cli` | Linux ELF（`CGO_ENABLED=0 GOOS=linux GOARCH=amd64`）だけ作り直したいとき |
| `make deploy-wsl` | `dist/linux/many-ai-cli` → WSL `~/.local/bin/many-ai-cli`（cp + chmod +x） | Linux バイナリだけ作り直した後、WSL に再転送だけしたいとき。中身は `scripts/deploy-wsl.ps1` |
| `make run` | `build-windows` 後に `dist/many-ai-cli.exe serve` | ローカルで Hub をすぐ立ち上げたいとき |
| `make clean` | `dist/` 配下と `cmd/*/rsrc_windows_*.syso` を削除 | リソース埋め込みを作り直したいとき |

```bash
# 通常はこれだけ
make build
# 出力: dist/many-ai-cli.exe / dist/many-ai-cli-launcher.exe / dist/linux/many-ai-cli
# 加えて WSL ~/.local/bin/many-ai-cli が最新に差し替わる
```

#### `make deploy-wsl` の中身

`scripts/deploy-wsl.ps1` が以下をやる：

1. `dist/linux/many-ai-cli` を `/mnt/c/...` 形式に変換
2. `wsl -d Ubuntu -- bash -c 'mkdir -p ~/.local/bin && cp ... && chmod +x ...'`
3. `ls -la` と `--version` で反映を確認

引数で上書き可能：`.\scripts\deploy-wsl.ps1 -Distro Ubuntu -Dest '~/.local/bin/many-ai-cli'`

実行中の Hub プロセスがあっても上書き可（Linux は inode 差し替え）。**ただし新バイナリは Hub 再起動まで有効にならない**点に注意。

#### 直接 `go build` を叩いてよいケース

- 急ぎの動作確認で **アイコン/バージョン情報の更新が不要**と分かっているとき
- `winres/winres.json` / `winres/winres-wsl.json` を編集していないとき

それ以外（特にリリース手前・ユーザーに配布する `dist/` を作るとき）は必ず `make build` を使うこと。クロスコンパイル（macOS 向け）は下記「Go バイナリビルド（クロスコンパイル）」のコマンドを使い、`go-winres` は Windows 専用なのでスキップする。

詳細な Windows 開発環境は `windows_setup.md` を参照。

## 配布

### v0.1〜v0.3 の手動配布（暫定）

手動でビルド成果物を共有：
- Windows: `many-ai-cli.exe` を `%LOCALAPPDATA%\Programs\many-ai-cli\` に配置 → PATH 追加
- macOS: `many-ai-cli` を `/usr/local/bin/` または `~/bin/` に配置
- Linux: 同上

### Windows 配布導線の原則

Windows では、ブラウザで直接ダウンロードした unsigned exe / zip は Mark-of-the-Web 付きになりやすく、SmartScreen や Smart App Control の警告・ブロックに入りやすい。そのため、公開導線は次の優先順位で設計する。

1. developer install の推奨導線は npm registry + `pnpm add -g many-ai-cli` にする
2. `winget` は Windows 標準 package manager 導線として扱う
3. Scoop は CLI ユーザー向けの追加導線として扱う
4. GitHub Releases zip は checksum / cosign / `unblock-windows.cmd` 付きの手動導線として維持する

package manager は発見性・更新性・再現性を改善し、ブラウザダウンロード由来の MotW 問題を避けやすくする。ただし Authenticode コード署名の代替ではないため、Smart App Control の完全ブロックや AppLocker / WDAC / EDR 等の組織ポリシーは別途扱う。

npm registry 導線は `npm` コマンド推奨ではない。pnpm / bun / yarn が取得する共有 registry として使い、README の主要コマンドは `pnpm add -g many-ai-cli` にする。`npm install -g many-ai-cli` は Node 標準 fallback として小さく扱う。

npm package を作る場合は platform 別 optional package に Go バイナリを同梱する方式を優先し、install 時に GitHub Releases から exe を後段ダウンロードする wrapper は避ける。

> **実装済み（v0.3.0）**: `npm/many-ai-cli/`（root shim）+ `npm/many-ai-cli-<os>-<arch>/`（platform 別、バイナリは gitignore）。`scripts/sync-npm-version.mjs`（tag→version 同期）/ `scripts/stage-npm-binaries.mjs`（`dist/artifacts.json`→bin 配置）/ `scripts/smoke-npm.mjs`（pack 検証）。release.yml が GoReleaser 後に publish（`NPM_TOKEN` secret 必須・未設定ならスキップ）。詳細は `docs/manual_release.md` の「npm registry 配布」節。

Hub は引き続き `127.0.0.1` 固定で bind し、外部公開用の Windows Firewall 例外を要求しない設計を維持する。

### v0.4+ の CI/CD 配布（予定）

- GitHub Actions で OS 別バイナリビルド + リリースタグ自動生成
- `goreleaser` または手書きワークフローで `dist/{win,mac,mac-arm,linux}/many-ai-cli` をリリース成果物として添付
- 自動更新機能（`many-ai-cli update`）は MVP では作らない

## go:embed の運用

- `internal/hub/embed.go`（仮）に `//go:embed all:web/dist` を書く想定
- `web/dist/` が空のままビルドすると `embed: no matching files found` で失敗するので、CI / Makefile / 手元手順で **必ず先にフロントをビルド**してから Go ビルド
- 開発時のホットリロードは Vite dev server を別ポートで起動し、Go 側からプロキシする方法を実装時に検討（決まったらここに追記）

## 設定ファイルとログのデフォルト位置

| 種別 | 全 OS 共通の表記 | 実体 |
|---|---|---|
| 設定 | `~/.many-ai-cli/config.yaml` | Win: `%USERPROFILE%\.many-ai-cli\config.yaml` |
| ログ（JSONL） | `~/.many-ai-cli/logs/YYYYMMDD.jsonl` | 同上 |
| PTY ログ | `~/.many-ai-cli/logs/sessions/<id>.log` | 同上 |

`os.UserHomeDir()` を使い、`/` ハードコードを避けること。

## ローカル動作確認フロー

```
1. cd web && bun install && bun run build    # web/dist/ を生成
2. cd .. && go build ./...     # 全パッケージのビルド確認
3. go test ./...               # 単体テスト
4. ./many-ai-cli serve          # Hub 起動
5. （別ターミナルで）./many-ai-cli wrap claude    # ラッパー起動
6. ブラウザで http://127.0.0.1:47777/?token=... を開いて動作確認
```

UI 確認は実機ブラウザで実施。Hub UI のレイアウトは設計書 §9 のスクリーンショットと一致するか目視確認すること。

## サブコマンド一覧（実装時参照）

```
many-ai-cli serve [--open] [--port N]
    Hub単体を起動

many-ai-cli wrap <provider> [args...]
    ラッパーとしてCLIを起動

many-ai-cli shell-init
    シェル統合用のスクリプトを標準出力へ
    eval "$(many-ai-cli shell-init)" で取り込む

many-ai-cli stop
    動作中のHubを停止

many-ai-cli status
    Hubの状態確認・接続中セッション数

many-ai-cli --version / -v
many-ai-cli --help / -h
```
