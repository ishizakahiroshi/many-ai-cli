# any-ai-cli リリース手順

> 最終更新: 2026-06-05(金) 13:21:18 — v0.3.0 の artifact 名・GoReleaser / winres 運用へ更新

この手順は GitHub Actions の `Release` workflow と GoReleaser で GitHub Releases を作成するための運用メモ。

v0.1.0 は試験リリース扱いとし、初回正式リリースは v0.1.1 とする。

## 現在のリリース方式

- タグ `v*.*.*` を push すると `.github/workflows/release.yml` が起動する
- workflow は Ubuntu 上で GoReleaser v2 系を実行する
- GoReleaser は `.goreleaser.yaml` に従ってリリース成果物を作る
- 現在のビルド対象は `windows/amd64`（`any-ai-cli.exe` と `any-ai-cli-launcher.exe`）, `linux/amd64`, `darwin/amd64`, `darwin/arm64`
- checksum は `SHA256SUMS.txt` として生成される
- checksum ファイルは cosign keyless signing で署名される

リリースに添付される想定の主要ファイル:

- `any-ai-cli-<version>-windows-x64.zip`（`any-ai-cli.exe` / `any-ai-cli-launcher.exe`）
- `any-ai-cli-<version>-linux-x64.zip`
- `any-ai-cli-<version>-macos-intel.zip`
- `any-ai-cli-<version>-macos-apple-silicon.zip`
- `SHA256SUMS.txt`
- `SHA256SUMS.txt.sig`
- `SHA256SUMS.txt.pem`

## タグを打つ前の確認

リリース前に、作業ツリーを確認する。

```powershell
git status --short
```

未コミット変更がある場合は、リリースに含めるものだけをコミットする。意図しない変更が混ざっている状態でタグを打たない。

バージョン表記の確認：Go バイナリ本体と release archive 名は GoReleaser のタグ版数が **single source of truth**。詳細は次節「バージョン文字列の single source 設計」を参照。

ただし、リリースごとに人間が更新する公開文書・Windows resource template は残るため、タグを打つ前に確認すること。

「タグ → 自動追従」になっているもの：

- Go バイナリの内蔵バージョン文字列（`main.version`、ldflags 注入）
- Web UI 上のバージョン表示（`/api/info` 経由で runtime fetch）
- GitHub Releases 上の zip 名と SHA256SUMS（GoReleaser がタグから生成）
- GoReleaser before hook で生成する Windows `.syso` の FileVersion / ProductVersion

リリース前に手動確認・更新するもの：

- `CHANGELOG.md`（`[Unreleased]` を新バージョン節へ確定し、比較リンクを更新）
- `README.md` / `README.ja.md`（追加機能、検証状況、artifact 名、セキュリティ説明）
- `winres/winres.json` / `winres/winres-launcher.json`（manifest identity 等の template 版数）
- `THIRD_PARTY_NOTICES.md` / `web/src/vendor/THIRD_PARTY_LICENSES.txt`（依存・vendored license 表記）

Linux / macOS が未検証のまま出す場合は、README の検証状況と `.goreleaser.yaml` のビルド対象が矛盾していないことを確認する。

## バージョン文字列の single source 設計

現在の設計：

```
git tag v0.3.0  ── push ──┐
                          │
                  goreleaser がタグから "0.3.0" を取り出す
                          │
                          ├── ldflags: -X main.version=0.3.0
                          │      └─ Go バイナリ内蔵
                          │            └─ /api/info の "version" フィールド
                          │                  └─ Web UI が runtime fetch して表示
                          │
                          ├── archive 名: any-ai-cli-0.3.0-{os}-{arch}.zip
                          │      └─ SHA256SUMS と一緒に Release page へ
                          │
                          └── go-winres before hook
                                 └─ Windows .syso の FileVersion / ProductVersion
```

**実装場所**

- `cmd/any-ai-cli/main.go`: `var version = "dev"`（package-level、ldflags の注入先）
- `.goreleaser.yaml`: `builds.[].ldflags` に `-X main.version={{.Version}}` を追加
- `internal/hub/server.go`: `NewServer` のシグネチャに `version string` を追加し、`/api/info` JSON に含める
- `web/src/app.ts`: 起動時に `/api/info` を fetch、`.settings-app-version` と `.about-version` に値を流し込む
- `web/src/i18n/{ja,en}.json` の `about_version`: 版番号を含む文字列ではなく `"Version {0} [Hub UI]"` のような placeholder に
- `web/src/index.html`: 版番号を直接書かず、空 or skeleton（runtime で書き換え）

`go-winres` は `.goreleaser.yaml` の before hook で実行する。`.syso` は git 追跡外の生成物として扱い、タグ版数は hook の `--product-version={{ .Version }}` / `--file-version={{ .Version }}` で注入する。

## スラッシュコマンド等 runtime-served resources の差分確認

`resources/` 配下の以下は、リリースのタグやバイナリとは独立に、実行時に GitHub の `main` ブランチから raw fetch される（リビルド不要・トークン消費 0 で更新できる仕組み。実装は `internal/config/config.go` の `Default*Source` と `internal/hub/slash_cmd_fetch.go` 等を参照）。

- `resources/slash-commands/claude.md` / `codex.md` / `copilot.md`
- `resources/approval-patterns/claude.md` / `codex.md` / `copilot.md` / `common.md`
- `resources/usage-links/defaults.json`
- `resources/models/defaults.json`

