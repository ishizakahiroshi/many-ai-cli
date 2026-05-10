# ai-cli-hub ビルド・配布・デプロイ

> 最終更新: 2026-05-07(木) 19:24:03

`ai-cli-hub` は **Go 単一バイナリ + go:embed フロント** の構成。サーバーへのデプロイは無し（ユーザー PC にバイナリを置くだけ）。

設計書: [../docs/ai-cli-hub-design-v0.1.0.md §4・§17](../docs/ai-cli-hub-design-v0.1.0.md)

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
GOOS=windows GOARCH=amd64 go build -o dist/win/ai-cli-hub.exe ./cmd/ai-cli-hub

# macOS (Intel)
GOOS=darwin GOARCH=amd64 go build -o dist/mac/ai-cli-hub ./cmd/ai-cli-hub

# macOS (Apple Silicon)
GOOS=darwin GOARCH=arm64 go build -o dist/mac-arm/ai-cli-hub ./cmd/ai-cli-hub

# Linux (x86_64)
GOOS=linux GOARCH=amd64 go build -o dist/linux/ai-cli-hub ./cmd/ai-cli-hub
```

### CGO の扱い

- 標準は `CGO_ENABLED=0`（純 Go ビルド）を目指す
- PTY ライブラリが CGO 必須の場合は OS 別に build tag で分離し、Linux/Mac は CGO 有効・Windows は ConPTY API を使う純 Go 実装に倒す方針
- 詳細は実装時に決める。決まったら本ドキュメントに追記

### Windows での開発フロー

ローカル PC（Windows 11）で開発し、ビルドは Git Bash (MSYS2) または PowerShell から行う：

```bash
# Git Bash
GOOS=windows GOARCH=amd64 go build -o ai-cli-hub.exe ./cmd/ai-cli-hub
```

```powershell
# PowerShell
$env:GOOS="windows"; $env:GOARCH="amd64"; go build -o ai-cli-hub.exe ./cmd/ai-cli-hub
```

詳細な Windows 開発環境は `windows_setup.md` を参照。

## 配布

### v0.1〜v0.3 の手動配布（暫定）

手動でビルド成果物を共有：
- Windows: `ai-cli-hub.exe` を `%LOCALAPPDATA%\Programs\ai-cli-hub\` に配置 → PATH 追加
- macOS: `ai-cli-hub` を `/usr/local/bin/` または `~/bin/` に配置
- Linux: 同上

### v0.4+ の CI/CD 配布（予定）

- GitHub Actions で OS 別バイナリビルド + リリースタグ自動生成
- `goreleaser` または手書きワークフローで `dist/{win,mac,mac-arm,linux}/ai-cli-hub` をリリース成果物として添付
- 自動更新機能（`ai-cli-hub update`）は MVP では作らない

## go:embed の運用

- `internal/hub/embed.go`（仮）に `//go:embed all:web/dist` を書く想定
- `web/dist/` が空のままビルドすると `embed: no matching files found` で失敗するので、CI / Makefile / 手元手順で **必ず先にフロントをビルド**してから Go ビルド
- 開発時のホットリロードは Vite dev server を別ポートで起動し、Go 側からプロキシする方法を実装時に検討（決まったらここに追記）

## 設定ファイルとログのデフォルト位置

| 種別 | 全 OS 共通の表記 | 実体 |
|---|---|---|
| 設定 | `~/.ai-cli-hub/config.yaml` | Win: `%USERPROFILE%\.ai-cli-hub\config.yaml` |
| ログ（JSONL） | `~/.ai-cli-hub/logs/YYYYMMDD.jsonl` | 同上 |
| PTY ログ | `~/.ai-cli-hub/logs/sessions/<id>.log` | 同上 |

`os.UserHomeDir()` を使い、`/` ハードコードを避けること。

## ローカル動作確認フロー

```
1. cd web && pnpm run build    # web/dist/ を生成
2. cd .. && go build ./...     # 全パッケージのビルド確認
3. go test ./...               # 単体テスト
4. ./ai-cli-hub serve          # Hub 起動
5. （別ターミナルで）./ai-cli-hub wrap claude    # ラッパー起動
6. ブラウザで http://127.0.0.1:47777/?token=... を開いて動作確認
```

UI 確認は実機ブラウザで実施。Hub UI のレイアウトは設計書 §9 のスクリーンショットと一致するか目視確認すること。

## サブコマンド一覧（実装時参照）

```
ai-cli-hub serve [--open] [--port N]
    Hub単体を起動

ai-cli-hub wrap <provider> [args...]
    ラッパーとしてCLIを起動

ai-cli-hub shell-init
    シェル統合用のスクリプトを標準出力へ
    eval "$(ai-cli-hub shell-init)" で取り込む

ai-cli-hub stop
    動作中のHubを停止

ai-cli-hub status
    Hubの状態確認・接続中セッション数

ai-cli-hub --version / -v
ai-cli-hub --help / -h
```
