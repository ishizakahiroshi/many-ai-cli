# ai-cli-hub リリース手順

> 最終更新: 2026-05-11(月) 03:01:46

この手順は GitHub Actions の `Release` workflow と GoReleaser で GitHub Releases を作成するための運用メモ。

v0.1.0 は試験リリース扱いとし、初回正式リリースは v0.1.1 とする。

## 現在のリリース方式

- タグ `v*.*.*` を push すると `.github/workflows/release.yml` が起動する
- workflow は Ubuntu 上で GoReleaser v2 系を実行する
- GoReleaser は `.goreleaser.yaml` に従ってリリース成果物を作る
- 現在のビルド対象は `windows/amd64`, `linux/amd64`, `darwin/amd64`, `darwin/arm64`
- checksum は `SHA256SUMS.txt` として生成される
- checksum ファイルは cosign keyless signing で署名される

リリースに添付される想定の主要ファイル:

- `ai-cli-hub-<version>-windows-amd64.zip`
- `ai-cli-hub-<version>-linux-amd64.zip`
- `ai-cli-hub-<version>-darwin-amd64.zip`
- `ai-cli-hub-<version>-darwin-arm64.zip`
- `SHA256SUMS.txt`
- `SHA256SUMS.txt.sig`
- `SHA256SUMS.txt.pem`

## タグを打つ前の確認

リリース前に、作業ツリーを確認する。

```powershell
git status --short
```

未コミット変更がある場合は、リリースに含めるものだけをコミットする。意図しない変更が混ざっている状態でタグを打たない。

バージョン表記を確認する。

- `winres/winres.json`
- `web/src/index.html`
- `web/src/i18n/ja.json`
- `web/src/i18n/en.json`
- `README.md`
- `README.ja.md`

v0.1.1 のように Windows のみ実機検証済みで、Linux / macOS は未検証のまま出す場合は、README の検証状況と `.goreleaser.yaml` のビルド対象が矛盾していないことを確認する。

## ローカル検証

タグを打つ前に最低限これを通す。

```powershell
go test ./...
go vet ./...
go mod verify
```

`goreleaser` がローカルに入っている場合は設定チェックも行う。

```powershell
goreleaser check
```

ローカルに `goreleaser` がない場合は、GitHub Actions 側で初めて検出される設定ミスが残る可能性がある。リリース直前だけでも入れて確認するのが望ましい。

## コミットと push

リリース対象の変更をコミットして `main` に push する。

```powershell
git add .
git commit -m "chore: v0.1.1 リリース準備"
git push origin main
```

既にコミット済みなら `git status --short` が空であることだけ確認する。

## タグ作成

ローカルでタグを作成する。

```powershell
git tag v0.1.1
```

タグが指しているコミットを確認する。

```powershell
git show --stat v0.1.1
```

問題なければタグを push する。

```powershell
git push origin v0.1.1
```

この push により GitHub Actions の `Release` workflow が起動する。

## GitHub Actions 確認

GitHub の Actions タブで `Release` workflow を確認する。

成功条件:

- `actions/checkout` が成功する
- `actions/setup-go` が成功する
- `sigstore/cosign-installer` が成功する
- `goreleaser/goreleaser-action` が成功する

失敗した場合は、失敗ログを確認してから修正コミットを作る。タグは同じ名前を使い回さず、原則として修正後に `v0.1.2` など次のパッチバージョンで出す。

## リリース後確認

GitHub Releases に対象バージョンが作成されていることを確認する。

確認項目:

- release title / tag が意図したバージョンになっている
- zip が添付されている
- `SHA256SUMS.txt` が添付されている
- `SHA256SUMS.txt.sig` が添付されている
- `SHA256SUMS.txt.pem` が添付されている
- README の検証手順と実際の添付ファイル名が一致している

必要なら、ダウンロードした成果物を検証する。

```powershell
cosign verify-blob `
  --certificate SHA256SUMS.txt.pem `
  --signature SHA256SUMS.txt.sig `
  --certificate-identity-regexp "https://github.com/ishizakahiroshi/ai-cli-hub/.github/workflows/release.yml@refs/tags/v.*" `
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" `
  SHA256SUMS.txt
```

Windows では `sha256sum` がない環境もあるため、PowerShell で個別に照合してよい。

```powershell
Get-FileHash .\ai-cli-hub.exe -Algorithm SHA256
```

## 失敗時の扱い

タグ push 後に workflow が失敗した場合:

- まだ公開リリースが作成されていないなら、原因を直して次のタグで出す
- GitHub Release が作成済みで成果物が不完全なら、その Release を draft に戻すか削除して、次のタグで出す
- 公開後に利用者が取得した可能性がある場合は、同じタグを差し替えず、次のパッチバージョンを作る

原則として、公開済みタグの force push や成果物差し替えは避ける。

## 配布対象を変更する場合

現在の `.goreleaser.yaml` は以下をビルドする。

現在の対象:

- `windows/amd64`
- `linux/amd64`
- `darwin/amd64`
- `darwin/arm64`

Linux / macOS を未検証のまま添付する場合は、README と release note に未検証であることを明記する。Linux arm64 や Windows arm64 を追加する場合は、GoReleaser の `ignore` 設定も見直す。
