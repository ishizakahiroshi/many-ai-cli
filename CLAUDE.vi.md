# many-ai-cli — Hướng dẫn phát triển

> Bản dịch tiếng Việt của [`CLAUDE.md`](CLAUDE.md) (bản gốc tiếng Nhật).  
> Cập nhật lần cuối (nguồn): 2026-08-15 — bổ sung 7 mục còn thiếu so với bản gốc (chính sách provider, chính sách app desktop, tài nguyên phân phối lúc chạy, mã quan sát, thiết kế thu hồi, tính đồng nhất của phê duyệt, ranh giới bằng chứng kiểm toán).
> Bản dịch ban đầu: 2026-07-18 (Nga Vo).

> **⚠️ Bản dịch này là ảnh chụp tại một thời điểm và không còn được đồng bộ.**
>
> Kể từ 2026-08-15, các file `*.vi.md` trong repo này **không còn được cập nhật theo bản gốc**. Bản gốc [`CLAUDE.md`](CLAUDE.md) là chuẩn duy nhất — vui lòng đối chiếu bản gốc trước khi làm theo nội dung ở đây. (Lần dịch gần nhất: 2026-08-15.)
>
> Xin chân thành cảm ơn người đã đóng góp bản dịch. Chúng tôi vẫn hoan nghênh đóng góp dịch thuật; chỉ là không thể hứa giữ đồng bộ, nên bản được merge cũng sẽ là ảnh chụp có ghi ngày.
>
> *(EN: This translation is a dated snapshot and is no longer kept in sync with the original. Please refer to the source file linked above. The Vietnamese **UI** locale `web/src/i18n/vi.json` is a shipped feature and is still maintained.)*

> Chi tiết theo từng loại việc nằm trong `CLAUDE/*.md`. File này chỉ chứa phần luôn được nạp vào ngữ cảnh.

## Tổng quan dự án

**many-ai-cli** — công cụ **quản lý tập trung thao tác phê duyệt và theo dõi tiến độ** khi chạy song song nhiều CLI lập trình bằng AI (Claude Code / Codex CLI, …) trên **một bảng điều khiển Web**. Một binary Go duy nhất (Hub daemon + wrapper) + UI trình duyệt (xterm.js / TypeScript).

> **Gemini CLI không thuộc phạm vi wrap** (quyết định 2026-05-06 / ràng buộc điều khoản sử dụng). Chi tiết: [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md) mục «2. Phạm vi công bố».
> Ngày 2026-08-13 đã xem xét lại dựa trên «phương thức host nông» của Spotify Xirp nhưng **giữ nguyên chính sách** (hồ sơ tạm hoãn: `docs/local/archive/v0.8/pending_gemini-shallow-host-option.md`). Không lấy việc các hãng khác hỗ trợ Gemini, hay số lượng provider trông kém hơn, làm căn cứ để đề xuất bắt tay làm.

**Trạng thái**: v0.6.0 đã phát hành (v0.1.1 là bản chính thức đầu tiên, v0.1.0 mang tính thử nghiệm; v0.5.2 được gộp vào v0.6.0 nên không có bản v0.5.2). v0.1.2 tái thiết kế chuỗi phiên bản qua ldflags + `/api/info` làm single source of truth; v0.2.0 thêm WSL launcher, Files/Git/Chat/Split/Multi, Commit all, Ollama routing, cấu hình người dùng phía server; v0.3.0 thêm Workbench (lịch sử session SQLite), PWA/Web Push, launcher thống nhất đa nền tảng (SSH mọi OS, WSL chỉ Windows), tài sản deploy remote/Docker, phân phối npm, và đổi tên any-ai-cli → many-ai-cli. **v0.4.0 đã gỡ Workbench và proxy chat tích hợp Hub (`internal/proxy/` · `chat_proxy`)** (hiệu ứng phụ: ngữ cảnh 1M mặc định của Sonnet 5 trở đi hoạt động trở lại qua Hub). Tài liệu thiết kế đã cập nhật theo mã nguồn làm chuẩn.

**Thiết kế (chuẩn)**: [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md)

## Không đề xuất thêm provider mới (quy định 2026-08-13)

