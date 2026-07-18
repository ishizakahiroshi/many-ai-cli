# many-ai-cli — Hướng dẫn phát triển

> Bản dịch tiếng Việt của [`CLAUDE.md`](CLAUDE.md) (bản gốc tiếng Nhật).  
> Cập nhật lần cuối (nguồn): 2026-07-05 — phản ánh audit (bổ sung provider opencode, danh sách subcommand, `internal/log` đã có, sửa đường dẫn archive).  
> Bản dịch: 2026-07-18.

> Chi tiết theo từng loại việc nằm trong `CLAUDE/*.md`. File này chỉ chứa phần luôn được nạp vào ngữ cảnh.

## Tổng quan dự án

**many-ai-cli** — công cụ **quản lý tập trung thao tác phê duyệt và theo dõi tiến độ** khi chạy song song nhiều CLI lập trình bằng AI (Claude Code / Codex CLI, …) trên **một bảng điều khiển Web**. Một binary Go duy nhất (Hub daemon + wrapper) + UI trình duyệt (xterm.js / TypeScript).

> **Gemini CLI không thuộc phạm vi wrap** (quyết định 2026-05-06 / ràng buộc điều khoản sử dụng). Chi tiết: [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md) mục «2. Phạm vi công bố».

**Trạng thái**: Dòng v0.3.x đã phát hành (tag mới nhất theo tài liệu gốc: v0.3.4; v0.1.1 là bản chính thức đầu tiên, v0.1.0 mang tính thử nghiệm). v0.1.2 tái thiết kế chuỗi phiên bản qua ldflags + `/api/info` làm single source of truth; v0.2.0 thêm WSL launcher, Files/Git/Chat/Split/Multi, Commit all, Ollama routing, cấu hình người dùng phía server; v0.3.0 thêm Workbench (lịch sử session SQLite), PWA/Web Push, launcher thống nhất đa nền tảng (SSH mọi OS, WSL chỉ Windows), tài sản deploy remote/Docker, phân phối npm, và đổi tên any-ai-cli → many-ai-cli. **v0.4.0 (Unreleased trong tài liệu gốc) đã gỡ Workbench và proxy chat tích hợp Hub (`internal/proxy/` · `chat_proxy`)** (hiệu ứng phụ: ngữ cảnh 1M mặc định của Sonnet 5 trở đi hoạt động trở lại qua Hub). Tài liệu thiết kế đã cập nhật theo mã nguồn làm chuẩn.

> Ghi chú bản dịch: HEAD hiện tại của repo đã có tag **v0.5.0**. Khi làm việc, ưu tiên `CHANGELOG.md` và mã nguồn nếu lệch với mô tả phiên bản ở trên.

**Thiết kế (chuẩn)**: [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md)

## Trạng thái triển khai hiện tại

Đến v0.3.x (tag v0.3.4 theo tài liệu gốc) các mục sau đã có; v0.4.0 Unreleased gỡ Workbench / chat_proxy và bổ sung provider opencode / Grok Build CLI cùng cấu hình Ollama `base_url`:

- `many-ai-cli serve` khởi động Hub
- `many-ai-cli claude` / `codex` / `copilot` / `cursor-agent` tự khởi động Hub nếu chưa chạy rồi kết nối
- Hub UI hiển thị luồng PTY thời gian thực qua xterm.js
- Quét buffer xterm.js phát hiện chờ phê duyệt và hiện action-bar
- Chỉ thị marker phê duyệt được inject idempotent vào instruction file của Claude / Codex / Copilot / Cursor Agent; xóa khi không còn session active tham chiếu file
- Lựa chọn trên Hub UI được gửi lại vào PTY
- Nếu phê duyệt được giải quyết bằng nhập trực tiếp trên terminal, action-bar biến mất
- Bắt mở rộng vùng gập của Claude Code (ctrl+o) hoạt động
- Đính kèm ảnh (paste/D&D → lưu local → inject PTY) hoạt động
- `/api/spawn` cho phép spawn session từ UI

