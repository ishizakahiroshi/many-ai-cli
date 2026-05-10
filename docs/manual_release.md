# ai-cli-hub リリース手順

> 最終更新: 2026-05-11(月) 04:30:00

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

バージョン表記の確認：v0.1.2 以降は **single source of truth** 化されているため、ファイル間で grep して付け合わせる必要は無い。詳細は次節「バージョン文字列の single source 設計」を参照。

ただし v0.1.2 時点では一部のメタデータは依然として手動 bump 対象なので、リリース前に確認すること。

v0.1.2 時点で「タグ → 自動追従」になっているもの：

- Go バイナリの内蔵バージョン文字列（`main.version`、ldflags 注入）
- Web UI 上のバージョン表示（`/api/info` 経由で runtime fetch）
- GitHub Releases 上の zip 名と SHA256SUMS（GoReleaser がタグから生成）

v0.1.2 時点で「手動 bump が必要」なもの（次の release で自動化候補）：

- `winres/winres.json`（Windows .exe の Properties に出る ファイル/プロダクトバージョン）
- `cmd/ai-cli-hub/rsrc_windows_*.syso`（`go-winres make` で `winres.json` から再生成）
- `README.md` / `README.ja.md`（asset URL の version 部分）
- `docs/v0.1.x-ai-cli-hub-design.md` / `CLAUDE.md`（記述上の参照）

Linux / macOS が未検証のまま出す場合は、README の検証状況と `.goreleaser.yaml` のビルド対象が矛盾していないことを確認する。

## バージョン文字列の single source 設計

v0.1.2 時点での設計：

```
git tag v0.1.2  ── push ──┐
                          │
                  goreleaser がタグから "0.1.2" を取り出す
                          │
                          ├── ldflags: -X main.version=0.1.2
                          │      └─ Go バイナリ内蔵
                          │            └─ /api/info の "version" フィールド
                          │                  └─ Web UI が runtime fetch して表示
                          │
                          └── archive 名: ai-cli-hub-0.1.2-{os}-{arch}.zip
                                └─ SHA256SUMS と一緒に Release page へ
```

**実装場所**

- `cmd/ai-cli-hub/main.go`: `var version = "dev"`（package-level、ldflags の注入先）
- `.goreleaser.yaml`: `builds.[].ldflags` に `-X main.version={{.Version}}` を追加
- `internal/hub/server.go`: `NewServer` のシグネチャに `version string` を追加し、`/api/info` JSON に含める
- `web/src/app.js`: 起動時に `/api/info` を fetch、`.settings-app-version` と `.about-version` に値を流し込む
- `web/src/i18n/{ja,en}.json` の `about_version`: 版番号を含む文字列ではなく `"Version {0} [Hub UI]"` のような placeholder に
- `web/src/index.html`: 版番号を直接書かず、空 or skeleton（runtime で書き換え）

**手動 bump がまだ必要な場所（次回 release までに自動化候補）**

- `winres/winres.json` → `cmd/ai-cli-hub/rsrc_windows_*.syso`：`go-winres` がリポジトリ committed の `.syso` を生成。CI で `goreleaser release` の前に `go-winres make --in winres/winres.json --product-version=...` を走らせる pre-build step を入れれば自動化できる
- `README.md` / `README.ja.md`：asset の URL に `0.1.2` が含まれている部分。GitHub Releases の "Latest" link を使う形に切り替えれば固定版番号は消せる
- `docs/v0.1.x-ai-cli-hub-design.md` / `CLAUDE.md`：プロセ的な参照は手動。CHANGELOG 系へリンクする形へ切り替える検討

これらは v0.1.3 以降で対応していく。

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

`THIRD_PARTY_NOTICES.md` を更新する Go 依存変更があった場合は、`scripts/local/check-third-party.ps1` がローカルで通ることも確認する。CI 上で fail させてリリース工程をブロックさせない。

```powershell
.\scripts\local\check-third-party.ps1
```

## Validate workflow が green であることの事前確認

