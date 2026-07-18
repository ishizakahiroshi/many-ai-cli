# Quy trình release many-ai-cli

> Bản dịch tiếng Việt của [`manual_release.md`](manual_release.md).  
> Bản dịch: 2026-07-18.

Tài liệu vận hành lâu dài để tạo GitHub Releases bằng workflow `Release` của GitHub Actions và GoReleaser.  
**Checklist chạy cho từng version** chuẩn bị riêng (v0.3.0: `docs/local/manual_release-v0-3-0_2026-06-13.md`). Tài liệu này chỉ xử lý phương pháp · thiết kế · lưu ý **không phụ thuộc version**.

v0.1.0 coi là release thử; release chính thức đầu là v0.1.1. Tên project đổi từ `any-ai-cli` sang `many-ai-cli` từ v0.3.0 (binary · config · env · tên npm package đều đổi).

## Bẫy và phòng tái phát (vấn đề thật khi public v0.3.0)

> Public v0.3.0 lộ liên tiếp vấn đề sau tag, kéo dài cỡ 3 giờ. **Lần sau đọc mục này trước, dẹp trước khi tag**. «✅ đã sửa» = đã phản ánh workflow/cấu hình; «⚠️ chưa xong» = việc còn tới lần sau.

### Quan trọng nhất: trước tag phải có «CI green trên mọi OS»

- Workflow `Release` **không gọi test**. **Push tag = build + public ngay**. Nội dung tag chưa qua CI → lỗi build/test/lint lộ **sau tag**, vòng thu hồi lặp.
- `Validate` chỉ chạy trên **`push: main` và `pull_request`**. **Push develop không chạy multi-OS Validate**. Chỉ verify local (Windows) trên develop rồi tag / merge thẳng main → **lỗi riêng linux/macOS lộ sau**. Thực tế v0.3.0:
  - `gosec` G703 (file unix như `pty_unix.go`; local Windows file khác nên không bắt)
  - `staticcheck` ST1018 (ký tự ẩn U+00AD trong `output_encoding_other_test.go`)
  - Fail `go test` phụ thuộc OS (`open_browser_test.go` phán đoán WSL, `whisper_manage_test.go` nhánh platform)
  - Diff `go mod tidy` → kéo `THIRD_PARTY_NOTICES.md` cũ
- Biện pháp (bắt buộc một trong các cách):
  - Commit release ra **PR vào main, xác nhận `Validate` (ubuntu/windows/macOS) green** rồi mới tag. Đây là đường chính.
  - Tối thiểu local: `GOOS=linux go build/vet ./...` (compile file unix build tag). Có thể thì WSL/Linux: `go test ./...` + `staticcheck` + `gosec` **cho mọi OS**. Chỉ Windows local = phía unix không được verify gì.
  - Trước tag: `go run github.com/rhysd/actionlint/cmd/actionlint@latest .github/workflows/*.yml` + `goreleaser check` + `goreleaser release --snapshot --clean` (tới giai đoạn build, gồm before hook).

### Mìn goreleaser / workflow

- **frontend embed (go build fail)**: `//go:embed all:dist` trong `web/web.go` cần `web/dist` trước go build. Before hook `.goreleaser.yaml` **phải** build frontend. ⚠️ `bun --cwd web run build` trong goreleaser hook **thành no-op** (web/dist trống → build → `pattern all:dist: no matching files found`) — không dùng. ✅ Đã sửa thành `sh -c 'cd web && bun install --frozen-lockfile && bun run build'` (`before.hooks` **chỉ chuỗi** · map `{cmd, dir}` không được ở v2.16. Release chạy ubuntu → `sh -c` OK).
- **startup_failure release.yml**: `if: ${{ secrets.X != '' }}` ở step **không được** (`secrets` không dùng trong if context → "Unrecognized named-value: 'secrets'" → cả workflow fail lúc start). ✅ Nâng lên `env:` cấp job, `if: ${{ env.X != '' }}`. `actionlint` bắt được **trước tag**.
- **Release note không đọc được**: auto changelog goreleaser là liệt kê commit message (auto Git tab kiểu «cập nhật web» «phản ánh thay đổi internal»…) — vô giá trị làm release note. ✅ Đổi: `changelog.disable: true` + trích mục version trong CHANGELOG.md truyền `--release-notes`. ⚠️ **Lần đó `--release-notes` không hiệu lực**, body GitHub Release trống (hiện commit message của tag), phải `gh release edit v0.3.0 --notes-file <trích CHANGELOG>`. **Workflow chưa xong = lần sau thêm step sau goreleaser: `gh release edit --notes-file`**.
- Liên quan: đổi `before.hooks` goreleaser thì xác nhận `goreleaser release --snapshot --clean` **thật sự chạy before hook** (snapshot trên Windows có thể thiếu `sh` và hook fail — cuối cùng verify trên CI hoặc WSL).

### Mìn phân phối npm

