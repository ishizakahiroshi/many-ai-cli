# Dùng local LLM (Ollama / LM Studio) trên host Windows từ guest Hyper-V

> Bản dịch tiếng Việt của [`manual_local-llm-hyperv-host.md`](manual_local-llm-hyperv-host.md).  
> Bản dịch: 2026-07-18.

## Tổng quan

Chạy Ollama và model trên host Windows; từ many-ai-cli trong guest Windows (Hyper-V) nối API Ollama của host. Cấu hình khi muốn dùng GPU host (ví dụ RTX 4060).

LM Studio trên host cũng cùng nguyên lý (chạy host, guest nối API). Khác: cổng (mặc định 1234) · cách public server · key cấu hình many-ai-cli — xem «Trường hợp LM Studio (khác Ollama)» cuối tài liệu.

Ví dụ đã xác nhận:

- Ollama: `0.30.6`
- GPU: NVIDIA GeForce RTX 4060 / VRAM 8188 MiB
- Cổng API Ollama: `11434`
- IP host phía external switch: `192.168.11.50`
- IP host phía Default Switch: `172.20.224.1`

**Không** public API ra internet. **Không** port forward trên router.

## Phía host Windows

### Cho Ollama listen mọi interface

Chạy trên PowerShell host Windows.

```powershell
[Environment]::SetEnvironmentVariable(
    "OLLAMA_HOST",
    "0.0.0.0:11434",
    "User"
)
```

Xác nhận:

```powershell
[Environment]::GetEnvironmentVariable("OLLAMA_HOST", "User")
```

Kỳ vọng:

```text
0.0.0.0:11434
```

Sau khi set, thoát Ollama trên system tray rồi start lại. Nếu tray không thoát được:

```powershell
Get-Process | Where-Object {
    $_.ProcessName -like "ollama*"
} | Stop-Process

Start-Process "$env:LOCALAPPDATA\Programs\Ollama\ollama app.exe"
```

### Kiểm tra trạng thái listen

```powershell
Get-NetTCPConnection -LocalPort 11434 -State Listen |
    Select-Object LocalAddress, LocalPort, OwningProcess
```

`LocalAddress` phải là `0.0.0.0`. Nếu `127.0.0.1` thì guest không nối được — kiểm tra đã restart Ollama sau khi set `OLLAMA_HOST`.

Host tự kiểm tra API:

```powershell
Invoke-RestMethod http://127.0.0.1:11434/api/tags
```

### Cấu hình Windows Firewall

Chạy PowerShell admin. Xem IPv4 guest bằng `ipconfig`, chỉ cho phép dải tương ứng.

Guest `192.168.11.x`:

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

Guest `172.20.x.x`:

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

Dải Default Switch có thể đổi theo Windows. Sau khi đổi, kiểm tra lại:

```powershell
Get-NetIPAddress -InterfaceAlias "vEthernet (Default Switch)"
```

Xác nhận rule:

```powershell
Get-NetFirewallRule -DisplayName "Ollama API from Hyper-V Guest"
```

Tạo lại:

```powershell
Remove-NetFirewallRule -DisplayName "Ollama API from Hyper-V Guest"
```

## Lưu model trên host

VRAM 8GB: bắt đầu model quantized lớp 7B–8B.

```powershell
ollama pull qwen3:8b
ollama list
ollama run qwen3:8b "日本語で短く自己紹介してください"
```

Trong lúc model chạy, PowerShell khác kiểm tra GPU:

```powershell
nvidia-smi
ollama ps
```

`PROCESSOR` trong `ollama ps` chỉ GPU → đang suy luận bằng GPU host.

## Phía guest Windows

### Thử kết nối

External switch:

```powershell
Test-NetConnection 192.168.11.50 -Port 11434
Invoke-RestMethod http://192.168.11.50:11434/api/tags
```

Default Switch:

```powershell
Test-NetConnection 172.20.224.1 -Port 11434
Invoke-RestMethod http://172.20.224.1:11434/api/tags
```

Điều kiện thành công:

```text
TcpTestSucceeded : True
```

### Cấu hình đích many-ai-cli