## Thuật ngữ · tên gọi

| Hạng mục | Giá trị |
|------|------|
| Tên sản phẩm | `many-ai-cli` |
| Tên binary | `many-ai-cli` (Windows: `many-ai-cli.exe`) |
| Subcommand | `serve` / `wrap <provider>` / `shell-init` / `stop` / `status` / `uninstall` / `version` |
| Hub URL | `http://127.0.0.1:47777/?token=<random>` |
| File cấu hình | `~/.many-ai-cli/config.yaml` (Win: `%USERPROFILE%\.many-ai-cli\config.yaml`) |
| Log | `~/.many-ai-cli/logs/sessions/<provider>_<thời-gian>_<folder>_s<id>.log/.jsonl/.txt` (PTY thô + lịch sử sự kiện JSONL + text sạch) |
| Biến môi trường trong suốt | `MANY_AI_CLI_AUTO=1` |
| Provider | `claude` / `codex` / `copilot` / `cursor-agent` (v0.4.0 Unreleased thêm `opencode` / `grok`. `gemini` ngoài phạm vi — xem mục phạm vi) |

> Trong markdown, thống nhất dùng `many-ai-cli` (tên cũ `any-ai-cli` chỉ giữ trong mô tả lịch sử). Đường dẫn project local ghi trong `CLAUDE.local.md`.

## Tech stack

| Lớp | Lựa chọn |
|------|------|
| Ngôn ngữ | Go (cross-compile binary đơn cho Win/Mac/Linux) |
| PTY | `creack/pty` (Unix) + `aymanbagabas/go-pty` (Windows / ConPTY) |
| HTTP | `net/http` chuẩn |
| WebSocket | `golang.org/x/net/websocket` |
| Frontend | HTML/CSS tĩnh + TypeScript (ESM) + esbuild transpile theo file + xterm.js vendored (`web/dist/` embed bằng `go:embed`) |
| Cấu hình | YAML (`gopkg.in/yaml.v3`) |
| Log | `log/slog` chuẩn |

## Cấu trúc thư mục (thực tế)

Xem thiết kế `docs/v0.3.x-many-ai-cli-design.md`.

```
many-ai-cli/
├─ cmd/many-ai-cli/main.go    # entrypoint binary đơn
├─ internal/
│  ├─ hub/        # HTTP+WS / quản lý session / attach / spawn
│  ├─ wrapper/    # PTY wrapper / PTY theo OS / attach inject
│  ├─ shell/      # output shell-init (bash/zsh)
│  ├─ proto/      # định nghĩa message WS
│  ├─ attach/     # lưu ảnh · sinh inject
│  ├─ config/
│  ├─ log/        # file logger slog (xoay vòng lumberjack)
│  └─ ...         # launcher / notify / orchestrate / sessionlog / sessionstore / uninstall / usagerelay / whisperruntime / wslutil
├─ web/src/       # HTML/CSS/TypeScript tĩnh + xterm.js vendored
├─ web/dist/      # artifact `bun run build` (đối tượng go:embed / gitignore)
└─ docs/local/    # thiết kế · roadmap (không public)
```

## Nguyên tắc đa nền tảng

