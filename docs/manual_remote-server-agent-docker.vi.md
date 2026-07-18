# Task cấu hình AI agent: remote server (Docker / đa user · cô lập)

> Bản dịch tiếng Việt của [`manual_remote-server-agent-docker.md`](manual_remote-server-agent-docker.md).  
> Bản dịch: 2026-07-18.

File này là **chỉ dẫn task cấu hình dán nguyên vào AI agent** (Claude Code / Codex CLI, v.v. trên PC local). Agent chạy trên PC local, qua SSH triển khai Docker compose `many-ai-cli` trên remote, khởi động thường trú có auto-restart, rồi tạo launcher profile (chế độ tunnel) trên máy local.

Khác với bản server đơn (pnpm + `serve`):

| | Server đơn | Bản Docker (file này) |
|---|---|---|
| Cài remote | pnpm global install | Pull image GHCR bằng compose |
| Khởi động Hub | `serve` tay hoặc systemd/tmux, v.v. (mỗi lần hoặc thường trú) | `restart: unless-stopped` thường trú + auto-restart |
| Chế độ launcher | `serve` | `tunnel` (chỉ lập tunnel) |
| Phù hợp | Bắt đầu nhanh · một user · không cần cô lập (có thể thường trú) | Cô lập nhiều user · chuẩn hóa auto-restart / auto-update |

※ Vẫn có thể để `serve` remote thường trú rồi nối bằng launcher `tunnel`. Chế độ ≠ khả năng thường trú.

Cách dùng:

1. Điền «biến» bên dưới.
2. Dán toàn bộ file vào AI agent và bảo «thực hiện quy trình này».
3. Agent tự triển khai · khởi động · verify · tạo profile.

