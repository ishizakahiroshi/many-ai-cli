# Manual vận hành Docker đa user many-ai-cli (remote server admin)

> Bản dịch tiếng Việt của [`manual_docker-multiuser.md`](manual_docker-multiuser.md).  
> Bản dịch: 2026-07-18.

> **⚠️ Bản dịch này là ảnh chụp tại một thời điểm và không còn được đồng bộ.**
>
> Kể từ 2026-08-15, các file `*.vi.md` trong repo này **không còn được cập nhật theo bản gốc**. Bản gốc [`manual_docker-multiuser.md`](manual_docker-multiuser.md) là chuẩn duy nhất — vui lòng đối chiếu bản gốc trước khi làm theo nội dung ở đây. (Lần dịch gần nhất: 2026-07-18.)
>
> Xin chân thành cảm ơn người đã đóng góp bản dịch. Chúng tôi vẫn hoan nghênh đóng góp dịch thuật; chỉ là không thể hứa giữ đồng bộ, nên bản được merge cũng sẽ là ảnh chụp có ghi ngày.
>
> *(EN: This translation is a dated snapshot and is no longer kept in sync with the original. Please refer to the source file linked above. The Vietnamese **UI** locale `web/src/i18n/vi.json` is a shipped feature and is still maintained.)*

Hướng dẫn vận hành nhiều người trên remote server XServer (tên server `admin`) theo mô hình «1 user = 1 Docker container» cho `many-ai-cli`. Đường dẫn vật lý phía server (`/opt/any-ai-cli/` `/srv/any-ai-cli/`), tên compose project và docker volume **vẫn dùng tên cũ `any-ai-cli`** trong vận hành thực tế, nên tài liệu này cũng ghi theo thực tế đó. Lịch sử thiết kế / quyết định: [plan_docker-multiuser-isolation.md](plan_docker-multiuser-isolation.md).

**Thông tin kết nối (IP server · đường dẫn private key · token, v.v.) để ở file credentials local của admin (ngoài Git). Không ghi giá trị thật trong tài liệu này.**

## Tổng quan

```text
Trình duyệt PC thành viên
  -> http://127.0.0.1:478NN/?token=<token riêng của người đó>
  -> SSH local forward (tài khoản OS + public key của người đó)
  -> host remote server 127.0.0.1:478NN
  -> (docker publish) container aac-<user> :48000
  -> (socat relay) 127.0.0.1:478NN trong container = Hub
```

- Hub listen trong container tại **cổng được gán cho người đó 478NN** (Host/Origin của Hub yêu cầu khớp cổng tuyệt đối → mọi cổng trên đường đi thống nhất 478NN; chỉ cổng nhận socat trong container `48000` là cố định nội bộ)
- Publish phía host chỉ `127.0.0.1`. Không lộ global IP của remote. Đường vào từ ngoài chỉ qua SSH tunnel
- Token tự sinh lần đầu container start, lưu bền trong volume `aac-home-<user>`

## Bố cục phía server

```text
/opt/any-ai-cli/                 # root sở hữu · chỉ admin thao tác
├─ compose.yaml                  # include users/*.yaml
├─ users/<user>.yaml             # định nghĩa service theo user (cổng · volume · network riêng)
├─ templates/sshd-member.conf.template   # mẫu cấu hình sshd cho thành viên
├─ assign.md                     # bảng gán user ⇔ cổng ⇔ volume (đồng bộ với bảng trong tài liệu này)
└─ src/                          # source repo (build image. Nguồn chuẩn: deploy/docker/)

/srv/any-ai-cli/work/<user>/     # chỗ đặt repo làm việc (bind mount, sở hữu uid 1000)
named volume: aac-home-<user>    # home trong container (auth AI CLI · ~/.many-ai-cli)
```

