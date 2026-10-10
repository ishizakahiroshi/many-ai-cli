<#
.SYNOPSIS
  many-ai-cli の Rust 候補を、手元でワンコマンドでビルドし、隔離（trial）で起動する。

.DESCRIPTION
  - 通常は修正元のworktreeをそのまま使い、未commit変更を含む画面資材（Bun）と Rust（cargo +1.90.0）をビルドする。
  - -Branch を明示した候補検査だけ、リモートから別フォルダへ取得する。
  - 起動は必ず trial モード（実際の ~/.many-ai-cli と、ポート 47777 の Go 版 Hub には触れない）。
  - 既存の Go 版や実 home は変更しない。作るのは $Worktree と $TrialRoot の中だけ。

.EXAMPLE
  pwsh scripts\rust-local.ps1              # 修正元をビルド（debug）→ version 表示
  pwsh scripts\rust-local.ps1 -Run         # 上に加えて serve を別ウィンドウで起動し、URL を表示
  pwsh scripts\rust-local.ps1 -Stop        # 起動した trial の Hub を止める
  pwsh scripts\rust-local.ps1 -Release     # release ビルド（CI と同じ形・時間がかかる）
  pwsh scripts\rust-local.ps1 -Clean       # worktree を片付ける（trial root は残す）
  pwsh scripts\rust-local.ps1 -Real -Run -Source <worktree のフォルダ>   # 手元の worktree をそのままビルドして起動（push 不要・-Stop にも同じ -Source）
