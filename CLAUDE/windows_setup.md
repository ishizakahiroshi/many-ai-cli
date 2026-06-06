# any-ai-cli Windows 開発環境

> 最終更新: 2026-06-07(日) 01:46:08

開発端末は Windows 11。`any-ai-cli` 自体はクロスプラットフォーム（Win/Mac/Linux）だが、本ドキュメントはユーザー環境固有の手順をまとめる。

## 開発ツール

| ツール | 用途 | 備考 |
|---|---|---|
| Go 1.22+ | 本体ビルド | `winget install GoLang.Go` または公式インストーラ |
| Node.js 20+ | フロント (`web/`) ビルドスクリプト実行 | `winget install OpenJS.NodeJS.LTS` |
| Bun 1.3+ | フロント (`web/`) 依存取得・スクリプト起動 | `winget install Oven-sh.Bun` |
| Git | バージョン管理 | Git Bash (MSYS2) 同梱 |
| VS Code | エディタ | Go・Vue Language Features・ESLint 拡張 |
| PowerShell 7+ (`pwsh`) | スクリプト・タイムスタンプ取得 | 本リポジトリのコマンドはすべて `pwsh` 想定 |
| Git Bash (MSYS2) | bash 互換シェル | 設計書のクロスコンパイルコマンド実行用 |

## ローカル開発フロー

### 初回セットアップ

```bash
# Git Bash で
cd /c/dev/cli-popup
go mod download         # go.mod が用意されたら
cd web
bun install             # npm / pnpm は使わない（bun.lock が正）
```

### 反復開発（実装着手後）

```bash
# 1. フロントを変更したらビルド
cd web && bun run build && cd ..

# 2. Go 側を変更したら go build / go test
go build ./...
go test ./...

# 3. 起動確認
./any-ai-cli.exe serve

# 4. 別ターミナルでラッパー起動
./any-ai-cli.exe wrap claude
```

### ホットリロード（フロント開発時）

実装方針は `deployment.md` 参照。Vite dev server を別ポート（例: 5173）で起動し、Hub は dev mode のときだけ Vite へプロキシする想定。

## PowerShell でのタイムスタンプ取得

`docs/` 配下や `CLAUDE/*.md` の `> 最終更新: ...` 行を更新するときに使用：

```powershell
$d = Get-Date; "{0}({1}) {2}" -f $d.ToString("yyyy-MM-dd"), "日月火水木金土"[$d.DayOfWeek.value__], $d.ToString("HH:mm:ss")
```

出力例: `2026-05-06(水) 10:38:55`

## Windows 固有の注意点

### パス区切り

- Go コードでは `filepath.Join` を必ず使う（`/` ハードコード禁止）
- ドキュメント・README では `/` で書いて構わない（人間が読むため）
- `os.UserHomeDir()` は Windows で `C:\Users\<name>` を返す。`~/.any-ai-cli/` 表記は Windows でも `C:\Users\<name>\.any-ai-cli\` の意味で扱う

### 改行コード

- `.gitattributes` で `* text=auto eol=lf` を基本にする（実装着手時に整備）
- Windows ネイティブのファイル（`.bat` / `.ps1`）のみ `eol=crlf`

### Windows Defender / SmartScreen

- 自前ビルドの `any-ai-cli.exe` は署名なしのため初回起動時に警告が出る可能性
- `127.0.0.1` バインドのため Defender Firewall の警告は出ない見込み（出たら `localhost のみ` で許可）
- 詳細は v0.4 で署名 / 配布方針を決める

### ConPTY（Windows 10 1809+）

- Windows での PTY は ConPTY 経由（Win10 1809+ 必須）
- 古い Windows 7/8 はサポート対象外（README に明記する）

## VS Code 推奨設定

`.vscode/settings.json`（実装時に整備）：

```jsonc
{
  "go.formatTool": "gofmt",
  "go.lintTool": "golangci-lint",
  "[go]": { "editor.defaultFormatter": "golang.go" },
  "[vue]": { "editor.defaultFormatter": "Vue.volar" },
  "files.eol": "\n"
}
```

## 4 ペイン手動テスト手順

設計書 §9 の Hub UI 動作確認用：

1. Windows Terminal で 4 ペイン分割
2. 各ペインで `cd C:\dev\project-X` してから `any-ai-cli.exe wrap <provider>` を起動（または `AI_HUB_AUTO=1` + `eval "$(any-ai-cli.exe shell-init)"` 経由）
3. 別ウィンドウでブラウザを開き `http://127.0.0.1:47777/?token=<起動時に表示>` を表示
4. ブラウザを画面下部または右側に常時固定し、4 ペインが上に並ぶレイアウトで動作確認
