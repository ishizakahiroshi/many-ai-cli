# Tài liệu hướng dẫn (bản tiếng Việt)

> Mục lục các manual đã dịch sang tiếng Việt.  
> **Bản gốc** (tiếng Nhật hoặc tiếng Anh) vẫn là chuẩn vận hành khi có xung đột; bản `.vi.md` phục vụ team đọc tiếng Việt.

> **⚠️ Bản dịch này là ảnh chụp tại một thời điểm và không còn được đồng bộ.**
>
> Kể từ 2026-08-15, các file `*.vi.md` trong repo này **không còn được cập nhật theo bản gốc**. Bản gốc của từng tài liệu (cột «File gốc» trong bảng bên dưới) là chuẩn duy nhất — vui lòng đối chiếu bản gốc trước khi làm theo. (Lần dịch gần nhất: 2026-07-18.)
>
> Xin chân thành cảm ơn người đã đóng góp bản dịch. Chúng tôi vẫn hoan nghênh đóng góp dịch thuật; chỉ là không thể hứa giữ đồng bộ, nên bản được merge cũng sẽ là ảnh chụp có ghi ngày.
>
> *(EN: This translation is a dated snapshot and is no longer kept in sync with the original. Please refer to the source file linked above. The Vietnamese **UI** locale `web/src/i18n/vi.json` is a shipped feature and is still maintained.)*

| File gốc | Bản tiếng Việt | Nội dung |
|---|---|---|
| (repo root) [`README.md`](../README.md) | [`../README.vi.md`](../README.vi.md) | Hướng dẫn sản phẩm / cài đặt / tính năng |
| (repo root) [`CLAUDE.md`](../CLAUDE.md) | [`../CLAUDE.vi.md`](../CLAUDE.vi.md) | Hướng dẫn phát triển cho AI & contributor |
| [`manual_remote-server-overview.md`](manual_remote-server-overview.md) | [`manual_remote-server-overview.vi.md`](manual_remote-server-overview.vi.md) | Lối vào remote / server: cài gì, đi quy trình nào |
| [`manual_mobile-access.md`](manual_mobile-access.md) | [`manual_mobile-access.vi.md`](manual_mobile-access.vi.md) | Kết nối mobile / remote (📱) & bảo mật |
| [`manual_whisper.md`](manual_whisper.md) | [`manual_whisper.vi.md`](manual_whisper.vi.md) | Cài đặt nhập liệu giọng Whisper |
| [`manual_slash_commands_update.md`](manual_slash_commands_update.md) | [`manual_slash_commands_update.vi.md`](manual_slash_commands_update.vi.md) | Cập nhật slash-commands |
| [`manual_remote-server-ssh-tunnel.md`](manual_remote-server-ssh-tunnel.md) | [`manual_remote-server-ssh-tunnel.vi.md`](manual_remote-server-ssh-tunnel.vi.md) | Cơ chế & xử lý sự cố SSH tunnel |
| [`manual_remote-server-agent-single.md`](manual_remote-server-agent-single.md) | [`manual_remote-server-agent-single.vi.md`](manual_remote-server-agent-single.vi.md) | Agent setup server đơn (pnpm) |
| [`manual_remote-server-agent-docker.md`](manual_remote-server-agent-docker.md) | [`manual_remote-server-agent-docker.vi.md`](manual_remote-server-agent-docker.vi.md) | Agent setup Docker |
| [`manual_docker-multiuser.md`](manual_docker-multiuser.md) | [`manual_docker-multiuser.vi.md`](manual_docker-multiuser.vi.md) | Docker đa user |
| [`manual_ollama-cloud-routing.md`](manual_ollama-cloud-routing.md) | [`manual_ollama-cloud-routing.vi.md`](manual_ollama-cloud-routing.vi.md) | Ollama Cloud routing |
| [`manual_local-llm-hyperv-host.md`](manual_local-llm-hyperv-host.md) | [`manual_local-llm-hyperv-host.vi.md`](manual_local-llm-hyperv-host.vi.md) | Local LLM trên Hyper-V host |
| [`manual_release.md`](manual_release.md) | [`manual_release.vi.md`](manual_release.vi.md) | Quy trình release |
| [`v0.3.x-many-ai-cli-design.md`](v0.3.x-many-ai-cli-design.md) | _(chưa dịch — dài; chuẩn kỹ thuật vẫn là bản gốc)_ | Thiết kế v0.3.x |

## Nguyên tắc dịch

- Giữ nguyên **tên lệnh, path, API, mã lỗi, tên file** (`many-ai-cli serve`, `/api/spawn`, `127.0.0.1`, …).
- Thuật ngữ sản phẩm giữ tiếng Anh khi đã thành convention: Hub, PTY, wrapper, spawn, action-bar, provider, token.
- Giọng văn: **trang trọng, kỹ thuật, rõ ràng** — phù hợp tài liệu vận hành nội bộ, không dùng tiếng lóng.