Trên guest, trong `~/.many-ai-cli/config.yaml`, set base URL Ollama của host. **Không** gắn `/v1` hay `/api/tags`.

External switch:

```yaml
ollama:
  base_url: "http://192.168.11.50:11434"
```

Default Switch:

```yaml
ollama:
  base_url: "http://172.20.224.1:11434"
```

Với cấu hình này, `/api/models` của Hub lấy `<base_url>/api/tags`; session spawn route Ollama được inject env:

- Claude Code: `ANTHROPIC_BASE_URL=<base_url>`
- Codex: `OPENAI_BASE_URL=<base_url>/v1`

## Trường hợp LM Studio (khác Ollama)

LM Studio cũng chạy host Windows, guest nối vào. Gần giống Ollama; khác 3 điểm: «cách public server» · «cổng (mặc định 1234)» · «key cấu hình many-ai-cli».

Điều kiện: **LM Studio 0.4.1 trở lên**. Bắt buộc nếu nối Claude Code kiểu Anthropic-compatible (bản cũ không có endpoint Anthropic → Claude không dùng được). Chỉ Codex (OpenAI-compatible) thì bản cũ hơn vẫn được.

### Host: public ra local network

Trên tab Developer (server) của LM Studio, start local server và **bật public ra local network**. Mặc định chỉ listen localhost → guest không nối (tương đương `OLLAMA_HOST=0.0.0.0` của Ollama).

- Server Port: `1234` (mặc định)
- Bật «Serve on Local Network»
- Version có Anthropic-compatible (0.4.1+) nếu dùng Claude

### Firewall: mở 1234

Ngoài 11434 của Ollama, cho phép 1234 tương tự (đổi port rule Ollama thành 1234).

```powershell
New-NetFirewallRule `
    -DisplayName "LM Studio API from Hyper-V Guest" `
    -Direction Inbound `
    -Action Allow `
    -Protocol TCP `
    -LocalPort 1234 `
    -RemoteAddress 192.168.11.0/24 `
    -Profile Private
```

Phía Default Switch: `-RemoteAddress 172.20.224.0/20` (hoặc dải subnet guest tương ứng).

### Guest: kiểm tra thông

```powershell
Test-NetConnection <hostIP> -Port 1234
Invoke-RestMethod http://<hostIP>:1234/v1/models
```

`/v1/models` là OpenAI-compatible (cho Codex). Claude cần version có Anthropic-compatible `/v1/messages` (0.4.1+).

### Cấu hình đích many-ai-cli

Trong config.yaml set `lm_studio.base_url`. **Không** gắn `/v1` (many-ai-cli tự gắn).

```yaml
lm_studio:
  base_url: "http://<hostIP>:1234"
```

### Lưu ý context window của model

Claude Code dùng nhiều context; trên LM Studio cần context model load **≥ 32K** mới thực dụng (dưới 32K chỉ demo chat).

## Troubleshooting

### `TcpTestSucceeded` = `False`

Kiểm tra lần lượt phía host:

```powershell
[Environment]::GetEnvironmentVariable("OLLAMA_HOST", "User")

Get-NetTCPConnection -LocalPort 11434 -State Listen

Get-NetFirewallRule -DisplayName "Ollama API from Hyper-V Guest"
```

Điểm kiểm:

- Đã restart Ollama sau khi set `OLLAMA_HOST`?
- `LocalAddress` là `0.0.0.0` hoặc IP host?
- `RemoteAddress` firewall cùng subnet với IP guest?
- IP host khớp virtual switch Hyper-V mà guest đang dùng?

### API nối được nhưng không dùng GPU

Trong lúc model chạy:

```powershell
ollama ps
nvidia-smi
```

Restart Ollama vẫn không cải thiện → cập nhật NVIDIA driver và Ollama.

### Model không vừa VRAM

Dùng model nhỏ hơn hoặc mức quantize nhỏ hơn. Model quá lớn trên VRAM 8GB → một phần chuyển CPU, tốc độ giảm.

## Thông tin chính thức

- Ollama Windows: https://docs.ollama.com/windows
- Ollama API: https://docs.ollama.com/api
- Danh sách model Ollama: https://ollama.com/search
- Tài liệu LM Studio: https://lmstudio.ai/docs
