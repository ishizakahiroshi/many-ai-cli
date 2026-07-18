# Task cấu hình AI agent: remote server (server đơn / bản pnpm)

> Bản dịch tiếng Việt của [`manual_remote-server-agent-single.md`](manual_remote-server-agent-single.md).  
> Bản dịch: 2026-07-18.

File này là **chỉ dẫn task cấu hình dán nguyên vào AI agent** (Claude Code / Codex CLI, v.v. trên PC local). Agent chạy trên PC local, cấu hình remote qua SSH, rồi tạo launcher profile trên máy local. **Không dùng Docker** (cài `many-ai-cli` bằng pnpm trên remote, khởi động Hub bằng `serve` — cấu hình server đơn). `serve` có thể chỉ bật khi cần, hoặc thường trú bằng systemd / tmux / nohup (thường trú không phải đặc quyền của bản Docker).

Cách dùng:

1. Điền «biến» bên dưới bằng giá trị của bạn (hoặc truyền miệng cho agent khi dán).
2. Dán toàn bộ nội dung file vào AI agent và bảo «thực hiện quy trình này».
3. Agent tự làm SSH · cài đặt · verify · tạo profile.

---

## Chỉ dẫn cho agent (từ đây trở xuống: thực thi nguyên văn)

Bạn là AI agent chạy trên PC local. Theo các bước dưới, cài `many-ai-cli` bằng pnpm trên remote server và cho phép mở Hub từ PC local qua SSH tunnel. **Không thao tác phá hủy · không công khai ra ngoài.** Mỗi bước chỉ sang bước tiếp khi đạt verify; nếu không, báo nguyên nhân và dừng.

### Biến (chốt trước khi chạy)

| Biến | Mô tả | Ví dụ |
|---|---|---|
| `SSH_TARGET` | Đích SSH (`user@host` hoặc alias `~/.ssh/config`) | `your-user@remote.example.com` |
| `SSH_KEY` | Đường dẫn private key (để trống nếu đã khai trong config) | `C:\dev\.ssh\id_ed25519` |
| `HUB_PORT` | Cổng Hub (cùng số trên local và remote) | `47777` |
| `REMOTE_CWD` | Thư mục làm việc remote cho session | `~/work` |
| `PROVIDERS` | Provider CLI cần cài | `claude codex` |
| `PROFILE_NAME` | Tên launcher profile trên local | `my-remote` |

> Nếu giá trị chưa chốt, hỏi user **một lần gộp** rồi mới tiếp tục.

### Kiểm tra điều kiện (C1)

Kiểm tra SSH thông và OS.

```bash
ssh SSH_TARGET 'uname -a && echo OK-SSH'
```

verify: trả về `OK-SSH`. Nếu không, rà thông tin kết nối rồi dừng.

### Chuẩn bị pnpm / Node (C2)

Nếu remote chưa có pnpm thì cài; chuyển Node sang quản lý bởi pnpm. Viết **idempotent** (đã có thì không làm gì).

```bash
ssh SSH_TARGET 'bash -lc "
  set -e
  command -v pnpm >/dev/null || curl -fsSL https://get.pnpm.io/install.sh | sh -
  export PNPM_HOME=\"\$HOME/.local/share/pnpm\"; export PATH=\"\$PNPM_HOME:\$PATH\"
  command -v node >/dev/null || pnpm env use --global lts
  echo PNPM=\$(pnpm -v) NODE=\$(node -v)
"'
```

verify: hiện cả `PNPM=...` và `NODE=...`.

> Lưu ý: installer pnpm ghi PATH vào `~/.bashrc`. Các bước sau chạy bằng `bash -lc` (login shell) để PATH được nạp.

### Cài many-ai-cli và provider CLI (C3)

```bash
ssh SSH_TARGET 'bash -lc "
  set -e
  pnpm add -g many-ai-cli
  pnpm add -g @anthropic-ai/claude-code @openai/codex @github/copilot
  many-ai-cli --version
"'
```

Theo `PROVIDERS`, có thể bỏ gói không cần. `cursor-agent` không cài được bằng pnpm — nếu cần, hướng dẫn installer chính thức riêng (task này không xử lý).

verify: `many-ai-cli --version` in ra version.

> Đăng nhập provider CLI (`claude` / `codex login`, v.v.) cần tương tác · ủy quyền cá nhân — **không làm trong task này**. Báo user tự chạy sau trên remote shell hoặc Hub UI.

### Test khởi động và kiểm tra bảo mật (C4)

Khởi động tạm Hub, xác nhận listen chỉ trên loopback, rồi dừng ngay.

```bash
ssh SSH_TARGET 'bash -lc "
  set -e
  mkdir -p REMOTE_CWD
  cd REMOTE_CWD
  nohup many-ai-cli serve --port HUB_PORT >/tmp/aac-serve.log 2>&1 &
  sleep 4
  echo === listen ===; ss -ltnp | grep :HUB_PORT || true
  echo === banner ===; grep -m1 Open: /tmp/aac-serve.log || true
  many-ai-cli stop || pkill -f \"many-ai-cli serve\" || true
"'
```

verify (cả hai; nếu không đạt thì dừng và báo):

