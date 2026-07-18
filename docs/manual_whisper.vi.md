# Cài đặt nhập liệu giọng Whisper

> Bản dịch tiếng Việt của [`manual_whisper.md`](manual_whisper.md). Bản dịch: 2026-07-18.

Hướng dẫn dùng **Whisper local** cho nhập liệu giọng trên Hub UI.

## Cài managed: Hub Windows x64

Cài **managed** chỉ khả dụng khi **process Hub** chạy trên **Windows x64**.

1. Khởi động Hub và mở UI.
2. Mở **Settings → Voice**.
3. Chọn `Whisper (local)`.
4. Trong Local Whisper, chọn model.
5. Bấm **Install** và đợi thanh tiến trình xong.
6. Bấm **Start**, rồi dùng nút micro hoặc `Alt+V`.

Installer lưu file dưới `~/.many-ai-cli/whisper/`:

| Đường dẫn | Mục đích |
|---|---|
| `bin/` | Binary whisper.cpp server đã giải nén + DLL runtime đi kèm |
| `models/` | File model ggml đã tải |
| `tmp/` | Tải tạm |
| `whisper-server.log` | stdout/stderr của server managed |

Hub khởi động server managed trên `127.0.0.1` và ghi URL local đã chọn vào `voice.whisper.server_url`.

### Runtime VC++ tự chứa (không phụ thuộc System32)

`whisper-server.exe` liên kết Microsoft Visual C++ runtime — đặc biệt `vcomp140.dll` (OpenMP), vốn **không** có trong `whisper-bin-x64.zip` chính thức. Máy thiếu VC++ redistributable sẽ lỗi khởi động (`0xC0000135` / `STATUS_DLL_NOT_FOUND`).

Để tự chứa, many-ai-cli nhúng bốn DLL x64  
(`vcomp140.dll`, `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`) trong binary (`go:embed`) và copy cạnh `whisper-server.exe` trong `bin/` khi cài và trước mỗi lần start. **Không** ghi System32; gỡ (`RemoveAll` của `~/.many-ai-cli/whisper/`) không để vết. UCRT (`ucrtbase.dll`) là thành phần Windows 10/11 — không bundle.

DLL lấy từ thư mục `\VC\redist` của Visual Studio qua  
`internal/whisperruntime/fetch_windows_runtime.ps1` (kiểm Authenticode + kiểu máy PE). File bị gitignore (không commit), nên phải đặt vào  
`internal/whisperruntime/files/windows-amd64/` **trước khi build Go** để `go:embed` nhận — chạy script trên máy build, hoặc commit DLL đã ký.

> Lưu ý: bước đặt DLL **chưa tự động hóa trong pipeline release**  
> (`release.yml` build trên Linux bằng GoReleaser, không chạy script Windows).  
> Cho đến khi nối (ví dụ leg `windows-latest` chạy script, hoặc commit DLL), binary Windows phát hành chỉ embed runtime nếu DLL có mặt lúc build. Nếu thiếu, `whisperruntime.Ensure` là no-op an toàn và Whisper managed vẫn phụ thuộc VC++ cài system-wide (vẫn có rủi ro `0xC0000135`).

Điều khoản phân phối (Microsoft «Distributable Code», redistrib app-local): xem  
`docs/local/plan_unified-local-whisper-all-os.md` (C2).

## Tải xuống và xác minh

Cài managed là **opt-in**. Sẽ tải:

- Archive release whisper.cpp Windows x64 từ `https://github.com/ggml-org/whisper.cpp/releases`
- Model ggml đã chọn từ `https://huggingface.co/ggerganov/whisper.cpp`

Archive whisper.cpp được **xác minh SHA-256** trước khi giải nén. Model chưa có hash công bố vẫn tải qua HTTPS và UI ghi **chưa xác minh hash**.

## Cài managed: Docker (Linux / remote server) kèm server sẵn

Use-case remote chính: iPhone → SSH tunnel → Hub trên remote server → Whisper localhost.  
Image Docker many-ai-cli **nhúng** binary `whisper-server` (build từ whisper.cpp với `GGML_OPENMP=OFF`, không phụ thuộc libgomp) và trỏ Hub qua biến môi trường `MANY_AI_CLI_WHISPER_SERVER`.

