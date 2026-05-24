# any-ai-cli ビルド・配布・デプロイ

> 最終更新: 2026-05-24(日) 17:01:04

`any-ai-cli` は **Go 単一バイナリ + go:embed フロント** の構成。サーバーへのデプロイは無し（ユーザー PC にバイナリを置くだけ）。

設計書: [../docs/v0.2.0-any-ai-cli-design.md §4・§17](../docs/v0.2.0-any-ai-cli-design.md)

## ビルド前提

- Go 1.22+
- Node.js 20+ / pnpm or npm（フロント `web/` のビルド用）
- フロントは事前に `web/dist/` をビルドし、Go の `//go:embed web/dist` で同梱

### フロントビルド

```bash
cd web
pnpm install        # または npm install
pnpm run build      # web/dist/ が生成される
cd ..
```

`web/dist/` が `internal/hub/` の `embed.FS` に取り込まれる前提で実装すること。

### Go バイナリビルド（クロスコンパイル）

```bash
# Windows (x86_64)
GOOS=windows GOARCH=amd64 go build -o dist/win/any-ai-cli.exe ./cmd/any-ai-cli

# macOS (Intel)
GOOS=darwin GOARCH=amd64 go build -o dist/mac/any-ai-cli ./cmd/any-ai-cli

# macOS (Apple Silicon)
GOOS=darwin GOARCH=arm64 go build -o dist/mac-arm/any-ai-cli ./cmd/any-ai-cli

# Linux (x86_64)
GOOS=linux GOARCH=amd64 go build -o dist/linux/any-ai-cli ./cmd/any-ai-cli
```

### CGO の扱い

- 標準は `CGO_ENABLED=0`（純 Go ビルド）を目指す
- PTY ライブラリが CGO 必須の場合は OS 別に build tag で分離し、Linux/Mac は CGO 有効・Windows は ConPTY API を使う純 Go 実装に倒す方針
- 詳細は実装時に決める。決まったら本ドキュメントに追記

### Windows での開発フロー

**ローカルビルドは原則 `make build` を使う**。`go build` を素で叩くと `go-winres` がスキップされて、`cmd/any-ai-cli/rsrc_windows_*.syso`（アプリアイコン等の Windows リソース）が古いまま `dist/any-ai-cli.exe` に embed される。

#### Makefile ターゲット一覧（v0.2.0 時点）

種類が増えてきたのでここに集約する。**他所に分散させない**。

| ターゲット | 出力物 / 動作 | 使う場面 |
|---|---|---|
| `make build` | 下 4 つ（windows + wsl-launcher + linux + deploy-wsl）を順に実行 | 通常はこれ 1 本。Windows.exe・WSL ランチャー・Linux ELF を作って WSL 側へ自動転送まで完了する |
| `make build-windows` | `dist/any-ai-cli.exe` | Windows 本体だけ作り直したいとき（go-winres → go build） |
| `make build-wsl-launcher` | `dist/any-ai-cli-wsl.exe` | WSL ランチャー（`winres/winres-wsl.json` のアイコン付き）だけ作り直したいとき |
| `make build-linux` | `dist/linux/any-ai-cli` | Linux ELF（`CGO_ENABLED=0 GOOS=linux GOARCH=amd64`）だけ作り直したいとき |
| `make deploy-wsl` | `dist/linux/any-ai-cli` → WSL `~/.local/bin/any-ai-cli`（cp + chmod +x） | Linux バイナリだけ作り直した後、WSL に再転送だけしたいとき。中身は `scripts/deploy-wsl.ps1` |
| `make run` | `build-windows` 後に `dist/any-ai-cli.exe serve` | ローカルで Hub をすぐ立ち上げたいとき |
| `make clean` | `dist/` 配下と `cmd/*/rsrc_windows_*.syso` を削除 | リソース埋め込みを作り直したいとき |

```bash
# 通常はこれだけ
make build
# 出力: dist/any-ai-cli.exe / dist/any-ai-cli-wsl.exe / dist/linux/any-ai-cli
# 加えて WSL ~/.local/bin/any-ai-cli が最新に差し替わる
```

