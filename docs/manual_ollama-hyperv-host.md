# Hyper-V ゲストからホスト Windows の Ollama を使う手順

> 最終更新: 2026-06-19(金) 07:51:46

## 概要

Ollama とモデルをホスト Windows 側で動かし、Hyper-V ゲスト Windows 上の many-ai-cli からホストの Ollama API へ接続する。RTX 4060 などホスト側 GPU を使いたい場合の構成。

確認済み例:

- Ollama: `0.30.6`
- GPU: NVIDIA GeForce RTX 4060 / VRAM 8188 MiB
- Ollama API ポート: `11434`
- 外部スイッチ側ホスト IP: `192.168.11.50`
- Default Switch 側ホスト IP: `172.20.224.1`

API をインターネットへ公開しない。ルーターのポート転送は設定しない。

## ホスト Windows 側

### Ollama を全インターフェイスで待ち受ける

ホスト Windows の PowerShell で実行する。

```powershell
[Environment]::SetEnvironmentVariable(
    "OLLAMA_HOST",
    "0.0.0.0:11434",
    "User"
)
```

確認:

```powershell
[Environment]::GetEnvironmentVariable("OLLAMA_HOST", "User")
```

期待値:

```text
0.0.0.0:11434
```

設定後、タスクトレイの Ollama を終了して起動し直す。タスクトレイから終了できない場合:

```powershell
Get-Process | Where-Object {
    $_.ProcessName -like "ollama*"
} | Stop-Process

Start-Process "$env:LOCALAPPDATA\Programs\Ollama\ollama app.exe"
```

### 待受状態を確認する

```powershell
Get-NetTCPConnection -LocalPort 11434 -State Listen |
    Select-Object LocalAddress, LocalPort, OwningProcess
```

`LocalAddress` が `0.0.0.0` になっていることを確認する。`127.0.0.1` の場合はゲストから接続できないため、`OLLAMA_HOST` 設定後に Ollama が再起動されているか確認する。

ホスト自身から API を確認する。

```powershell
Invoke-RestMethod http://127.0.0.1:11434/api/tags
```

### Windows Firewall を設定する

管理者 PowerShell で実行する。ゲスト側の `ipconfig` で IPv4 アドレスを確認し、該当する範囲だけ許可する。

ゲストが `192.168.11.x` の場合:

```powershell
New-NetFirewallRule `
    -DisplayName "Ollama API from Hyper-V Guest" `
    -Direction Inbound `
    -Action Allow `
    -Protocol TCP `
    -LocalPort 11434 `
    -RemoteAddress 192.168.11.0/24 `
    -Profile Private
```

ゲストが `172.20.x.x` の場合:

```powershell
New-NetFirewallRule `
    -DisplayName "Ollama API from Hyper-V Guest" `
    -Direction Inbound `
    -Action Allow `
    -Protocol TCP `
    -LocalPort 11434 `
    -RemoteAddress 172.20.224.0/20 `
    -Profile Private
```

Default Switch の範囲は Windows により変わる場合がある。変更後は次で再確認する。

```powershell
Get-NetIPAddress -InterfaceAlias "vEthernet (Default Switch)"
```

ルール確認:

```powershell
Get-NetFirewallRule -DisplayName "Ollama API from Hyper-V Guest"
```

作り直す場合:

```powershell
Remove-NetFirewallRule -DisplayName "Ollama API from Hyper-V Guest"
```

## モデルをホストへ保存する

8GB VRAM では、最初は 7B から 8B クラスの量子化モデルを使う。

```powershell
ollama pull qwen3:8b
ollama list
ollama run qwen3:8b "日本語で短く自己紹介してください"
```

モデル実行中に別 PowerShell で GPU 使用状況を確認する。

```powershell
nvidia-smi
ollama ps
```

`ollama ps` の `PROCESSOR` が GPU 使用を示していれば、ホスト側 GPU で推論している。

## ゲスト Windows 側

### 接続試験

外部スイッチを使用している場合:

```powershell
Test-NetConnection 192.168.11.50 -Port 11434
Invoke-RestMethod http://192.168.11.50:11434/api/tags
```

Default Switch を使用している場合:

```powershell
Test-NetConnection 172.20.224.1 -Port 11434
Invoke-RestMethod http://172.20.224.1:11434/api/tags
```

成功条件:

```text
TcpTestSucceeded : True
```

### many-ai-cli の接続先を設定する

ゲスト Windows 側の `~/.many-ai-cli/config.yaml` に、ホストの Ollama base URL を設定する。`/v1` や `/api/tags` は付けない。

外部スイッチの場合:

```yaml
ollama:
  base_url: "http://192.168.11.50:11434"
```

Default Switch の場合:

```yaml
ollama:
  base_url: "http://172.20.224.1:11434"
```

この設定により、Hub の `/api/models` は `<base_url>/api/tags` を取得し、Ollama route で spawn したセッションには次の env が注入される。

- Claude Code: `ANTHROPIC_BASE_URL=<base_url>`
- Codex: `OPENAI_BASE_URL=<base_url>/v1`

## トラブルシューティング

### `TcpTestSucceeded` が `False`

ホスト側で順番に確認する。

```powershell
[Environment]::GetEnvironmentVariable("OLLAMA_HOST", "User")

Get-NetTCPConnection -LocalPort 11434 -State Listen

Get-NetFirewallRule -DisplayName "Ollama API from Hyper-V Guest"
```

確認点:

- `OLLAMA_HOST` 設定後に Ollama を再起動したか
- `LocalAddress` が `0.0.0.0` またはホスト IP か
- Firewall の `RemoteAddress` とゲスト IP が同じサブネットか
- ゲストが接続している Hyper-V 仮想スイッチに対応するホスト IP を使っているか

### API には接続できるが GPU を使わない

モデル実行中に確認する。

```powershell
ollama ps
nvidia-smi
```

Ollama を再起動しても改善しない場合は、NVIDIA ドライバーと Ollama を更新する。

### モデルが VRAM に収まらない

より小さいモデルまたは量子化サイズを使う。8GB VRAM で大きすぎるモデルを選ぶと、一部が CPU へ移り速度が低下する。

## 公式情報

- Ollama Windows: https://docs.ollama.com/windows
- Ollama API: https://docs.ollama.com/api
- Ollama モデル一覧: https://ollama.com/search