Ngày 2026-08-13 đã khảo sát 20 sản phẩm CLI lập trình bằng AI và **chốt bỏ qua toàn bộ** (tài liệu: `docs/local/design_cli-agent-provider-candidates_2026-08-13.html`). Lý do không nằm ở chất lượng của các ứng viên, mà vì **với quy mô của công cụ này (6 star / 11 lượt tải npm mỗi tuần), khuôn khổ «quyết định nhận hay không dựa trên số người dùng của ứng viên» không thành lập được**.

- **Không lấy độ phổ biến, đà tăng trưởng, hay việc hãng khác đã hỗ trợ làm căn cứ để đề xuất thêm provider** (cùng hệ với mục Gemini)
- Điểm khởi đầu của việc cân nhắc là «tác giả có đang dùng CLI đó một cách tự nhiên và nó đã vào guồng chạy song song hằng ngày chưa». Không triển khai trước khi dùng
- Chi phí thêm mới, theo thực tế của opencode, là 18 file · 180 dòng. Chi phí thật là **có thêm một đích cần theo kịp** trong `resources/approval-patterns/` và `resources/models/` — hai nơi không có cơ chế kiểm tra độ tươi

## Không đề xuất đóng gói thành app desktop hay nộp Microsoft Store (quy định 2026-08-14)

Ngày 2026-08-14 đã cân nhắc đóng gói app Windows bằng Tauri và nộp Microsoft Store, và **chốt bỏ qua toàn bộ khi chưa viết một dòng nào** (diễn biến và toàn bộ căn cứ: `docs/local/archive/declined/plan_ms-store-tauri-windows-app.md`, tiêu đề H1 là `[廃止]`; 6 plan con nằm cùng thư mục).

- **Giữ nguyên hình thức mở UI bằng tab của trình duyệt mặc định.** Không đề xuất cửa sổ độc lập (Tauri / Electron / PWA đều vậy). Tác giả vừa dùng vừa qua lại giữa nhiều tab trình duyệt, nên việc Hub là một trong các tab chính là điểm mạnh. Cửa sổ chiếm một khung màn hình và tranh chỗ với các cửa sổ khác, nên đó là **bước lùi**
- `web/src/manifest.webmanifest` đang ở trạng thái `display: standalone` và có thể cài như PWA, nhưng vì lý do trên nên **không được dùng**. Đừng khuyên «cài PWA là thành cửa sổ»
- **Không triển khai thông báo gốc của hệ điều hành (toast).** Tác giả không thích. Việc báo có phê duyệt đang chờ đã đủ với 3 cơ chế hiện có (nhấp nháy tiêu đề `web/src/app/session-list.ts:1060` / badge số lượng trên favicon, cùng file 1039-1080 / âm báo `user_prefs.notify_sound`)
- **Không đề xuất tăng thêm kênh phân phối.** Đã có sẵn 4 kênh: winget / npm / Homebrew / Docker. Số đo thực tế (2026-08-14): 6 star, 2–3 lượt truy cập duy nhất mỗi ngày vào trang repo, 11 lượt npm mỗi tuần. Bài viết cũng đã có 132 bài trong 2 tháng, riêng bài giới thiệu sản phẩm đã ra 3 thế hệ v0.3.2 / v0.4.0 / v0.5.0. **Cả kênh lẫn bài viết đều đã cạn nước cờ; trần được quyết định ở phía cầu** (tập giao hẹp gồm những người vừa chạy song song nhiều AI CLI vừa bị nghẽn ở khâu phê duyệt)
- Ngoại lệ duy nhất: **khi chính tác giả muốn dùng dạng cửa sổ**. Không lấy độ phổ biến, việc hãng khác hỗ trợ, hay triển vọng tăng người dùng làm căn cứ (cùng hệ với mục Gemini và mục provider mới)

Vấn đề thực tế còn lại (việc khởi động và dừng bị tách thành 2 icon trên desktop) đã được tách riêng thành `docs/local/plan_tray-resident-hub-lifecycle.md` dưới dạng thường trú ở khay hệ thống mà không tạo cửa sổ.

