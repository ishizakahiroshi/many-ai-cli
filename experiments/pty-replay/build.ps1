# User-run only. Does not install toolchains or build the many-ai-cli product.
#Requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$RustVersion)
. "$PSScriptRoot/common.ps1"
Assert-WindowsHost
Assert-DefaultRuntimeEnvironment
$arch = [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant()
$goArch = switch ($arch) { 'x64' {'amd64'} 'arm64' {'arm64'} default { throw 'Only x64/arm64 are supported.' } }
$rustTarget = if ($goArch -eq 'amd64') { 'x86_64-pc-windows-msvc' } else { 'aarch64-pc-windows-msvc' }
foreach ($item in (Get-ChildItem Env:)) {
    if ($item.Name -match '^CARGO_PROFILE_RELEASE_' -or $item.Name -match '^CARGO_TARGET_.*_(RUSTFLAGS|LINKER)$' -or $item.Name -in @('RUSTC','RUSTDOC')) {
        if ($item.Value) { throw "Unset $($item.Name); custom compiler/profile settings are outside the baseline." }
    }
}
foreach ($n in @('GOFLAGS','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_BUILD_TARGET','CARGO_INCREMENTAL','CARGO_BUILD_RUSTFLAGS','CARGO_BUILD_RUSTC','CARGO_BUILD_RUSTC_WRAPPER','CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER','CARGO_BUILD_INCREMENTAL','RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER')) {
    if ([Environment]::GetEnvironmentVariable($n)) { throw "Unset $n; special optimization/wrapper settings are outside the baseline." }
}
# User/global Cargo config could silently add native CPU/LTO/wrappers. Refuse instead of changing it.
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
$walk = [IO.DirectoryInfo]::new($PSScriptRoot)
$configPaths = @((Join-Path $cargoHome 'config'),(Join-Path $cargoHome 'config.toml'))
while ($null -ne $walk) { $configPaths += Join-Path $walk.FullName '.cargo/config'; $configPaths += Join-Path $walk.FullName '.cargo/config.toml'; $walk=$walk.Parent }
$configPaths += Join-Path $PSScriptRoot 'rust/.cargo/config'; $configPaths += Join-Path $PSScriptRoot 'rust/.cargo/config.toml'
if (@($configPaths | Where-Object { Test-Path -LiteralPath $_ }).Count) { throw 'Cargo config exists. Review/use a clean configuration before this baseline build.' }
$old = @{}
foreach ($n in @('GOENV','GOTOOLCHAIN','GOOS','GOARCH','GOAMD64','GOARM64','CGO_ENABLED','GOWORK','RUSTUP_TOOLCHAIN','CARGO_TARGET_DIR')) { $old[$n]=[Environment]::GetEnvironmentVariable($n) }
try {
    $env:GOENV='off'; $env:GOTOOLCHAIN='local'; $env:GOWORK='off'; $env:GOOS='windows'; $env:GOARCH=$goArch; $env:CGO_ENABLED='0'
    $env:GOAMD64='v1'; $env:GOARM64='v8.0'
    # Select only an already-installed full version; check before invoking Cargo so rustup cannot fetch it.
    $installed = @(& rustup toolchain list 2>$null)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot list installed Rust toolchains.' }
    $selection = if ($installed -match "^$([regex]::Escape($RustVersion))-$rustTarget(\s|$)") { "$RustVersion-$rustTarget" } elseif ($installed -match "^stable-$rustTarget(\s|$)") { "stable-$rustTarget" } else { throw 'Requested Rust toolchain is not installed; install it yourself first.' }
    $env:RUSTUP_TOOLCHAIN=$selection
    $env:CARGO_TARGET_DIR=Join-Path $PSScriptRoot 'rust/target'
    $goVersion = (& go version) -join ' '; if ($LASTEXITCODE -ne 0 -or $goVersion -cne "go version go1.26.8 windows/$goArch") { throw 'Go 1.26.8 for the native architecture is required.' }
    $rustVersionText = (& rustc --version --verbose) -join "`n"; if ($LASTEXITCODE -ne 0 -or $rustVersionText -notmatch "(?m)^release: $([regex]::Escape($RustVersion))$") { throw 'Rust version check failed.' }
    if ($rustVersionText -notmatch "(?m)^host: $([regex]::Escape($rustTarget))$") { throw 'Rust target does not match native Windows architecture.' }
    $cargoVersion = (& cargo --version) -join ' '; if ($LASTEXITCODE -ne 0) { throw 'Cargo version check failed.' }
    $bin=Join-Path $PSScriptRoot 'bin'; New-Item -ItemType Directory -Path $bin -Force | Out-Null
    # Remove stale metadata first: a partial/failed build must never pass measurement admission.
    $metadataPath=Join-Path $bin 'build-metadata.json'
    if (Test-Path -LiteralPath $metadataPath) { Remove-Item -LiteralPath $metadataPath }
    Push-Location (Join-Path $PSScriptRoot 'go')
    try { & go build -trimpath -o (Join-Path $bin 'replay-bench-go.exe') .; if ($LASTEXITCODE -ne 0) { throw 'Go build failed.' } } finally { Pop-Location }
    Push-Location (Join-Path $PSScriptRoot 'rust')
    try { & cargo build --release --locked --offline; if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' } } finally { Pop-Location }
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'rust/target/release/replay-bench-rust.exe') -Destination $bin
    $executables=[ordered]@{}
    foreach ($name in @('replay-bench-go.exe','replay-bench-rust.exe')) { $f=Get-Item -LiteralPath (Join-Path $bin $name); $executables[$name]=@{sha256=(Get-FileHash $f.FullName -Algorithm SHA256).Hash.ToLowerInvariant();bytes=$f.Length} }
    Write-JsonFile $metadataPath ([ordered]@{
        schema_version=1; created_utc=[DateTime]::UtcNow.ToString('o'); base_sha='21d0bc7935a2c4696fb89ccff2e324157a528c2d'
        architecture=$arch; go_version=$goVersion; rust_version=$rustVersionText; cargo_version=$cargoVersion
        rust_pin=$RustVersion; rust_target=$rustTarget; go_flags=@('build','-trimpath'); cargo_flags=@('build','--release','--locked','--offline')
        environment=Get-SafeEnvironment; source_sha256=Get-SourceHashes $PSScriptRoot; executables=$executables
        provenance=Get-Content -LiteralPath (Join-Path $PSScriptRoot 'provenance.json') -Raw | ConvertFrom-Json
    })
    Write-Host 'Both EXEs built. Correctness and performance are still unmeasured.'
} finally { foreach ($n in $old.Keys) { [Environment]::SetEnvironmentVariable($n,$old[$n]) } }
