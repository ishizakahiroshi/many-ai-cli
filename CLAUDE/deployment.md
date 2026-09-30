# many-ai-cli ビルド・配布・デプロイ

> 最終更新: 2026-09-30(水) 09:59:15 — 実物と食い違っていた記述を直した（make build は WSL へ送らない・Go の版・go:embed の場所・CGO・ログの場所など）。古いサブコマンド一覧と v0.1〜v0.3 の手動配布の節を削った
> 2026-09-27 05:23:16 — 調査用機能全件のリリース前 purge と共通検査への導線を追加

`many-ai-cli` は **Go 単一バイナリ + go:embed フロント** の構成。サーバーへのデプロイは無し（ユーザー PC にバイナリを置くだけ）。

v0.3.x 設計書（非公開・履歴）: [../docs/local/archive/v0.3.x/v0.3.x-many-ai-cli-design.md](../docs/local/archive/v0.3.x/v0.3.x-many-ai-cli-design.md)

## ビルド前提

リリース前に調査用のログ・追跡・監視・計測機能を全件 `debug-purge` し、`node scripts/check-instrumentation.mjs --release` を通す。方針と分離方法は [coding.md の「調査用機能の分離と purge」](coding.md#調査用機能の分離と-purge)、台帳は `instrumentation.json`。GitHub Release workflow とローカル GoReleaser の before hook の両方で検査し、未パージなら止める。ビルド対象からの除外と、パージ完了は別の条件である。

- Go 1.26.8+（`go.mod` の `go` 行が正本）
- Node.js 20+（フロントのビルドスクリプト `web/scripts/build.mjs`、`scripts/` の検査スクリプト、`node --test` の実行用）
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

`web/dist/` は `web/web.go` の `//go:embed all:dist` で取り込まれる。
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

- 全ターゲット `CGO_ENABLED=0`（純 Go ビルド）。正本は `.goreleaser.yaml` と `Makefile` の `build-linux`
- PTY は Unix が `creack/pty`、Windows が ConPTY（`aymanbagabas/go-pty`）で、どちらも CGO を使わない

### Windows での開発フロー

**ローカルビルドは原則 `make build` を使う**。`go build` を素で叩くと `go-winres` がスキップされて、`cmd/many-ai-cli/rsrc_windows_*.syso`（アプリアイコン等の Windows リソース）が古いまま `dist/many-ai-cli.exe` に embed される。

#### Makefile ターゲット一覧

種類が増えてきたのでここに集約する。**他所に分散させない**。

| ターゲット | 出力物 / 動作 | 使う場面 |
|---|---|---|
| `make build` | 下の windows + launcher + linux を順に実行（WSL へは送らない） | 通常はこれ 1 本。Windows.exe・統合ランチャー・Linux ELF を作る |
| `make build-web` | `web/dist/`（`web` で `bun install` と `bun run build`） | フロントだけ作り直したいとき。`build-windows` と `build-linux` はこれを先に実行する |
| `make build-windows` | `dist/many-ai-cli.exe` | Windows 本体だけ作り直したいとき（go-winres → go build） |
| `make build-launcher` | `dist/many-ai-cli-launcher.exe` | 統合ランチャー（`winres/winres-launcher.json` のアイコン付き）だけ作り直したいとき |
| `make build-linux` | `dist/linux/many-ai-cli` | Linux ELF（`CGO_ENABLED=0 GOOS=linux GOARCH=amd64`）だけ作り直したいとき |
| `make deploy-wsl` | `dist/linux/many-ai-cli` → WSL `~/.local/bin/many-ai-cli`（cp + chmod +x） | Linux バイナリを WSL へ送りたいとき（`make build` の後に続けて実行する）。中身は `scripts/deploy-wsl.ps1` |
| `make run` | `build-windows` 後に `dist/many-ai-cli.exe serve` | ローカルで Hub をすぐ立ち上げたいとき |
| `make clean` | `dist/` 配下と `cmd/*/rsrc_windows_*.syso` を削除 | リソース埋め込みを作り直したいとき |
| `make fmt-check` / `make fmt` | `scripts/check-gofmt.mjs` で gofmt との差を確かめる / 整形する | Go ファイルの整形を確かめたいとき |
| `make debug-purge id=<id>` / `make debug-restore id=<id>` | 調査用機能の撤去 / 復元 | リリース前（`coding.md` の「調査用機能の分離と purge」） |

```bash
# 通常はこれだけ
make build
# 出力: dist/many-ai-cli.exe / dist/many-ai-cli-launcher.exe / dist/linux/many-ai-cli
# WSL の ~/.local/bin/many-ai-cli も差し替えるときは続けて make deploy-wsl
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
- `winres/winres.json` / `winres/winres-launcher.json` を編集していないとき

それ以外（特にリリース手前・ユーザーに配布する `dist/` を作るとき）は必ず `make build` を使うこと。クロスコンパイル（macOS 向け）は下記「Go バイナリビルド（クロスコンパイル）」のコマンドを使い、`go-winres` は Windows 専用なのでスキップする。

詳細な Windows 開発環境は `windows_setup.md` を参照。

## 配布

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

### CI/CD 配布（v0.3.0 で実装済み）

- GitHub Actions（`release.yml`）+ GoReleaser によるタグ駆動リリース（OS 別バイナリ + npm publish。詳細は `docs/manual_release.md`）
- 自動更新機能（`many-ai-cli update`）は未実装

## go:embed の運用

- 取り込みは `web/web.go` の `//go:embed all:dist`
- `web/dist/` が空のままビルドすると `embed: no matching files found` で失敗するので、CI / Makefile / 手元手順で **必ず先にフロントをビルド**してから Go ビルド
- 開発時のホットリロードは未整備（Vite は不採用・esbuild のみ）。フロント変更時は `cd web && bun run build` で `web/dist/` を再生成してから Go ビルド／Hub 再起動する

## 設定ファイルとログのデフォルト位置

| 種別 | 全 OS 共通の表記 | 実体 |
|---|---|---|
| 設定 | `~/.many-ai-cli/config.yaml` | Win: `%USERPROFILE%\.many-ai-cli\config.yaml` |
| Hub のログ | `~/.many-ai-cli/logs/hub.log` | 同上 |
| セッションのログ | `~/.many-ai-cli/logs/sessions/<provider>_<日時>_<folder>_s<id>.log/.jsonl/.txt` | 同上 |

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

UI 確認は実機ブラウザで実施する（画面の仕様は README とソースが正本）。

## サブコマンド一覧

一覧は `CLAUDE.md` の用語表にある（正本は `cmd/many-ai-cli/main.go` のサブコマンド分岐）。ここには写さない。