Nguồn chuẩn trong repo: `deploy/docker/` (Dockerfile / entrypoint.sh / compose.yaml / users/*.yaml / sshd-member.conf.template). `/opt/any-ai-cli/compose.yaml` · `users/` · `templates/` trên server là bản copy triển khai — khi sửa, sửa phía repo rồi triển khai lại.

Điểm cấu hình chính (entrypoint pre-generate vào config.yaml lần start đầu):

- `idle_timeout_min: 0` — tắt mặc định 60 phút «sau khi UI disconnect thì kill mọi session PTY» (remote thường đứt tunnel; đổi được từ UI settings)
- `auto_shutdown: false` / `open_browser: false` — giả định Hub thường trú · headless
- Mỗi container trên network riêng (`aac-net-<user>`), chặn reach trực tiếp giữa container
- `memswap_limit` = `mem_limit` để tránh tranh swap host (vượt → OOM chỉ container đó)

## Bảng gán

| User | Cổng | Container | volume | work dir | Quyền SSH |
|---|---|---|---|---|---|
| admin | 47801 | aac-admin | aac-home-admin | /srv/any-ai-cli/work/admin | Admin (shell thường + nhóm docker) |

- Cổng đánh số liên tiếp từ 47801
- Đổi thì cập nhật cả `/opt/any-ai-cli/assign.md` và bảng trong tài liệu này

## Thêm thành viên (việc của admin)

Điều kiện: đã nhận public key SSH của thành viên mới (1 dòng `ssh-ed25519 ...`). **Không nhận private key**.

Dưới đây `<user>` = tên thành viên mới, `<port>` = cổng kế tiếp trong bảng gán (478NN).

### 1. Tạo tài khoản OS (chỉ tunnel)

```bash
adduser --disabled-password --gecos "" <user>
install -d -m 700 -o <user> -g <user> /home/<user>/.ssh
echo '<1 dòng public key>' > /home/<user>/.ssh/authorized_keys
chmod 600 /home/<user>/.ssh/authorized_keys
chown <user>:<user> /home/<user>/.ssh/authorized_keys
```

**Không** gán nhóm docker hay sudo.

### 2. Áp cấu hình sshd chỉ tunnel

```bash
sed -e 's/__USER__/<user>/g' -e 's/__PORT__/<port>/g' \
  /opt/any-ai-cli/templates/sshd-member.conf.template \
  > /etc/ssh/sshd_config.d/aac-member-<user>.conf
sshd -t                      # không lỗi cú pháp (có lỗi thì không apply)
systemctl reload ssh
```

Thành viên «không shell · chỉ local forward đúng cổng của mình» (`ForceCommand /usr/sbin/nologin` + `PermitOpen 127.0.0.1:<port>`).

### 3. Thêm định nghĩa container và start

```bash
# work dir (chown theo uid 1000 = ubuntu trong container)
mkdir -p /srv/any-ai-cli/work/<user>
chown 1000:1000 /srv/any-ai-cli/work/<user>

# định nghĩa user (copy admin.yaml làm template, thay thế.
# tên service · volume · network riêng · cổng được thay cùng lúc)
cd /opt/any-ai-cli
sed -e 's/admin/<user>/g' -e 's/47801/<port>/g' users/admin.yaml > users/<user>.yaml

# append include trong compose.yaml → kiểm tra cú pháp → start
sed -i 's|^\(  - users/admin.yaml\)$|\1\n  - users/<user>.yaml|' compose.yaml
docker compose config --quiet && docker compose up -d aac-<user>
```

> Đồng bộ thêm cùng `<user>.yaml` vào `deploy/docker/users/` phía repo.

### 4. Đăng nhập AI CLI lần đầu (tài khoản của chính thành viên)

Thành viên không vào được remote server nên **lệnh do shell admin chạy · ủy quyền trên trình duyệt của thành viên**. Chỉ cần chat qua lại URL / code — không cần chia sẻ màn hình hay cấp quyền vào server cho thành viên.

```bash
docker exec -it aac-<user> bash   # shell sau đây (admin chạy)
```

**claude (dán code trở lại)**

1. Admin: `command claude` → chọn login → gửi URL hiện ra cho thành viên qua chat  
2. Thành viên: mở bằng trình duyệt của mình, ủy quyền bằng tài khoản Anthropic → gửi code hiện ra về admin  
3. Admin: dán vào terminal → xong  

**codex (device code · bắt buộc `--device-auth`)**

1. Admin: `codex login --device-auth` → gửi URL (`https://auth.openai.com/codex/device`) và mã one-time (15 phút) cho thành viên  
2. Thành viên: mở trình duyệt, nhập mã, ủy quyền OpenAI → terminal tự nhận hoàn tất (không cần gửi code về)  
3. Admin: `codex login status` xác nhận «Logged in»  

**copilot (GitHub device flow)**

1. Admin: `command copilot` → xác nhận tin cậy (Yes) → `/login` → gửi device code và `https://github.com/login/device` cho thành viên  
2. Thành viên: mở trình duyệt, nhập mã, ủy quyền GitHub (cần subscription Copilot) → terminal tự nhận hoàn tất  

**cursor-agent (URL polling)**

1. Admin: `NO_OPEN_BROWSER=1 command cursor-agent login` → gửi URL (`https://cursor.com/loginDeepControl?...`) cho thành viên  
2. Thành viên: mở trình duyệt, phê duyệt tài khoản Cursor → CLI tự nhận hoàn tất (không callback localhost; xong trong container)  

Lưu ý:

- `command` trong `command claude` để tránh wrap trong suốt (shell function). Login không cần wrap; terminal VT yếu (conhost cũ, v.v.) đôi khi không hiện màn hình qua wrap (thực tế 2026-06-04)
- `codex login` thuần trong container chờ callback `localhost:1455`; redirect trình duyệt về localhost của PC đã ủy quyền → **luôn fail** (thực tế 2026-06-04). Nếu đã lỡ: trung chuyển query URL fail trên trình duyệt bằng `docker exec aac-<user> curl "http://127.0.0.1:1455/auth/callback?code=...&scope=...&state=..."` (trong lúc process `codex login` còn chờ)
- **Mô hình tin cậy**: OAuth token sau khi xong nằm trong volume `aac-home-<user>`; admin **có thể** truy cập kỹ thuật. Khi thêm thành viên, nói rõ «vận hành dựa trên tin cậy admin»

Thông tin auth lưu trong volume `aac-home-<user>`, giữ sau restart container.

### 5. Xác nhận token và giao cho chính chủ

```bash
docker exec aac-<user> grep '^token:' /home/ubuntu/.many-ai-cli/config.yaml
# hoặc từ banner start:
docker logs aac-<user> 2>&1 | grep -m1 'Open:'
```

Gửi cho chính chủ: ① cổng gán `<port>` ② token (tiện nhất dạng URL `http://127.0.0.1:<port>/?token=...`).  
Nhắc **không chia sẻ token**. Lộ thì admin xóa `token:` trong config.yaml trong volume → restart container → sinh lại.

## Kết nối của thành viên (trên PC từng người)

Cổng `<port>` · token nhận từ admin.

```powershell
# Windows (PowerShell) — cửa sổ giữ kết nối
ssh -N -L <port>:127.0.0.1:<port> <user>@<server-ip> -i <đường dẫn private key đã nhận>
```

```bash
# macOS / Linux
ssh -N -L <port>:127.0.0.1:<port> <user>@<server-ip> -i <đường dẫn private key>
```

Mở trình duyệt: `http://127.0.0.1:<port>/?token=<token>`.

- **Cổng hai bên bắt buộc cùng số** (Host verification của Hub yêu cầu khớp cổng)
- Không mở được shell khi kết nối (`-N` bắt buộc; thử shell cũng bị nologin cắt)
- Forward sang cổng người khác bị `PermitOpen` từ chối

## Vận hành cho admin

Chạy bằng `admin` (nhóm docker) hoặc root, tại `/opt/any-ai-cli`.

| Thao tác | Lệnh |
|---|---|
| Start mọi container | `docker compose up -d` |
| Restart từng cái | `docker compose restart aac-<user>` (vòng die ngay: xem troubleshoot) |
| Dừng | `docker compose stop aac-<user>` |
| Xem log | `docker logs -f aac-<user>` |
| Đo tài nguyên | `docker stats` |
| Shell trong container | `docker exec -it aac-<user> bash` |
| Cập nhật image | chuyển source lại từ local (dưới) → `docker compose build && docker compose up -d` |
| Xóa thành viên | `docker compose stop aac-<user> && docker compose rm -f aac-<user>` → `docker volume rm any-ai-cli_aac-home-<user>` → xóa users/<user>.yaml và dòng include → sau khi người đó ngắt tunnel `userdel -r <user>` → xóa `/etc/ssh/sshd_config.d/aac-member-<user>.conf` → `sshd -t && systemctl reload ssh` → backup hoặc xóa `/srv/any-ai-cli/work/<user>` → cập nhật assign.md |

> ⚠️ `docker compose down` không đối số **dừng và xóa container mọi user**. Thao tác lẻ: luôn kèm tên service hoặc dùng stop/rm.

### Quy trình dừng

Dừng thường: `docker compose stop aac-<user>`. Nên kết thúc session của user đó trên Hub UI trước khi stop.

Khi stop, Docker gửi SIGTERM cho entrypoint; entrypoint gửi SIGTERM tới nhóm wrapper `many-ai-cli wrap` / `many-ai-cli claude` / `many-ai-cli codex` / `many-ai-cli copilot` / `many-ai-cli cursor-agent`. Wrapper (implementation sẵn có) chuyển SIGTERM xuống AI CLI con, nên ghi file / git dở dang vẫn có thời gian thoát bình thường.

Grace: entrypoint tối đa 20 giây cho wrapper; compose `stop_grace_period: 40s` cho cả container. Quá 40 giây Docker SIGKILL — ngoài khẩn cấp không dùng `docker kill` hay `docker compose stop -t 0`. Kiểm tra dừng: trong `docker logs aac-<user>` thấy thứ tự `sending TERM to wrapper processes` → `wrapper processes exited` → `sending TERM to Hub` → `Hub exited`.

Chuyển source lại (từ root repo PC local):

```bash
# Chuyển trạng thái ổn định đã commit (HEAD). tar working tree có thể
# nhặt file dở dang từ session AI / editor khác và làm hỏng build
# (đã xảy ra 2026-06-04)
git archive HEAD | gzip | \
  ssh root@<server-ip> 'rm -rf /opt/any-ai-cli/src && mkdir -p /opt/any-ai-cli/src && tar -xzf - -C /opt/any-ai-cli/src'

# Chỉ khi deploy/ chưa commit: ghi đè từ worktree (sau commit thì không cần).
# Bỏ CR trên entrypoint.sh luôn được (chống autocrlf của git archive)
tar -czf - deploy | \
  ssh root@<server-ip> "tar -xzf - -C /opt/any-ai-cli/src && sed -i 's/\r\$//' /opt/any-ai-cli/src/deploy/docker/entrypoint.sh"
```

### Chính sách quản lý version provider CLI

**Version pin trong Dockerfile; cập nhật chỉ bằng rebuild image và rollout đồng loạt. Không self-update / update tay trong container (`claude update` / `copilot update`, v.v.).**

Quy trình cập nhật: nâng pin Dockerfile → rebuild → **xác nhận detection action-bar phê duyệt còn đúng** (phụ thuộc chuỗi màn hình CLI, UI CLI đổi có thể hỏng) → `docker compose up -d` tái tạo mọi container (volume giữ nên login còn nguyên).

Lý do cấm update trong container:

- CLI trong image thuộc root (/usr/lib, /opt) nên non-root update thường fail; nhưng self-updater cài vào `~/.local` thì **đọng trên volume, cướp PATH, sau này update image user đó vẫn dùng bản cũ tự cài** (version ma)
- Version lệch giữa user → mất khả năng tái hiện lỗi
- Đã phòng: claude `DISABLE_AUTOUPDATER=1`; 3 gói npm + cursor-agent vùng root, non-root không update được
- cursor-agent lấy bằng versioned package URL qua `CURSOR_AGENT_VERSION` trong Dockerfile (không dùng installer chính thức). `cursor-agent --version` không khớp pin → build fail
- Nghi «version ma»: `docker exec aac-<user> sh -c 'which claude; claude --version'` — nếu dưới `~/.local/bin` thì xóa

## Lưu ý bảo mật

- **Không mở cổng Hub (478NN) trên packet filter XServer**. Chỉ mở SSH(22). Publish host đã giới hạn `127.0.0.1` nên mở cũng không lộ trực tiếp, nhưng giữ đóng theo defense-in-depth (kiểm tra panel XServer)
- Thành viên chỉ tunnel (`ForceCommand /usr/sbin/nologin` + `PermitOpen 127.0.0.1:<cổng mình>` + `PermitListen none`). Không shell · SFTP · cổng khác
- Thành viên không thao tác gì trên server; restart container · thao tác work dir phía host đều qua admin
- Token từng container chỉ chính chủ biết. Không dán URL (có token) vào chat, v.v.
- Login AI CLI đóng trong volume từng container; thành viên khác không reach (3 lớp: ranh container + tách cổng + token)

## Ghi chép chuẩn bị server ban đầu (nội dung C1 · để dựng lại)

Thực hiện 2026-06-04. Từ Ubuntu 24.04 sạch (chỉ root · không Docker/Node).

```bash
# 1. Docker Engine + compose plugin (apt repo chính thức)
apt-get update && apt-get install -y ca-certificates curl
install -m 0755 -d /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
chmod a+r /etc/apt/keyrings/docker.asc
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu noble stable" > /etc/apt/sources.list.d/docker.list
apt-get update && apt-get install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
docker run --rm hello-world

# 2. User admin (tái dùng public key của private key admin hiện có <admin-key>.pem)
adduser --disabled-password --gecos "" admin
usermod -aG docker admin
install -d -m 700 -o admin -g admin /home/admin/.ssh
# đưa output ssh-keygen -y -f <admin-key>.pem vào authorized_keys
chmod 600 /home/admin/.ssh/authorized_keys && chown admin:admin /home/admin/.ssh/authorized_keys

# 3. Ước lệ thư mục
mkdir -p /opt/any-ai-cli/users /opt/any-ai-cli/templates /srv/any-ai-cli/work/admin
chown admin:admin /srv/any-ai-cli/work/admin   # uid 1000 (khớp ubuntu trong container)

# 4. Template sshd cho thành viên → /opt/any-ai-cli/templates/sshd-member.conf.template
#    (xem «Thêm thành viên bước 2». Apply khi thêm thành viên thật)
```

Trạng thái ban đầu đã xác nhận: chỉ listen 22 / swap 2 GiB bật / `unattended-upgrades` enabled / ufw inactive (filter quản lý phía panel XServer).

Version lúc đưa vào: Docker 29.5.3 / Docker Compose v5.1.4 / image: Ubuntu 24.04 + Node.js 22 + `@anthropic-ai/claude-code` 2.1.162 + `@openai/codex` 0.137.0 + `@github/copilot` 1.0.59 + cursor-agent 2026.06.03-0bbb28e (versioned package URL → `/opt/cursor-agent` — dưới home bị volume che). Mọi cập nhật CLI bằng rebuild image.

## Troubleshoot

| Triệu chứng | Kiểm tra / xử lý |
|---|---|
| Trình duyệt «không truy cập được» | Tunnel SSH còn sống? Cổng hai bên cùng số? Container Up? (`docker ps`) |
| 401 | Gõ nhầm token · dùng token user khác |
| 403 / API fail | Cổng URL ≠ cổng gán (Host/Origin yêu cầu khớp tuyệt đối) |
| «Mở thư mục» / picker trên UI không phản ứng | Container headless không có GUI. Nhập path tay. Tạo thư mục mới: Files tab «Thư mục mới» (📁+) |
| Không lập được tunnel | Cổng có trong `PermitOpen`? (cổng người khác bị từ chối) / public key đã đăng ký? |
| Container restart lặp | `docker logs aac-<user>`. Thiếu `HUB_PORT` → entrypoint thoát ngay |
| Sau restart vòng die exit 137 (không log · die ~0.2s) | Race đơn lẻ phía docker (quan sát 1 lần 2026-06-04, Docker 29.5.3; điều kiện tái hiện chưa rõ; restart thường ~0.5s OK). **Khôi phục: `docker compose up -d --force-recreate aac-<user>`**. Volume giữ → token · auth không mất |
| Thiếu bộ nhớ | `docker stats` đo thực → chỉnh `mem_limit` trong `users/<user>.yaml` (ước số người: kết quả C5 của plan) |
