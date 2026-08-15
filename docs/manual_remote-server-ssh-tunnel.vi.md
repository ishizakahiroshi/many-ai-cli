# Quy trình vận hành SSH tunnel tới remote server

> Bản dịch tiếng Việt của [`manual_remote-server-ssh-tunnel.md`](manual_remote-server-ssh-tunnel.md).  
> Bản dịch: 2026-07-18.

> **⚠️ Bản dịch này là ảnh chụp tại một thời điểm và không còn được đồng bộ.**
>
> Kể từ 2026-08-15, các file `*.vi.md` trong repo này **không còn được cập nhật theo bản gốc**. Bản gốc [`manual_remote-server-ssh-tunnel.md`](manual_remote-server-ssh-tunnel.md) là chuẩn duy nhất — vui lòng đối chiếu bản gốc trước khi làm theo nội dung ở đây. (Lần dịch gần nhất: 2026-07-18.)
>
> Xin chân thành cảm ơn người đã đóng góp bản dịch. Chúng tôi vẫn hoan nghênh đóng góp dịch thuật; chỉ là không thể hứa giữ đồng bộ, nên bản được merge cũng sẽ là ảnh chụp có ghi ngày.
>
> *(EN: This translation is a dated snapshot and is no longer kept in sync with the original. Please refer to the source file linked above. The Vietnamese **UI** locale `web/src/i18n/vi.json` is a shipped feature and is still maintained.)*

## Tài liệu này là gì

Hướng dẫn **mở an toàn Hub `many-ai-cli` đang chạy trên remote server** từ trình duyệt trên PC local, **qua SSH tunnel**.

Hub của `many-ai-cli` theo thiết kế chỉ dành cho local và chỉ bind vào `127.0.0.1`. Vì vậy, để xem “Hub đang chạy trên remote” từ máy mình, cần SSH local forward (`-L`) nối 1-1 giữa *`127.0.0.1:<port>` trên máy local* và *`127.0.0.1:<port>` trên remote*. **Không** dùng public IP, bind `0.0.0.0`, hay reverse proxy (nginx / Caddy / Cloudflare Tunnel, v.v.).

Phạm vi: “đăng nhập SSH được vào remote server riêng, nối Hub trên đó an toàn qua SSH tunnel từ PC local”. Cả hai kiểu dùng đều thuộc phạm vi: khởi động `serve` mỗi lần cần, hoặc để `serve` thường trú trên remote (systemd / tmux, …) rồi chỉ tunnel khi cần.

## Quy tắc quan trọng nhất (kết luận trước)

**Mọi số cổng trên đường đi — cả phía local và remote — phải giống nhau.**

Hub kiểm tra `Host` / `Origin` theo `127.0.0.1:<cổng Hub>` (hoặc `localhost:<cổng Hub>`). Nếu cổng local và remote lệch nhau, HTML vẫn mở được nhưng WebSocket / API sẽ fail với `host not allowed`. Luôn viết `-L` với **cùng số cổng hai bên**.

```text
Trình duyệt PC local
  http://127.0.0.1:47777/?token=<token>
        │
        │ ssh -L 127.0.0.1:47777:127.0.0.1:47777   ← hai bên cùng 47777
        ▼
Hub many-ai-cli trên remote server
  http://127.0.0.1:47777/?token=<token>
```

## Quy trình ngắn nhất (thử cái này trước)

1. **Cài many-ai-cli trên remote (pnpm)**
   ```bash
   curl -fsSL https://get.pnpm.io/install.sh | sh -   # nếu chưa có pnpm
   pnpm env use --global lts                          # nếu chưa có Node
   pnpm add -g many-ai-cli
   many-ai-cli --version
   ```
2. **Khởi động Hub trên remote**
   ```bash
   cd ~/work && many-ai-cli serve --port 47777
   ```
   Từ log khởi động `Open: http://127.0.0.1:47777/?token=<token>`, **ghi lại cổng và token**.
3. **Lập tunnel trên PC local** (ví dụ PowerShell)
   ```powershell
   ssh -N -T -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 `
     -L 127.0.0.1:47777:127.0.0.1:47777 user@remote.example.com
   ```
4. **Mở trên trình duyệt local**: `http://127.0.0.1:47777/?token=<token>`

> Nếu lập tunnel tay mỗi lần phiền, dùng **kết nối tự động bằng launcher** (một profile tự động hóa bước 1–4) ở dưới. Muốn AI agent cấu hình trọn gói, xem các manual cấu hình ở mục «Liên quan» cuối file.

## Điều kiện tiên quyết