タグを打つ前に、`main` 上の最新コミットに対する `Validate` workflow が green であることを必ず確認する。Validate fail のままタグを打つと、`Release` workflow（goreleaser）は test を呼ばず通ってしまうため、test fail を抱えたバイナリが公開される事故になる。

```powershell
gh run list --repo ishizakahiroshi/ai-cli-hub --workflow=Validate --limit 1
```

ステータスが `success` であることを確認してからタグを打つ。`failure` のままなら、まず Validate を直してから次に進むこと。

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

### 同一タグを使い回せる条件と手順

利用者にまだ取得されていない、かつ Release が draft / 撤回直後である場合に限り、同じタグを使い回して出し直す手段が取れる。判断材料は次の通り。

- リポジトリの star / fork / clone traffic が 0
- Release が `draft: true` のまま、または `gh release delete` 直後で公開実績がない
- リリース直後で時間が経っていない（数十分以内）

上記が満たされない場合は、同じタグの再利用はせず、原因を直したうえで `v0.1.2` 等の次のパッチで出すこと。

#### 撤回 + 出し直しの実コマンド

進行中の Release run があればキャンセルする。

```powershell
gh run list --repo ishizakahiroshi/ai-cli-hub --workflow=Release --status=in_progress --limit 5
gh run cancel <run-id> --repo ishizakahiroshi/ai-cli-hub
```

draft / 公開直後の Release を削除する（`--cleanup-tag` でリモートタグも一緒に消える）。

```powershell
gh release delete v0.1.1 --repo ishizakahiroshi/ai-cli-hub --yes --cleanup-tag
```

`--cleanup-tag` で消えなかった場合や、Release が無くてタグだけ残っている場合はタグを個別に削除する。

```powershell
git push origin :refs/tags/v0.1.1
git tag -d v0.1.1
```

原因コミットを追加 → Validate green を確認 → 同タグを再付与 → push、で再度 Release workflow が起動する。

## Line ending（CRLF / LF）の運用注意

`.gitattributes` で `THIRD_PARTY_NOTICES.md` と `web/src/vendor/THIRD_PARTY_LICENSES.txt` は `eol=lf` 固定にしてある。Windows ローカルでは Git for Windows の `core.autocrlf=true` が一般的なため、何もしないと worktree が CRLF 化されて、`scripts/local/gen-third-party-notices.ps1` の LF 出力と byte-level で一致せず CI が fail する。

新たに「自動生成」で repo 配下に置くテキストを追加するときは、次のいずれかを必ず満たす。

- 生成器側で改行を LF に正規化して書き出す（`scripts/local/gen-third-party-notices.ps1` の `WriteAllText` + LF 結合パターンを参照）
- `.gitattributes` で `<path> text eol=lf` を追記する

PowerShell スクリプト（`*.ps1`）は逆に CRLF に固定している。Windows 上で読みやすさを保つため。

## CI で使っている Actions の更新

GitHub の `windows-latest` / `ubuntu-latest` runner は、Node.js 20 ベースの actions に対して deprecation 警告を出している（2026-06-02 以降は Node.js 24 強制、2026-09-16 で Node.js 20 削除）。リリース workflow が触る次の actions は、上限期限までに新メジャーへ更新するか、`FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true` を環境変数で付ける。

- `actions/checkout@v4`
- `actions/setup-go@v5`
- `goreleaser/goreleaser-action@v6`
- `sigstore/cosign-installer@v3`

更新時は `Validate` workflow を先に当て、test と third-party チェックが green であることを確認してから `Release` workflow も追従させる。

## 配布対象を変更する場合

現在の `.goreleaser.yaml` は以下をビルドする。

現在の対象:

- `windows/amd64`
- `linux/amd64`
- `darwin/amd64`
- `darwin/arm64`

Linux / macOS を未検証のまま添付する場合は、README と release note に未検証であることを明記する。Linux arm64 や Windows arm64 を追加する場合は、GoReleaser の `ignore` 設定も見直す。