#>
[CmdletBinding()]
param(
    [switch]$Run,
    [switch]$Stop,
    [switch]$Release,
    [switch]$Clean,
    [switch]$SkipWeb,
    [switch]$CopySettings,   # 実際の config.yaml を trial root へコピー（読むだけ・実物は変更しない。秘密を含む）
    [switch]$Real,           # trial でなく通常モード。ただし home を F:\build\rust-home に向け、実際の ~/.many-ai-cli・DB・Go 版の Hub には触れない
    [switch]$CopyDb,         # -Real のとき、DB（any-ai-cli.db と -wal/-shm）のスナップショットも、コピーして持ち込む（予定の実行が二重に走る可能性があるので既定では持ち込まない）
    [string]$Branch = 'dots/rust-recovery-resume-3',
    [string]$CodexHome,      # -Real の Hub に CODEX_HOME を渡す（例: %USERPROFILE%\.codex を展開した値）。「全部更新」で、本物の Codex のインストールを更新する。省略時は、仮 home の中だけを見る
    [string]$Source,         # 手元の worktree（rust\Cargo.toml があるフォルダ）を、取得せずそのままビルドする。push 不要。-Stop も同じ -Source を付ける
    [int]$Port = 49400
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Repo      = Split-Path -Parent $PSScriptRoot   # このスクリプトは <repo>\scripts にある
$BuildRoot = 'F:\build'                          # ビルド・trial・仮 home の置き場（実際の home とは別のドライブ）
$Worktree  = Join-Path $BuildRoot 'rust-local'
. (Join-Path $PSScriptRoot 'rust-local-source.ps1')
$Source = Resolve-RustLocalSource -Repo $Repo -Source $Source -Branch $Branch `
    -RemoteCandidate:($PSBoundParameters.ContainsKey('Branch') -and -not $Source)
if ($Source) {
    $Worktree = (Resolve-Path -LiteralPath $Source).Path   # 以降のビルド・exe・-Stop は、すべてこのフォルダを使う
}
$TrialRoot = Join-Path $BuildRoot 'rust-accept-user'
$Toolchain = '1.90.0'   # CI の固定版（scripts/rust-candidate-ci.py と .github/workflows と同じ）
$RealUserHome = $env:USERPROFILE                 # 実際の home（読むだけ。ここへは書かない）
$FakeHome  = Join-Path $BuildRoot 'rust-home'    # -Real のときの home（Rust は USERPROFILE を home とする）
$AppHome   = Join-Path $FakeHome '.many-ai-cli'

# 子プロセスだけ、home を差し替えて実行する（呼び出し元のシェルの環境は、必ず元に戻す）
function Invoke-WithFakeHome([scriptblock]$cmd) {
    $oldUser = $env:USERPROFILE; $oldHome = $env:HOME
    try { $env:USERPROFILE = $FakeHome; $env:HOME = $FakeHome; & $cmd }
    finally { $env:USERPROFILE = $oldUser; $env:HOME = $oldHome }
}

# junction はリンクだけを消す。再帰削除でリンク先（実際の認証情報）を消さないため、必ず先に外す
function Remove-FakeHome {
    if ($FakeHome -ne (Join-Path $BuildRoot 'rust-home') -or -not (Test-Path -LiteralPath $FakeHome)) { return }
    $junction = Join-Path $AppHome 'subscriptions'
    if (Test-Path -LiteralPath $junction) {
        $item = Get-Item -LiteralPath $junction -Force
        if ($item.LinkType -eq 'Junction') { [System.IO.Directory]::Delete($junction, $false) }
        elseif ($item.PSIsContainer) { throw "subscriptions が junction ではありません。安全のため削除しません: $junction" }
    }
    Remove-Item -LiteralPath $FakeHome -Recurse -Force
    Write-Host "削除しました: $FakeHome"
}

# 秘密（config.yaml・DB）を置くフォルダは、作った時点で、自分と SYSTEM だけに絞る（置き場のドライブは、継承で他のユーザーにも読めることがある）。
# すでにあるフォルダは触らず、所有者だけ確かめる（他人が先に作っておいたフォルダへ秘密を書かないため）。
function New-PrivateDir([string]$path) {
    $me = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    if (Test-Path -LiteralPath $path) {
        $owner = (Get-Acl -LiteralPath $path).Owner
        if ($owner -ne $me -and $owner -ne 'BUILTIN\Administrators') { throw "既存のフォルダの所有者が自分ではありません。秘密を置かず中止します: $path（所有者: $owner）" }
        return
    }
    New-Item -ItemType Directory -Force -Path $path | Out-Null
    & icacls $path /inheritance:r /grant:r "${me}:(OI)(CI)F" 'SYSTEM:(OI)(CI)F' | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "権限を絞れませんでした（exit $LASTEXITCODE）: $path" }
}

function Step([string]$text) { Write-Host "`n== $text" -ForegroundColor Cyan }
function Native([string]$what, [scriptblock]$cmd) {
    & $cmd
    if ($LASTEXITCODE -ne 0) { throw "$what が失敗しました（exit $LASTEXITCODE）" }
}

if ($Port -eq 47777) { throw '47777 は Go 版の既定ポートです。別のポートを指定してください。' }

$buildKind = if ($Release) { 'release' } else { 'debug' }
$exe = Join-Path $Worktree "rust\target\$buildKind\many-ai-cli.exe"
$trialArgs = @('--trial-root', $TrialRoot, '--trial-port', "$Port")

# --- 停止（worktree があり、exe があるときだけ） ---
if ($Stop) {
    $stopExe = @("rust\target\debug\many-ai-cli.exe", "rust\target\release\many-ai-cli.exe") |
        ForEach-Object { Join-Path $Worktree $_ } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $stopExe) { throw '実行ファイルが見つかりません（まだビルドしていません）' }
    if ($Real) {
        Step '通常モード（-Real）の Hub を止める'
        Set-Location -LiteralPath $FakeHome
        Invoke-WithFakeHome { & $stopExe stop }
        if ($LASTEXITCODE -ne 0) { throw "stop が失敗しました（exit $LASTEXITCODE）" }
        return
    }
    Step 'trial の Hub を止める'
    $stopWorkspace = Join-Path $TrialRoot 'workspace'
    New-Item -ItemType Directory -Force -Path $stopWorkspace | Out-Null
    Set-Location -LiteralPath $stopWorkspace   # trial はカレントが trial root の中であることを要求する
    Native 'stop' { & $stopExe @trialArgs stop }
    return
}

