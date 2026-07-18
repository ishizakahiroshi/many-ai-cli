# Lối vào remote / server (cài gì, dùng quy trình nào)

> Bản dịch tiếng Việt của [`manual_remote-server-overview.md`](manual_remote-server-overview.md).  
> Cập nhật nguồn: 2026-06-14. Bản dịch: 2026-07-18.

many-ai-cli phân biệt **người chạy AI trên máy mình** và **người dùng AI trên remote server** — **gói cài đặt khác nhau**. Trang này là **mục lục một bảng**: “mình cần cài gì, rồi đi theo hướng dẫn nào”. Chi tiết từng luồng nằm ở các file `manual_remote-server-agent-*.md` được liên kết.

## Tách ba vai trò

- **A. Host chạy AI** — nơi AI CLI thực sự chạy (`many-ai-cli serve`). Có PTY và dashboard. Thường gọi là **Hub**.
- **B. Connector (máy “mẹ” tunnel)** — giữ tunnel SSH/WSL tới một host A (`many-ai-cli-launcher`; Hub đầy đủ cũng nhúng connector qua `/api/servers`). Thường gọi là **máy mẹ / connector**.
- **C. Client** — chỉ xem và thao tác: trình duyệt / PWA / điện thoại. Không cần process thường trú.

Cùng một máy có thể kiêm **A và B** (vừa chạy AI local vừa nối remote). Bảng dưới quy về câu hỏi: **“máy mình cài gì?”**.

## Mình cài gì? → Đi quy trình nào?

| Cách dùng | Cài trên máy mình | Vai trò | Thường trú | Quy trình tiếp theo |
|---|---|---|---|---|
| Chỉ chạy AI trên PC (không remote) | `many-ai-cli` (đầy đủ) | A | Có | README «Quick Start» / «Bắt đầu nhanh» |
| Chạy AI trên PC **và** nối remote | Chỉ `many-ai-cli` (đầy đủ) ※ B đã nhúng | A＋B | Có | README «Quick Start» + các bước remote bên dưới |
| Chỉ dùng AI remote · qua PC · **agent cấu hình tự động** | `many-ai-cli-launcher` | B | Có (tối thiểu) | [Server đơn (pnpm)](manual_remote-server-agent-single.md) / [Docker](manual_remote-server-agent-docker.md) |
| Chỉ dùng AI remote · qua PC · **nhập profile thủ công bằng nút UI** | `many-ai-cli-launcher` | B | Có (tối thiểu) | [plan_server-profile-export-import](local/plan_server-profile-export-import.md) (đang lên kế hoạch) |
| Chỉ dùng AI remote · **VPN thẳng từ điện thoại** | Không cài gì (PWA) | C | Không | Hub UI → «📱 Kết nối mobile» |

※ Người dùng cả hai phía: **chỉ cần một bản full**. Connector B đã nhúng trong `serve` (`/api/servers`) — không bắt buộc cài thêm launcher.  
※ Phía “bị nối tới” vẫn cần A (`many-ai-cli serve`) trên server. **Chính việc dựng A** là hai quy trình agent bên dưới.

> **Nguyên tắc: dùng server thì `many-ai-cli-launcher` đơn lẻ là đường chính.** Chỉ dùng AI remote thì **không** cần full Hub (`serve`) trên máy local. Launcher lập SSH tunnel và mở UI của Hub remote là đủ. Connector nhúng trong full Hub (`/api/servers`) dành cho người **vừa chạy AI local vừa nối remote** — không ép người chỉ dùng server phải cài full Hub.

## Chọn quy trình remote (serve / tunnel)

Khi agent cấu hình tự động cho “dùng AI trên server”, có **hai** quy trình. Khác nhau **không** nằm ở kỹ thuật tunnel, mà ở **Hub remote đã chạy sẵn hay chưa**:

| | [Server đơn (pnpm)](manual_remote-server-agent-single.md) | [Docker](manual_remote-server-agent-docker.md) |
|---|---|---|
| Cài remote | pnpm global install | Pull image GHCR bằng compose |
| Khởi động Hub | `serve` (mỗi lần, hoặc systemd / tmux / nohup thường trú) | `restart: unless-stopped` thường trú |
| Chế độ launcher | `serve` (không cần token trước) | `tunnel` (lấy token bằng `token_command`) |
| Phù hợp | Một user · không cần cô lập mạnh (**cả máy vật lý 1 node**) | Nhiều user cô lập · cập nhật tự động có quy trình |

> **“Thường trú” không độc quyền của Docker.** Một máy Linux vật lý cũng có thể cho `serve` chạy systemd/tmux rồi đặt launcher ở chế độ tunnel (ví dụ `token_command` xem [mục vận hành server đơn](manual_remote-server-agent-single.md)).

## Liên quan

- Cơ chế & xử lý sự cố: [manual_remote-server-ssh-tunnel.md](manual_remote-server-ssh-tunnel.md)
- Launcher tổng quát (tiếng Anh, gồm cách lấy bản phát hành): README mục «Unified launcher»
- Nhập profile bằng UI (không agent, đang lên kế hoạch): [local/plan_server-profile-export-import.md](local/plan_server-profile-export-import.md)
