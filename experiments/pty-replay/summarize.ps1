# Read-only toward raw input files. Produces summary.json, pair-metrics.csv and summary.md.
#Requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string]$ResultDir)
. "$PSScriptRoot/common.ps1"
$ResultDir=(Resolve-Path -LiteralPath $ResultDir).Path
$metadata=Get-Content -LiteralPath (Join-Path $ResultDir 'metadata.json') -Raw | ConvertFrom-Json
$schedule=Get-Content -LiteralPath (Join-Path $ResultDir 'schedule.json') -Raw | ConvertFrom-Json
$correctPath=Join-Path $ResultDir 'correctness.json'
$correct=if (Test-Path $correctPath) { Get-Content $correctPath -Raw | ConvertFrom-Json } else { $null }
$runsPath=Join-Path $ResultDir 'runs.csv'
$runs=if (Test-Path $runsPath) { @(Import-Csv -LiteralPath $runsPath) } else { @() }
function Number-OrNull($Value) {
    if ($null -eq $Value -or [string]::IsNullOrWhiteSpace([string]$Value)) { return $null }
    $v=0.0
    if (-not [double]::TryParse([string]$Value,[Globalization.NumberStyles]::Float,[cultureinfo]::InvariantCulture,[ref]$v) -or [double]::IsNaN($v) -or [double]::IsInfinity($v)) { return $null }
    return $v
}
function Format-Number($Value) { if ($null -eq $Value) { return 'null (unavailable)' }; return ([double]$Value).ToString('0.######',[cultureinfo]::InvariantCulture) }
$metrics=@('startup_ready_ms','wall_ms','controller_wall_ms','snapshot_ms','cpu_seconds','cpu_seconds_per_gib','average_cores','cpu_one_core_percent','cpu_machine_percent','controller_cpu_seconds','throughput_mib_per_second','ws_median_mib','ws_p95_mib','ws_sample_max_mib','private_median_mib','private_p95_mib','private_sample_max_mib','lifetime_peak_ws_mib','max_lateness_ms')
$exclusions=@{}
$exclusionsPath=Join-Path $ResultDir 'exclusions.csv'
if (Test-Path $exclusionsPath) {
    foreach ($e in (Import-Csv -LiteralPath $exclusionsPath)) {
        if (-not $e.pair_id -or -not $e.reason -or $e.pair_id -notin @($schedule.runs.pair_id)) { throw 'exclusions.csv needs a scheduled pair_id and nonempty reason on every row.' }
        $exclusions[$e.pair_id]=@($exclusions[$e.pair_id]) + $e.reason
    }
}
$pairRows=[Collections.Generic.List[object]]::new();$admission=[Collections.Generic.List[object]]::new();$acceptedRuns=[Collections.Generic.List[object]]::new()
$gateOk=$null -ne $correct -and $correct.ok -eq $true -and @($correct.results).Count -eq 2
if ($gateOk) {
    foreach ($language in @('go','rust')) {
        $v=@($correct.results | Where-Object language -eq $language)
        if ($v.Count -ne 1 -or $v[0].ok -ne $true -or $v[0].result.ok -ne $true -or $v[0].result.language -cne $language -or $v[0].result.event -cne 'VERIFY' -or $v[0].result.concurrency_ok -ne $true -or $v[0].result.cases -le 0 -or $v[0].result.checkpoints -le 0) { $gateOk=$false }
    }
    foreach ($key in @('build_sha256','fixture_manifest_sha256','scenario_sha256')) {
        if ($correct.identity.$key -cne $metadata.identity.$key) { $gateOk=$false }
    }
    if ($correct.results[0].result.checkpoints -ne $correct.results[1].result.checkpoints -or $correct.results[0].result.cases -ne $correct.results[1].result.cases) { $gateOk=$false }
}
$scheduleIntact=(Get-FileHash (Join-Path $ResultDir 'schedule.json') -Algorithm SHA256).Hash.ToLowerInvariant() -ceq $metadata.schedule_sha256
foreach ($group in (@($schedule.runs) | Group-Object pair_id)) {
    $pairId=$group.Name;$reasons=[Collections.Generic.List[string]]::new()
    $p=@($runs | Where-Object pair_id -eq $pairId);$go=@($p | Where-Object language -eq 'go');$rust=@($p | Where-Object language -eq 'rust')
    if (-not $gateOk) { $reasons.Add('correctness_gate_missing_or_failed') }
    if (-not $scheduleIntact) { $reasons.Add('frozen_schedule_changed') }
    if ($p.Count -ne 2 -or $go.Count -ne 1 -or $rust.Count -ne 1) { $reasons.Add('pair_missing_or_duplicate_run') }
    foreach ($r in $p) {
        $planned=@($group.Group | Where-Object run_id -eq $r.run_id)
        if ($planned.Count -ne 1 -or $r.attempt_id -cne $schedule.attempt_id -or $r.repetitions -ne [string]$schedule.repetitions) { $reasons.Add('schedule_identity_mismatch') }
        elseif ($r.language -cne $planned[0].language -or $r.condition -cne $planned[0].condition -or $r.order -ne [string]$planned[0].order) { $reasons.Add('scheduled_run_fields_mismatch') }
        if ($r.correct -cne 'True' -or $r.run_valid -cne 'True' -or $r.exit_code -ne '0') { $reasons.Add("$($r.language):$($r.excluded_reason)") }
        foreach ($m in $metrics) { if ($null -eq (Number-OrNull $r.$m)) { $reasons.Add("missing_metric:$($r.language):$m") } }
    }
    if ($go.Count -eq 1 -and $rust.Count -eq 1) {
        foreach ($f in @('condition','expected_bytes','completed_bytes','expected_ops','completed_ops','total','output_hash')) {
            if ([string]::IsNullOrWhiteSpace($go[0].$f) -or $go[0].$f -cne $rust[0].$f) { $reasons.Add("pair_mismatch:$f") }
        }
    }
    if ($exclusions.ContainsKey($pairId)) { foreach ($reason in $exclusions[$pairId]) { $reasons.Add("manual:$reason") } }
    $valid=$reasons.Count -eq 0
    $admission.Add([pscustomobject]@{pair_id=$pairId;condition=$group.Group[0].condition;valid=$valid;reason=(@($reasons | Select-Object -Unique) -join ';')})
    if (-not $valid) { continue }
    $g=$go[0];$u=$rust[0];$acceptedRuns.Add($g);$acceptedRuns.Add($u)
    foreach ($m in $metrics) {
        $gv=Number-OrNull $g.$m;$uv=Number-OrNull $u.$m
        $ratio=if ($gv -ne 0) { $uv/$gv } else { $null }
        $percent=if ($null -ne $ratio) { 100*($ratio-1) } else { $null }
        $pairRows.Add([pscustomobject]@{pair_id=$pairId;condition=$g.condition;metric=$m;go=$gv;rust=$uv;rust_over_go=$ratio;rust_minus_go=$uv-$gv;rust_change_percent=$percent;go_minus_rust_saving=$gv-$uv;ratio_missing_reason=$(if ($null -eq $ratio) {'go_denominator_zero'} else {$null})})
    }
}
$summary=[Collections.Generic.List[object]]::new();$lines=[Collections.Generic.List[string]]::new()
$lines.Add('# PTY replay comparison');$lines.Add('')
$lines.Add("Stage: $($metadata.stage). Attempt: $($metadata.attempt_id).")
$lines.Add('')
if ($metadata.stage -eq 'verify') {
    $status=if ($gateOk) { 'unmeasured: correctness only' } else { 'invalid: correctness incomplete/failed' }
} elseif (-not $gateOk -or (Test-Path (Join-Path $ResultDir 'failure.json'))) { $status='invalid: correctness/protocol/benchmark failure' }
elseif ($metadata.stage -ne 'measure') { $status='inconclusive: pilot only; not main measurement' }
elseif (@($admission | Where-Object valid).Count -ne 20) { $status='inconclusive: fewer than 10 valid pairs in one or both conditions' }
else { $status='complete paired measurement; no migration decision or threshold claim' }
$lines.Add("Status: $status.");$lines.Add('')
foreach ($condition in @('paced','saturated')) {
    $validCount=@($admission | Where-Object { $_.condition -eq $condition -and $_.valid }).Count
    $lines.Add("## $condition ($validCount valid pairs)")
    $lines.Add('')
    if ($validCount -eq 0) { $lines.Add('No valid paired performance data.');$lines.Add('');continue }
    $lines.Add('| Metric | Go median | Go run-p95 | Rust median | Rust run-p95 | Median paired R/G | Median paired Rust-Go |')
    $lines.Add('|---|---:|---:|---:|---:|---:|---:|')
    foreach ($metric in $metrics) {
        $p=@($pairRows | Where-Object { $_.condition -eq $condition -and $_.metric -eq $metric })
        $row=[pscustomobject]@{condition=$condition;metric=$metric;valid_pairs=$validCount;go_median=Get-Median @($p.go);go_run_p95=Get-P95 @($p.go);rust_median=Get-Median @($p.rust);rust_run_p95=Get-P95 @($p.rust);paired_ratio_median=Get-Median @($p.rust_over_go);paired_ratio_p95=Get-P95 @($p.rust_over_go);paired_rust_minus_go_median=Get-Median @($p.rust_minus_go);paired_percent_change_median=Get-Median @($p.rust_change_percent);paired_go_minus_rust_saving_median=Get-Median @($p.go_minus_rust_saving)}
        $summary.Add($row)
        $lines.Add("| $metric | $(Format-Number $row.go_median) | $(Format-Number $row.go_run_p95) | $(Format-Number $row.rust_median) | $(Format-Number $row.rust_run_p95) | $(Format-Number $row.paired_ratio_median) | $(Format-Number $row.paired_rust_minus_go_median) |")
    }
    $lines.Add('')
    foreach ($metric in @('average_cores','ws_median_mib','private_median_mib')) {
        $x=@($summary | Where-Object { $_.condition -eq $condition -and $_.metric -eq $metric })[0]
        $lines.Add("Paired absolute saving (Go minus Rust), $metric: $(Format-Number $x.paired_go_minus_rust_saving_median); Rust change: $(Format-Number $x.paired_percent_change_median)%.")
    }
    $lines.Add('')
}
$lines.Add('## Exclusions and missing data');$lines.Add('')
foreach ($p in $admission) { if (-not $p.valid) { $lines.Add("- $($p.pair_id): $($p.reason)") } }
$lines.Add('')
$lines.Add('## Interpretation limits');$lines.Add('')
$lines.Add('- Raw runs.csv and samples.csv retain every original run/sample. Invalid or incomplete pairs are excluded together; originals are never rewritten. Manual exclusions.csv is retained and hashed in summary.json.')
$lines.Add('- A null ratio means Go was zero or data was unavailable. Zero CPU at OS timer resolution cannot establish a percentage improvement. No missing value is replaced by zero.')
$lines.Add('- CPU seconds/GiB uses completed measured append bytes, excluding prefill. Average cores uses matching external handshake wall time, not child-only wall time. CPU includes START dispatch and MEASURED serialization/flush; checksum/oracle work occurs after CHECK, outside CPU timing.')
$lines.Add('- Only measured process samples feed Working Set/Private Bytes aggregates. Working Set includes shared resident pages; Private Bytes is private committed memory, not resident RAM. Lifetime peak includes startup/warmup and is not a steady peak.')
$lines.Add('- Every p95 uses nearest rank. Run-p95 summarizes independent runs, not operations. Within-run sampled memory p95 is a different statistic. With 10 pairs the run-p95 is the maximum and is coarse.')
$lines.Add('- Each paired ratio is Rust divided by Go before aggregation. Ratios below 1 mean a smaller value, which is favorable for costs but unfavorable for throughput. Absolute deltas and percentages are reported alongside ratios.')
$lines.Add('- PC CPU includes the controller and OS; subtracted other-load estimates are approximate. Polling may miss brief peaks. AC/power overlay and transient external work need user observation; active scheme is checked before and after.')
$lines.Add('- Synthetic standalone EXEs do not quantify real wrapper bottlenecks, whole-product memory savings, FFI or IPC overhead, or a language migration decision. No proposed adoption thresholds are automatically applied.')
$inputHashes=[ordered]@{}
foreach ($name in @('metadata.json','schedule.json','correctness.json','runs.csv','samples.csv','pairs.json','failure.json','exclusions.csv')) { $path=Join-Path $ResultDir $name;if (Test-Path $path) { $inputHashes[$name]=(Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant() } }
Write-JsonFile (Join-Path $ResultDir 'summary.json') @{status=$status;created_utc=[DateTime]::UtcNow.ToString('o');input_sha256=$inputHashes;pair_admission=$admission.ToArray();metrics=$summary.ToArray();paired_metrics=$pairRows.ToArray()}
$pairPath=Join-Path $ResultDir 'pair-metrics.csv'
if ($pairRows.Count) { $pairRows | Export-Csv -LiteralPath $pairPath -NoTypeInformation -Encoding utf8 } else { '"pair_id","condition","metric","go","rust","rust_over_go","rust_minus_go","rust_change_percent","go_minus_rust_saving","ratio_missing_reason"' | Set-Content $pairPath -Encoding utf8 }
$lines | Set-Content -LiteralPath (Join-Path $ResultDir 'summary.md') -Encoding utf8
Write-Host $status