- Dòng listen chỉ là `127.0.0.1:HUB_PORT`. Nếu thấy `0.0.0.0:HUB_PORT` hoặc public IP thì **dừng cấu hình và cảnh báo**.
- Banner có `Open: http://127.0.0.1:HUB_PORT/?token=...`.

### Tạo launcher profile trên local (C5)

**Thêm** profile chế độ serve vào `~/.many-ai-cli/launcher-profiles.yaml` trên PC local (Windows: `%USERPROFILE%\.many-ai-cli\launcher-profiles.yaml`). Nếu đã có `profiles:` thì append vào mảng; không thì tạo mới.

```yaml
version: 1
profiles:
  - name: PROFILE_NAME
    type: ssh
    mode: serve
    host: <phần host của SSH_TARGET>
    user: <phần user của SSH_TARGET>
    identity_file: <giá trị SSH_KEY. Để trống thì bỏ cả dòng>
    hub_port: HUB_PORT
    cwd: REMOTE_CWD
```

Agent đã biết các giá trị khi SSH (host/user từ `SSH_TARGET`, port từ `HUB_PORT`, cwd từ `REMOTE_CWD`). Thực chất chỉ cần user xác nhận **khóa (`SSH_KEY`)**:

- Có `SSH_KEY` → ghi nguyên vào `identity_file:`.
- `SSH_KEY` trống (ủy quyền cho ssh-agent / key trong `~/.ssh/config`) → bỏ hẳn dòng `identity_file:`.
- `SSH_TARGET` là alias → ghi alias vào `host:`, có thể bỏ `user:` (key cũng do config).
- `host:` phải là **tên local reach được** (DNS VPS / public IP / alias config). Không ghi IP LAN/container chỉ thấy trên server.

verify: YAML hợp lệ, không phá profile cũ (backup trước khi append).

### Xác nhận khởi động (C6)

```powershell
many-ai-cli-launcher.exe --profile PROFILE_NAME
```

verify: trình duyệt mặc định mở `http://127.0.0.1:HUB_PORT/?token=...`, hiện Hub UI. Badge env_kind thuộc nhóm `Remote server`.

> Nếu `many-ai-cli-launcher.exe` không có trong PATH, hướng dẫn user lấy từ zip release và thêm vào PATH (mục «Launcher» trong README).

### Báo cáo hoàn tất (C7)

Gộp một lần:

- Version `many-ai-cli` / provider đã cài
- Kết quả: listen remote chỉ `127.0.0.1:HUB_PORT`
- Tên launcher profile và đường dẫn file
- **Việc user làm tiếp bằng tay**: đăng nhập provider CLI (ủy quyền cá nhân), spawn session từ Hub UI

## Cấm (bắt buộc)

- Bind `0.0.0.0` / public IP / LAN IP; `ssh -g` / `ssh -R` / `GatewayPorts`; reverse proxy public
- **Công khai bằng Tailscale Funnel** (toàn cầu — trái chính sách. Mobile: HTTPS trong tailnet qua `tailscale serve`, hoặc SSH tunnel)
- Dán URL có token vào log · chat; commit token vào file
- Thay đổi phá hủy file remote hiện có · SSH config · profile khác
- Tự ý thao tác thông tin đăng nhập provider

## Hình thức vận hành (mỗi lần / thường trú)

C4 chỉ khởi động tạm để kiểm tra listen. Production có thể: (a) chỉ bật `serve` khi dùng, hoặc (b) systemd service / tmux·screen / nohup thường trú. Thường trú không chỉ của Docker — server đơn cũng để `serve` chạy liên tục được. Với Hub thường trú (b), chuyển launcher sang profile chế độ tunnel để reconnect mà không restart `serve` mỗi lần.

### Ví dụ profile tunnel tới Hub thường trú (1 máy vật lý · không Docker)

Khi `serve` thường trú trên máy vật lý và chuyển launcher từ `serve` → `tunnel`. **Không ghi nhớ giá trị token**; đặt lệnh lấy token từ `config.yaml` qua SSH vào `token_command` (dạng thuần, không bọc `docker exec` như bản Docker).

```yaml
profiles:
  - name: PROFILE_NAME
    type: ssh
    mode: tunnel
    host: <phần host của SSH_TARGET>
    user: <phần user của SSH_TARGET>
    identity_file: <giá trị SSH_KEY. Để trống thì bỏ cả dòng>
    hub_port: HUB_PORT          # cùng số với serve --port · cố định (tunnel không cho auto)
    token_command: "grep ^token ~/.many-ai-cli/config.yaml | cut -d' ' -f2"
```

Lưu ý:

- Nếu chưa từng chạy `serve`, remote chưa có token trong `~/.many-ai-cli/config.yaml` → `token_command` trả rỗng → kết nối fail. **Chạy `serve` ít nhất một lần** để sinh token trước.
- Giữ `serve` thường trú với `--port HUB_PORT` cố định (chế độ tunnel không cho `hub_port` auto=0).

## Liên quan

- Lối vào (cài gì, đi quy trình nào): [manual_remote-server-overview.md](manual_remote-server-overview.md)
- Cơ chế & xử lý sự cố: [manual_remote-server-ssh-tunnel.md](manual_remote-server-ssh-tunnel.md)
- Task cấu hình bản Docker: [manual_remote-server-agent-docker.md](manual_remote-server-agent-docker.md)