- **Mã đặc thù OS tách bằng build tag** (ví dụ `pty_unix.go` / `pty_windows.go`)
- **Thao tác path dùng `filepath.Join` / `os.UserHomeDir`** (cấm hard-code `/`)
- **Khác biệt xuống dòng · hành vi PTY** được hấp thụ trong `internal/wrapper/`; lớp trên giữ OS-agnostic
- **Thư mục mặc định cấu hình · log** thống nhất `~/.many-ai-cli/` trên mọi OS (Windows: `%USERPROFILE%\.many-ai-cli\`)

## Ràng buộc thiết kế server local

- **Bind cố định `127.0.0.1`** (không public ra ngoài)
- **Port mặc định 47777** (xung đột thì tự dò 47778, 47779…)
- **Token ngẫu nhiên sinh lúc khởi động, gắn vào URL** (`?token=xxx`)
- **Không public ra ngoài** (`127.0.0.1` cố định). Bản thân `many-ai-cli` không gửi telemetry, nhưng lấy danh sách slash-command có thể HTTPS tới GitHub (xem mục bảo mật README)
- **Không ghi vĩnh viễn vào `.bashrc` v.v.** (chế độ trong suốt chỉ qua biến môi trường + `eval "$(many-ai-cli shell-init)"` opt-in)

## Quy tắc chung khi AI làm việc

Cấm build/commit, trách nhiệm secrets-scan, quy tắc tạo plan/bugfix/pending md, … thuộc cấu hình AI **toàn cục** của từng người dùng (ví dụ môi trường tác giả: `~/.claude/CLAUDE.md` và `~/.claude/guides/`). Quy tắc cá nhân (ngôn ngữ, xác nhận, format câu hỏi, …) cũng để ở cấu hình toàn cục; file này **chỉ** quy tắc riêng dự án.

Quy tắc bổ sung riêng dự án:

- Không chỉ build — **chạy, khởi động Hub, reload trình duyệt đều do người dùng thực hiện** (`go run` / `many-ai-cli serve` / `many-ai-cli stop` / khởi động·tắt·khởi động lại process Hub / reload browser, …).
- **Lệnh build chuẩn là `make build`**. Không dùng riêng `bun run build` (kể cả khi hướng dẫn người dùng). `make build` gom: build web (bun install + bun run build) → binary Windows/Linux → triển khai WSL. Lệnh “build giúp” của người dùng nghĩa là **chạy `make build`**.

## Hướng dẫn chi tiết (theo loại nhiệm vụ)

Chỉ **Read** các file md **khớp** nhiệm vụ. **Không** đọc md không liên quan (tiết kiệm context).

| Loại nhiệm vụ | File cần đọc |
|---|---|
| Điều tra · chỉ đọc · hỏi đáp | (chỉ `CLAUDE.md` root; **không** đọc `CLAUDE/*`) |
| Implement · code (Go / TypeScript) | `CLAUDE/coding.md` |
| Build · phân phối · cross-compile | `CLAUDE/deployment.md` |
| Tách context · đặt tên docs · mô hình làm việc AI · plan tự chạy/điều kiện dừng | `CLAUDE/development.md` |
| Git · commit · quy tắc output | `CLAUDE/operations.md` |
| Cấu hình môi trường dev Windows | `CLAUDE/windows_setup.md` |

## Quy tắc bắt buộc khi làm plan · docs

Trước khi **tạo/chạy** `plan_*.md` hoặc **tạo/cập nhật** `.md` dưới `docs/`, **bắt buộc** Read `CLAUDE/development.md` (chuẩn về tách context, điều kiện tự chạy, điều kiện dừng, ghi ngày cập nhật cuối, …).

## Liên kết tham chiếu

| Hạng mục | Đường dẫn |
|------|------|
| Thiết kế v0.3.0 (hiện hành · chuẩn) | [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md) |
| Thiết kế v0.2.0 (lịch sử) | [docs/v0.2.x-any-ai-cli-design.md](docs/v0.2.x-any-ai-cli-design.md) |
| Thiết kế v1 (lịch sử) | [docs/local/archive/v0.1.3/cli-popup-design-v1.md](docs/local/archive/v0.1.3/cli-popup-design-v1.md) |
| Bổ sung cho Codex | [AGENTS.md](AGENTS.md) (local: `AGENTS.local.md` nếu có) |
| Bổ sung cho Gemini | [GEMINI.md](GEMINI.md) (**không** wrap trong many-ai-cli; giữ để dùng Gemini CLI hỗ trợ dev repo này) |
| Bản hướng dẫn tiếng Việt | [CLAUDE.vi.md](CLAUDE.vi.md) (file này) |
| README tiếng Việt | [README.vi.md](README.vi.md) |
| Mục lục manual tiếng Việt | [docs/README.vi.md](docs/README.vi.md) |
