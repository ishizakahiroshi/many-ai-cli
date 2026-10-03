# Shared I/O and statistics only. Dot-source from PowerShell 7 scripts.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[System.Threading.Thread]::CurrentThread.CurrentCulture = [cultureinfo]::InvariantCulture
[System.Threading.Thread]::CurrentThread.CurrentUICulture = [cultureinfo]::InvariantCulture

function Assert-WindowsHost {
    if (-not $IsWindows -or $PSVersionTable.PSVersion.Major -lt 7 -or -not [Environment]::Is64BitProcess) {
        throw 'Use native 64-bit Windows PowerShell 7 (pwsh), not Windows PowerShell 5 or WSL.'
    }
    if ([Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne [Runtime.InteropServices.RuntimeInformation]::OSArchitecture) { throw 'Use native-architecture pwsh; emulated processes are outside this comparison.' }
}
function Write-JsonFile($Path, $Value) {
    ConvertTo-Json -InputObject $Value -Depth 30 | Set-Content -LiteralPath $Path -Encoding utf8
}
function Add-CsvRow($Path, $Value) {
    # Invariant culture above makes numbers portable. Empty CSV cell means null, never zero.
    $Value | Export-Csv -LiteralPath $Path -NoTypeInformation -Append -Encoding utf8
}
function Get-Median($Values) {
    $v = @($Values | Where-Object { $null -ne $_ } | Sort-Object)
    if ($v.Count -eq 0) { return $null }
    $i = [int][math]::Floor($v.Count / 2)
    if ($v.Count % 2) { return [double]$v[$i] }
    return ([double]$v[$i-1] + [double]$v[$i]) / 2
}
function Get-P95($Values) {
    $v = @($Values | Where-Object { $null -ne $_ } | Sort-Object)
    if ($v.Count -eq 0) { return $null }
    return [double]$v[[int][math]::Ceiling($v.Count * 0.95) - 1]
}
function Get-SourceHashes($Root) {
    $result = [ordered]@{}
    foreach ($file in (Get-ChildItem -LiteralPath $Root -Recurse -File | Sort-Object FullName)) {
        $rel = [IO.Path]::GetRelativePath($Root, $file.FullName).Replace('\','/')
        if ($rel -match '^(bin|results|rust/target|fixtures/generated)/') { continue }
        if ($file.Extension -in @('.ps1','.go','.rs','.toml','.lock') -or $file.Name -in @('go.mod','scenario.json','provenance.json')) {
            $result[$rel] = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    return $result
}
function Get-PowerScheme {
    $s = & powercfg.exe /getactivescheme 2>$null
    if ($LASTEXITCODE -ne 0 -or ($s -join ' ') -notmatch '[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}') {
        throw 'Cannot read active power scheme with powercfg.'
    }
    return $Matches[0].ToLowerInvariant()
}
function Get-SafeEnvironment {
    # Record only bounded public configuration values, never arbitrary flags/paths/secrets.
    $v = [ordered]@{}
    $patterns = @{
        GOOS='^windows$';GOARCH='^(amd64|arm64)$';GOAMD64='^v[1-4]$';GOARM64='^v8\.0$'
        GOTOOLCHAIN='^local$';CGO_ENABLED='^[01]$';GOENV='^off$';GOWORK='^off$'
        RUSTUP_TOOLCHAIN='^(stable|[0-9]+\.[0-9]+\.[0-9]+)-(x86_64|aarch64)-pc-windows-msvc$'
    }
    foreach ($n in $patterns.Keys | Sort-Object) {
        $value = [Environment]::GetEnvironmentVariable($n)
        $v[$n] = if (-not $value) { $null } elseif ($value -match $patterns[$n]) { $value } else { 'present_redacted' }
    }
    foreach ($n in @('GOGC','GOMEMLIMIT','GOMAXPROCS','GODEBUG','GOEXPERIMENT')) { $v[$n] = if ([Environment]::GetEnvironmentVariable($n)) { 'present_redacted' } else { 'unset_default' } }
    return $v
}
function Assert-DefaultRuntimeEnvironment {
    foreach ($n in @('GOGC','GOMEMLIMIT','GOMAXPROCS','GODEBUG','GOEXPERIMENT')) {
        if ([Environment]::GetEnvironmentVariable($n)) { throw "Unset $n for this baseline experiment." }
    }
}
function Start-BenchProcess($Exe, $Mode, $FixtureDir, $Repetitions, $LogPrefix) {
    $si = [Diagnostics.ProcessStartInfo]::new()
    $si.FileName = $Exe
    $si.UseShellExecute = $false
    $si.RedirectStandardInput = $true
    $si.RedirectStandardOutput = $true
    $si.RedirectStandardError = $true
    $si.CreateNoWindow = $true
    foreach ($a in @('--mode',$Mode,'--fixture-dir',$FixtureDir,'--repetitions',[string]$Repetitions)) { $si.ArgumentList.Add($a) }
    $p = [Diagnostics.Process]::new(); $p.StartInfo = $si
    $outWriter=$null;$errWriter=$null
    try {
        # Construct log files before starting the process. Failure cannot orphan a child.
        $outWriter=[IO.StreamWriter]::new("$LogPrefix.stdout.jsonl",$false,[Text.UTF8Encoding]::new($false))
        $errWriter=[IO.StreamWriter]::new("$LogPrefix.stderr.log",$false,[Text.UTF8Encoding]::new($false))
        $clock = [Diagnostics.Stopwatch]::StartNew()
        if (-not $p.Start()) { throw 'Process start failed.' }
        $p.StandardInput.AutoFlush = $true
        return [pscustomobject]@{
        process=$p; clock=$clock; lines=[Collections.Generic.Queue[string]]::new()
        outTask=$p.StandardOutput.ReadLineAsync(); errTask=$p.StandardError.ReadLineAsync()
        outWriter=$outWriter;errWriter=$errWriter
        outClosed=$false; errClosed=$false
        }
    } catch {
        try { if ($p.Id -and -not $p.HasExited) { $p.Kill($true);$null=$p.WaitForExit(5000) } } catch { }
        if ($null -ne $outWriter) { $outWriter.Dispose() };if ($null -ne $errWriter) { $errWriter.Dispose() }
        $p.Dispose();throw
    }
}
function Receive-BenchOutput($Child) {
    # .NET asynchronous reads drain both pipes; no PowerShell event callbacks/runspace deadlocks.
    foreach ($stream in @('out','err')) {
        $taskKey = $stream+'Task'; $closedKey = $stream+'Closed'; $writerKey = $stream+'Writer'
        while (-not $Child.$closedKey -and $Child.$taskKey.IsCompleted) {
            $line = $Child.$taskKey.GetAwaiter().GetResult()
            if ($null -eq $line) { $Child.$closedKey = $true; break }
            $Child.$writerKey.WriteLine($line); $Child.$writerKey.Flush() # Preserve before JSON parsing.
            if ($stream -eq 'out') {
                $Child.lines.Enqueue($line)
                $Child.$taskKey = $Child.process.StandardOutput.ReadLineAsync()
            } else { $Child.$taskKey = $Child.process.StandardError.ReadLineAsync() }
        }
    }
}
function Read-BenchEvent($Child, $Expected, $TimeoutSeconds) {
    $deadline = $Child.clock.Elapsed.TotalSeconds + $TimeoutSeconds
    while ($Child.clock.Elapsed.TotalSeconds -lt $deadline) {
        Receive-BenchOutput $Child
        if ($Child.lines.Count) {
            $e = $Child.lines.Dequeue() | ConvertFrom-Json
            if ($e.event -cne $Expected) { throw "Expected $Expected event." }
            return $e
        }
        if ($Child.outClosed -and $Child.lines.Count -eq 0) { throw "Stdout ended before $Expected." }
        Start-Sleep -Milliseconds 10
    }
    throw "Timeout waiting for $Expected."
}
function Stop-BenchProcess($Child) {
    if ($null -eq $Child) { return }
    try {
        if (-not $Child.process.HasExited) { $Child.process.Kill($true) }
        if (-not $Child.process.WaitForExit(5000)) { throw 'Child cleanup timed out.' }
        # Drains buffered output after a normal exit or a kill; never fabricates DONE.
        $limit = [Diagnostics.Stopwatch]::StartNew()
        while ((-not $Child.outClosed -or -not $Child.errClosed) -and $limit.Elapsed.TotalSeconds -lt 5) {
            Receive-BenchOutput $Child
            Start-Sleep -Milliseconds 10
        }
    } finally {
        $Child.outWriter.Dispose(); $Child.errWriter.Dispose(); $Child.process.Dispose()
    }
}