# --- 片付け ---
if ($Clean) {
    Step '-Real 用の home を片付ける（junction は、リンクだけを外す）'
    Remove-FakeHome
    if (Test-Path -LiteralPath (Join-Path $RealUserHome '.many-ai-cli\subscriptions')) { Write-Host '確認: 実際の subscriptions は、そのまま残っています。' }
    Step 'worktree を片付ける'
    if ($Source) { Write-Host "-Source で指定したフォルダは、自分のものではないので消しません: $Worktree"; return }
    $registered = (& git -C $Repo worktree list --porcelain) -match [regex]::Escape(($Worktree -replace '\\', '/'))
    if (-not $registered) { Write-Host "登録された worktree ではありません。何もしません: $Worktree"; return }
    Native 'git worktree remove' { & git -C $Repo worktree remove --force $Worktree }
    Write-Host "削除しました: $Worktree"
    # trial root（コピーした config.yaml など）も、決め打ちの場所に限って削除する
    if ((Test-Path -LiteralPath $TrialRoot) -and ($TrialRoot -eq (Join-Path $BuildRoot 'rust-accept-user'))) {
        Remove-Item -LiteralPath $TrialRoot -Recurse -Force
        Write-Host "削除しました: $TrialRoot"
    }
    return
}

# --- 道具の確認（読み取りのみ） ---
Step '道具の確認'
foreach ($tool in 'git', 'bun', 'cargo', 'rustup') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "$tool が見つかりません" }
}
if (-not ((& rustup toolchain list) -match [regex]::Escape($Toolchain))) {
    throw "Rust $Toolchain が入っていません。`rustup toolchain install $Toolchain` を実行してから、やり直してください。"
}
Write-Host ("bun {0} / cargo +{1} / " -f (& bun --version), $Toolchain) -NoNewline
Write-Host (& cargo "+$Toolchain" --version)

# --- 取得（fetch は取り込むだけで、今の作業ブランチは変えない） ---
if ($Source) {
    Step "手元の worktree をそのまま使う（取得しない・push 不要）: $Worktree"
    $sha = (& git -C $Worktree rev-parse HEAD).Trim()
    $changed = @(& git -C $Worktree status --porcelain --untracked-files=normal).Count
    Write-Host "commit: $sha / 未コミットの変更: $changed 件（あれば、その内容でビルドされます）"
} else {
    Step "ブランチを取得して worktree に取り出す（$Branch）"
    Native 'git fetch' { & git -C $Repo fetch origin $Branch }
    $ref = "origin/$Branch"
    $sha = (& git -C $Repo rev-parse $ref).Trim()
    if (-not (Test-Path -LiteralPath $Worktree)) {
        Native 'git worktree add' { & git -C $Repo worktree add --detach $Worktree $ref }
    } else {
        & git -C $Worktree rev-parse --git-dir 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "$Worktree はありますが git の worktree ではありません（前回の残骸）。中身を確かめて、別の名前へ移すか消してから、やり直してください" }
        $dirty = & git -C $Worktree status --porcelain --untracked-files=no
        if ($dirty) { throw "worktree に未コミットの変更があります。確認してから -Clean するか、手で戻してください: $Worktree" }
        Native 'git checkout' { & git -C $Worktree checkout --detach $ref }
    }
    Write-Host "取り出した commit: $sha"
}

# --- 画面資材（Rust のビルドが web/dist を要求する） ---
if ($SkipWeb) {
    Step '画面資材: 省略（-SkipWeb）'
} else {
    Step '画面資材をビルド（bun）'
    Push-Location (Join-Path $Worktree 'web')
    try {
        Native 'bun install' { & bun install --frozen-lockfile }
        Native 'bun run build' { & bun run build }
    } finally { Pop-Location }
}

# --- Rust ---
Step "Rust をビルド（cargo +$Toolchain / $buildKind）"
$cargoArgs = @("+$Toolchain", 'build', '--locked', '--manifest-path', (Join-Path $Worktree 'rust\Cargo.toml'))
if ($Release) { $cargoArgs += '--release' }
Native 'cargo build' { & cargo @cargoArgs }
if (-not (Test-Path -LiteralPath $exe)) { throw "実行ファイルができていません: $exe" }