- **Hiểu sai publish path**: `npm publish "npm/<pkg>"` bị npm hiểu nhầm **GitHub shorthand (`owner/repo`)** → thử `git ls-remote ssh://git@github.com/npm/<pkg>.git` → fail `Permission denied`. ✅ Luôn **`npm publish "./npm/<pkg>"`** (có `./` đầu).
- **Token phải có quyền publish**: token read-only → publish fail **`E429 "Could not publish, as user undefined: rate limited exceeded"`** (thiếu quyền trả bằng **câu rate limit gây nhầm**. `npm whoami` pass vẫn publish không được nếu thiếu quyền). Granular token: **Permissions = Read and write / Packages = All packages**, hoặc Classic **Automation**. Secret CI `NPM_TOKEN` cùng yêu cầu. Granular hết hạn tối đa 90 ngày → vận hành CI dài hạn nên Classic Automation (có thể vô hạn).
- **E429 thật (rate limit)**: publish dồn trong thời gian ngắn → npm khóa. **Thử liên tục phản tác dụng khi reset cửa sổ**. Cách 15–20 phút, mỗi lần 1 lần. Step npm publish trong release.yml cần **retry/backoff** (v0.3.0 chưa = ⚠️ việc còn).
- **Không publish platform package từ Windows**: `npm pack` trên Windows **không gắn execute bit unix** (trong tarball `-rw-r--r--` = 0644). Root shim `spawnSync(binary)` exec trực tiếp → package **linux/macOS fail start (EACCES)**. **Đường đúng: publish trên CI (Linux)**. Khôi phục tay: **WSL(ext4) `chmod 0755` rồi `npm pack`/`publish`** (trên /mnt/c chmod đôi khi không ăn — dùng native fs). Expand trên Windows · pack/publish trên WSL.

### Khác

- **Khớp tên repo**: `.goreleaser.yaml` / docs / certificate-identity cosign / winget / Homebrew giả định `ishizakahiroshi/many-ai-cli`. **Trước tag** xác nhận `gh repo view` khớp tên GitHub repo (đã rename?). Ngay trước v0.3.0 repo còn `any-ai-cli` → mọi thứ gh 404.
- **Tag immutable**: fail cũng chỉ tái dùng tag khi «download asset public = 0». Có download → patch kế. Thu hồi: `gh release delete <tag> --cleanup-tag --yes`. Xuất lại winget: xóa branch tương ứng trên fork (= PR tự close) → re-run để goreleaser tạo lại.

### Vấn đề khi public v0.3.2 (fail main không thấy trên develop + dọn dẹp)

v0.3.2 chỉ nhìn `develop` trước tag → fail CI lần đầu lộ lúc push main, 3 tầng, mỗi lần cần fixup commit. Lần sau: **«trước tag» = đưa được vào main, không chỉ xong trên develop**.

