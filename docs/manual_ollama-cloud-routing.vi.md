# Quy trình dùng Codex / Claude Code qua Ollama Cloud

> Bản dịch tiếng Việt của [`manual_ollama-cloud-routing.md`](manual_ollama-cloud-routing.md).  
> Bản dịch: 2026-07-18.

## Tổng quan

Hướng dẫn vận hành để nối Codex CLI và Claude Code tới model Ollama Cloud **qua Ollama local**. Môi trường không có API key Anthropic / OpenAI vẫn dùng được model cloud nếu đã `ollama signin`.

Nhờ `plan_spawn-model-picker-ollama.md`, many-ai-cli v0.3.0+ cho phép **chọn trực tiếp model Ollama Cloud / Local trên form spawn của Hub rồi khởi động**. Không cần set env hay profile trên parent shell (Hub inject env cần thiết lúc spawn).

Quy trình cũ «set env tay trên shell» bị hạ xuống [Phụ lục: set env trên parent shell](#phụ-lục-set-env-trên-parent-shell) cuối tài liệu. Giữ làm fallback khi không dùng được cách Hub (ví dụ khởi động CLI trực tiếp).

## Điều kiện tiên quyết

- Ollama 0.15 trở lên (cần subcommand `ollama launch`)
- Đã `ollama signin` (tài khoản gắn quyền dùng cloud model)
- Binary `claude` / `codex` có trên PATH
- many-ai-cli v0.3.0 trở lên

## Đường kết nối và trách nhiệm API key

| Đường | Cần gì | Key thật Anthropic / OpenAI |
|---|---|---|
| Qua daemon local (khuyến nghị) | Ollama local đã `ollama signin` | Không cần |
| Thẳng ollama.com | `OLLAMA_API_KEY` (do ollama.com cấp) | Không cần |
| API gốc thẳng | Key thật Anthropic / OpenAI | Cần |

Qua daemon local: `OPENAI_API_KEY` (Codex) / `ANTHROPIC_AUTH_TOKEN` (Claude Code) chỉ cần **chuỗi dummy `ollama`** (API required nhưng giá trị bị bỏ qua).

## Codex × Ollama Cloud

### Base URL và dạng kết nối của Codex

- base URL: `http://localhost:11434/v1`
- Auth: không cần (khuyến nghị qua profile; cũng chỉ định trực tiếp bằng CLI arg)

### Ví dụ profile `~/.codex/config.toml`

```toml
[model_providers.ollama-launch]
name = "Ollama"
base_url = "http://localhost:11434/v1"
wire_api = "responses"

[profiles.ollama-cloud]
model = "gpt-oss:120b-cloud"
model_provider = "ollama-launch"
```

### Cách khởi động

Qua profile:

```powershell
codex --profile ollama-cloud
```

Chỉ định trực tiếp bằng CLI arg:

```powershell
codex --oss -m gpt-oss:120b-cloud
```

Qua subcommand `ollama launch` (Ollama 0.15+):

```powershell
ollama launch codex
```

### Tên model cloud dùng được với Codex (dạng `<name>:<size>-cloud`)

- `gpt-oss:120b-cloud`
- `gpt-oss:20b-cloud`
- `deepseek-v3.1:671-cloud`
- `qwen3-coder:480b-cloud`

Danh sách mới nhất: `https://ollama.com/search?c=cloud`.

## Claude Code × Ollama Cloud

### Base URL và dạng kết nối của Claude Code

- base URL: `http://localhost:11434` (**không** gắn `/v1`. SDK tự gắn `/v1/messages`)
- Auth: `ANTHROPIC_AUTH_TOKEN` = chuỗi dummy; **bắt buộc set rõ** `ANTHROPIC_API_KEY` thành chuỗi rỗng

### Biến môi trường (ví dụ PowerShell)

```powershell
$env:ANTHROPIC_AUTH_TOKEN = "ollama"
$env:ANTHROPIC_API_KEY = ""
$env:ANTHROPIC_BASE_URL = "http://localhost:11434"
claude --model kimi-k2.5:cloud
```

Nếu không set rõ `ANTHROPIC_API_KEY` rỗng, Claude Code có thể fallback về kết nối Anthropic gốc (cần lưu ý).

### Qua subcommand `ollama launch` (Ollama 0.15+)

```powershell
ollama launch claude
ollama launch claude --model kimi-k2.5:cloud
```

`ollama launch claude` tự set env → an toàn hơn env tay.

### Alias model (khi cần)

Nếu code phía Claude Code yêu cầu tên mặc định `claude-3-5-sonnet`, tạo alias phía Ollama:

```powershell
ollama cp qwen3-coder claude-3-5-sonnet
```

### Tên model cloud dùng được với Claude Code (dạng `<name>:cloud`)

Đã xác nhận qua example chính thức:

- `kimi-k2.5:cloud`
- `glm-5:cloud`
- `qwen3.5:cloud`
- `qwen3-coder:cloud`
- `deepseek-v3.2:cloud`
- `minimax-m2.5:cloud`

Danh sách mới nhất: `https://ollama.com/search?c=cloud`.

## Khởi động qua many-ai-cli (khuyến nghị: cách Hub UI)

### Cơ chế

- Form spawn Hub UI: mở combobox «Model» → chọn từ nhóm `[Anthropic]` / `[OpenAI]` / `[Ollama Cloud]` / `[Ollama Local]`
- Backend `/api/models` gộp:
  - Anthropic / OpenAI: danh sách hardcode
  - Ollama Cloud: alias có `remote_host` từ `/api/tags` của daemon local
  - Ollama Local: `/api/tags` tại `ollama.base_url` (mặc định `http://localhost:11434/api/tags`, cache 60s) + merge `local_models:` viết tay trong `~/.many-ai-cli/config.yaml`
- Tự suy route từ model đã chọn → gắn field `route` trong spawn payload
- Hub inject env preset theo route vào process con:
  - `route=ollama` × claude → `ANTHROPIC_AUTH_TOKEN=ollama` / `ANTHROPIC_API_KEY=` (rỗng) / `ANTHROPIC_BASE_URL=<ollama.base_url>`
  - `route=ollama` × codex → `OPENAI_API_KEY=ollama` / `OPENAI_BASE_URL=<ollama.base_url>/v1`
  - `route=anthropic` / `route=openai` → kế thừa shell env hiện có (không inject)

### Spawn Codex với Ollama Cloud (Hub UI)

1. `many-ai-cli serve` khởi động Hub (tự mở trình duyệt)
2. Mở form spawn, provider = `codex`
3. Bấm Model → nhóm `[Ollama Cloud]` chọn ví dụ `gpt-oss:120b-cloud`
4. Launch
5. Process con được inject `OPENAI_BASE_URL=http://localhost:11434/v1`, v.v. và chạy qua Ollama

### Spawn Claude Code với Ollama Cloud (Hub UI)

1. Provider = `claude`
2. Model → nhóm `[Ollama Cloud]` chọn ví dụ `kimi-k2.5:cloud`
3. Launch
4. Tự inject `ANTHROPIC_BASE_URL=http://localhost:11434`, v.v.

### Khi daemon Ollama ở host khác

Ví dụ Hub trong guest Hyper-V, Ollama + GPU trên host Windows — set đích trong `~/.many-ai-cli/config.yaml`:

```yaml
ollama:
  base_url: "http://192.168.11.50:11434"
```

Ví dụ phía Default Switch:

```yaml
ollama:
  base_url: "http://172.20.224.1:11434"
```

Giá trị này dùng cho cả lấy `/api/tags` của `/api/models` và inject `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` lúc spawn. **Không** gắn `/v1` hay `/api/tags` vào `base_url`.

### Dùng Local LLM (đã pull)

- Daemon Ollama đang chạy → nhóm `[Ollama Local]` liệt kê model từ `/api/tags`
- Daemon tắt vẫn hiện nếu viết tay trong `~/.many-ai-cli/config.yaml`:

```yaml
local_models:
  - id: "llama3.2:3b"
    label: "Llama 3.2 (nhẹ)"
  - id: "qwen3:14b"
```

### Làm mới

Nút `↻` cạnh ô Model ép lấy lại `/api/models`. Sau `ollama pull <model>` bấm nút này để danh sách cập nhật.

### Có cần restart process Hub không?

Env inject **theo từng process con lúc spawn** — không cần restart Hub. Đổi route và spawn session mới là đủ.

## Chính sách metadata hiển thị (đặc tả tương lai)

Khi hoàn tất provider generalization, dự kiến Hub UI / log hiển thị các field sau (v0.3.0 chưa implement — ghi memo thiết kế).

| Field | Ví dụ |
|---|---|
| `provider` | `codex`, `claude` |
| `route` | `ollama`, `anthropic`, `openai` |
| `model` | `gpt-oss:120b-cloud`, `kimi-k2.5:cloud` |
| `displayTitle` | `Codex (Ollama)`, `Claude Code (Ollama)` |
| `displaySubtitle` | `gpt-oss:120b-cloud` |

Hiển thị cơ bản (vùng rộng):

```text
Codex (Ollama)
gpt-oss:120b-cloud
```

```text
Claude Code (Ollama)
kimi-k2.5:cloud
```

Rút gọn vùng hẹp:

```text
Codex · Ollama · gpt-oss
Claude · Ollama · kimi-k2.5
```

Hiển thị đầy đủ log / chi tiết session:

```text
Provider: Codex
Route: Ollama
Model: gpt-oss:120b-cloud
Command: codex --profile ollama-cloud
```

Thứ tự ưu tiên lấy tên model:

1. `model` chỉ rõ trên spawn UI hoặc config
2. CLI arg `--model` / `-m`
3. Model resolve an toàn từ profile đã biết
4. Không rõ thì để trống, chỉ hiện tới route (ví dụ chỉ `Codex (Ollama)`)

## Trở lại kết nối thường

### Đưa Codex về OpenAI gốc

Chỉ cần đổi profile:

```powershell
codex --profile default        # nếu ~/.codex/config.toml có profile thường
# hoặc khởi động không arg (dùng profile mặc định)
codex
```

Nếu đã set riêng `OPENAI_API_KEY`, key đó vẫn dùng cho auth OpenAI thẳng.

### Đưa Claude Code về Anthropic gốc

Gỡ env:

```powershell
Remove-Item Env:ANTHROPIC_AUTH_TOKEN
Remove-Item Env:ANTHROPIC_BASE_URL
$env:ANTHROPIC_API_KEY = "<key Anthropic thật>"   # khi dùng kết nối gốc
claude
```

Env còn sót → Claude Code vẫn trỏ Ollama local. Phải **restart Hub + dọn env shell** cùng lúc.

## Quy trình kiểm chứng

Kiểm chứng trên máy thật do user chạy. Lần lượt 6 góc dưới.

### 1. Daemon Ollama Cloud offline

- Tình huống: daemon local tắt nhưng reach được `ollama.com/api/tags`
- Kỳ vọng: ô Model Hub UI chỉ hiện `[Anthropic]` / `[OpenAI]` / `[Ollama Cloud]`
- Kiểm tra: response `/api/models?token=...` có `warnings` chứa `ollama_daemon_unreachable` (khi config không có `local_models`)

### 2. Daemon Ollama chưa start + Cloud không reach

- Tình huống: cả daemon và Cloud API đều không dùng được
- Kỳ vọng: chỉ `[Anthropic]` / `[OpenAI]`. Bỏ cả `[Ollama Cloud]` / `[Ollama Local]`
- `warnings` có cả `ollama_cloud_fetch_failed` và `ollama_daemon_unreachable`

### 3. Cả hai đang chạy

- Tình huống: daemon local + Cloud API OK
- Kỳ vọng: đủ mọi nhóm. Chọn → Launch → inject env preset và start
- Trong PTY con, `echo $env:ANTHROPIC_BASE_URL` (PowerShell) thấy giá trị đã inject

### 4. Tự suy route khi gõ tay

- Gõ tên không có trong datalist kiểu `something:cloud` rồi Launch
- Kỳ vọng: có `:cloud` → ước `route=ollama` → inject env Ollama
- Tên model Anthropic kiểu `claude-opus-4-7` → `route=anthropic` (không inject)

### 5. Khởi động thường hiện có

- Không chọn gì (trống) hoặc gõ `claude-sonnet-4-5` rồi Launch
- Kỳ vọng: không inject env. Đi đường Anthropic / OpenAI hiện có (tương thích hành vi cũ)

### 6. Nút refresh

- Sau `ollama pull <model>`, bấm `↻` cạnh ô Model
- Kỳ vọng: model vừa pull xuất hiện trong nhóm `[Ollama Local]`

## Phân tách khi sự cố

| Triệu chứng | Ứng viên nguyên nhân | Kiểm tra |
|---|---|---|
| Claude Code nối Anthropic gốc | `ANTHROPIC_API_KEY` chưa rỗng / `ANTHROPIC_BASE_URL` chưa set | `Get-ChildItem Env:ANTHROPIC_*` |
| Codex trả model OpenAI kiểu `gpt-4o-mini` | Chưa apply profile / thiếu `--profile` | `codex --profile ollama-cloud --help` |
| `ollama: command not found` | Chưa cài Ollama 0.15+ | `ollama --version` |
| Không pull được cloud model | Chưa `ollama signin` / không có quyền cloud | `ollama whoami` |
| Qua Hub thì env không có hiệu lực | Env process Hub cũ | `many-ai-cli stop && many-ai-cli serve` |
| Không có `ollama launch` | Ollama < 0.15 | Cập nhật version |

## Liên quan

- Kế hoạch cha: [plan_ollama-cloud-codex-claude.md](plan_ollama-cloud-codex-claude.md)
- Kế hoạch implement: [plan_spawn-model-picker-ollama.md](plan_spawn-model-picker-ollama.md)
- Kế hoạch provider generalization: `docs/local/archive/v0.1.3/plan_provider-extensibility_20260510.md`
- Danh sách model cloud Ollama: `https://ollama.com/search?c=cloud`
- Tài liệu tích hợp chính thức Ollama: `https://docs.ollama.com/integrations/codex` / `https://docs.ollama.com/integrations/claude-code`

---

## Phụ lục: set env trên parent shell

many-ai-cli v0.3.0+ ưu tiên cách Hub UI; các trường hợp sau vẫn dùng cách shell env cũ:

- Khởi động `claude` / `codex` trực tiếp, không qua `many-ai-cli`
- Fallback khi setup chưa hỗ trợ inject env preset của Hub
- Qua subcommand `ollama launch <provider>` (Ollama 0.15+)

### Spawn Codex với Ollama Cloud (cách shell)

```powershell
codex --profile ollama-cloud
# hoặc
codex --oss -m gpt-oss:120b-cloud
# hoặc
ollama launch codex
```

### Spawn Claude Code với Ollama Cloud (cách shell)

```powershell
$env:ANTHROPIC_AUTH_TOKEN = "ollama"
$env:ANTHROPIC_API_KEY = ""
$env:ANTHROPIC_BASE_URL = "http://localhost:11434"
claude --model kimi-k2.5:cloud
# hoặc
ollama launch claude --model kimi-k2.5:cloud
```

`ollama launch claude` tự set env → an toàn hơn env tay.

### Khởi động many-ai-cli serve kèm shell env

Muốn cả many-ai-cli «Ollama hóa» theo cách shell: start Hub từ shell đã export env:

```powershell
$env:ANTHROPIC_AUTH_TOKEN = "ollama"
$env:ANTHROPIC_API_KEY = ""
$env:ANTHROPIC_BASE_URL = "http://localhost:11434"
many-ai-cli serve
```

Cách này cố định cả Hub chỉ Ollama; muốn trộn session nhiều route thì dùng cách Hub UI (đổi route từng lần spawn).