# --- -Real: 通常モード。ただし home を $FakeHome に向けるので、実際の ~/.many-ai-cli・DB・Go 版の Hub（47777）には触れない ---
if ($Real) {
    Step "通常モード用の home を用意する（$FakeHome。実際の home は読むだけ）"
    New-PrivateDir $FakeHome
    New-PrivateDir $AppHome
    # git は仮 home の .gitconfig を見る。無いと、仮 home から起動した AI のセッションの git commit が
    # 「Author identity unknown」で止まる（2026-10-10）。実際の home の .gitconfig を include で読むだけにし、書き換えない。
    $fakeGitConfig = Join-Path $FakeHome '.gitconfig'
    $realGitConfig = Join-Path $RealUserHome '.gitconfig'
    if (-not (Test-Path -LiteralPath $fakeGitConfig) -and (Test-Path -LiteralPath $realGitConfig)) {
        $includePath = $realGitConfig -replace '\\', '/'
        [IO.File]::WriteAllText($fakeGitConfig, "[include]`n`tpath = $includePath`n", (New-Object System.Text.UTF8Encoding($false)))
        Write-Host "git の設定は、実際の home の .gitconfig を読み込むようにしました（読むだけ）: $fakeGitConfig"
    }
    $realApp = Join-Path $RealUserHome '.many-ai-cli'
    $dstCfg = Join-Path $AppHome 'config.yaml'
    if ($CopySettings -or -not (Test-Path -LiteralPath $dstCfg)) {
        $srcCfg = Join-Path $realApp 'config.yaml'
        if (-not (Test-Path -LiteralPath $srcCfg)) { throw "コピー元が見つかりません: $srcCfg" }
        $text = [IO.File]::ReadAllText($srcCfg)
        $count = 0
        # 設定には、実際の home を指す絶対パス（log_dir・承認パターンの取得元など）が入っている。
        # そのままだと Rust 版が実際のログや承認パターンへ書き込むので、$AppHome へ付け替える（\ / \\ の 3 表記）。
        foreach ($pair in @(
                @($realApp, $AppHome),
                @(($realApp -replace '\\', '/'), ($AppHome -replace '\\', '/')),
                @(($realApp -replace '\\', '\\'), ($AppHome -replace '\\', '\\')))) {
            $count += [regex]::Matches($text, [regex]::Escape($pair[0]), 'IgnoreCase').Count
            $text = [regex]::Replace($text, [regex]::Escape($pair[0]), $pair[1].Replace('$', '$$'), 'IgnoreCase')
        }
        [IO.File]::WriteAllText($dstCfg, $text, (New-Object System.Text.UTF8Encoding($false)))
        Write-Host "config.yaml をコピーし、実際の home を指すパスを $count 箇所、付け替えました（秘密を含みます。中身を人に見せないでください。-Clean で削除されます）。" -ForegroundColor Yellow
    }
    # 認証情報（subscriptions）は、コピーせず junction で参照する（実際のフォルダは変更しない。-Clean ではリンクだけを外す）
    $junction = Join-Path $AppHome 'subscriptions'
    if (-not (Test-Path -LiteralPath $junction)) {
        $target = Join-Path $realApp 'subscriptions'
        if (Test-Path -LiteralPath $target) { New-Item -ItemType Junction -Path $junction -Target $target | Out-Null }
        else { Write-Host '実際の subscriptions が見つかりません。サブスクリプションは使えません。' -ForegroundColor Yellow }
    }
    if ($CopyDb) {
        foreach ($f in 'any-ai-cli.db', 'any-ai-cli.db-wal', 'any-ai-cli.db-shm') {
            $s = Join-Path $realApp $f
            if (Test-Path -LiteralPath $s) { Copy-Item -LiteralPath $s -Destination (Join-Path $AppHome $f) -Force }
        }
        Write-Host 'DB のスナップショットをコピーしました（Go 版の Hub が書き込み中のため、整合しない可能性があります。予定された実行が二重に走る可能性にも注意）。' -ForegroundColor Yellow
    }
    Set-Location -LiteralPath $FakeHome
    Step 'version（通常モード・home を差し替え）'
    Invoke-WithFakeHome { & $exe version }
    if ($LASTEXITCODE -ne 0) { throw "version が失敗しました（exit $LASTEXITCODE）" }
    Write-Host "実行ファイル: $exe"
    if ($Run) {
        Step "serve を別ウィンドウで起動（通常モード・ポート $Port・home=$FakeHome）"
        $serveCmd = "`$env:USERPROFILE='$FakeHome'; `$env:HOME='$FakeHome'; "
        if ($CodexHome) {
            if (-not (Test-Path -LiteralPath $CodexHome -PathType Container)) { throw "-CodexHome のフォルダが見つかりません: $CodexHome" }
            Write-Host "CODEX_HOME=$CodexHome を Hub に渡します。「全部更新」は、本物の Codex のインストールを書き換えます。" -ForegroundColor Yellow
            $serveCmd += "`$env:CODEX_HOME='$CodexHome'; "
        }
        $serveCmd += "& '$exe' serve --port $Port"
        Start-Process -FilePath 'pwsh' -ArgumentList @('-NoExit', '-Command', $serveCmd) -WorkingDirectory $FakeHome
        Step 'status（URL。ブラウザで開く。トークン付きなので他人に見せない）'
        $ok = $false
        for ($i = 1; $i -le 15; $i++) {
            Start-Sleep -Seconds 2
            $out = Invoke-WithFakeHome { & $exe status 2>&1 }
            if ($LASTEXITCODE -eq 0) { $ok = $true; $out; break }
        }
        if (-not $ok) {
            Write-Host '30 秒待っても status が成功しませんでした。別ウィンドウ（serve）に出ているメッセージを確認してください。' -ForegroundColor Yellow
            Write-Host "最後の status の出力: $out"
        }
        Write-Host "`n止めるとき: pwsh $PSCommandPath -Real -Stop"
    } else {
        Write-Host "`n起動するとき: pwsh $PSCommandPath -Real -Run -SkipWeb"
    }
    return
}