- **Phạm vi scan secret-scan / Validate khác develop vs main**: secret-scan (gitleaks) quét từ base push tới HEAD. develop push thường xuyên → chỉ diff sau push trước; main quét gộp hàng chục commit từ release trước → **lỗi commit cũ chỉ lộ khi vào main**. Validate cũng chỉ `push: main` + `pull_request` (đã nói). ⚠️ Biện pháp (nhắc lại quan trọng nhất): **PR release commit vào main, green Validate + secret-scan trên main rồi mới tag**. Green trên develop không đáng tin.
- **gitleaks false positive generic-api-key trên «bí mật giả» trong `_test.go`**: test `MaskSecrets()` với `PASSWORD=topsecretvalue` kiểu **cố ý** kích generic-api-key. Đầu chỉ allowlist từng file → test file khác lại dẫm, whack-a-mole. ✅ Thêm `_test\.go$` vào global allowlist `.gitleaks.toml` (`_test.go` không vào production build, không đường rò). Test mới cũng yên.
- **`filepath.ToSlash` chỉ đổi PathSeparator của OS runtime**: runtime Linux `os.PathSeparator == '/'` nên test data path Windows có `\` **không đổi, lọt thẳng** → test hook shell POSIX (`TestCodexStopHookBlock_QuotesSpaceyExePath`, v.v.) fail trên Linux CI. ✅ `toShellPath` trong `internal/wrapper/usage_hooks.go` dùng `strings.ReplaceAll(p, "\\", "/")`. Nguyên tắc: **chuyển `\` → `/` cho shell/POSIX không dùng `filepath.ToSlash`**.
- **Phán đoán cross-OS kiểu broad-root dựa `os.UserHomeDir()` động sẽ sót**: spawn cwd quá rộng lấy động `/home` làm «parent mọi user»; macOS CI HOME=`/Users/runner` → `filepath.Dir(home)` = `/Users` → **sót `/home`**. ✅ Liệt kê tường minh `/home` `/Users` `C:\Users` bất kể OS. Cần **cả** resolve động **và** liệt kê tường minh.
- **Không nhét tàn MVP «dự định chưa dùng» vào commit release**: `internal/proxy/proxy.go` mới thêm còn **const/field/func dở** (`hdrAuthorization`, field `sinkMu`, func `stripAuthHeaders`) → staticcheck U1000 nhiều chỗ. ⚠️ Biện pháp: local bắt buộc trước tag thêm `go run honnef.co/go/tools/cmd/staticcheck@latest ./...`. Chỉ `go build ./...` không bắt unused.
- **`THIRD_PARTY_NOTICES.md` và link refs `CHANGELOG.md` phải cập nhật tay**: cái trước — `go.sum` đổi thì regenerate (v0.3.2 `golang-jwt/jwt/v5` v5.2.1→v5.2.2 lệch → Validate fail). Cái sau — mỗi lần cập nhật `[Unreleased]: compare/vX.Y.Z...HEAD` và thêm dòng `[NEW.VER]: compare/...` (v0.3.2 quên → patch commit sau). ⚠️ Biện pháp: checklist «trước tag» ghi rõ «chạy `scripts/local/gen-third-party-notices.ps1` rồi `git status` clean» và «thêm `[NEW.VER]` vào link refs cuối CHANGELOG.md».
- **Dọn npm zombie (rác thử publish)**: lần publish đầu ra 0.0.1 «giữ chỗ» → sau production vẫn thấy 0.0.1 trong `npm view <pkg> versions`. Tag `latest` trỏ bản mới nên user không ảnh hưởng, nhưng nên chặn cho sạch: **`npm deprecate <pkg>@0.0.1 "Use <pkg>@<NEW.VER> or later. ..."`**. **Tài khoản 2FA: mỗi `npm deprecate` mở web auth trình duyệt** (không phải OTP — flow Authorize web) → mở URL Authorize → Enter terminal, lặp **số version × số package**. Token granular publish:write bỏ qua được nhưng có hạn (90 ngày).
- **Không viết winget «đã public»**: PR 3 release (v0.3.0 / v0.3.1 / v0.3.2) **vẫn OPEN** trên microsoft/winget-pkgs (chờ moderator New-Package). GoReleaser tạo PR nhưng merge do Microsoft duyệt tay — phía mình không đẩy được. README / note / thông báo phân phối viết «cài bằng winget» = **sai**. Ghi «đang xin» hoặc bỏ hẳn dòng winget, hướng dẫn 3 đường npm / Homebrew / tải thẳng GitHub Release. Merge xong mới bổ sung.

## Vận hành nhánh và backport hotfix về develop (đã tự động hóa)

Project dùng 2 nhánh main / develop.

- **main** = bản release ổn định. Tag (`vX.Y.Z`) **luôn** trên main.
- **develop** = nơi phát triển version kế. Xong thì merge main rồi tag.
- **hotfix** = sửa khẩn bug trên main. Nhánh từ main, sửa, về main, tag patch mới (ví dụ `v0.3.3`).

### Auto backport (từ v0.3.4)

Push tag `v*.*.*` kích hoạt `.github/workflows/backport-to-develop.yml`, tự merge main → develop. Thủ công về develop về nguyên tắc không cần:

- Merge sạch → push thẳng develop
- Conflict → tạo issue báo người (release competition hiếm nhưng xảy ra khi CHANGELOG `## [Unreleased]` và mục release mới sửa đồng thời)

Sự cố cũ: v0.3.3 quên đưa hotfix về develop → build develop hiện v0.3.2. v0.3.4 chạy `git merge hotfix/v0.3.4` trên develop → nội dung vào nhưng commit tag v0.3.4 (`193f94f`) không là tổ tiên develop → lại hiện v0.3.3 cũ. **Tự động hóa ra đời để chặn 2 vụ này**.

### Cấm

**Sau khi merge hotfix vào main, không merge trực tiếp nhánh hotfix vào develop** (không `git merge hotfix/*` trên develop). Nội dung giống nhưng không đưa merge commit phía main (đã gắn tag v0.3.4) làm tổ tiên develop → `git describe` không nhận tag mới → version local build kẹt bản cũ. Về develop **luôn chỉ định main** (`git merge main`). Workflow auto backport cũng dùng `git merge origin/main` — thường không cần đụng.

### Fallback thủ công khi auto backport không chạy

CI tạm tắt, hoặc backport workflow fail:

```powershell
git switch develop
git merge main --no-edit   # conflict CHANGELOG: giữ cả hai mục (Unreleased và mục release)
git push origin develop
```

Xác nhận đã về (tag mới nhất là tổ tiên develop = OK):

```powershell
git merge-base --is-ancestor (git describe --tags --abbrev=0 main) develop
# exit 0 = OK / 1 = develop chưa nhận tag mới nhất
```

### Phát hiện bất thường bằng version local build (lưới an toàn kép)

`Makefile` mỗi `make build` inject `git describe --tags --always --dirty` vào `-X main.version`. develop quên nhận tag mới nhất main → Hub UI dạng `0.3.3-21-g3664cac` («xa tag») — nhìn thấy. Backport sạch → dạng `0.3.4-N-g<sha>` từ tag gần. **Version trên modal settings Hub UI trỏ «major cũ» → trước hết `git merge-base --is-ancestor v<latest> develop`**.

## Cách release hiện tại

- Push tag `v*.*.*` → `.github/workflows/release.yml` chạy
- Workflow chạy GoReleaser v2 trên Ubuntu
- GoReleaser theo `.goreleaser.yaml` tạo artifact
- Target build hiện tại: `windows/amd64`, `linux/amd64`, `darwin/amd64`, `darwin/arm64`. **Gói cả 2 binary** trên mọi OS: `many-ai-cli` + launcher tích hợp `many-ai-cli-launcher` (v0.3.0 cross-platform hóa launcher; trước đó launcher chỉ windows)
- Checksum: `SHA256SUMS.txt`
- File checksum ký cosign keyless
- SBOM từng archive (SPDX JSON, syft) cũng generate và đính kèm

