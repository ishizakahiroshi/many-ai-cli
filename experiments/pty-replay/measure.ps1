# User-run only. Every invocation creates a fresh immutable attempt directory.
#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('verify','pilot','measure')][string]$Stage,
    [Parameter(Mandatory)][string]$OutputDir,
    [string]$FixtureDir = (Join-Path $PSScriptRoot 'fixtures/generated'),
    [ValidateRange(1,2147483647)][int]$Repetitions = 1,
    [string]$PilotDir,
    [switch]$ConditionsConfirmed,
    [ValidateRange(60,86400)][int]$RunTimeoutSeconds = 600
)
. "$PSScriptRoot/common.ps1"
Assert-WindowsHost
Assert-DefaultRuntimeEnvironment
if ($Stage -ne 'verify' -and -not $ConditionsConfirmed) {
    throw 'Confirm AC power, unchanged power mode, and no interactive/background heavy work with -ConditionsConfirmed.'
}
$scenarioPath=Join-Path $PSScriptRoot 'scenario.json'
$s=Get-Content -LiteralPath $scenarioPath -Raw | ConvertFrom-Json
if ($s.schema_version -ne 1 -or $s.seed -ne 42 -or $s.pairs_per_condition -ne 10 -or $s.pilot_runs_per_language_condition -ne 2 -or $s.process_sample_ms -ne 250 -or $s.machine_sample_ms -ne 1000 -or $s.warmup_seconds -ne 3 -or $s.paced_ops -ne 1500 -or $s.paced_duration_seconds -ne 30) { throw 'Unsupported scenario contract; review scripts before changing the experiment.' }
if ($s.limit_bytes -ne 2097152 -or $s.paced_interval_ms -ne 20 -or $s.fixture_cycles -ne 100 -or ($s.chunk_lengths -join ',') -cne '64,64,64,64,64,64,512,512,512,4096') { throw 'Unsupported fixture/pacing constants.' }
$FixtureDir=(Resolve-Path -LiteralPath $FixtureDir).Path
$buildPath=Join-Path $PSScriptRoot 'bin/build-metadata.json'
$build=Get-Content -LiteralPath $buildPath -Raw | ConvertFrom-Json -AsHashtable
if ($build.architecture -cne [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant()) { throw 'Built EXEs do not match native host architecture.' }
$manifestPath=Join-Path $FixtureDir 'manifest.json'
$manifest=Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$scenarioHash=(Get-FileHash $scenarioPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($manifest.scenario_sha256 -cne $scenarioHash) { throw 'Fixture scenario hash changed; prepare again.' }
if ($manifest.files.Count -lt 3 -or $manifest.cases -lt 1) { throw 'Empty/incomplete fixture manifest.' }
foreach ($f in $manifest.files) {
    if ($f.name -notmatch '^[a-zA-Z0-9_.-]+$') { throw 'Unsafe fixture manifest filename.' }
    $path=Join-Path $FixtureDir $f.name
    if ((Get-Item -LiteralPath $path).Length -ne $f.bytes -or (Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $f.sha256) { throw "Fixture hash/size mismatch: $($f.name)" }
}
$currentHashes=Get-SourceHashes $PSScriptRoot
if ($currentHashes.Count -ne $build.source_sha256.Count) { throw 'Sources changed since build.' }
foreach ($n in $currentHashes.Keys) { if (-not $build.source_sha256.Contains($n) -or $build.source_sha256[$n] -cne $currentHashes[$n]) { throw "Source changed since build: $n" } }
$exe=@{}
foreach ($language in @('go','rust')) {
    $name="replay-bench-$language.exe"; $exe[$language]=Join-Path $PSScriptRoot "bin/$name"
    if ((Get-FileHash $exe[$language] -Algorithm SHA256).Hash.ToLowerInvariant() -cne $build.executables[$name].sha256) { throw "EXE hash mismatch: $name" }
}
$identity=[ordered]@{build_sha256=(Get-FileHash $buildPath -Algorithm SHA256).Hash.ToLowerInvariant(); fixture_manifest_sha256=(Get-FileHash $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant();scenario_sha256=$scenarioHash}
$os=Get-CimInstance Win32_OperatingSystem -OperationTimeoutSec 5
$cpus=@(Get-CimInstance Win32_Processor -OperationTimeoutSec 5)
$logicalCpus=($cpus | Measure-Object NumberOfLogicalProcessors -Sum).Sum
if ($logicalCpus -lt 1 -or $os.TotalVisibleMemorySize -le 0) { throw 'Missing machine hardware counters.' }
$powerScheme=Get-PowerScheme
$machine=[ordered]@{os_version=$os.Version;os_build=$os.BuildNumber;architecture=[Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString();cpu_models=@($cpus.Name);logical_cpus=$logicalCpus;total_ram_bytes=[long]$os.TotalVisibleMemorySize*1024;power_scheme_guid=$powerScheme}
$machineJson=$machine | ConvertTo-Json -Depth 5 -Compress
if ($Stage -eq 'measure') {
    if (-not $PilotDir) { throw '-PilotDir is required for main measurement.' }
    $pilot=Get-Content -LiteralPath (Join-Path $PilotDir 'pilot.json') -Raw | ConvertFrom-Json
    if (-not $pilot.accepted -or $pilot.repetitions -ne $Repetitions) { throw 'Need an accepted full pilot at exactly these fixed repetitions.' }
    foreach ($k in $identity.Keys) { if ($pilot.identity.$k -cne $identity[$k]) { throw "Pilot identity mismatch: $k" } }
    if ($pilot.machine_json -cne $machineJson) { throw 'Pilot machine, OS, architecture or power scheme changed.' }
}
$OutputDir=$ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDir)
if (Test-Path -LiteralPath $OutputDir) { throw 'OutputDir must not exist. Keep original attempts and choose a new directory.' }
New-Item -ItemType Directory -Path $OutputDir | Out-Null
$attemptId=[guid]::NewGuid().ToString('N')
$rules=[ordered]@{
    idle_seconds=$s.idle_seconds;idle_cpu_median_below_percent=$s.idle_cpu_percent_max;idle_available_ram_above_percent=$s.idle_available_ram_percent_min
    running_ram_below_percent=$s.run_available_ram_percent_min;other_cpu_above_percent=$s.other_cpu_percent_max;other_cpu_for_seconds=$s.other_cpu_seconds_max
    process_max_gap_ms=1000;machine_max_gap_ms=2500;minimum_machine_samples=3;minimum_process_samples=3
    minimum_saturated_seconds=$s.minimum_saturated_seconds;paced_max_lateness_ms=250;handshake_overhead_max_ms=1000
    missing_counter='invalid pair; null with reason, never zero';failed_correctness='stop experiment';invalid_pair='both runs excluded; keep originals; no automatic rerun'
    manual_exclusion='Document observed updates, sleep, interactive work or power/AC changes in exclusions.csv (pair_id,reason); applies to whole pair only.'
}
$rng=[Random]::new([int]$s.seed); $schedule=[Collections.Generic.List[object]]::new()
if ($Stage -ne 'verify') {
    foreach ($condition in @('paced','saturated')) {
        $count=if ($Stage -eq 'pilot') { 2 } else { 10 }
        $orders=@(for ($i=0;$i -lt $count;$i++) { if ($i -lt $count/2) { 'go' } else { 'rust' } })
        for ($i=$orders.Count-1;$i -gt 0;$i--) { $j=$rng.Next($i+1);$tmp=$orders[$i];$orders[$i]=$orders[$j];$orders[$j]=$tmp }
        for ($i=0;$i -lt $count;$i++) {
            $pairId='{0}-{1:D2}' -f $condition,($i+1)
            $langs=if ($orders[$i] -eq 'go') { @('go','rust') } else { @('rust','go') }
            for ($j=0;$j -lt 2;$j++) { $schedule.Add([pscustomobject]@{pair_id=$pairId;condition=$condition;order=$j+1;language=$langs[$j];run_id="$attemptId-$pairId-$($langs[$j])"}) }
        }
    }
}
# Freeze rules, ordering, identity and workload before any correctness or benchmark child starts.
Write-JsonFile (Join-Path $OutputDir 'schedule.json') ([ordered]@{schema_version=1;attempt_id=$attemptId;stage=$Stage;seed=$s.seed;repetitions=$Repetitions;rules=$rules;runs=$schedule.ToArray()})
Copy-Item $scenarioPath (Join-Path $OutputDir 'scenario.json')
Copy-Item $buildPath (Join-Path $OutputDir 'build-metadata.json')
Copy-Item $manifestPath (Join-Path $OutputDir 'fixture-manifest.json')
Write-JsonFile (Join-Path $OutputDir 'metadata.json') ([ordered]@{
    schema_version=1;attempt_id=$attemptId;stage=$Stage;started_utc=[DateTime]::UtcNow.ToString('o');identity=$identity;machine=$machine
    powershell=$PSVersionTable.PSVersion.ToString();runtime_environment=Get-SafeEnvironment;conditions_confirmed=[bool]$ConditionsConfirmed
    run_timeout_seconds=$RunTimeoutSeconds;source_sha256=$currentHashes;base_sha=$build.base_sha
    schedule_sha256=(Get-FileHash (Join-Path $OutputDir 'schedule.json') -Algorithm SHA256).Hash.ToLowerInvariant()
    cpu_boundary='Process CPU sampled immediately before START and on MEASURED receipt, before CHECK. Includes command dispatch and MEASURED JSON/flush; excludes warmup, oracle, checksum, DONE and EXIT.'
    memory_boundary='Only phase=measured process samples; start/end boundaries and post-DONE held samples excluded. Lifetime peak includes startup/warmup.'
    machine_counter='Invariant CIM classes/properties in separate in-process runspace; whole-PC CPU includes observer. CPU load subtraction is approximate.'
    limitations=@('AC and power-mode overlay manually confirmed; active scheme checked before/after each run. Transient changes require manual exclusion.','CIM CPU describes its provider interval, which may not exactly align with process CPU intervals.','CPU model/OS/RAM are recorded; usernames, hostnames, paths and environment secrets are omitted.')
})

function Start-MachineSampler($State) {
    $queue=[Collections.Concurrent.ConcurrentQueue[object]]::new()
    $ps=[powershell]::Create()
    $code={param($Queue,$State,$ControllerPid)
        [Threading.Thread]::CurrentThread.CurrentCulture=[cultureinfo]::InvariantCulture
        $next=[Diagnostics.Stopwatch]::GetTimestamp();$frequency=[Diagnostics.Stopwatch]::Frequency
        while (-not $State.stop) {
            $begin=[Diagnostics.Stopwatch]::GetTimestamp();$phase=$State.phase
            $cpu=$null;$ram=$null;$target=$null;$observer=$null;$reason=[Collections.Generic.List[string]]::new()
            try {
                $c=Get-CimInstance -ClassName Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'" -OperationTimeoutSec 2 -ErrorAction Stop
                if ($null -eq $c.PercentProcessorTime) { throw 'missing' };$cpu=[double]$c.PercentProcessorTime
                if ($cpu -lt 0 -or $cpu -gt 100) { throw 'range' }
            } catch { $cpu=$null;$reason.Add('machine_cpu_unavailable') }
            try {
                $m=Get-CimInstance -ClassName Win32_OperatingSystem -OperationTimeoutSec 2 -ErrorAction Stop
                if ($null -eq $m.FreePhysicalMemory -or $m.TotalVisibleMemorySize -le 0) { throw 'missing' }
                $ram=100.0*[double]$m.FreePhysicalMemory/[double]$m.TotalVisibleMemorySize
            } catch { $ram=$null;$reason.Add('available_ram_unavailable') }
            try { $p=Get-Process -Id $ControllerPid -ErrorAction Stop;$observer=$p.TotalProcessorTime.TotalSeconds;$p.Dispose() } catch { $reason.Add('controller_cpu_unavailable') }
            if ($State.target_pid -gt 0) {
                try { $p=Get-Process -Id $State.target_pid -ErrorAction Stop;$target=$p.TotalProcessorTime.TotalSeconds;$p.Dispose() } catch { $reason.Add('target_cpu_unavailable') }
            } else { $target=0.0 } # No benchmark process exists during idle baseline.
            $end=[Diagnostics.Stopwatch]::GetTimestamp()
            $Queue.Enqueue([pscustomobject]@{kind='machine';utc=[DateTime]::UtcNow.ToString('o');mono_ms=1000.0*$end/$frequency;phase=$phase;phase_end=$State.phase;pid=$State.target_pid;machine_cpu_percent=$cpu;available_ram_percent=$ram;target_cpu_seconds=$target;controller_cpu_seconds=$observer;query_ms=1000.0*($end-$begin)/$frequency;delay_ms=[math]::Max(0,1000.0*($begin-$next)/$frequency);missing_reason=($reason -join ';')})
            $next += $frequency
            $wait=1000.0*($next-[Diagnostics.Stopwatch]::GetTimestamp())/$frequency
            if ($wait -gt 0) { Start-Sleep -Milliseconds ([int]$wait) }
        }
    }
    $null=$ps.AddScript($code.ToString()).AddArgument($queue).AddArgument($State).AddArgument($PID)
    $handle=$ps.BeginInvoke()
    return [pscustomobject]@{ps=$ps;handle=$handle;queue=$queue;state=$State}
}
function Receive-MachineSamples($Sampler,$RunId,$Rows) {
    $x=$null
    while ($Sampler.queue.TryDequeue([ref]$x)) {
        $row=[pscustomobject][ordered]@{run_id=$RunId;kind=$x.kind;utc=$x.utc;mono_ms=$x.mono_ms;phase=$x.phase;phase_end=$x.phase_end;pid=$x.pid;cpu_seconds=$x.target_cpu_seconds;working_set_bytes=$null;private_bytes=$null;lifetime_peak_working_set_bytes=$null;controller_cpu_seconds=$x.controller_cpu_seconds;machine_cpu_percent=$x.machine_cpu_percent;available_ram_percent=$x.available_ram_percent;query_ms=$x.query_ms;delay_ms=$x.delay_ms;missing_reason=$x.missing_reason}
        Add-CsvRow (Join-Path $OutputDir 'samples.csv') $row;$Rows.Add($row);$x=$null
    }
}
function Save-ProcessSample($Child,$RunId,$Phase,$DelayMs,$Rows) {
    $cpu=$null;$ws=$null;$private=$null;$peak=$null;$observer=$null;$reason=''
    try { $Child.process.Refresh();$cpu=$Child.process.TotalProcessorTime.TotalSeconds;$ws=$Child.process.WorkingSet64;$private=$Child.process.PrivateMemorySize64;$peak=$Child.process.PeakWorkingSet64 } catch { $reason='process_counters_unavailable' }
    try { $p=[Diagnostics.Process]::GetCurrentProcess();$observer=$p.TotalProcessorTime.TotalSeconds;$p.Dispose() } catch { $reason+=';controller_cpu_unavailable' }
    $row=[pscustomobject][ordered]@{run_id=$RunId;kind='process';utc=[DateTime]::UtcNow.ToString('o');mono_ms=1000.0*[Diagnostics.Stopwatch]::GetTimestamp()/[Diagnostics.Stopwatch]::Frequency;phase=$Phase;phase_end=$Phase;pid=$Child.process.Id;cpu_seconds=$cpu;working_set_bytes=$ws;private_bytes=$private;lifetime_peak_working_set_bytes=$peak;controller_cpu_seconds=$observer;machine_cpu_percent=$null;available_ram_percent=$null;query_ms=$null;delay_ms=$DelayMs;missing_reason=$reason}
    Add-CsvRow (Join-Path $OutputDir 'samples.csv') $row;$Rows.Add($row)
    return $row
}

$correct=[Collections.Generic.List[object]]::new()
foreach ($language in @('go','rust')) {
    $child=$null
    try {
        $child=Start-BenchProcess $exe[$language] 'verify' $FixtureDir 1 (Join-Path $OutputDir "verify-$language")
        $event=Read-BenchEvent $child 'VERIFY' 120
        if ($event.language -cne $language -or $event.ok -ne $true -or $event.concurrency_ok -ne $true -or $event.cases -ne $manifest.cases -or $event.checkpoints -lt 1) { throw 'Correctness gate did not pass.' }
        if (-not $child.process.WaitForExit(5000) -or $child.process.ExitCode -ne 0) { throw 'Verify did not exit successfully.' }
        Receive-BenchOutput $child
        if ($child.lines.Count) { throw 'Unexpected extra verify output.' }
        $correct.Add([pscustomobject]@{language=$language;ok=$true;result=$event;reason=$null})
    } catch {
        $correct.Add([pscustomobject]@{language=$language;ok=$false;result=$null;reason=$_.Exception.Message})
        Write-JsonFile (Join-Path $OutputDir 'correctness.json') @{identity=$identity;ok=$false;results=$correct.ToArray()}
        throw
    } finally { Stop-BenchProcess $child }
}
if ($correct[0].result.checkpoints -ne $correct[1].result.checkpoints) { Write-JsonFile (Join-Path $OutputDir 'correctness.json') @{identity=$identity;ok=$false;results=$correct.ToArray();reason='Verify checkpoint counts differ.'};throw 'Verify checkpoint counts differ.' }
Write-JsonFile (Join-Path $OutputDir 'correctness.json') @{identity=$identity;ok=$true;results=$correct.ToArray()}
if ($Stage -eq 'verify') { Write-Host 'Both correctness checks passed. No performance measurements taken.'; return }

$allRuns=[Collections.Generic.List[object]]::new()
foreach ($entry in $schedule) {
    $child=$null;$sampler=$null;$rows=[Collections.Generic.List[object]]::new();$reasons=[Collections.Generic.List[string]]::new();$fatal=$false
    $r=[ordered]@{attempt_id=$attemptId;run_id=$entry.run_id;pair_id=$entry.pair_id;condition=$entry.condition;order=$entry.order;language=$entry.language;repetitions=$Repetitions;pid=$null;startup_ready_ms=$null;measurement_start_mono_ms=$null;measurement_end_mono_ms=$null;wall_ms=$null;controller_wall_ms=$null;cpu_seconds=$null;cpu_seconds_per_gib=$null;average_cores=$null;cpu_one_core_percent=$null;cpu_machine_percent=$null;controller_cpu_seconds=$null;throughput_mib_per_second=$null;snapshot_ms=$null;expected_bytes=$null;completed_bytes=$null;expected_ops=$null;completed_ops=$null;total=$null;output_hash=$null;max_lateness_ms=$null;ws_median_mib=$null;ws_p95_mib=$null;ws_sample_max_mib=$null;private_median_mib=$null;private_p95_mib=$null;private_sample_max_mib=$null;lifetime_peak_ws_mib=$null;process_samples=0;machine_samples=0;exit_code=$null;correct=$false;run_valid=$false;excluded_reason=$null}
    try {
        if ((Get-PowerScheme) -cne $powerScheme) { throw 'Power scheme changed before run.' }
        $state=[hashtable]::Synchronized(@{stop=$false;phase='baseline';target_pid=0})
        $sampler=Start-MachineSampler $state
        $wait=[Diagnostics.Stopwatch]::StartNew()
        while ($wait.Elapsed.TotalSeconds -lt ($s.idle_seconds+0.15)) { Receive-MachineSamples $sampler $entry.run_id $rows;Start-Sleep -Milliseconds 50 }
        Receive-MachineSamples $sampler $entry.run_id $rows
        $base=@($rows | Where-Object { $_.phase -eq 'baseline' -and $_.phase_end -eq 'baseline' })
        if ($base.Count -lt $s.idle_seconds -or @($base | Where-Object missing_reason).Count) { throw 'Baseline counters missing or insufficient.' }
        if ((Get-Median @($base.machine_cpu_percent)) -ge $s.idle_cpu_percent_max -or ($base.available_ram_percent | Measure-Object -Minimum).Minimum -le $s.idle_available_ram_percent_min) { throw 'Baseline CPU/RAM not idle; attempt retained without automatic rerun.' }
        $state.phase='startup'
        $child=Start-BenchProcess $exe[$entry.language] $entry.condition $FixtureDir $Repetitions (Join-Path $OutputDir $entry.run_id)
        $state.target_pid=$child.process.Id;$r.pid=$child.process.Id
        $ready=Read-BenchEvent $child 'READY' 30;$r.startup_ready_ms=$child.clock.Elapsed.TotalMilliseconds
        $state.phase='warmup';$child.process.StandardInput.WriteLine('WARMUP');$null=Read-BenchEvent $child 'WARMED' 30
        # Boundary CSV write is deferred until START is sent to avoid counting disk I/O in target CPU.
        $child.process.Refresh();$startCpu=$child.process.TotalProcessorTime.TotalSeconds
        $observer=[Diagnostics.Process]::GetCurrentProcess();$observer.Refresh();$startObserver=$observer.TotalProcessorTime.TotalSeconds
        $clock=[Diagnostics.Stopwatch]::StartNew();$r.measurement_start_mono_ms=1000.0*[Diagnostics.Stopwatch]::GetTimestamp()/[Diagnostics.Stopwatch]::Frequency;$state.phase='measured';$child.process.StandardInput.WriteLine('START')
        $nextSample=0.0;$measured=$null;$endCpu=$null;$endObserver=$null;$controllerWall=$null
        while ($clock.Elapsed.TotalSeconds -lt $RunTimeoutSeconds) {
            Receive-BenchOutput $child
            if ($child.lines.Count) {
                # CPU boundary precedes JSON parsing and CHECK; raw line has already been persisted.
                $child.process.Refresh();$endCpu=$child.process.TotalProcessorTime.TotalSeconds;$controllerWall=$clock.Elapsed.TotalMilliseconds
                $observer.Refresh();$endObserver=$observer.TotalProcessorTime.TotalSeconds
                $r.measurement_end_mono_ms=1000.0*[Diagnostics.Stopwatch]::GetTimestamp()/[Diagnostics.Stopwatch]::Frequency
                $state.phase='checking'
                $measured=$child.lines.Dequeue() | ConvertFrom-Json
                if ($measured.event -cne 'MEASURED') { throw 'Expected MEASURED at timing boundary.' }
                break
            }
            if ($child.outClosed) { throw 'Stdout ended during measured work.' }
            $now=$clock.Elapsed.TotalMilliseconds
            if ($now -ge $nextSample) { $null=Save-ProcessSample $child $entry.run_id 'measured' ($now-$nextSample) $rows;$nextSample += $s.process_sample_ms }
            Receive-MachineSamples $sampler $entry.run_id $rows
            Start-Sleep -Milliseconds 5
        }
        $observer.Dispose()
        if ($null -eq $measured) { throw 'Measured work timed out.' }
        $null=Save-ProcessSample $child $entry.run_id 'boundary_measured' 0 $rows
        $child.process.StandardInput.WriteLine('CHECK')
        $done=Read-BenchEvent $child 'DONE' 60
        Write-JsonFile (Join-Path $OutputDir "$($entry.run_id).result.json") $done
        if ($done.language -cne $entry.language -or $done.mode -cne $entry.condition -or $done.ok -ne $true -or $done.output_hash -notmatch '^[0-9a-f]{16}$') { throw 'Invalid DONE/correctness result.' }
        foreach ($field in @('wall_ms','snapshot_ms','expected_bytes','completed_bytes','expected_ops','completed_ops','total','max_lateness_ms')) {
            if ($null -eq $done.$field -or $null -eq $measured.$field -or $done.$field -ne $measured.$field) { throw "MEASURED/DONE mismatch: $field" }
            $r[$field]=$done.$field
        }
        $expectedOps=if ($entry.condition -eq 'paced') { [decimal]$s.paced_ops } else { [decimal]$manifest.mixed_chunks*$Repetitions }
        $chunks=@(Get-Content -LiteralPath (Join-Path $FixtureDir 'chunks.csv') | ForEach-Object { [long]$_ })
        $expectedBytes=if ($entry.condition -eq 'saturated') { [decimal]$manifest.mixed_bytes*$Repetitions } else { $b=[decimal]0;for($i=0;$i -lt $s.paced_ops;$i++){$b+=$chunks[$i%$chunks.Count]};$b }
        if ($done.expected_ops -ne $expectedOps -or $done.completed_ops -ne $expectedOps -or $done.expected_bytes -ne $expectedBytes -or $done.completed_bytes -ne $expectedBytes -or $done.total -ne $expectedBytes+$s.limit_bytes) { throw 'Same-work byte/operation/total check failed.' }
        if ($done.wall_ms -le 0 -or $done.snapshot_ms -lt 0 -or $done.snapshot_ms -gt $done.wall_ms -or $endCpu -lt $startCpu) { throw 'Invalid timing/counter result.' }
        $r.output_hash=$done.output_hash;$r.correct=$true;$r.controller_wall_ms=$controllerWall;$r.cpu_seconds=$endCpu-$startCpu
        $r.controller_cpu_seconds=$endObserver-$startObserver;$r.average_cores=$r.cpu_seconds/($controllerWall/1000);$r.cpu_one_core_percent=100*$r.average_cores;$r.cpu_machine_percent=$r.cpu_one_core_percent/$logicalCpus
        $r.cpu_seconds_per_gib=$r.cpu_seconds/([double]$expectedBytes/1GB);$r.throughput_mib_per_second=([double]$expectedBytes/1MB)/($done.wall_ms/1000)
        if ($controllerWall-$done.wall_ms -gt $rules.handshake_overhead_max_ms -or $controllerWall -lt $done.wall_ms) { $reasons.Add('handshake_boundary_delay') }
        if ($entry.condition -eq 'saturated' -and $done.wall_ms -lt 1000*$s.minimum_saturated_seconds) { $reasons.Add('saturated_duration_under_minimum') }
        if ($entry.condition -eq 'paced' -and ($done.max_lateness_ms -gt $rules.paced_max_lateness_ms -or $done.wall_ms -lt 1000*$s.paced_duration_seconds)) { $reasons.Add('paced_timing_violation') }
        # Keep snapshot/process alive long enough to collect the endpoint, separate from steady metrics.
        $state.phase='held';$null=Save-ProcessSample $child $entry.run_id 'held' 0 $rows
        Receive-MachineSamples $sampler $entry.run_id $rows
        $child.process.StandardInput.WriteLine('EXIT')
        if (-not $child.process.WaitForExit(5000)) { throw 'EXIT handshake timed out.' }
        $r.exit_code=$child.process.ExitCode
        Receive-BenchOutput $child
        if ($r.exit_code -ne 0 -or $child.lines.Count) { throw 'Nonzero exit or extra protocol output.' }
        if ((Get-PowerScheme) -cne $powerScheme) { $reasons.Add('power_scheme_changed') }
    } catch {
        $reasons.Add($_.Exception.Message)
        # Crashes, protocol failures and correctness failures stop further performance work.
        if ($null -ne $child) { $fatal=$true }
    } finally {
        if ($null -ne $sampler) {
            $sampler.state.stop=$true
            if (-not $sampler.handle.AsyncWaitHandle.WaitOne(5000)) { $sampler.ps.Stop();$reasons.Add('machine_sampler_stop_timeout') }
            try { $null=$sampler.ps.EndInvoke($sampler.handle) } catch { $reasons.Add('machine_sampler_failed') }
            Receive-MachineSamples $sampler $entry.run_id $rows
            if ($sampler.ps.Streams.Error.Count) { $reasons.Add('machine_sampler_error') }
            $sampler.ps.Dispose()
        }
        if ($null -ne $child -and $child.process.HasExited) { $r.exit_code=$child.process.ExitCode }
        Stop-BenchProcess $child
    }
    $proc=@($rows | Where-Object { $_.kind -eq 'process' -and $_.phase -eq 'measured' })
    $mach=@($rows | Where-Object { $_.kind -eq 'machine' -and $_.phase -eq 'measured' -and $_.phase_end -eq 'measured' })
    $r.process_samples=$proc.Count;$r.machine_samples=$mach.Count
    if ($null -ne $r.measurement_start_mono_ms -and $null -ne $r.measurement_end_mono_ms) {
        if ($proc.Count -and ($proc[0].mono_ms-$r.measurement_start_mono_ms -gt $rules.process_max_gap_ms -or $r.measurement_end_mono_ms-$proc[-1].mono_ms -gt $rules.process_max_gap_ms)) { $reasons.Add('process_boundary_sampling_gap') }
        if ($mach.Count -and ($mach[0].mono_ms-$r.measurement_start_mono_ms -gt $rules.machine_max_gap_ms -or $r.measurement_end_mono_ms-$mach[-1].mono_ms -gt $rules.machine_max_gap_ms)) { $reasons.Add('machine_boundary_sampling_gap') }
    }
    if ($proc.Count -lt $rules.minimum_process_samples -or $mach.Count -lt $rules.minimum_machine_samples) { $reasons.Add('insufficient_measured_samples') }
    if (@($proc | Where-Object missing_reason).Count -or @($mach | Where-Object missing_reason).Count) { $reasons.Add('missing_measured_counter') }
    if (@($proc | Where-Object { $_.delay_ms -gt $rules.process_max_gap_ms-$s.process_sample_ms }).Count) { $reasons.Add('process_sampling_delay') }
    for ($i=1;$i -lt $proc.Count;$i++) { if ($proc[$i].mono_ms-$proc[$i-1].mono_ms -gt $rules.process_max_gap_ms) { $reasons.Add('process_sampling_gap') } }
    $highSeconds=0.0;$previous=$null
    foreach ($m in $mach) {
        if ($null -ne $m.available_ram_percent -and $m.available_ram_percent -lt $s.run_available_ram_percent_min) { $reasons.Add('low_available_ram') }
        if ($null -ne $previous) {
            $elapsed=($m.mono_ms-$previous.mono_ms)/1000
            if ($elapsed*1000 -gt $rules.machine_max_gap_ms) { $reasons.Add('machine_sampling_gap') }
            if ($elapsed -gt 0 -and -not $m.missing_reason -and -not $previous.missing_reason) {
                $targetPercent=100*($m.cpu_seconds-$previous.cpu_seconds)/$elapsed/$logicalCpus
                $observerPercent=100*($m.controller_cpu_seconds-$previous.controller_cpu_seconds)/$elapsed/$logicalCpus
                $other=[math]::Max(0,$m.machine_cpu_percent-$targetPercent-$observerPercent)
                if ($other -gt $s.other_cpu_percent_max) { $highSeconds+=$elapsed } else { $highSeconds=0 }
                if ($highSeconds -ge $s.other_cpu_seconds_max) { $reasons.Add('sustained_other_cpu_estimate') }
            } else { $highSeconds=0 }
        }
        $previous=$m
    }
    if ($proc.Count -gt 0 -and -not @($proc | Where-Object missing_reason).Count) {
        $ws=@($proc | ForEach-Object { $_.working_set_bytes/1MB });$private=@($proc | ForEach-Object { $_.private_bytes/1MB })
        $r.ws_median_mib=Get-Median $ws;$r.ws_p95_mib=Get-P95 $ws;$r.ws_sample_max_mib=($ws | Measure-Object -Maximum).Maximum
        $r.private_median_mib=Get-Median $private;$r.private_p95_mib=Get-P95 $private;$r.private_sample_max_mib=($private | Measure-Object -Maximum).Maximum
    }
    $peaks=@($rows | Where-Object { $_.kind -eq 'process' -and $null -ne $_.lifetime_peak_working_set_bytes } | ForEach-Object { $_.lifetime_peak_working_set_bytes/1MB })
    if ($peaks.Count) { $r.lifetime_peak_ws_mib=($peaks | Measure-Object -Maximum).Maximum }
    $r.excluded_reason=(@($reasons | Select-Object -Unique) -join ';');$r.run_valid=$r.correct -and $r.exit_code -eq 0 -and $reasons.Count -eq 0
    $row=[pscustomobject]$r;Add-CsvRow (Join-Path $OutputDir 'runs.csv') $row;$allRuns.Add($row)
    $pairSoFar=@($allRuns | Where-Object pair_id -eq $entry.pair_id)
    if ($pairSoFar.Count -eq 2 -and $pairSoFar[0].correct -and $pairSoFar[1].correct) {
        if ($pairSoFar[0].output_hash -cne $pairSoFar[1].output_hash -or $pairSoFar[0].completed_bytes -ne $pairSoFar[1].completed_bytes -or $pairSoFar[0].completed_ops -ne $pairSoFar[1].completed_ops) {
            Write-JsonFile (Join-Path $OutputDir 'failure.json') @{pair_id=$entry.pair_id;reason='pair_same_work_or_hash_mismatch'}
            throw 'Pair byte/operation/hash mismatch; stop before further performance work.'
        }
    }
    Write-Host "$($entry.pair_id) $($entry.language): correct=$($r.correct), valid=$($r.run_valid) $($r.excluded_reason)"
    if ($fatal) { Write-JsonFile (Join-Path $OutputDir 'failure.json') @{run_id=$entry.run_id;reason=$r.excluded_reason};throw 'Benchmark/protocol/correctness failed; attempt retained. Fix before measuring again.' }
    if (-not $r.correct) { break } # No unattended idle retries; stop when the machine is not ready.
}
# Pair admission is persisted separately; runs.csv is never rewritten to hide an original run.
$pairs=@(foreach ($group in ($allRuns | Group-Object pair_id)) {
    $g=@($group.Group | Where-Object language -eq 'go');$u=@($group.Group | Where-Object language -eq 'rust')
    $ok=$g.Count -eq 1 -and $u.Count -eq 1 -and $g[0].run_valid -and $u[0].run_valid
    $reason=(@($group.Group.excluded_reason | Where-Object { $_ }) -join ';')
    if ($g.Count -eq 1 -and $u.Count -eq 1 -and ($g[0].output_hash -cne $u[0].output_hash -or $g[0].completed_bytes -ne $u[0].completed_bytes -or $g[0].completed_ops -ne $u[0].completed_ops)) { $ok=$false;$reason+=';pair_same_work_or_hash_mismatch' }
    [pscustomobject]@{pair_id=$group.Name;valid=$ok;reason=$reason}
})
Write-JsonFile (Join-Path $OutputDir 'pairs.json') $pairs
if ($Stage -eq 'pilot') {
    $sat=@($allRuns | Where-Object { $_.condition -eq 'saturated' -and $_.correct -and $_.wall_ms -gt 0 })
    $advised=$null
    if ($sat.Count -eq 4) { $fast=($sat.wall_ms | Measure-Object -Minimum).Minimum;$advised=[math]::Ceiling($Repetitions*[math]::Max(1,1.2*$s.minimum_saturated_seconds*1000/$fast)) }
    $accepted=$allRuns.Count -eq 8 -and @($pairs | Where-Object { -not $_.valid }).Count -eq 0 -and $sat.Count -eq 4
    Write-JsonFile (Join-Path $OutputDir 'pilot.json') @{accepted=$accepted;repetitions=$Repetitions;advised_repetitions=$advised;identity=$identity;machine_json=$machineJson;note='If not accepted, repeat all 8 pilot runs in a new directory at advised repetitions. Freeze an accepted value before main measurement.'}
    Write-Host "Pilot accepted=$accepted; advised repetitions=$advised. Main must use an accepted pilot at exactly the same repetitions."
}
Write-Host "Raw attempt saved. Run summarize.ps1 -ResultDir <this directory>."