Khi biến trỏ tới executable có sẵn, Hub coi Whisper là «managed và đã cài»: **bỏ** bước tải binary, chỉ tải model vào  
`~/.many-ai-cli/whisper/models/`. Compose mặc định map path đó lên volume home user → model **sống sót** khi recreate container, không tải lại.

Service compose chạy `init: true` (tini là PID 1) để reaping process con `whisper-server`, tránh zombie/orphan nếu Hub crash; Hub còn kill process group khi shutdown.

**Không** publish port Whisper — lắng nghe `127.0.0.1` trong namespace mạng container, chỉ Hub cùng container gọi được.

## Server thủ công: macOS, Linux, WSL, hoặc build tùy chỉnh

Trên môi trường Hub không Windows và không có server nhúng, tự chạy server tương thích Whisper rồi trỏ Hub tới.

Ví dụ:

```bash
whisper-server -m /path/to/ggml-large-v3-turbo-q5_0.bin --host 127.0.0.1 --port 8080
```

Sửa `~/.many-ai-cli/config.yaml`:

```yaml
voice:
  whisper:
    managed: false
    server_url: "http://127.0.0.1:8080"
    request_path: ""
    language: "ja"
    timeout_seconds: 60
```

Hub thử OpenAI-compatible `/v1/audio/transcriptions` trước, rồi fallback `/inference`. Chỉ set `request_path` khi server bắt buộc path cố định.

## Chọn model

Whisper managed chạy **CPU** (build whisper.cpp đi kèm **không** GPU), latency phụ thuộc số nhân. Đo trên máy ~10 logical CPU, câu tiếng Nhật 5–7 giây:

| Model | Latency | Độ chính xác |
|---|---|---|
| `small` (mặc định) | 2–3 s | Cấu trúc câu tốt; thuật ngữ kỹ thuật có thể sai chính tả |
| `large-v3-turbo-q5_0` | ~7 s | Chính xác nhất; latency tăng nhanh khi ít core |
| `tiny-q5_1` | <1 s | Chỉ smoke test; không dùng nhập thật |

Bắt đầu với `small`. Chỉ chuyển `large-v3-turbo-q5_0` trên CPU nhiều nhân mạnh, hoặc khi `server_url` trỏ server Whisper có GPU bên ngoài.

## Tham chiếu cấu hình

| Key | Ý nghĩa |
|---|---|
| `voice.whisper.managed` | `true`: Hub quản lý whisper.cpp local. Hỗ trợ Windows x64, và image/host cung cấp server qua `MANY_AI_CLI_WHISPER_SERVER`. |
| `voice.whisper.model` | ID model managed. Mặc định: `small`. |
| `voice.whisper.server_url` | URL server Whisper local. Chế độ managed ghi tự động. |
| `voice.whisper.server_port` | Port ưa thích cho server managed. `0` = tự chọn. |
| `voice.whisper.request_path` | Ghi đè endpoint (tùy chọn). Rỗng = tự dò. |
| `voice.whisper.language` | Gợi ý ngôn ngữ: `ja`, `en`, `auto`, … |
| `voice.whisper.timeout_seconds` | Timeout proxy Hub cho một request transcription. |

## Tùy chọn server khuyến nghị

Whisper có thể “ảo giác” cụm cố định khi im lặng / nhiễu nền. Nên bật các lớp sau khi build hỗ trợ:

- VAD hoặc lọc no-speech phía server
- Decoding deterministic hoặc temperature thấp
- Server chỉ bind `127.0.0.1`
- **Đọc lại** text chèn trước khi gửi (Whisper mặc định chèn vào ô nhập)

Phía trình duyệt cũng loại bản ghi gần im lặng và bỏ khớp chính xác các cụm ảo giác đã biết.

## Xử lý sự cố

| Triệu chứng | Kiểm tra |
|---|---|
| `Whisper server is not installed` | Windows x64 (hoặc Docker có server nhúng): Settings → Voice → Install. Nền tảng khác: cấu hình `server_url`. |
| `Whisper server is not configured` | Set `voice.whisper.server_url` hoặc bật cài managed. |
| Lỗi kết nối | Server đang listen `127.0.0.1` và port khớp cấu hình. |
| Transcription chậm | Thử model nhỏ hơn (`small` / `tiny-q5_1`). |
| Kết quả rỗng hoặc ảo giác | Bật VAD/no-speech; tắt auto-submit đến khi kiểm chứng ổn. |
| Server managed không start | Xem `~/.many-ai-cli/whisper/whisper-server.log`. |