Kênh phân phối GoReleaser chạy song song (`.goreleaser.yaml`):

| Kênh | Đối tượng | Secret cần |
|---|---|---|
| GitHub Releases (zip / SHA256SUMS / cosign / SBOM) | Mọi OS | `GITHUB_TOKEN` + `id-token: write` |
| npm registry (root + 4 platform package) | Mọi OS (hậu release.yml) | `NPM_TOKEN` (chưa set → skip) |
| winget (`ishizakahiroshi/winget-pkgs` fork→PR upstream) | Windows x64 | `PUBLISH_GITHUB_TOKEN` |
| Homebrew cask (`ishizakahiroshi/homebrew-tap`) | macOS | `PUBLISH_GITHUB_TOKEN` |
| deb / rpm (nfpms, binary chính + launcher) | Linux x64 | không |

### Điều kiện bật native signing · notarization

Windows Authenticode và macOS notarization bật sau khi chốt certificate · signing backend và cấu hình CI. **Hiện chưa bật**; package manager, checksum, cosign, `unblock-windows.cmd` **không** thay Authenticode / Gatekeeper.

Sau khi bật, release workflow tuân:

1. Windows: ký Authenticode + verify timestamp cả hai `.exe` phân phối **trước** archive.
2. macOS: Developer ID (hardened runtime) → `notarytool submit --wait` → `stapler` → verify `codesign` / `spctl` **xong trước** archive.
3. Sau archive mới tạo `SHA256SUMS.txt` và chữ ký cosign. Platform package npm lấy từ archive GitHub Release canonical, verify cùng checksum.
4. Release đã bật ký mà verify fail → fail closed; **không** public asset unsigned cùng tag.

CI chỉ chứa **tên secret**; không ghi PFX/p12, password, Apple private key, Apple ID vào repo · release md · Actions log. Nguồn certificate (OV/EV) · cách lưu · chọn và nạp credential Apple là việc người vận hành còn giữ.

Sau release, xác nhận asset đã expand (CI cũng bắt buộc verify tương đương):

```powershell
Get-AuthenticodeSignature .\many-ai-cli.exe
Get-AuthenticodeSignature .\many-ai-cli-launcher.exe
```

```sh
codesign --verify --deep --strict many-ai-cli
spctl --assess --type execute --verbose=4 many-ai-cli
xcrun stapler validate many-ai-cli
```

Tag prerelease (`v0.3.0-rc.1`, v.v.): npm dist-tag `next`; winget/homebrew `skip_upload: auto` không push.

File chính dự kiến đính kèm release:

- `many-ai-cli-<version>-windows-x64.zip` (`many-ai-cli.exe` / `many-ai-cli-launcher.exe` / `unblock-windows.cmd`)
- `many-ai-cli-<version>-linux-x64.zip`
- `many-ai-cli-<version>-macos-intel.zip`
- `many-ai-cli-<version>-macos-apple-silicon.zip`
- `SHA256SUMS.txt`
- `SHA256SUMS.txt.sig`
- `SHA256SUMS.txt.pem`

## Binary Windows chưa ký và unblock helper

Zip Windows kèm `unblock-windows.cmd`. Sau expand, user chạy trong thư mục expand → PowerShell `Unblock-File` lên `many-ai-cli*.exe` cùng thư mục.

Phạm vi và giới hạn helper:

- Chỉ exe kèm theo (`many-ai-cli.exe` / `many-ai-cli-launcher.exe`, v.v. `many-ai-cli*.exe`)
- Không yêu cầu admin
- Không đổi vĩnh viễn execution policy bằng `Set-ExecutionPolicy`
- Không auto-start app
- Chủ yếu gỡ cảnh báo/block do Mark-of-the-Web; **không** né reputation SmartScreen, block hoàn toàn Smart App Control, AppLocker / WDAC / EDR / antivirus, v.v. (policy tổ chức)

README phân biệt:

- Mark-of-the-Web: đối tượng chính `unblock-windows.cmd` cải thiện được
- SmartScreen: cảnh báo publisher lạ · lịch sử dùng. Khác verify checksum / cosign
- Smart App Control: một số môi trường Windows 11 block hoàn toàn exe chưa ký. Phân phối unsigned không có workaround hỗ trợ
- PC quản lý tổ chức: theo AppLocker / WDAC / EDR, v.v. **Không** hướng dẫn tắt tính năng bảo mật

Quy trình zip Windows khuyến nghị:

1. Tải `many-ai-cli-<version>-windows-x64.zip` từ GitHub Releases
2. Nếu cần, verify `SHA256SUMS.txt` / chữ ký cosign
3. Expand zip
4. Chạy `unblock-windows.cmd`
5. Start tay `many-ai-cli.exe` hoặc `many-ai-cli-launcher.exe`

## Sắp xếp kênh phân phối miễn phí

Ưu tiên ngắn hạn:

