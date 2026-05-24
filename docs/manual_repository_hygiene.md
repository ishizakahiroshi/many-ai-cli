# any-ai-cli リポジトリ整理ルール

> 最終更新: 2026-05-24(日) 17:11:27

このドキュメントは、GitHub に上げるものとローカルに残すものを判断するための整理メモ。
実際の判定は `.gitignore` / `.gitattributes` / `.goreleaser.yaml` / GitHub Actions の設定と突き合わせる。

## 公開リポジトリに含めるもの

通常は以下を Git 管理する。

- `.github/workflows/`: CI / Release workflow
- `.goreleaser.yaml`, `.gitattributes`, `.gitignore`: リリース・改行・ignore の運用設定
- `cmd/any-ai-cli/`: メインバイナリのエントリポイント
- `cmd/any-ai-cli-wsl/`: Windows 用 WSL ランチャー
- `internal/`: Go の内部実装（Hub / wrapper / sessionlog / wslutil など）
- `web/web.go`, `web/src/`: Hub UI のソースと vendored JS/CSS
- `resources/`: go:embed で同梱する定義（approval patterns / slash commands / model defaults / usage links）
- `assets/`: アイコンなど公開してよい素材
- `winres/`: Windows リソース定義
- `scripts/`: 開発・検証用スクリプト（`scripts/local/` を含む）
- `docs/`: 公開してよい設計書・手順書
- `docs/v0.2.0-any-ai-cli-design.md`: 現行リリースの設計書正本
- `CLAUDE/`, `CLAUDE.md`, `AGENTS.md`, `GEMINI.md`: 公開してよい運用ガイド
- `README.md`, `README.ja.md`, `CHANGELOG.md`, `LICENSE`, `THIRD_PARTY_NOTICES.md`
- `go.mod`, `go.sum`, `Makefile`

Windows リソース用の `.syso` は、GitHub Actions 上の Windows 向けビルドでも使うため Git 管理する。

- `cmd/any-ai-cli/rsrc_windows_*.syso`
- `cmd/any-ai-cli-wsl/rsrc_windows_*.syso`

vendored 依存のライセンス一覧 `web/src/vendor/THIRD_PARTY_LICENSES.txt` は、`.gitignore` の `*.txt` 例外として Git 管理する。
改行を byte-stable に保つ必要があるため、`THIRD_PARTY_NOTICES.md` と `web/src/vendor/THIRD_PARTY_LICENSES.txt` は `.gitattributes` で LF 固定する。

## GitHub に上げないもの

以下はローカル専用、または生成物なので Git 管理しない。

- `dist/`: ローカルビルド成果物
- root 直下の `any-ai-cli.exe`: ローカル実行・検証用バイナリ
- root 直下の `rsrc_windows_*.syso`: 誤って root に生成された Windows リソース
- `docs/local/`: ローカル計画、作業ログ、非公開メモ
- `*.local.md`, `*.local.json`: 端末固有・非公開設定
- `.claude/settings.json`: Claude / Codex のローカル権限設定
- `.claude/settings.local.json`: Claude / Codex のローカル設定
- `.codex-tmp/`: Codex 作業中の一時領域
- `build/`: ローカル生成物を置く場合の一時領域（使う場合は Git 管理対象に混ぜない）
- `*.log`, `*.jsonl`: 実行ログ
- `.env`, `.env.*`: 環境変数・秘密情報
- `*.tmp`, `*.bak`, `*.orig`, `*.rej`: 一時ファイル、パッチ失敗残骸
- `*.txt`: 一般の一時テキスト（例外: `web/src/vendor/THIRD_PARTY_LICENSES.txt`）
- `*gopty_lic.json`: `go-pty` ライセンス確認用の一時JSON
- `.DS_Store`, `Thumbs.db`: OS 生成ファイル

`~/.any-ai-cli/` 配下の設定・ログはリポジトリ外に置く。誤って repo 配下にコピーした場合も、機密値やセッションログを Git 管理しない。