#### `make deploy-wsl` の中身

`scripts/deploy-wsl.ps1` が以下をやる：

1. `dist/linux/any-ai-cli` を `/mnt/c/...` 形式に変換
2. `wsl -d Ubuntu -- bash -c 'mkdir -p ~/.local/bin && cp ... && chmod +x ...'`
3. `ls -la` と `--version` で反映を確認

引数で上書き可能：`.\scripts\deploy-wsl.ps1 -Distro Ubuntu -Dest '~/.local/bin/any-ai-cli'`

実行中の Hub プロセスがあっても上書き可（Linux は inode 差し替え）。**ただし新バイナリは Hub 再起動まで有効にならない**点に注意。

#### 直接 `go build` を叩いてよいケース

- 急ぎの動作確認で **アイコン/バージョン情報の更新が不要**と分かっているとき
- `winres/winres.json` / `winres/winres-wsl.json` を編集していないとき

それ以外（特にリリース手前・ユーザーに配布する `dist/` を作るとき）は必ず `make build` を使うこと。クロスコンパイル（macOS 向け）は下記「Go バイナリビルド（クロスコンパイル）」のコマンドを使い、`go-winres` は Windows 専用なのでスキップする。

詳細な Windows 開発環境は `windows_setup.md` を参照。

## 配布

### v0.1〜v0.3 の手動配布（暫定）

手動でビルド成果物を共有：
- Windows: `any-ai-cli.exe` を `%LOCALAPPDATA%\Programs\any-ai-cli\` に配置 → PATH 追加
- macOS: `any-ai-cli` を `/usr/local/bin/` または `~/bin/` に配置
- Linux: 同上

### v0.4+ の CI/CD 配布（予定）

- GitHub Actions で OS 別バイナリビルド + リリースタグ自動生成
- `goreleaser` または手書きワークフローで `dist/{win,mac,mac-arm,linux}/any-ai-cli` をリリース成果物として添付
- 自動更新機能（`any-ai-cli update`）は MVP では作らない

## go:embed の運用

- `internal/hub/embed.go`（仮）に `//go:embed all:web/dist` を書く想定
- `web/dist/` が空のままビルドすると `embed: no matching files found` で失敗するので、CI / Makefile / 手元手順で **必ず先にフロントをビルド**してから Go ビルド
- 開発時のホットリロードは Vite dev server を別ポートで起動し、Go 側からプロキシする方法を実装時に検討（決まったらここに追記）

## 設定ファイルとログのデフォルト位置

| 種別 | 全 OS 共通の表記 | 実体 |
|---|---|---|
| 設定 | `~/.any-ai-cli/config.yaml` | Win: `%USERPROFILE%\.any-ai-cli\config.yaml` |
| ログ（JSONL） | `~/.any-ai-cli/logs/YYYYMMDD.jsonl` | 同上 |
| PTY ログ | `~/.any-ai-cli/logs/sessions/<id>.log` | 同上 |

`os.UserHomeDir()` を使い、`/` ハードコードを避けること。

## ローカル動作確認フロー

```
1. cd web && pnpm run build    # web/dist/ を生成
2. cd .. && go build ./...     # 全パッケージのビルド確認
3. go test ./...               # 単体テスト
4. ./any-ai-cli serve          # Hub 起動
5. （別ターミナルで）./any-ai-cli wrap claude    # ラッパー起動
6. ブラウザで http://127.0.0.1:47777/?token=... を開いて動作確認
```

UI 確認は実機ブラウザで実施。Hub UI のレイアウトは設計書 §9 のスクリーンショットと一致するか目視確認すること。

## サブコマンド一覧（実装時参照）

```
any-ai-cli serve [--open] [--port N]
    Hub単体を起動

any-ai-cli wrap <provider> [args...]
    ラッパーとしてCLIを起動

any-ai-cli shell-init
    シェル統合用のスクリプトを標準出力へ
    eval "$(any-ai-cli shell-init)" で取り込む

any-ai-cli stop
    動作中のHubを停止

any-ai-cli status
    Hubの状態確認・接続中セッション数

any-ai-cli --version / -v
any-ai-cli --help / -h
```