1. Package developer install trên npm registry; README khuyến nghị `pnpm add -g many-ai-cli`
2. Manifest winget để user Windows tìm bằng tool chuẩn
3. Giữ GitHub Releases zip + checksum / cosign + `unblock-windows.cmd` làm đường tay
4. Scoop bucket cân nhắc thêm cho user CLI
5. Chocolatey để sau, theo nhu cầu và chi phí bảo trì

Trên Windows, ưu tiên package manager OS chuẩn hoặc cho user CLI hơn tải zip/exe trực tiếp bằng trình duyệt. Lý do:

- Zip/exe qua trình duyệt dễ dính Mark-of-the-Web → SmartScreen / Smart App Control / policy tổ chức
- `pnpm add -g` / `bun install -g` / `npm install -g` tránh đường tải exe bằng trình duyệt; shim lệnh global sinh local
- Package manager = cài rõ ràng bằng tool chuẩn → dễ tìm · cập nhật · tái lập
- Package manager **không** thay Authenticode. Cảnh báo publisher lạ, block Smart App Control, AppLocker / WDAC / EDR vẫn là vấn đề riêng
- Hub `many-ai-cli` bind `127.0.0.1` — thiết kế **không** yêu cầu ngoại lệ Windows Firewall public

Chính sách đăng ký ecosystem ngôn ngữ khác:

- Publish lên npm registry. Không có nghĩa khuyến nghị lệnh `npm` — pnpm / bun / yarn cũng lấy từ cùng registry
- Lệnh khuyến nghị README: `pnpm add -g many-ai-cli`, **không** `npm install -g many-ai-cli`. `npm install -g` chỉ ghi nhỏ làm fallback tương thích
- `bun install -g many-ai-cli` có thể ghi kèm cho user Bun; Bun vẫn là tool chuẩn frontend build trong repo
- Ưu tiên gói npm optional theo platform kèm binary Go. Tránh wrapper download exe sau từ GitHub Releases lúc install (phân tán trách nhiệm · đường cập nhật · giải thích bảo mật)
- Windows chuẩn chính thức: winget; developer primary: pnpm; tải tay: GitHub Releases

Phạm vi cải thiện bằng free:

- Xác nhận nguồn và tính toàn vẹn (GitHub Releases, SHA256SUMS, cosign)
- Giảm khó start do Mark-of-the-Web (`unblock-windows.cmd`)
- Khám phá · đường cập nhật qua package manager (winget / Scoop)

Free distribution **không** giải quyết:

- Chứng minh publisher bằng Authenticode
- Môi trường Smart App Control block hoàn toàn exe chưa ký
- Policy AppLocker / WDAC / EDR / antivirus trên PC tổ chức

Biện pháp gốc cho Smart App Control và policy tổ chức: code signing sau này hoặc allowlist phía tổ chức — unblock helper / package manager manifest **không** thay được.

## Phân phối npm registry (package many-ai-cli)

Đường khuyến nghị developer: `pnpm add -g many-ai-cli`. Chỉ publish npm registry là tránh trải nghiệm tải exe trình duyệt (MotW/SmartScreen). Không tạo standalone exe; binary Go từng OS/arch gói trong optional dependency package theo platform (`many-ai-cli-<os>-<arch>`).

### Cấu trúc

- `npm/many-ai-cli/`: root shim. `bin/many-ai-cli.mjs` resolve platform package từ `process.platform`/`process.arch` rồi exec binary kèm (argv/stdio/exit code trong suốt). `optionalDependencies` pin **exact version** 4 platform package.
- `npm/many-ai-cli-<os>-<arch>/`: khai `os`/`cpu`. Binary trong `bin/` gitignore (stage lúc release).
- Tên platform package không scope (không cần npm org).

### Auto publish trong release workflow (`.github/workflows/release.yml`)

Sau `release --clean` GoReleaser thành công, cùng job (secret `NPM_TOKEN` chưa set → skip mọi step, không phá release hiện có):

1. `node scripts/stage-npm-binaries.mjs` — từ `dist/artifacts.json` đặt binary `many-ai-cli` từng OS/arch vào `npm/*/bin/` (non-Windows gắn execute bit).
2. `node scripts/sync-npm-version.mjs "<tag>"` — cập nhật đồng loạt version mọi `npm/*/package.json` và optionalDependencies root từ tag (chống drift).
3. `npm publish` — platform package trước, root sau. Có `--access public --provenance`. Tag prerelease (có `-`) → dist-tag `next`; stable → `latest`.

### Verify local

- `node scripts/sync-npm-version.mjs 0.3.0` đồng bộ version.
- `node scripts/smoke-npm.mjs` verify nội dung pack (chỉ shim+binary+metadata). Nếu đã có binary trong `dist/` (ví dụ `make build`), sau `stage-npm-binaries.mjs` cũng verify `--version`/`version` qua shim OS hiện tại.
- Smoke install global (đường shim `.cmd`) đổi trạng thái global → user chạy tay: `pnpm add -g ./npm/many-ai-cli/*.tgz` → `many-ai-cli --version`.

### Chuẩn bị phía user

- Tài khoản npm publish được `many-ai-cli` (tên package đã reserve).
- Secret GitHub Actions `NPM_TOKEN` (khuyến nghị automation token. Provenance: workflow đã cấp `id-token: write`).
- Public v0.3.0 rồi mới npm publish sau: `workflow_dispatch` hoặc exact tag publish cùng version; **không** thay zip GitHub Release hiện có. Checksum lệch → chuyển v0.3.1.