- Đăng nhập SSH được vào remote server
- Remote đã có `many-ai-cli` và provider CLI cần dùng (`claude` / `codex` / `copilot` / `cursor-agent`)
- Firewall / security group remote **không** mở cổng Hub ra ngoài (chỉ mở SSH)
- PC local có OpenSSH client
- Không chia sẻ `?token=...` trong URL Hub cho ai

## Kết nối tự động bằng launcher (khuyến nghị)

`many-ai-cli-launcher` (hỗ trợ Windows / Linux / macOS; trên Windows là `many-ai-cli-launcher.exe`) tự động hóa: lập SSH tunnel, khởi động Hub, mở trình duyệt trong một bước. Profile SSH `serve` / `tunnel` chạy trên mọi OS (`wsl` chỉ dành cho Windows). Mục «Thủ công» bên dưới là phương án thay thế khi không dùng được launcher.

Profile nằm trong `~/.many-ai-cli/launcher-profiles.yaml`. Có 2 chế độ kết nối:

| Chế độ | Khi nào dùng |
|---|---|
| `serve` | Mỗi lần kết nối, khởi động `serve` trên remote rồi nối (phù hợp khởi động theo nhu cầu) |
| `tunnel` | Chỉ nối tới Hub đã chạy sẵn (Hub `serve` thường trú trên remote, hoặc Docker compose thường trú) |

### Chế độ serve (khởi động Hub remote khi kết nối)

```yaml
version: 1
profiles:
  - name: my-remote
    type: ssh
    mode: serve           # khi kết nối, chạy many-ai-cli serve trên remote
    host: remote.example.com
    user: your-user
    hub_port: 47777       # 0 = tự chọn cổng
    cwd: /home/your-user/projects
```

`host` có thể là alias trong `~/.ssh/config`, hoặc dạng `user@host`.

Khởi động:

```powershell
many-ai-cli-launcher.exe --profile my-remote
```

Launcher làm gì:

1. Chạy `ssh.exe -t -L 47777:127.0.0.1:47777 your-user@remote.example.com -- bash -ilc "many-ai-cli serve --port 47777"`
2. Bắt URL Hub từ banner khởi động remote (nếu cổng trùng, thử lại tối đa 5 lần, mỗi lần +100)
3. Mở Hub UI bằng trình duyệt mặc định (Windows)
4. Khi Ctrl+C hoặc thoát, dọn process `serve` trên remote

### Chế độ tunnel (chỉ lập tunnel tới Hub thường trú)

Dùng khi Hub đã thường trú trên remote (`serve` đơn lẻ bằng systemd / tmux / nohup, hoặc Docker compose với `restart: unless-stopped`). Không khởi động `serve` trên remote; chỉ tunnel tới Hub sẵn có.

```yaml
version: 1
profiles:
  - name: remote-docker
    type: ssh
    mode: tunnel          # chỉ tunnel tới Hub thường trú
    host: remote.example.com
    user: your-user
    hub_port: 47801       # cùng số cổng Hub đang listen
    token_command: "docker exec aac-user1 sh -c 'grep ^token ~/.many-ai-cli/config.yaml | cut -d\" \" -f2'"
```

`token_command` chạy trên remote; stdout (đã trim) trở thành token. Điều chỉnh tên container / đường dẫn config.yaml theo môi trường thực tế.

Khởi động:

```powershell
many-ai-cli-launcher.exe --profile remote-docker
```

Launcher làm gì:

1. Lập tunnel bằng `ssh.exe -N -L 47801:127.0.0.1:47801 your-user@remote.example.com`
2. Chạy `token_command` qua SSH để lấy token
3. Kiểm tra kết nối bằng `/api/info?token=<token>`
4. Đăng ký `/api/net-hint` với `ssh=true` · `host_label` · `env_kind=remote-tunnel`
5. Mở trình duyệt mặc định tới `http://127.0.0.1:47801/?token=<token>&via=ssh&host_label=<host>&env_kind=remote-tunnel`
6. Ctrl+C đóng tunnel (không dừng Hub remote)

### Khi có nhiều profile

```powershell
many-ai-cli-launcher.exe           # ≥ 2 profile → hiện màn hình chọn
many-ai-cli-launcher.exe --last    # dùng profile lần trước
many-ai-cli-launcher.exe --ui      # luôn hiện màn hình chọn
```

### Hiển thị nhận diện môi trường (tránh nhầm đối tượng thao tác)

Hub UI dựa trên `env_kind` từ `/api/info` để đổi title tab, favicon, badge header, và Settings/About. Khi mở nhiều Hub cùng lúc, kiểm tra màu tab và badge.