## リリース成果物の考え方

GitHub Releases の成果物は、ローカルの `dist/` をアップロードしない。タグ push 後に GitHub Actions 上で GoReleaser が新しくビルドして添付する。

そのため、ローカルの `dist/` はいつ消してもよい検証用ディレクトリとして扱う。ただし `dist/any-ai-cli.exe` や `dist/any-ai-cli-wsl.exe` を起動中の場合は削除できないため、Hub や WSL ランチャーを止めてから掃除する。

現在の主な Release 添付物は `docs/manual_release.md` を正本とする。現状は Windows amd64 zip に `any-ai-cli.exe` と `any-ai-cli-wsl.exe` を含め、Linux amd64 / macOS amd64 / macOS arm64 も GoReleaser で生成する。

## リリース前チェック

リリース前に以下を確認する。

```powershell
git status --short --ignored
git ls-files docs/local
git ls-files docs/v0.2.0-any-ai-cli-design.md
git status --short docs/v0.2.0-any-ai-cli-design.md
git ls-files "*.local.md" "*.local.json"
git ls-files "*.txt"
git ls-files "*.syso"
```

期待値:

- `docs/local/` は `!! docs/local/` として ignore 済み
- `git ls-files docs/local` は空
- `git ls-files docs/v0.2.0-any-ai-cli-design.md` は 1 行出る
- `git status --short docs/v0.2.0-any-ai-cli-design.md` は空（`??` ではない）
- `*.local.md` / `*.local.json` は Git 管理されていない
- `dist/` と root の `any-ai-cli.exe` は `!!` として ignore 済み
- `git ls-files "*.txt"` は `web/src/vendor/THIRD_PARTY_LICENSES.txt` のみ
- `git ls-files "*.syso"` は `cmd/any-ai-cli/` と `cmd/any-ai-cli-wsl/` 配下のみ
- root 直下に `.env*`, `*.log`, `*.jsonl`, `*.tmp`, `*.bak`, `*.orig`, `*.rej` が混ざっていない

大きいファイルを確認したい場合:

```powershell
Get-ChildItem -Recurse -Force -File |
  Where-Object { $_.FullName -notmatch '\\.git\\' } |
  Sort-Object Length -Descending |
  Select-Object -First 30 FullName,Length
```

## 掃除するとき

ローカル生成物だけを消す。

```powershell
Remove-Item -LiteralPath .\dist -Recurse -Force
Remove-Item -LiteralPath .\any-ai-cli.exe -Force
```

root 直下に誤生成された Windows リソースがある場合だけ削除する。

```powershell
Remove-Item -LiteralPath .\rsrc_windows_386.syso -Force
Remove-Item -LiteralPath .\rsrc_windows_amd64.syso -Force
```

`cmd/any-ai-cli/rsrc_windows_*.syso` と `cmd/any-ai-cli-wsl/rsrc_windows_*.syso` は Git 管理対象なので、掃除目的で削除しない。

`dist/any-ai-cli.exe` が使用中で削除できない場合は、Hub を停止してから再実行する。

```powershell
Get-Process any-ai-cli | Select-Object Id,ProcessName,Path
```

公開済み・共有済みのタグや Release 成果物は、掃除目的で差し替えない。問題があれば次のパッチバージョンで出し直す。

## 依存ライセンス一覧を触るとき

Go 依存や vendored JS/CSS を更新した場合は、`THIRD_PARTY_NOTICES.md` と `web/src/vendor/THIRD_PARTY_LICENSES.txt` の整合性を確認する。

```powershell
.\scripts\local\check-third-party.ps1
```

差分を再生成する必要がある場合:

```powershell
.\scripts\local\gen-third-party-notices.ps1
```

この 2 ファイルは LF 固定なので、手作業で編集した場合も CRLF 化していないことを確認する。