## Xác nhận trước khi tag

Trước release, kiểm tra worktree:

```powershell
git status --short
```

Có thay đổi chưa commit → chỉ commit phần thuộc release. Không tag khi lẫn thay đổi ngoài ý.

Version string: binary Go và tên archive release lấy tag GoReleaser làm **single source of truth**. Chi tiết mục sau «Thiết kế single source cho chuỗi version».

Tài liệu public và Windows resource template người cập nhật mỗi release vẫn còn — xác nhận trước tag.

Cái «tag → tự theo»:

- Chuỗi version trong binary Go (`main.version`, inject ldflags)
- Version trên Web UI (runtime fetch qua `/api/info`)
- Tên zip và SHA256SUMS trên GitHub Releases (GoReleaser từ tag)
- FileVersion / ProductVersion Windows `.syso` do before hook GoReleaser sinh

Cái xác nhận / cập nhật tay trước release:

- `CHANGELOG.md` (chốt `[Unreleased]` thành mục version mới, cập nhật compare link)
- `README.md` / `README.ja.md` (tính năng mới, trạng thái verify, tên artifact, giải thích bảo mật)
- `winres/winres.json` / `winres/winres-launcher.json` (template version identity manifest, v.v.)
- `THIRD_PARTY_NOTICES.md` / `web/src/vendor/THIRD_PARTY_LICENSES.txt` (phụ thuộc · license vendored)

Linux / macOS chưa verify mà vẫn phát hành: đảm bảo trạng thái verify README không mâu thuẫn target build `.goreleaser.yaml`.

## Thiết kế single source cho chuỗi version

Thiết kế hiện tại:

```
git tag v0.3.0  ── push ──┐
                          │
                  goreleaser lấy "0.3.0" từ tag
                          │
                          ├── ldflags: -X main.version=0.3.0
                          │      └─ trong binary Go
                          │            └─ field "version" của /api/info
                          │                  └─ Web UI runtime fetch và hiện
                          │
                          ├── tên archive: many-ai-cli-0.3.0-{os}-{arch}.zip
                          │      └─ cùng SHA256SUMS lên Release page
                          │
                          └── go-winres before hook
                                 └─ FileVersion / ProductVersion Windows .syso
```

**Chỗ implement**

- `cmd/many-ai-cli/main.go`: `var version = "dev"` (package-level, đích inject ldflags)
- `.goreleaser.yaml`: `builds.[].ldflags` thêm `-X main.version={{.Version}}`
- `internal/hub/server.go`: `NewServer` nhận `version string`, đưa vào JSON `/api/info`
- `web/src/app.ts`: lúc start fetch `/api/info`, đổ vào `.settings-app-version` và `.about-version`
- `web/src/i18n/{ja,en}.json` `about_version`: không hardcode số version — placeholder kiểu `"Version {0} [Hub UI]"`
- `web/src/index.html`: không ghi số version; trống hoặc skeleton (runtime ghi đè)

`go-winres` chạy trong before hook `.goreleaser.yaml`. `.syso` là artifact generate ngoài git; version tag inject bằng `--product-version={{ .Version }}` / `--file-version={{ .Version }}` của hook.

## Xác nhận diff tài nguyên runtime-served (slash command, v.v.)

Các file dưới `resources/` **độc lập** tag/binary release; runtime raw-fetch từ nhánh `main` GitHub (không rebuild · tốn 0 token khi cập nhật. Implement: `Default*Source` trong `internal/config/config.go`, `internal/hub/slash_cmd_fetch.go`, v.v.).

- `resources/slash-commands/claude.md` / `codex.md` / `copilot.md` / `cursor-agent.md` / `opencode.md`
- `resources/approval-patterns/claude.md` / `codex.md` / `copilot.md` / `common.md`
- `resources/usage-links/defaults.json`
- `resources/models/defaults.json`

Nguồn: `https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/...`. Tức **push `main` = nguồn mọi user đổi ngay** (phản ánh dần trong TTL cache 24h phía Hub). Live, không phụ thuộc version binary — trước khi đưa commit release vào `main`, **bắt buộc** so diff worktree local với phần public trên GitHub `main`.

> **Freshness và drift là hai việc khác**: diff dưới đây xem «local có lệch public không (drift)»; «`resources/slash-commands/*.md` có cũ so với lệnh gốc provider (freshness)» là chuyện khác.
>
> Freshness đã gắn precheck skill `release`: khi `repo == many-ai-cli`, **preflight** `slash-commands-update` chạy — xác nhận có `docs/local/slash-command-freshness_YYYY-MM-DD.md` mới nhất trong hạn, **không còn diff chưa quyết (`decision = pending`)**. Còn pending → không cho tag (block tới khi ghi accepted / deferred: lý do vào release md). `unknown` (không lấy source · opencode, v.v.) mặc định chỉ cảnh báo.
>
> Muốn nhận lệnh add/remove gốc trước release: chạy mode `report` của `slash-commands-update` trước (C1–C2 trong [manual_slash_commands_update.md](manual_slash_commands_update.md)), quyết accepted/deferred, `apply` vào `resources/slash-commands/*.md`, rồi mới qua kiểm tra drift dưới. Kết quả freshness ghi bảng «Slash command freshness» trong `## 申し送り` (handover) của release md.