Nguồn chuẩn trong repo: `deploy/docker/` (Dockerfile / entrypoint.sh / compose.yaml / users/*.yaml). Chi tiết vận hành đa user: [manual_docker-multiuser.md](manual_docker-multiuser.md) (task này là cấu hình tối thiểu «một user + nối từ local»).

---

## Chỉ dẫn cho agent (từ đây trở xuống: thực thi nguyên văn)

Bạn là AI agent chạy trên PC local. Theo các bước dưới, dựng **một** Hub `many-ai-cli` thường trú bằng Docker trên remote và cho phép mở từ PC local qua SSH tunnel. **Không thao tác phá hủy · không công khai ra ngoài.** Mỗi bước chỉ sang bước tiếp khi đạt verify; nếu không, báo nguyên nhân và dừng.

### Biến (chốt trước khi chạy)

| Biến | Mô tả | Ví dụ |
|---|---|---|
| `SSH_TARGET` | Đích SSH (`user@host` hoặc alias). Cần quyền chạy docker | `your-user@remote.example.com` |
| `SSH_KEY` | Đường dẫn private key (để trống nếu đã khai trong config) | `C:\dev\.ssh\id_ed25519` |
| `USER_TAG` | Tên phân biệt service/container/volume | `user1` |
| `HUB_PORT` | Cổng publish phía host (cùng số với local) | `47801` |
| `AAC_TAG` | Tag image theo dõi (`latest`=main / `develop`=thử) | `latest` |
| `REMOTE_BASE` | Thư mục đặt compose trên server | `/opt/many-ai-cli` |
| `WORK_DIR` | Thư mục làm việc mount vào container (phía host) | `/srv/many-ai-cli/work/user1` |
| `PROFILE_NAME` | Tên launcher profile trên local | `remote-docker` |

> Nếu giá trị chưa chốt, hỏi user **một lần gộp** rồi mới tiếp tục.  
> Trong container, Hub listen tại `HUB_PORT`; publish phía host là `127.0.0.1:HUB_PORT:48000` qua cổng nhận socat `48000` (cố định nội bộ). Host/Origin của Hub yêu cầu khớp cổng tuyệt đối nên `HUB_PORT` (env · host publish · tunnel local) **phải cùng một số**.

### Kiểm tra điều kiện (C1)

SSH thông · OS · Docker / compose.

```bash
ssh SSH_TARGET 'bash -lc "uname -a; docker version --format \"{{.Server.Version}}\"; docker compose version" && echo OK-PREREQ'
```

verify: ra `OK-PREREQ` và version Docker / compose. Không có Docker thì **dừng tại đây**, hướng dẫn cài ([manual_docker-multiuser.md](manual_docker-multiuser.md) mục «Ghi chép chuẩn bị server ban đầu») — task này **không** cài Docker.

### Triển khai cấu hình compose (C2)

Chuyển `deploy/docker/` trong repo sang `REMOTE_BASE/src` trên remote; chuẩn bị `compose.yaml` / `.env` / `users/USER_TAG.yaml` tại `REMOTE_BASE`. **Nếu file đã có, không ghi đè — so diff**.

Từ root repo trên PC local (chuyển HEAD đã commit):

```bash
git archive HEAD deploy | ssh SSH_TARGET 'bash -lc "
  set -e
  mkdir -p REMOTE_BASE/src REMOTE_BASE/users WORK_DIR
  tar -xf - -C REMOTE_BASE/src
  echo OK-SYNC
"'
```

Sinh `.env` (tag theo dõi) và định nghĩa user. User lấy template `deploy/docker/users/example.yaml`, thay `example`→`USER_TAG` · `47801`→`HUB_PORT`:

```bash
ssh SSH_TARGET 'bash -lc "
  set -e
  cd REMOTE_BASE
  printf \"AAC_TAG=AAC_TAG\n\" > .env
  sed -e \"s/example/USER_TAG/g\" -e \"s/47801/HUB_PORT/g\" \
    src/deploy/docker/users/example.yaml > users/USER_TAG.yaml
  # work dir khớp uid 1000 (ubuntu) trong container
  chown 1000:1000 WORK_DIR || sudo chown 1000:1000 WORK_DIR
  echo OK-RENDER
"'
```

Đưa `users/USER_TAG.yaml` vào `include` của `compose.yaml` (nếu chưa có, tạo từ mẫu `deploy/docker/compose.yaml`). Dạng tối thiểu có `name: many-ai-cli` và `include:`:

```yaml
name: many-ai-cli
include:
  - users/USER_TAG.yaml
```

verify: `ssh SSH_TARGET 'cd REMOTE_BASE && docker compose config --quiet && echo OK-CONFIG'` trả `OK-CONFIG` (cú pháp compose · tham chiếu hợp lệ).

### Pull image và khởi động (C3)

```bash
ssh SSH_TARGET 'bash -lc "
  set -e
  cd REMOTE_BASE
  docker compose pull --quiet aac-USER_TAG
  docker compose up -d aac-USER_TAG
  sleep 5
  docker ps --filter name=aac-USER_TAG --format \"{{.Names}} {{.Status}}\"
"'
```

verify: `aac-USER_TAG` ở trạng thái `Up` (sau đó `healthy`). Healthcheck mỗi 60s nên có thể mất 1–2 phút mới `healthy`. Vòng `Restarting` → xem `docker logs aac-USER_TAG` rồi dừng (điển hình: thiếu `HUB_PORT` khiến entrypoint thoát ngay).

### Kiểm tra bảo mật (C4)

Xác nhận publish phía host chỉ trên loopback.

```bash
ssh SSH_TARGET 'bash -lc "ss -ltnp | grep :HUB_PORT || true"'
```

verify: chỉ `127.0.0.1:HUB_PORT`. Nếu `0.0.0.0:HUB_PORT` hoặc public IP → **dừng cấu hình và cảnh báo** (kiểm tra `ports:` compose là `127.0.0.1:HUB_PORT:48000`). Nhắc user firewall / security group **không** inbound `HUB_PORT` (chỉ mở SSH).

### Lấy token (C5)

Token tự sinh lần đầu container start, lưu bền trong volume.

```bash
ssh SSH_TARGET 'bash -lc "docker exec aac-USER_TAG sh -c \"grep ^token: /home/ubuntu/.many-ai-cli/config.yaml\""'
```

verify: lấy được `token: ...`. **Không để token trong log · chat** (mỗi lần kết nối launcher lấy lại bằng `token_command`; giá trị ghi tạm ở đây **không** lưu file).

> **Đường mobile (📱)**: trong container không có CLI `tailscale` → không dùng `tailscale serve`. Điện thoại nối qua **SSH tunnel / launcher** (wizard mobile degrade dẫn sang SSH tunnel). Chi tiết: [local/plan_mobile-connect-flow-redesign.md](local/plan_mobile-connect-flow-redesign.md).  
> **Khác biệt mô hình token**: launcher mỗi lần lấy lại bằng `token_command` và **không lưu file**. Mobile (📱) **lưu token trên máy** qua QR (xử lý mất token: plan khác `plan_remote-auth-hardening-future.md`). Mobile trên Hub remote vẫn đọc token của chính Hub đó nên không mâu thuẫn với `token_command`.

### Tạo launcher profile trên local (C6)

Thêm profile **chế độ tunnel** vào `~/.many-ai-cli/launcher-profiles.yaml` trên PC local (Windows: `%USERPROFILE%\.many-ai-cli\launcher-profiles.yaml`) — backup trước.

```yaml
version: 1
profiles:
  - name: PROFILE_NAME
    type: ssh
    mode: tunnel
    host: <phần host của SSH_TARGET>
    user: <phần user của SSH_TARGET>
    identity_file: <giá trị SSH_KEY. Để trống thì bỏ cả dòng>
    hub_port: HUB_PORT
    token_command: "docker exec aac-USER_TAG sh -c 'grep ^token ~/.many-ai-cli/config.yaml | cut -d\" \" -f2'"
```

Agent đã biết giá trị khi SSH (host/user từ `SSH_TARGET`, port từ `HUB_PORT`). Thực chất chỉ cần user xác nhận **khóa (`SSH_KEY`)**:

- Có `SSH_KEY` → ghi nguyên vào `identity_file:`. Trống (ssh-agent / config) → bỏ cả dòng.
- `host:` là **đích SSH** (DNS VPS / public IP / alias config). Chế độ tunnel SSH tới host này rồi forward `127.0.0.1:48000` của container — **không ghi IP trong container** (local không reach được).
- `SSH_TARGET` là alias → ghi alias vào `host:`, có thể bỏ `user:`.

verify: YAML hợp lệ, không phá profile cũ.

### Xác nhận khởi động (C7)

```powershell
many-ai-cli-launcher.exe --profile PROFILE_NAME
```

verify: trình duyệt mặc định mở `http://127.0.0.1:HUB_PORT/?token=...&env_kind=remote-tunnel`, hiện Hub UI. Badge là `Remote server (tunnel)` (đỏ + `T`).

> Nếu `many-ai-cli-launcher.exe` không có trong PATH, hướng dẫn lấy từ zip release và thêm vào PATH.

### Báo cáo hoàn tất (C8)

Gộp một lần:

- Đường dẫn `REMOTE_BASE` đã triển khai và status `aac-USER_TAG` (nếu đang chờ `healthy` thì ghi rõ)
- Kết quả: listen host chỉ `127.0.0.1:HUB_PORT`
- Tên launcher profile và đường dẫn file
- **Việc user làm tiếp bằng tay**: đăng nhập provider CLI (trong `docker exec -it aac-USER_TAG bash`, ủy quyền cá nhân; quy trình: [manual_docker-multiuser.md](manual_docker-multiuser.md) «Đăng nhập AI CLI lần đầu»), cập nhật tag theo dõi nếu cần (`docker compose pull && docker compose up -d`)

## Cấm (bắt buộc)

- `ports:` thành `0.0.0.0:...` / public IP; mở cổng Hub trên firewall; reverse proxy public
- **Công khai bằng Tailscale Funnel** (toàn cầu — trái chính sách. Mobile: SSH tunnel / launcher)
- `docker compose down` không đối số (**kéo theo container mọi user**). Thao tác từng service: luôn kèm tên service hoặc dùng `stop` / `rm`
- Để URL/token trong log · chat · commit
- Thay đổi phá hủy compose hiện có / định nghĩa user khác / volume trên remote
- Self-update provider CLI trong container (version thống nhất bằng rebuild image)

## Liên quan

- Lối vào (cài gì, đi quy trình nào): [manual_remote-server-overview.md](manual_remote-server-overview.md)
- Cơ chế & xử lý sự cố: [manual_remote-server-ssh-tunnel.md](manual_remote-server-ssh-tunnel.md)
- Nguồn chuẩn vận hành Docker đa user: [manual_docker-multiuser.md](manual_docker-multiuser.md)
- Task cấu hình server đơn (pnpm): [manual_remote-server-agent-single.md](manual_remote-server-agent-single.md)