**Các phương án đã bỏ qua được tổng hợp thành sổ cái tại `docs/local/reference_declined-directions.md`.** Sổ này gồm 4 mục: wrap Gemini, thêm provider mới, giao diện kanban cho session, và mục này; mỗi mục đều ghi «điều kiện được phép xem xét lại» và «thứ không được dùng làm căn cứ để xem xét lại». **Bắt buộc đọc sổ này trước khi đưa ra đề xuất cùng loại.** Bỏ qua không phải là phủ định vĩnh viễn, nên nếu điều kiện đã thỏa thì được phép xem xét lại.

## Trạng thái triển khai hiện tại

Đến v0.6.0 các mục sau đã có. v0.4.0 gỡ Workbench / chat_proxy và bổ sung provider opencode / Grok Build CLI cùng cấu hình Ollama `base_url`; v0.5.0 thêm subcommand `setup` / `doctor`, autoapproval và diff theo lượt; v0.6.0 thêm nội dung chat dựa trên transcript, đưa tiến độ Workflow về quyền quyết định của Hub, khởi động OpenCode ở chế độ cho phép toàn bộ, cảnh báo bản build cũ, và xử lý các mục kiểm toán trước phát hành A-01/A-02/A-03/A-05:

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
| Provider | `claude` / `codex` / `copilot` / `cursor-agent` (v0.4.0 thêm `opencode` / `grok`. `gemini` ngoài phạm vi — xem mục phạm vi) |

> Trong markdown, thống nhất dùng `many-ai-cli` (tên cũ `any-ai-cli` chỉ giữ trong mô tả lịch sử). Đường dẫn project local ghi trong `CLAUDE.local.md`.
>
> **Lưu ý khi grep**: tên cũ `any-ai-cli` là **chuỗi con** của tên mới `many-ai-cli` (`m` + `any-ai-cli`). Khi tìm phần sót của marker tên cũ (`<!-- any-ai-cli:approval-rules -->` v.v.) bằng mẫu tên mới thì **kết quả sẽ ra 0 dòng**. Dùng mẫu phía tên cũ thì bắt được cả hai.

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

## Tài nguyên phân phối lúc chạy (thứ cập nhật được mà không cần phát hành)

4 thư mục dưới `resources/` **không được nhúng vào binary, mà được fetch lúc chạy từ raw URL của nhánh `main`** (hằng số URL nằm ở `Default*Source` trong `internal/config/config.go`).

| Thư mục | Nội dung |
|---|---|
| `resources/approval-patterns/` | trigger phrase của phê duyệt |
| `resources/models/` | danh sách model gợi ý ở panel spawn (`defaults.json`) |
| `resources/slash-commands/` | bộ chọn slash command |
| `resources/usage-links/` | liên kết tình hình sử dụng |

**Hệ quả (không cần tra lại mỗi lần)**:

- Sửa nội dung thì **hoàn toàn không cần rebuild, gắn tag hay phát hành**. Vừa push lên `main` là phản ánh tới toàn bộ người dùng
- Ngược lại, **đặt ở `develop` thì không đến được ai**. Nguồn phân phối chỉ có `main`
- Vì phát trực tiếp tới mọi người dùng ngay lập tức nên **sai sót cũng công khai y nguyên**. Không tự động phản ánh; con người quyết định nhận hay không
- Phá vỡ ràng buộc định dạng sẽ làm hỏng parser (`internal/hub/slash_cmd_fetch.go` và các file khác)

**Chỉ `slash-commands` mới có cơ chế kiểm tra độ tươi** (`.claude/skills/slash-commands-update` với 3 chế độ report / apply / preflight; preflight được gọi từ bước kiểm tra tiền đề của release). `models` / `usage-links` / `approval-patterns` thì không có, nên **có lỗi thời cũng không ai nhận ra** (ngày 2026-08-11 phát hiện danh sách model bị thiếu nguyên thế hệ Claude 5 mà vẫn để nguyên).

Chi tiết xem các mục approval pattern / model / slash command trong [docs/v0.3.x-many-ai-cli-design.md](docs/v0.3.x-many-ai-cli-design.md) và [docs/manual_slash_commands_update.md](docs/manual_slash_commands_update.md).

## Quy tắc khi chèn mã quan sát để điều tra (bắt buộc)