| env_kind | Hiển thị | favicon | Trường hợp chính |
|---|---|---|---|
| `local` | Local | xanh + `L` | Hub local trên PC |
| `wsl` | WSL | xanh dương + `W` | Hub WSL / Hub WSL qua launcher Windows |
| `remote` | Remote server | cam + `R` | Hub trên remote do SSH khởi động |
| `remote-tunnel` | Remote server (tunnel) | đỏ + `T` | Chỉ tunnel tới Hub đã chạy (serve thường trú / Docker thường trú) |

Dù URL là `127.0.0.1`, nếu hiển thị `Remote server (tunnel)` thì filesystem / Git / log đang thao tác **toàn bộ ở phía remote**.

---

## Thủ công (khi không dùng được launcher)

### 1. Cài many-ai-cli trên remote (pnpm)

```bash
curl -fsSL https://get.pnpm.io/install.sh | sh -   # nếu chưa có pnpm
pnpm env use --global lts                          # nếu chưa có Node
pnpm add -g many-ai-cli
many-ai-cli --version
```

Cài và đăng nhập provider CLI trên remote (session chạy trên remote). Hầu hết là gói npm nên cài bằng pnpm:

```bash
pnpm add -g @anthropic-ai/claude-code @openai/codex @github/copilot
# cursor-agent: installer chính thức (không qua pnpm · tùy chọn)
```

### 2. Khởi động Hub trên remote

```bash
cd ~/work
many-ai-cli serve --port 47777
```

Ghi lại cổng và token từ URL trong log:

```text
Open: http://127.0.0.1:47777/?token=<token>
```

Nếu cổng trùng và chuyển sang `47778` trở đi, thay mọi `47777` sau đó bằng cổng thực tế. Muốn thường trú: dùng `tmux` / `screen` / `systemd`.

### 3. Lập tunnel từ PC local

PowerShell:

```powershell
ssh -N -T `
  -o ExitOnForwardFailure=yes `
  -o ServerAliveInterval=30 `
  -L 127.0.0.1:47777:127.0.0.1:47777 `
  user@remote.example.com
```

Git Bash / WSL / Linux / macOS:

```bash
ssh -N -T \
  -o ExitOnForwardFailure=yes \
  -o ServerAliveInterval=30 \
  -L 127.0.0.1:47777:127.0.0.1:47777 \
  user@remote.example.com
```

Thay `user@remote.example.com` bằng đích thực tế.

### 4. Mở trên trình duyệt local

```text
http://127.0.0.1:47777/?token=<token>
```

`localhost` cũng chạy được nhưng thống nhất dùng `127.0.0.1` để tránh nhầm.

### 5. Khởi động session

Spawn provider từ Hub UI, hoặc từ shell SSH khác trên remote chạy `many-ai-cli claude` / `many-ai-cli codex`, v.v. Filesystem · Git · log · nơi lưu file đính kèm **đều ở phía remote** (không phải file trên PC local).

### 6. Kết thúc

1. Session trên Hub UI  
2. Terminal SSH tunnel (`Ctrl+C`)  
3. Hub trên remote (`Ctrl+C` hoặc shell khác: `many-ai-cli stop`)

## Đổi cổng

Cổng phía local và cổng Hub remote **phải cùng số**. Ví dụ: nếu `47777` đang bận trên local, khởi động Hub remote bằng cùng một số khác và tunnel cùng số đó.

```bash
many-ai-cli serve --port 47877
```

```powershell
ssh -N -T -o ExitOnForwardFailure=yes `
  -L 127.0.0.1:47877:127.0.0.1:47877 user@remote.example.com