# --- 動作確認（trial。実 home には触れない） ---
New-PrivateDir $TrialRoot   # trial root は、先に存在している必要がある見込み
# trial モードは、カレントディレクトリが trial root の中にないと拒否する（context.rs:29-31）。
# 以降の trial の呼び出しは、すべて trial root の中の作業場所から行う。
if ($CopySettings) {
    # 実際の設定（config.yaml）だけをコピーする。認証情報（subscriptions の各フォルダ）・DB・ログは運ばない。
    $src = Join-Path $env:USERPROFILE '.many-ai-cli\config.yaml'
    if (-not (Test-Path -LiteralPath $src)) { throw "コピー元が見つかりません: $src" }
    Copy-Item -LiteralPath $src -Destination (Join-Path $TrialRoot 'config.yaml') -Force
    Write-Host 'config.yaml をコピーしました（トークン等の秘密を含みます。中身を人に見せないでください。-Clean で削除されます）。' -ForegroundColor Yellow
    Write-Host '※ すでに serve が動いている場合は、先に -Stop してからやり直してください（起動時に設定を読むため）。'
}
$Workspace = Join-Path $TrialRoot 'workspace'
New-Item -ItemType Directory -Force -Path $Workspace | Out-Null
Set-Location -LiteralPath $Workspace
Step 'version（trial）'
Native 'version' { & $exe @trialArgs version }
Write-Host "実行ファイル: $exe"

if ($Run) {
    Step 'serve を別ウィンドウで起動（trial）'
    # -NoExit: 起動に失敗しても、別ウィンドウが閉じず、エラーを読める
    $serveCmd = "& '$exe' --trial-root '$TrialRoot' --trial-port $Port serve"
    Start-Process -FilePath 'pwsh' -ArgumentList @('-NoExit', '-Command', $serveCmd) -WorkingDirectory $Workspace
    Step 'status（URL。ブラウザで開く。トークン付きなので他人に見せない）'
    $ok = $false
    for ($i = 1; $i -le 15; $i++) {
        Start-Sleep -Seconds 2
        $out = & $exe @trialArgs status 2>&1
        if ($LASTEXITCODE -eq 0) { $ok = $true; $out; break }
    }
    if (-not $ok) {
        Write-Host '30 秒待っても status が成功しませんでした。別ウィンドウ（serve）に出ているメッセージを確認してください。' -ForegroundColor Yellow
        Write-Host "最後の status の出力: $out"
    }
    Write-Host "`n止めるとき: pwsh $PSCommandPath -Stop"
} else {
    Write-Host "`n起動するとき: pwsh $PSCommandPath -Run"
}