Khi cài log, dump hay debug endpoint tạm thời để điều tra lỗi, **phải đăng ký vào `instrumentation.json` trong cùng commit đó**. Mã quan sát không đăng ký sẽ bị `scripts/check-instrumentation.mjs` chặn (chạy ở job `instrumentation` của Validate và ở bước kiểm tra tiền đề của release).

- **Bắt buộc điền `gate`. Về nguyên tắc cấm `always-on`.** Đặc biệt thứ lưu lại byte có nguồn gốc từ input phải tuân theo opt-in sẵn có (`log.session_enabled`)
- **Bắt buộc ghi `due`.** Quá hạn sẽ bị chặn. Muốn gia hạn thì cập nhật `due` và viết thêm vào `reason` lý do «vì sao vẫn còn cần». Không âm thầm kéo dài
- Khi đã gỡ thì đặt `status: removed`. Nếu thực thể vẫn còn thì kiểm tra sẽ trượt

**Chỉ viết trong comment «xác định được nguyên nhân thì xóa» sẽ không làm nó biến mất.** `/api/debug/batch-log` của v0.5.1, trace input của v0.5.x (kiểm toán A-01), rồi `/api/debug/ui-log` và `ui_input_trace` của v0.6.0 — 3 lần liên tiếp còn sót đến sát lúc xuất bản. Riêng `ui_input_trace` không có file riêng mà nằm rải ở 3 file dùng chung nên bị bỏ sót khỏi đợt gỡ.

## Tính năng ghi vào file của người dùng phải thiết kế đến bước «thu hồi ở lần khởi động sau» (quy định 2026-08-14)

Cách làm «chỉ sửa trong lúc session chạy rồi trả lại khi kết thúc» sẽ bay mất khi bị kill, chừng nào phần dọn dẹp còn nằm ở `defer` hay graceful shutdown. Hơn nữa **lần chạy tiếp theo sẽ coi phần sót lại là «bản gốc» rồi ghi đè trở lại, nên một khi đã bị bỏ lại thì không tự phục hồi mà trở thành vĩnh viễn**. Qua 2 đường — `opencode.json` (`internal/wrapper/opencode_config.go`) và khối quy tắc phê duyệt trong AGENTS.md (`internal/hub/approval_rules_state.go`) — sự việc đã đi xa tới mức bị commit lên repository công khai.

- **Không nghĩ theo hướng nâng tỷ lệ chạy được phần dọn dẹp** (không ngăn được kill). Bắt buộc phải có đường thu hồi phần bị bỏ lại ở lần khởi động sau
- Muốn thu hồi thì cần lưu lại «mình đã ghi cái gì». **Nếu thực thể khác với thứ đó thì đã có người khác động vào, nên không hoàn trả**
- Tên file sinh ra cũng phải đưa vào `.gitignore`. Tuy nhiên **gitignore không có tác dụng với file đang được track**, nên thứ đã commit chỉ xóa được bằng `git rm`
- **Thứ đã bị commit trước khi kịp thu hồi thì không đi vào đường thu hồi.** Phần đó do bước kiểm tra tồn dư của `many-ai-cli doctor` phát hiện (`internal/doctor/residue.go`). Nguyên tắc thiết kế và phương tiện phát hiện đi thành cặp; chỉ sửa một bên thì không đến được tay người dùng

## Tính đồng nhất của phê duyệt chỉ giữ đúng một nguồn (quy định 2026-08-15)

**State quyết định «phê duyệt này đã được trả lời chưa» chỉ có duy nhất một: `candidateKey + sourceEpoch`** (`web/src/app/approval-answered.ts`).

Trước đây vai trò này bị tách làm 3, và **mỗi cái định nghĩa «thế nào là cùng một câu hỏi» một kiểu khác nhau**.

| State cũ | Định nghĩa đồng nhất | Hết hạn |
|---|---|---|
| `approvalConsumedSig` | chữ ký của tập lựa chọn | timer 5–10 giây |
| `answeredMarkerSigs` | hash toàn văn khối marker | không hết hạn (suốt session) |
| `approvalQuestionKey` | hash của câu hỏi | trong lúc dismiss thủ công |