```

```text
http://127.0.0.1:47877/?token=<token>
```

Dạng `-L 127.0.0.1:47778:127.0.0.1:47777` (cổng hai bên khác nhau) khiến HTML lấy được nhưng WebSocket / POST API fail với `host not allowed` / `origin not allowed`.

## Cấm

- Cho Hub listen trên remote tại `0.0.0.0:<port>` / public IP / LAN IP
- `ssh -L 0.0.0.0:<port>:...` hoặc `ssh -g` để mở tunnel phía local ra LAN
- `GatewayPorts yes` + `ssh -R` để tạo cổng public phía remote
- Công khai Hub UI qua nginx / Caddy / Cloudflare Tunnel / Tailscale Funnel, v.v.
- Chia sẻ URL có token qua chat · Issue · log · screenshot
- Nhiều người thao tác cùng một Hub cùng lúc

## Xử lý sự cố

| Triệu chứng | Nguyên nhân chính | Xử lý |
|---|---|---|
| `ssh: bind: Address already in use` | Cùng cổng đang dùng trên local | Khởi động Hub remote bằng số khác (cùng hai bên) và tunnel cùng số đó |
| HTML mở nhưng terminal không nối | Cổng local / remote lệch | Sửa thành `-L 127.0.0.1:<p>:127.0.0.1:<p>` |
| `host not allowed` / `origin not allowed` | URL qua public host · cổng khác · proxy | Mở `http://127.0.0.1:<cổng Hub>/?token=...` |
| `404` / `403` / trang trắng | Token sai · token cũ · URL sau khi Hub restart | Dùng token mới nhất từ log khởi động remote |
| Đối tượng thao tác không như mong đợi | Hub đang chạy trên remote | Kiểm tra cwd · nhánh Git · đường dẫn log có ở remote không |
| Tunnel đứt giữa chừng | Mất kết nối SSH | Thêm `ServerAliveInterval=30`, kết nối lại nếu cần |

## Kiểm tra bảo mật

Trước và sau thao tác, kiểm tra listen trên remote:

```bash
ss -ltnp | grep ':47777'
```

Thay `47777` bằng cổng Hub thực tế. Kỳ vọng: chỉ listen `127.0.0.1:<cổng Hub>`. Nếu listen `0.0.0.0:<cổng Hub>` hoặc public IP thì dừng lại.

Không cho phép inbound cổng Hub trên firewall / cloud security group remote. Chỉ dùng SSH (`22/tcp` hoặc cổng SSH đang vận hành).

## Kết nối trực tiếp từ điện thoại (📱 mobile)

Muốn mở Hub **trực tiếp từ trình duyệt / PWA điện thoại** mà không qua PC local: dùng «📱 Kết nối mobile» trên Hub UI. Tạo QR «quét là nối thật» qua HTTPS của `tailscale serve`.

**Thiết lập phía PC đã được wizard tự động hóa** (chạy `tailscale serve --bg <port>`, thêm tên tailnet vào `allowed_hosts`, tạo QR URL thật — Hub tự làm). Người dùng chỉ còn 3 bước thủ công:

1. Cài app Tailscale trên cả PC và điện thoại (wizard có link cài / QR store).
2. **Đăng nhập cùng một tài khoản** trên cả hai để cùng một tailnet.
3. **Bật HTTPS của tailnet một lần** trên admin console (wizard deep-link dẫn). Lúc đó **bắt buộc bỏ tick Funnel** (công khai toàn cầu). Dùng serve chỉ trong tailnet; không dùng Funnel.

Sau đó wizard tự chẩn đoán trạng thái `serve`; khi `ready` sẽ đưa QR `https://<tên DNS thật>.<tailnet>.ts.net/?token=`. Quét để mở dashboard. Muốn dừng public: bấm «Dừng công khai» trên wizard (tương đương `tailscale serve --https=443 off`).

Lưu ý:

- **Bind vẫn là `127.0.0.1`**. `tailscale serve` proxy về loopback nên Hub không lộ LAN / ngoài.
- **Ưu tiên HTTPS**. Không dùng đường IP thô (`100.x`). `http://100.x` không phải secure context → Web Push / Service Worker / cài PWA / mic thoại bị tắt. `https://…ts.net` của `tailscale serve` thì đủ chức năng. SSH local forward `http://127.0.0.1:<port>` cũng là secure context, đủ chức năng.
- **Hub trong Docker container không dùng được `tailscale serve`** (không có CLI `tailscale` trong container). Wizard degrade và dẫn sang mobile qua SSH tunnel / launcher.
- QR có token **tương đương mật khẩu**. Ảnh rò rỉ = full access Hub — không chia sẻ.
- Thiết kế chi tiết: [local/plan_mobile-connect-flow-redesign.md](local/plan_mobile-connect-flow-redesign.md).

## Liên quan

- Muốn AI agent cấu hình trọn gói (dán vào AI trên PC local, cấu hình qua SSH):
  - Server đơn (pnpm): [manual_remote-server-agent-single.md](manual_remote-server-agent-single.md)
  - Docker: [manual_remote-server-agent-docker.md](manual_remote-server-agent-docker.md)
- Vận hành Docker đa user: [manual_docker-multiuser.md](manual_docker-multiuser.md)
- [Thiết kế v0.2.x: Security / Privacy](v0.2.x-any-ai-cli-design.md#17-security--privacy)
- [README.ja.md: Bảo mật](../README.ja.md#セキュリティ)