So diff slash command (PowerShell):

```powershell
foreach ($p in 'claude','codex','copilot','cursor-agent','opencode') {
  $remote = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/$p.md"
  $gh = (Invoke-WebRequest -UseBasicParsing $remote).Content
  $local = Get-Content -Raw "resources/slash-commands/$p.md"
  if ($gh -eq $local) { "$p : 差分なし" }
  else { "$p : 差分あり"; Compare-Object ($gh -split "`n") ($local -split "`n") }
}
```

bash:

```bash
for p in claude codex copilot cursor-agent opencode; do
  curl -s "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/$p.md" -o "/tmp/gh_$p.md"
  diff "/tmp/gh_$p.md" "resources/slash-commands/$p.md" && echo "$p: 差分なし"
done
```

Phán đoán:

- Dòng diff có phải cố ý cho release này không (thêm/xóa command · sửa mô tả, v.v.).
- Diff cố ý → đưa vào commit release, push `main` là live cùng lúc public.
- Diff ngoài ý (sửa nhầm local · quên revert) → giải trước push.
- approval-patterns / usage-links / models cùng cơ chế — release đụng chúng thì so diff tương tự.

Sửa trước trên develop là bình thường: public chỉ khi merge `main`; worktree develop lệch public `main` là đúng (release sẽ giải).

## Verify local

Tối thiểu trước tag. Xác nhận directive `go` trong `go.mod` là baseline release v0.3.0 (hiện `go 1.25.11`).

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

Môi trường có CGO và C compiler: chạy thêm race detector. Local Windows không `gcc` → chạy WSL/Linux/CI khác, ghi kết quả vào precheck.

```powershell
go test -race ./...
```

Có `goreleaser` local thì check cấu hình:

```powershell
goreleaser check
```

Không có `goreleaser` local → có thể còn lỗi cấu hình chỉ lộ lần đầu trên GitHub Actions. Nên cài và check dù chỉ sát release.

Có thay đổi Go dependency cần cập nhật `THIRD_PARTY_NOTICES.md`: `scripts/local/check-third-party.ps1` local phải pass (tránh fail CI chặn release).

```powershell
.\scripts\local\check-third-party.ps1
```

Browser library vendored ngoài `npm audit` — khớp entity đã cập nhật · `web/src/vendor/THIRD_PARTY_LICENSES.txt` · version trên About. Defer cập nhật v0.3.0: ghi kết quả xác nhận lỗ hổng đã biết + lý do defer vào `docs/local/bugfix_v0-3-0-release-precheck_YYYY-MM-DD.md`.

Hub UI smoke tối thiểu: spawn (gồm Ollama Cloud route), reconnect, Files preview, path open, Git tab, approval action bar, settings save — ghi cùng precheck.

## Cập nhật CHANGELOG.md

Mỗi release thêm 1 mục tay. Giữ dạng Keep a Changelog; chốt thay đổi trong `[Unreleased]` thành mục version mới. Format tối thiểu:

```markdown
## [0.1.x] - YYYY-MM-DD

### Added
- Tính năng mới / thay đổi thiết kế

### Fixed
- Sửa bug

### Changed
- Thay đổi hành vi hiện có (có thể ảnh hưởng tương thích)

### Removed
- Tính năng / API đã xóa (nếu có)
```

Cập nhật compare link cuối:

```markdown
[Unreleased]: https://github.com/ishizakahiroshi/many-ai-cli/compare/v0.1.x...HEAD
[0.1.x]: https://github.com/ishizakahiroshi/many-ai-cli/releases/tag/v0.1.x
```

GoReleaser gắn thêm auto-generated changelog từ commit message lên Release page (cấu hình `changelog` trong `.goreleaser.yaml`). `CHANGELOG.md` = người đọc; auto-generated = commit list forensics — tách mục đích.

## Xác nhận Validate workflow green trước

Trước tag, **bắt buộc** xác nhận workflow `Validate` trên commit mới nhất `main` là green. Tag khi Validate fail → workflow `Release` (goreleaser) không gọi test vẫn pass → public binary mang test fail.

```powershell
gh run list --repo ishizakahiroshi/many-ai-cli --workflow=Validate --limit 1
```

Status `success` rồi mới tag. Còn `failure` → sửa Validate trước.

## Commit và push

Commit thay đổi thuộc release, push `main`.

```powershell
git add .
git commit -m "chore: v0.1.1 リリース準備"
git push origin main
```

Đã commit sẵn: chỉ cần `git status --short` trống.

## Tạo tag

Tạo tag local:

```powershell
git tag v0.1.1
```

Xác nhận commit tag trỏ tới:

```powershell
git show --stat v0.1.1
```

Ổn thì push tag:

```powershell
git push origin v0.1.1
```

Push này kích hoạt workflow `Release` trên GitHub Actions.

## Xác nhận GitHub Actions

Tab Actions GitHub: workflow `Release`.

Điều kiện thành công:

- `actions/checkout` thành công
- `actions/setup-go` thành công
- `sigstore/cosign-installer` thành công
- `goreleaser/goreleaser-action` thành công

Fail: xem log, commit sửa. **Không** tái dùng cùng tên tag; nguyên tắc patch kế `v0.1.2` sau khi sửa.

## Xác nhận sau release

Xác nhận GitHub Releases đã tạo version đích.

Checklist:

- release title / tag đúng version ý định
- Có zip đính kèm
- Có `SHA256SUMS.txt`
- Có `SHA256SUMS.txt.sig`
- Có `SHA256SUMS.txt.pem`
- Quy trình verify README khớp tên file đính kèm thực tế

Nếu cần, verify artifact đã tải:

```powershell
cosign verify-blob `
  --certificate SHA256SUMS.txt.pem `
  --signature SHA256SUMS.txt.sig `
  --certificate-identity-regexp "https://github.com/ishizakahiroshi/many-ai-cli/.github/workflows/release.yml@refs/tags/v.*" `
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" `
  SHA256SUMS.txt
```

Windows có thể không có `sha256sum` — so từng file bằng PowerShell:

```powershell
Get-FileHash .\many-ai-cli.exe -Algorithm SHA256
```

## Xử lý khi fail

Workflow fail sau push tag:

- Chưa tạo public release → sửa nguyên nhân, ra bằng tag kế
- GitHub Release đã tạo nhưng artifact incomplete → đưa draft hoặc xóa Release, ra bằng tag kế
- Public rồi có thể user đã lấy → **không** thay cùng tag; làm patch version kế

Nguyên tắc: tránh force push tag đã public và thay artifact.

### Điều kiện và quy trình tái dùng cùng tag

Chỉ khi user **chưa** lấy, và Release còn draft / vừa thu hồi. Căn cứ:

- star / fork / clone traffic repo = 0
- Release `draft: true`, hoặc ngay sau `gh release delete`, chưa có public thực tế
- Ngay sau release (trong vài chục phút)

Không đủ → không tái dùng tag; sửa rồi ra patch `v0.1.2`, v.v.

#### Lệnh thu hồi + ra lại

Hủy run Release đang chạy nếu có:

```powershell
gh run list --repo ishizakahiroshi/many-ai-cli --workflow=Release --status=in_progress --limit 5
gh run cancel <run-id> --repo ishizakahiroshi/many-ai-cli
```

Xóa Release draft / vừa public (`--cleanup-tag` xóa luôn remote tag):

```powershell
gh release delete v0.1.1 --repo ishizakahiroshi/many-ai-cli --yes --cleanup-tag
```

`--cleanup-tag` không xóa hết, hoặc không còn Release chỉ còn tag:

```powershell
git push origin :refs/tags/v0.1.1
git tag -d v0.1.1
```

Thêm commit sửa → Validate green → gắn lại cùng tag → push → Release workflow chạy lại.

## Lưu ý line ending (CRLF / LF)

`.gitattributes` cố định `eol=lf` cho `THIRD_PARTY_NOTICES.md` và `web/src/vendor/THIRD_PARTY_LICENSES.txt`. Local Windows thường `core.autocrlf=true` (Git for Windows) → worktree thành CRLF → lệch byte với output LF của `scripts/local/gen-third-party-notices.ps1` → CI fail.

Thêm text «auto-generate» dưới repo: luôn thỏa một trong:

- Generator chuẩn hóa LF khi ghi (mẫu `WriteAllText` + ghép LF trong `scripts/local/gen-third-party-notices.ps1`)
- Thêm `<path> text eol=lf` vào `.gitattributes`

PowerShell (`*.ps1`) ngược lại cố định CRLF (dễ đọc trên Windows).

## Cập nhật Actions dùng trong CI

Runner `windows-latest` / `ubuntu-latest` GitHub cảnh báo deprecation actions nền Node.js 20 (từ 2026-06-02 ép Node.js 24; 2026-09-16 xóa Node.js 20). Actions release workflow chạm dưới đây: cập nhật major mới trước hạn, hoặc set env `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true`.

- `actions/checkout@v4`
- `actions/setup-go@v5`
- `goreleaser/goreleaser-action@v6`
- `sigstore/cosign-installer@v3`

Cập nhật: áp `Validate` trước, green test + third-party rồi mới theo `Release`.

## Khi đổi đối tượng phân phối

`.goreleaser.yaml` hiện build:

Hiện tại:

- `windows/amd64` (binary chính + launcher, 2 bản)
- `linux/amd64` (binary chính + launcher, 2 bản)
- `darwin/amd64` (binary chính + launcher, 2 bản)
- `darwin/arm64` (binary chính + launcher, 2 bản)

Launcher tích hợp `many-ai-cli-launcher` từ v0.3.0 build cùng matrix binary chính, gói vào mọi archive OS · deb/rpm · Homebrew cask (chỉ profile `wsl` Windows-only; OS khác báo lỗi tường minh). `windows/arm64` và `linux/arm64` đã `ignore` cả binary chính và launcher.

Đính kèm Linux / macOS khi chưa verify: ghi rõ chưa verify trên README và release note. Thêm Linux arm64 hoặc Windows arm64: rà lại `ignore` GoReleaser (cả binary chính và launcher).