取得元はいずれも `https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/...`。つまり **`main` に push した瞬間、全ユーザーの取得元が切り替わる**（Hub 側 24h キャッシュ TTL の範囲で順次反映）。バイナリの version とは無関係に live に効くため、リリース commit を `main` に入れる前に、ローカル作業ツリーと GitHub `main` 公開分の差分を必ず確認する。

スラッシュコマンドの差分確認（PowerShell）:

```powershell
foreach ($p in 'claude','codex','copilot') {
  $remote = "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/$p.md"
  $gh = (Invoke-WebRequest -UseBasicParsing $remote).Content
  $local = Get-Content -Raw "resources/slash-commands/$p.md"
  if ($gh -eq $local) { "$p : 差分なし" }
  else { "$p : 差分あり"; Compare-Object ($gh -split "`n") ($local -split "`n") }
}
```

bash の場合:

```bash
for p in claude codex copilot; do
  curl -s "https://raw.githubusercontent.com/ishizakahiroshi/any-ai-cli/main/resources/slash-commands/$p.md" -o "/tmp/gh_$p.md"
  diff "/tmp/gh_$p.md" "resources/slash-commands/$p.md" && echo "$p: 差分なし"
done
```

判断:

- 差分が出た行が今回のリリースで意図して入れたものか確認する（新コマンドの追加 / 削除 / 説明文の更新など）。
- 意図した差分なら、リリース commit に含めて `main` を push すれば公開と同時に live 反映される。
- 意図しない差分（手元の編集ミス・revert 漏れ）なら、push 前に解消する。
- approval-patterns / usage-links / models も同じ仕組みなので、それらを変更した release では同じ要領で差分確認する。

なお develop 上で先行して編集している場合、ブランチ運用上は `main` への merge 時点で初めて公開されるため、develop の作業ツリーと `main` 公開分が食い違うのは正常（リリースで解消される）。

## ローカル検証

タグを打つ前に最低限これを通す。`go.mod` の `go` directive が v0.3.0 の release baseline（現在は `go 1.25.11`）になっていることも確認する。

```powershell
go test ./...
go vet ./...
go mod verify
go run golang.org/x/vuln/cmd/govulncheck@latest ./...
pushd web
bun run check
bun run test
popd
```

CGO と C compiler がある環境では race detector も通す。Windows ローカルで `gcc` がない場合は WSL/Linux/CI など別環境で実行し、結果を precheck 記録に残す。

```powershell
go test -race ./...
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

vendored browser library は `npm audit` の対象外なので、更新した実体・`web/src/vendor/THIRD_PARTY_LICENSES.txt`・About 画面の版数を一致させる。v0.3.0 で更新を defer する場合は、既知脆弱性確認結果と defer 理由を `docs/local/bugfix_v0-3-0-release-precheck_YYYY-MM-DD.md` に残す。

Hub UI smoke では少なくとも spawn（Ollama Cloud route を含む）、reconnect、Files preview、path open、Git tab、approval action bar、settings save、Workbench history/export を確認し、同じ precheck 記録に結果を書く。

## CHANGELOG.md の更新

`CHANGELOG.md` はリリース毎に手動で 1 節を追記する。Keep a Changelog 形式に揃え、`[Unreleased]` セクションに溜めていた変更を新バージョンの節として確定させる。最低限のフォーマット：

```markdown
## [0.1.x] - YYYY-MM-DD

### Added
- 新機能 / 設計変更

### Fixed
- バグ修正

### Changed
- 既存挙動の変更（互換性に影響しうるもの）

### Removed
- 削除した機能 / API（あれば）
```

末尾の compare リンクも更新する：

```markdown
[Unreleased]: https://github.com/ishizakahiroshi/any-ai-cli/compare/v0.1.x...HEAD
[0.1.x]: https://github.com/ishizakahiroshi/any-ai-cli/releases/tag/v0.1.x
```

GoReleaser は別途 commit message から auto-generated changelog を Release ページに付ける（`.goreleaser.yaml` の `changelog` 設定）。`CHANGELOG.md` は人間が読むもの、auto-generated は forensics 用の commit list、と用途を分けて運用する。

## Validate workflow が green であることの事前確認

タグを打つ前に、`main` 上の最新コミットに対する `Validate` workflow が green であることを必ず確認する。Validate fail のままタグを打つと、`Release` workflow（goreleaser）は test を呼ばず通ってしまうため、test fail を抱えたバイナリが公開される事故になる。

```powershell
gh run list --repo ishizakahiroshi/any-ai-cli --workflow=Validate --limit 1
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
  --certificate-identity-regexp "https://github.com/ishizakahiroshi/any-ai-cli/.github/workflows/release.yml@refs/tags/v.*" `
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" `
  SHA256SUMS.txt
```

Windows では `sha256sum` がない環境もあるため、PowerShell で個別に照合してよい。

```powershell
Get-FileHash .\any-ai-cli.exe -Algorithm SHA256
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
gh run list --repo ishizakahiroshi/any-ai-cli --workflow=Release --status=in_progress --limit 5
gh run cancel <run-id> --repo ishizakahiroshi/any-ai-cli
```

draft / 公開直後の Release を削除する（`--cleanup-tag` でリモートタグも一緒に消える）。

```powershell
gh release delete v0.1.1 --repo ishizakahiroshi/any-ai-cli --yes --cleanup-tag
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

- `windows/amd64`（`any-ai-cli.exe` / `any-ai-cli-launcher.exe`）
- `linux/amd64`
- `darwin/amd64`
- `darwin/arm64`

Linux / macOS を未検証のまま添付する場合は、README と release note に未検証であることを明記する。Linux arm64 や Windows arm64 を追加する場合は、GoReleaser の `ignore` 設定も見直す。
