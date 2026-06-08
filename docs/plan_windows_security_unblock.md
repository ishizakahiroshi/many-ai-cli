# [計画] Windows セキュリティブロック対策計画

> 親: [`local/plan_2026-06-09-batch.md`](local/plan_2026-06-09-batch.md)（2026-06-09 一括実行バッチの C1）
> 最終更新: 2026-06-09(火) 05:31:15

## context配分

| context | 状態 | 担当作業 | 対象ファイル | 依存 |
|---|---|---|---|---|
| C1 | plan | Windows 向け unblock 補助スクリプトを追加する | `scripts/` または release zip 同梱用ファイル | なし |
| C2 | plan | README / リリース手順に、SmartScreen・Smart App Control・Unblock-File の違いと対処手順を追記する | `README.md`, `README.ja.md`, `docs/manual_release.md` | C1 |
| C3 | plan | GoReleaser / 配布成果物に unblock 補助スクリプトを同梱する | `.goreleaser.yaml`, 必要なら `CHANGELOG.md` | C1 |
| C4 | plan | 無料配布経路の改善方針を整理する | `docs/manual_release.md` または別 manual | C2 |

実行順序: `C1 → C2 → C3 → C4`

## 概要

Windows 向けの `any-ai-cli.exe` / `any-ai-cli-launcher.exe` は、現時点では Authenticode コード署名されていない。そのため、ユーザー環境によっては SmartScreen、Mark-of-the-Web、Smart App Control、組織ポリシー、ウイルス対策ソフトによって起動時に警告またはブロックされる。

有料のコード署名証明書をすぐ導入しない前提で、無料でできる短期対策を追加する。ただし、Smart App Control が未署名 exe を完全ブロックする環境では、バッチや PowerShell での回避はできない。この限界は README に明記する。

## 目的

- ZIP 展開後に `Zone.Identifier` が原因で起動しづらいケースを、ユーザーが簡単に解消できるようにする
- SmartScreen 警告と Smart App Control 完全ブロックを混同しない説明にする
- 署名なし配布の限界を明記し、問い合わせ時の切り分けを容易にする
- 将来コード署名を導入するまでの暫定配布品質を上げる

## 非目的

- Authenticode コード署名の導入
- Smart App Control の回避手段の提供
- Windows Defender や組織管理ポリシーを無効化する案内
- ユーザー環境のセキュリティ設定変更を前提にした導入手順

## C1: unblock 補助スクリプト追加

### 方針

Windows release zip に同梱できる `unblock-windows.cmd` を追加する。PowerShell の `Unblock-File` を使い、同じフォルダにある `any-ai-cli*.exe` の Mark-of-the-Web を外す。

### スクリプト案

```bat
@echo off
setlocal
set "AAC_DIR=%~dp0"

powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "Get-ChildItem -LiteralPath $env:AAC_DIR -Filter 'any-ai-cli*.exe' | Unblock-File"

echo.
echo Unblocked any-ai-cli executables in:
echo %AAC_DIR%
echo.
pause
```

### 注意点

- 管理者権限は要求しない
- `Set-ExecutionPolicy` は使わない
- セキュリティ機能を無効化しない
- 対象は同梱 exe のみに限定する
- `start any-ai-cli.exe` まで自動実行するかは実装時に判断する。初期案では unblock だけに留める

### 完了条件

- `unblock-windows.cmd` が Windows release zip に同梱できる場所に追加されている
- PowerShell 5.1 以上の標準 Windows 環境で動く
- 同じフォルダの `any-ai-cli.exe` と `any-ai-cli-launcher.exe` の両方が対象になる

## C2: README / リリース手順更新

### 追記する内容

- Windows の「ブロック」は主に次の種類があること
  - ZIP や exe に付いた Mark-of-the-Web による警告
  - SmartScreen の警告
  - Smart App Control による未署名アプリの完全ブロック
  - 組織管理 PC の AppLocker / WDAC / EDR 等によるブロック
- `unblock-windows.cmd` で改善できるのは Mark-of-the-Web 起因のケースが中心であること
- Smart App Control の完全ブロックは、未署名 exe のままでは回避できないこと
- 推奨手順:
  1. GitHub Release から zip をダウンロード
  2. 必要なら checksum / cosign を検証
  3. zip を展開
  4. `unblock-windows.cmd` を実行
  5. `any-ai-cli.exe` または `any-ai-cli-launcher.exe` を起動

### 完了条件

- `README.ja.md` に日本語の対処手順がある
- `README.md` に英語の対処手順がある
- `docs/manual_release.md` に release zip 同梱物として記載されている
- Smart App Control の限界が明記されている

## C3: 配布成果物への同梱

### 方針

GoReleaser の Windows archive に `unblock-windows.cmd` を含める。Windows 以外の archive には含めない。

### 確認項目

- `.goreleaser.yaml` の Windows archive 設定に extra files を追加できるか確認する
- `windows-x64` zip に以下が入ることを確認する
  - `any-ai-cli.exe`
  - `any-ai-cli-launcher.exe`
  - `unblock-windows.cmd`
  - README / LICENSE 等の既存同梱物

### 完了条件

- ローカルまたは CI の release snapshot で Windows zip 内に `unblock-windows.cmd` が含まれる
- Windows 以外の zip に不要な `.cmd` が混入しない

## C4: 無料配布経路の改善整理

### 候補

- winget manifest を整備する
- Scoop bucket を用意する
- Chocolatey は必要性を見て判断する
- GitHub Release の checksum / cosign 検証手順を簡単にする

### 判断

短期の優先順位は `unblock-windows.cmd` 同梱と README 整備を上に置く。winget / Scoop は導入体験改善には有効だが、未署名 exe の Smart App Control 問題を根本解決するものではない。

### 完了条件

- 無料配布経路の優先順位が `docs/manual_release.md` に整理されている
- コード署名なしで解決できる範囲と、解決できない範囲が明記されている

## リスク

- `Unblock-File` はユーザー環境のポリシーで禁止されている場合がある
- Smart App Control 完全ブロックは、unblock しても解消しない
- セキュリティ対策を弱める案内に見えないよう、スクリプトの対象と説明を限定する必要がある
- 一部のウイルス対策ソフトは、未署名の自己配布 exe を別途隔離する可能性がある

## 受け入れ基準

- Windows release zip を展開したユーザーが、同梱 `unblock-windows.cmd` を実行するだけで Mark-of-the-Web 起因のブロックを外せる
- README を読めば、無料対策で解決できるケースとできないケースが分かる
- Smart App Control の完全ブロックについて「バッチでは回避不可」と明記されている
- コード署名が将来の根本対策として残されている
