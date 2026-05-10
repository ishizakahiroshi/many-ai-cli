# ai-cli-hub リポジトリ整理ルール

> 最終更新: 2026-05-11(月) 03:05:23

このドキュメントは、GitHub に上げるものとローカルに残すものを判断するための整理メモ。

## 公開リポジトリに含めるもの

通常は以下を Git 管理する。

- `.github/workflows/`: CI / Release workflow
- `cmd/`: Go のエントリポイント
- `internal/`: Go の内部実装
- `web/src/`: Hub UI のソースと vendored xterm.js
- `resources/`: 同梱する静的リソース
- `assets/`: アイコンなど公開してよい素材
- `winres/`: Windows リソース定義
- `scripts/`: 開発・検証用スクリプト
- `docs/`: 公開してよい設計書・手順書
- `CLAUDE/`, `CLAUDE.md`, `AGENTS.md`, `GEMINI.md`: 公開してよい運用ガイド
- `README.md`, `README.ja.md`, `LICENSE`, `THIRD_PARTY_NOTICES.md`
- `go.mod`, `go.sum`, `Makefile`

Windows リソース用の `cmd/ai-cli-hub/rsrc_windows_*.syso` は、GitHub Actions 上の Windows 向けビルドでも使うため Git 管理する。

## GitHub に上げないもの

以下はローカル専用、または生成物なので Git 管理しない。

- `dist/`: ローカルビルド成果物
- root 直下の `ai-cli-hub.exe`: ローカル実行・検証用バイナリ
- root 直下の `rsrc_windows_*.syso`: 誤って root に生成された Windows リソース
- `docs/local/`: ローカル計画、作業ログ、非公開メモ
- `*.local.md`, `*.local.json`: 端末固有・非公開設定
- `.claude/settings.json`: Claude / Codex のローカル権限設定
- `.claude/settings.local.json`: Claude / Codex のローカル設定
- `*.log`, `*.jsonl`: 実行ログ
- `.env`, `.env.*`: 環境変数・秘密情報
- `*.tmp`, `*.bak`, `*.orig`, `*.rej`: 一時ファイル、パッチ失敗残骸
- `.DS_Store`, `Thumbs.db`: OS 生成ファイル

## リリース成果物の考え方

GitHub Releases の成果物は、ローカルの `dist/` をアップロードしない。タグ push 後に GitHub Actions 上で GoReleaser が新しくビルドして添付する。

そのため、ローカルの `dist/` はいつ消してもよい検証用ディレクトリとして扱う。ただし `dist/ai-cli-hub.exe` を起動中の場合は削除できないため、Hub を止めてから掃除する。

## リリース前チェック

リリース前に以下を確認する。

```powershell
git status --short --ignored
git ls-files docs/local
git ls-files "*.local.md" "*.local.json"
```

期待値:

- `docs/local/` は `!! docs/local/` として ignore 済み
- `git ls-files docs/local` は空
- `*.local.md` / `*.local.json` は Git 管理されていない
- `dist/` と root の `ai-cli-hub.exe` は `!!` として ignore 済み

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
Remove-Item -LiteralPath .\ai-cli-hub.exe -Force
```

`dist/ai-cli-hub.exe` が使用中で削除できない場合は、Hub を停止してから再実行する。

```powershell
Get-Process ai-cli-hub | Select-Object Id,ProcessName,Path
```

公開済み・共有済みのタグや Release 成果物は、掃除目的で差し替えない。問題があれば次のパッチバージョンで出し直す。
