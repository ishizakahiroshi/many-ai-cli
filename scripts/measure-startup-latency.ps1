[CmdletBinding()]
param(
    # Run this only against a disposable Hub: each iteration starts a real AI CLI session.
    [Parameter(Mandatory)]
    [ValidatePattern('^https?://')]
    [string]$HubUrl,

    [Parameter(Mandatory)]
    [ValidateNotNullOrEmpty()]
    [string]$Token,

    [ValidateSet('claude', 'codex', 'copilot', 'cursor-agent', 'opencode', 'grok')]
    [string]$Provider = 'claude',

    [string]$Cwd = (Get-Location).Path,

    [ValidateRange(1, 100)]
    [int]$Iterations = 10,

    [ValidateRange(1, 60)]
    [int]$TimeoutSeconds = 15,

    [string]$LogPath = (Join-Path $HOME '.many-ai-cli/logs/hub.log'),

    # Calls /api/kill-all at the end. Never use this with a Hub that has work in progress.
    [switch]$Cleanup
)

$ErrorActionPreference = 'Stop'
$HubUrl = $HubUrl.TrimEnd('/')

if (-not (Test-Path -LiteralPath $Cwd -PathType Container)) {
    throw "Cwd does not exist or is not a directory: $Cwd"
}
if (-not (Test-Path -LiteralPath $LogPath -PathType Leaf)) {
    throw "Hub log does not exist: $LogPath. Start the Hub first and pass its log path with -LogPath if needed."
}

$headers = @{ Authorization = "Bearer $Token" }

function Read-LogDelta {
    param([long]$Offset)

    $stream = [System.IO.File]::Open(
        $LogPath,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read,
        [System.IO.FileShare]::ReadWrite
    )
    try {
        if ($stream.Length -le $Offset) {
            return ''
        }
        [void]$stream.Seek($Offset, [System.IO.SeekOrigin]::Begin)
        $reader = [System.IO.StreamReader]::new($stream)
        try {
            return $reader.ReadToEnd()
        }
        finally {
            $reader.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

$measurements = [System.Collections.Generic.List[double]]::new()
for ($i = 1; $i -le $Iterations; $i++) {
    $logOffset = (Get-Item -LiteralPath $LogPath).Length
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $body = @{ provider = $Provider; cwd = $Cwd } | ConvertTo-Json -Compress
    Invoke-RestMethod -Method Post -Uri "$HubUrl/api/spawn" -Headers $headers -ContentType 'application/json' -Body $body | Out-Null

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $measurement = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $delta = Read-LogDelta -Offset $logOffset
        if ($delta -match 'startup_latency_probe.*pre_ack_total_ms=(?<milliseconds>\d+)') {
            # The probe is emitted immediately before the wrapper receives its register ACK.
            # It intentionally measures Hub-side pre-prompt latency, not model response time.
            $measurement = [double]$Matches.milliseconds
            break
        }
        Start-Sleep -Milliseconds 50
    }
    $watch.Stop()

    if ($null -eq $measurement) {
        throw "Iteration $i did not emit startup_latency_probe within $TimeoutSeconds seconds (wall time: $([math]::Round($watch.Elapsed.TotalMilliseconds)) ms)."
    }
    $measurements.Add($measurement)
    Write-Host ("{0,2}/{1}: pre-ACK {2} ms" -f $i, $Iterations, $measurement)
}

$ordered = @($measurements | Sort-Object)
function Get-Percentile {
    param([double[]]$Values, [double]$Percentile)

    $index = [math]::Ceiling($Percentile * $Values.Count) - 1
    return $Values[[math]::Max(0, [math]::Min($index, $Values.Count - 1))]
}

$result = [ordered]@{
    provider = $Provider
    iterations = $ordered.Count
    p50_ms = Get-Percentile -Values $ordered -Percentile 0.50
    p95_ms = Get-Percentile -Values $ordered -Percentile 0.95
    max_ms = $ordered[$ordered.Count - 1]
}
$result | ConvertTo-Json

if ($Cleanup) {
    Invoke-RestMethod -Method Post -Uri "$HubUrl/api/kill-all" -Headers $headers | Out-Null
}
