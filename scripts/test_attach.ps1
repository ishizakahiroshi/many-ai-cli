#Requires -Version 7
# scripts/test_attach.ps1 — 画像転送（attach）機能の動作確認スクリプト
#
# 使い方:
#   pwsh scripts/test_attach.ps1
#   pwsh scripts/test_attach.ps1 -KeepHub   # Hub を停止しない

param(
    [int]$Port    = 47777,
    [switch]$KeepHub
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ProjectRoot = Split-Path $PSScriptRoot -Parent
$ExePath     = Join-Path $ProjectRoot 'ai-cli-hub.exe'

if (-not (Test-Path $ExePath)) {
    Write-Error "ai-cli-hub.exe が見つかりません: $ExePath`n先に 'go build ./cmd/ai-cli-hub' を実行してください"
}

# ---- WebSocket ヘルパー ----------------------------------------

function Connect-WS([string]$Url) {
    $ws = [System.Net.WebSockets.ClientWebSocket]::new()
    $ws.ConnectAsync([uri]$Url, [System.Threading.CancellationToken]::None).GetAwaiter().GetResult()
    return $ws
}

function Send-WsJson($ws, $obj) {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes(($obj | ConvertTo-Json -Compress))
    $ws.SendAsync([ArraySegment[byte]]::new($bytes),
        [System.Net.WebSockets.WebSocketMessageType]::Text, $true,
        [System.Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
}

function Send-WsBinary($ws, [byte[]]$data) {
    $ws.SendAsync([ArraySegment[byte]]::new($data),
        [System.Net.WebSockets.WebSocketMessageType]::Binary, $true,
        [System.Threading.CancellationToken]::None).GetAwaiter().GetResult() | Out-Null
}

function Recv-WsJson($ws, [int]$TimeoutMs = 3000) {
    $buf  = [byte[]]::new(65536)
    $task = $ws.ReceiveAsync([ArraySegment[byte]]::new($buf), [System.Threading.CancellationToken]::None)
    if (-not $task.Wait($TimeoutMs)) { throw "WS 受信タイムアウト (${TimeoutMs}ms)" }
    return [System.Text.Encoding]::UTF8.GetString($buf, 0, $task.Result.Count) | ConvertFrom-Json
}

# ---- Hub 起動 & トークン取得 ------------------------------------

$hubProcess = $null
$tmpLog     = [System.IO.Path]::GetTempFileName()

# Hub が未起動なら起動する
$alreadyRunning = $false
try {
    $r = Invoke-WebRequest "http://127.0.0.1:$Port/" -TimeoutSec 1 -ErrorAction Stop
    if ($r.StatusCode -in 200, 401) { $alreadyRunning = $true }
} catch {}

if ($alreadyRunning) {
    Write-Host "[INFO] Hub は既に起動中です（Port $Port）"
} else {
    Write-Host "[INFO] Hub を起動します..."
    $hubProcess = Start-Process -FilePath $ExePath -ArgumentList 'serve' `
        -PassThru -RedirectStandardOutput $tmpLog -NoNewWindow
    Write-Host "[INFO] Hub PID=$($hubProcess.Id)"
    Start-Sleep -Milliseconds 1200
}

# config.yaml からトークンを取得
$configPath = Join-Path $env:USERPROFILE '.ai-cli-hub\config.yaml'
$token = (Get-Content $configPath | Where-Object { $_ -match '^token:' }) -replace '^token:\s*', ''
if (-not $token) { throw "トークンを config.yaml から取得できませんでした: $configPath" }
Write-Host "[INFO] token=${token}"

# ---- 最小 PNG (1×1 白ピクセル) ---------------------------------

[byte[]]$pngBytes = (
    0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,  # PNG シグネチャ
    0x00,0x00,0x00,0x0D,0x49,0x48,0x44,0x52,  # IHDR
    0x00,0x00,0x00,0x01,0x00,0x00,0x00,0x01,  # 幅=1, 高さ=1
    0x08,0x02,0x00,0x00,0x00,0x90,0x77,0x53,0xDE,
    0x00,0x00,0x00,0x0C,0x49,0x44,0x41,0x54,  # IDAT
    0x08,0xD7,0x63,0xF8,0xFF,0xFF,0x3F,0x00,
    0x05,0xFE,0x02,0xFE,0xDC,0xCC,0x59,0xE7,
    0x00,0x00,0x00,0x00,0x49,0x45,0x4E,0x44,  # IEND
    0xAE,0x42,0x60,0x82
)

$wsUrl   = "ws://127.0.0.1:$Port/ws"
$pass    = 0
$fail    = 0

try {
    # Step 1: wrapper として接続
    Write-Host "`n[TEST 1/4] wrapper 接続 & register..."
    $wsWrapper = Connect-WS $wsUrl
    Send-WsJson $wsWrapper @{
        type         = 'register'
        role         = 'wrapper'
        provider     = 'claude'
        display_name = 'Claude Code (test)'
        cwd          = $PWD.Path
        pid          = $PID
        token        = $token
    }
    $regResp  = Recv-WsJson $wsWrapper
    $sessionID = $regResp.session_id
    Write-Host "[PASS] registered session_id=$sessionID"
    $pass++

    # Step 2: UI として接続
    Write-Host "`n[TEST 2/4] UI 接続..."
    $wsUI = Connect-WS $wsUrl
    Send-WsJson $wsUI @{ type = 'register'; role = 'ui'; token = $token; cols = 200; rows = 50 }
    # snapshot は受け取るが検証しない
    $null = Recv-WsJson $wsUI -TimeoutMs 1000
    Write-Host "[PASS] UI 接続完了"
    $pass++

    # Step 3: attach_request + PNG バイナリ送信
    Write-Host "`n[TEST 3/4] attach_request + PNG 送信 (session_id=$sessionID)..."
    Send-WsJson $wsUI @{ type = 'attach_request'; session_id = $sessionID }
    Send-WsBinary $wsUI $pngBytes
    Write-Host "[PASS] 送信完了"
    $pass++

    # Step 4: wrapper 側で attach_file を受信
    Write-Host "`n[TEST 4/4] wrapper 側の attach_file 受信を確認..."
    $attachMsg = $null
    for ($i = 0; $i -lt 30; $i++) {
        $buf  = [byte[]]::new(65536)
        $task = $wsWrapper.ReceiveAsync([ArraySegment[byte]]::new($buf), [System.Threading.CancellationToken]::None)
        if ($task.Wait(300)) {
            $json = [System.Text.Encoding]::UTF8.GetString($buf, 0, $task.Result.Count) | ConvertFrom-Json
            if ($json.type -eq 'attach_file') { $attachMsg = $json; break }
        }
    }

    if ($attachMsg) {
        Write-Host "[PASS] attach_file 受信"
        Write-Host "       path   = $($attachMsg.path)"
        Write-Host "       inject = $($attachMsg.inject.Trim())"

        if ($attachMsg.inject -like '@*') {
            Write-Host "[PASS] inject 形式 OK (claude: @<path>)"
        } else {
            Write-Warning "[WARN] inject 形式が期待値と異なります"
        }

        if (Test-Path $attachMsg.path) {
            Write-Host "[PASS] ファイル保存 OK: $($attachMsg.path)"
            $pass++
        } else {
            Write-Warning "[FAIL] ファイルが見つかりません: $($attachMsg.path)"
            $fail++
        }
    } else {
        Write-Warning "[FAIL] attach_file を受信できませんでした（タイムアウト）"
        $fail++
    }

} finally {
    if ($wsWrapper) { try { $wsWrapper.Dispose() } catch {} }
    if ($wsUI)      { try { $wsUI.Dispose()      } catch {} }
    if ($hubProcess -and -not $KeepHub) {
        Write-Host "`n[INFO] Hub を停止します (PID=$($hubProcess.Id))..."
        $hubProcess.Kill()
    }
    if (Test-Path $tmpLog) { Remove-Item $tmpLog -ErrorAction SilentlyContinue }
}

Write-Host "`n==============================="
Write-Host "結果: PASS=$pass / FAIL=$fail"
if ($fail -gt 0) { exit 1 }