**Chính sự lệch nhau giữa 3 cái đó là triệu chứng.** Kiểu dùng timer hết hạn ngay giữa lúc TUI còn đang vẽ lại nên hiện lại phê duyệt đã trả lời; kiểu hash toàn văn thì ngược lại, chôn luôn cả «câu hỏi mà agent thật sự hỏi lại». Ở v0.7 đã gỡ cả 3 và gom về một.

- **Gặp lỗi hiển thị sai phê duyệt cũng không thêm nguồn ức chế thứ hai.** Trước hết hãy xác nhận triệu chứng đó có giải thích được bằng một nguồn hiện có hay không. Nếu giải thích được thì thứ cần sửa là cách tạo `candidateKey` hoặc cách tăng `sourceEpoch`, chứ không phải một state mới
- `candidateKey` được tạo từ provider, loại phê duyệt, câu hỏi đã chuẩn hóa, số thứ tự lựa chọn và chuỗi gửi đi. **Không đưa nhãn, khoảng trắng, đường kẻ, hay vị trí xuống dòng vào** (đưa vào thì mỗi lần vẽ lại sẽ thành ứng viên khác)
- `sourceEpoch` chỉ tăng ở ranh giới prompt trực tiếp. **Không tăng khi replay hay reflow** (tăng thì chỉ khôi phục thôi cũng trông như phê duyệt mới)
- Khi thế hệ đã tăng thì dù câu hỏi giống hệt vẫn hiển thị như ứng viên mới — đó là **đặc tả**. Không quay lại kiểu ức chế vĩnh viễn theo hướng «không bao giờ hiện lại cùng một câu hỏi»

## Quy tắc chung khi AI làm việc

Cấm build/commit, trách nhiệm secrets-scan, quy tắc tạo plan/bugfix/pending md, … thuộc cấu hình AI **toàn cục** của từng người dùng (ví dụ môi trường tác giả: `~/.claude/CLAUDE.md` và `~/.claude/guides/`). Quy tắc cá nhân (ngôn ngữ, xác nhận, format câu hỏi, …) cũng để ở cấu hình toàn cục; file này **chỉ** quy tắc riêng dự án.

Quy tắc bổ sung riêng dự án:

- Không chỉ build — **chạy, khởi động Hub, reload trình duyệt đều do người dùng thực hiện** (`go run` / `many-ai-cli serve` / `many-ai-cli stop` / khởi động·tắt·khởi động lại process Hub / reload browser, …).
- **Lệnh build chuẩn là `make build`**. Không dùng riêng `bun run build` (kể cả khi hướng dẫn người dùng). `make build` gom: build web (bun install + bun run build) → binary Windows/Linux → triển khai WSL. Lệnh «build giúp» của người dùng nghĩa là **chạy `make build`**.
- Tính năng lưu log session của chính `many-ai-cli` (`~/.many-ai-cli/logs/sessions/`) là tính năng **không được khuyến khích**. Kể cả khi điều tra lỗi quanh phần phát hiện phê duyệt, hãy xem log thô do chính agent AI đang chạy ghi ra (Claude Code / Codex CLI, …), chứ không phải log này. Log ở lớp wrapper có mật độ thông tin thấp, không hợp để truy nguyên nhân.

## Ranh giới bằng chứng kiểm toán · phát hành

- Thay đổi cursor của Agent Chat không được coi là xong ở mức parser đơn lẻ; phải theo dõi và kiểm chứng đến bước caller nhận offset và lần poll kế tiếp khởi động lại.
- Runtime bị gitignore nhưng là đối tượng `go:embed` phải được lấy về, kiểm chứng và đóng thành artifact trong clean release job, đồng thời bắt buộc xác nhận input tồn tại trước khi build. Việc file có sẵn ở máy local không được dùng làm bằng chứng cho release.
- Quy trình chi tiết, SHA và scope của secret trong release workflow lấy [`.github/workflows/release.yml`](.github/workflows/release.yml) và [plan xử lý kiểm toán](docs/local/archive/plan_security-vulnerability-quality-remediation-2026-08-13.md) làm chuẩn. Không sao chép quy trình vào CLAUDE.md.
- Xác nhận tĩnh, test local, lần chạy CI, release artifact và xác nhận trên máy thật là **những bằng chứng khác nhau**, phải báo cáo riêng. Không coi CI / artifact / xác nhận trên máy thật chưa chạy là đã xong.

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
