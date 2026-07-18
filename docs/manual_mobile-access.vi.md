# Hướng dẫn kết nối điện thoại / remote (📱 Kết nối mobile)

> Bản dịch tiếng Việt của [`manual_mobile-access.md`](manual_mobile-access.md).  
> Cập nhật nguồn: 2026-06-16 — v0.3.1/v0.3.2 đã có wizard Tailscale (📱). Bản dịch: 2026-07-18.

---

# ⚠️ Quan trọng: chỉ dùng khi đã hiểu rủi ro bảo mật

**Kết nối Hub từ điện thoại / remote mang lại tiện lợi nhưng đi kèm rủi ro nghiêm trọng.** Chỉ bật khi đã hiểu và chấp nhận.

- **Lộ thông tin kết nối = bên thứ ba có thể điều khiển từ xa máy của bạn.** Hub many-ai-cli, qua AI CLI, có thể xem/sửa/xóa file, chạy lệnh tùy ý, và lấy credential. Lộ token / QR / khóa SSH / xác thực VPN có thể dẫn tới **chiếm toàn bộ máy**.
- **URL / QR chứa token = mật khẩu.** Cấm chia sẻ screenshot, chia sẻ màn hình, đăng SNS, đưa cho người khác. **Một lần lộ có thể bị lạm dụng ngay.**
- **Mở SSH server, cấu hình VPN, sửa `allowed_hosts` / `trusted_networks` là trách nhiệm người dùng.** Cấu hình sai có thể **vô tình expose máy ra internet**.
- Chỉ kết nối từ **mạng và thiết bị tin cậy**. **Cấm Wi-Fi công cộng** (café, sân bay, khách sạn — rủi ro nghe lén / MITM cao). Tránh dùng thiết bị người khác.
- Phần mềm cung cấp **không bảo đảm** (theo giấy phép OSS). Tác giả **không chịu trách nhiệm** thiệt hại phát sinh từ remote.

> Hub **luôn** chỉ bind `127.0.0.1`. Remote chỉ hoạt động khi **người dùng chủ động** bật SSH/VPN; rủi ro thuộc về người dùng.

---

# 🚨 Playbook mất điện thoại (làm theo thứ tự từ trên)

**Điểm nóng thật sự là khóa SSH lưu trên điện thoại**, không phải auth Hub. **Ưu tiên cắt đường SSH.**

1. **Khóa / xóa từ xa điện thoại** (iPhone «Find My» / Android «Find My Device»).
2. **Nếu dùng SSH: thu hồi khóa ngay** — xóa public key của máy mất khỏi `~/.ssh/authorized_keys` trên PC/server (hoặc đổi mật khẩu). **Cắt đường shell.** Nếu máy có SSH, kẻ phá khóa màn hình lấy được shell = làm được mọi thứ; **PIN Hub không chặn được**.
3. **Hub: nút «Thu hồi toàn bộ truy cập»** (Settings → Remote access protection) để regenerate token. Token Hub / cookie auth / phiên PIN đều vô hiệu — **kể cả máy này** phải đăng nhập lại URL mới.
4. **VPN: gỡ thiết bị mất** — xóa device trên console Tailscale / xóa peer WireGuard.
5. **Phòng ngừa**: passphrase cho khóa SSH · khóa màn hình bắt buộc · (nếu chỉ VPN, không SSH) bật PIN tùy chọn trên Hub.

> Vì sao không ép auth Hub nặng: cấu hình có SSH trên điện thoại thì PIN Hub **vô nghĩa** (kẻ tấn công làm việc qua shell). Cái có hiệu lực: khóa thiết bị · passphrase SSH · xóa key khỏi `authorized_keys`. Ngược lại, **chỉ VPN không SSH** thì cửa vào chỉ là Hub — PIN tùy chọn là «chìa khóa cửa cuối» có ý nghĩa.

## Nút thu hồi và PIN tùy chọn (Settings → Remote access protection)

- **Thu hồi toàn bộ truy cập (kill switch, bắt buộc có)**: regenerate `cfg.Token` và secret ký cookie auth; vô hiệu hóa URL token cũ, cookie, phiên PIN. Dùng khi mất máy hoặc nghi lộ QR/token. **Thu hồi token Hub không cắt SSH** — vẫn phải xử lý khóa SSH/VPN riêng.
- **PIN tùy chọn (mặc định TẮT)**: PIN số ≥ 6 chữ số. Khi bật, **truy cập non-loopback** (điện thoại/VPN) yêu cầu đăng nhập PIN thêm. Loopback (PC này) vẫn chỉ cần token. Sai liên tiếp → khóa (5 lần → 1 phút → 5 → 30, exponential backoff). PIN lưu bcrypt, không giữ plaintext. Phù hợp vận hành «chỉ VPN, không SSH».
- **Thông báo thiết bị mới (SEC-C)**: thiết bị lạ remote connect/login → ntfy / webhook / Web Push (nếu đã cấu hình). Phát hiện xâm nhập khi QR/token bị đánh cắp.

---

## 0. Đây là gì?

Tập hợp quy trình kết nối điện thoại / máy khác tới Hub many-ai-cli (dashboard phê duyệt · tiến độ). Nút 📱 góc phải Hub hiện QR để quét — **tránh gõ tay URL/token trên điện thoại**.

## 1. Tiên quyết bảo mật

- **Hub chỉ bind `127.0.0.1`** (không public). Đây là cam kết công bố, không đổi.
- Máy ngoài chỉ tới được qua một trong:
  1. **SSH local forward** (nhanh · trong nhà / tunnel)
  2. **VPN** (WireGuard / Tailscale / Headscale) (ra ngoài · luôn bật)
  3. **Docker publish loopback** (server thường trú)
- **Token luôn bắt buộc.** QR URL có token = **mật khẩu**. Cấm chia sẻ screenshot.

## 2. Cách dùng nút 📱 Kết nối mobile

Góc phải Hub, giữa Usage và nút nguồn (⏻). Popup 3 tab.

### Tab 1: Mở URL (khi tunnel / VPN đã sẵn)

- Hiện QR `http://127.0.0.1:<port>/?token=…`.
- SSH tunnel hoặc VPN đã lập → quét QR là mở Hub.
- ⚠️ Có token = mật khẩu.

### Tab 2: SSH tunnel

- QR import app SSH (`ssh://<user>@<host>:22`) + lệnh copy:  
  `ssh -L <port>:127.0.0.1:<port> <user>@<host>`.
- Cần OpenSSH Server trên máy đích.
- Quy trình: ① tunnel trên app SSH điện thoại → ② quét URL tab 1.
- ⚠️ Ghi chú (G3): QR `ssh://` chuẩn **không** mang thông tin port-forward (`-L`). Nhiều app vẫn phải cấu hình `-L` tay. Lệnh copy sẵn để dán.
- Windows làm máy đích (G8): bật OpenSSH Server  
  (`Add-WindowsCapability ... OpenSSH.Server~~~~0.0.1.0` → `Start-Service sshd` / `Set-Service sshd -StartupType Automatic` / mở inbound 22). Khuyến nghị key auth.

### Tab 3: VPN (WireGuard / Tailscale)

- **WireGuard**: dán client `.conf` vào ô text → QR tại chỗ (**không** lưu server). App WireGuard quét QR là xong profile.
- **Tailscale**: cài app, login, cùng tailnet → mở bằng QR tab 1 hoặc `http://<tailscale-ip>:<port>/?token=`.
- Sau khi VPN lên, luôn mở Hub bằng QR URL tab 1.

## 2.5. Chênh chức năng theo kiểu kết nối (secure context)

Trình duyệt chỉ cho một số API khi «localhost hoặc HTTPS». Kiểu kết nối quyết định tính năng trên điện thoại:

| Kiểu kết nối | Ví dụ URL | Service Worker / PWA / Web Push / micro (voice) |
|---|---|---|
| **SSH tunnel** | `http://127.0.0.1:<port>` | ✅ Đủ (localhost = secure context) |
| **VPN IP thô** | `http://100.x:<port>` | ❌ Không (không secure context) |
| **VPN + HTTPS** | `https://tên.tailnet.ts.net` | ✅ Được (HTTPS hóa bằng Tailscale `tailscale serve`) |

- **Muốn full feature: SSH tunnel (127.0.0.1) là tốt nhất** (Push · voice · Add to Home Screen).
- VPN IP thô: xem/thao tác được nhưng mất Push/voice/PWA. Muốn full trên VPN: HTTPS Tailscale (`https://...ts.net`).
- Mở bằng VPN-IP còn cần `allowed_hosts` và CSP Hub cho phép WebSocket origin đó.

## 3. Bảng pattern truy cập nhanh

| # | Thiết bị thao tác | Vị trí Hub | Đường | Cần gì |
|---|---|---|---|---|
| A1 | PC này | PC này | loopback trực tiếp | Không (mặc định) |
| B1 | Điện thoại / PC khác trên Wi-Fi nhà | PC này | SSH local forward | OpenSSH Server trên PC |
| C1 | Ra ngoài | PC/server nhà | VPN → SSH forward | WireGuard, … + SSH |
| C2 | Ra ngoài | Hub Docker thường trú nhà | VPN → loopback publish | WireGuard + Docker |
| D1 | PC nhà | VPS (start tại chỗ) | Launcher SSH serve | SSH VPS |
| D2 | PC nhà / điện thoại | VPS (thường trú) | Launcher SSH tunnel | Hub thường trú trên VPS |
| D3 | Ra ngoài / nhà | Docker thường trú trên VPS | VPN → loopback publish | WireGuard + Docker(VPS) |

> B2 (bind thẳng LAN `0.0.0.0`) **không** dùng — trái cam kết public. Cùng mục đích an toàn hơn: B1 (SSH tunnel).

## 4. Ba cách VPN (cho người mới)

**Cả ba đều WireGuard bên trong.** Khác nhau chỉ ở **ai phát «danh bạ khóa»**. Traffic luôn mã hóa peer-to-peer; nhà cung cấp không đọc được nội dung.

| Cách | Một câu | Độ khó | Chi phí | Có «nhà cung cấp» | Phù hợp |
|---|---|---|---|---|---|
| WireGuard thuần | Tự dựng hết engine VPN | Cao (khóa · mở port) | Miễn phí | Không | Muốn nắm hết |
| Tailscale | WireGuard đóng gói dịch vụ | Thấp (chỉ login) | Cá nhân miễn phí | Có (coordination; không đọc payload) | Muốn nhanh |
| Headscale | «Danh bạ» Tailscale tự host trên VPS | Trung bình | Miễn phí (+ VPS) | Không (tự host) | Vừa tiện vừa tự chủ |

Ví dụ:

- WireGuard thuần — tự làm khóa và wiring: tự do, tốn công.
- Tailscale — «chuyển nhà» lo wiring + phân khóa: dễ nhất.
- Headscale — dựng «văn phòng danh bạ» trên VPS của mình; client vẫn dùng app Tailscale.

### IP Tailscale (100.x) và bảo mật

- Login → mỗi thiết bị có IP ảo cố định `100.x.y.z` (`100.64.0.0/10`), chỉ trong tailnet.
- Hai lớp: **coordination server = chỉ phân danh bạ khóa** / **traffic thật = WireGuard P2P E2E**. Kể cả khi qua DERP, payload vẫn mã hóa.
- An toàn: E2E; private key không rời thiết bị; login qua IdP (Google/GitHub, …).
- Chỗ tin cậy: ai được «vào» tailnet do coordination quyết. Muốn siết: **Tailnet Lock** hoặc **Headscale tự vận hành**.

## 5. Gợi ý thứ tự nếu đã có VPS

1. **Tailscale** trước: điện thoại · VPS · PC nhà — không mở port.
2. Muốn tự chủ: **Headscale** trên VPS, client chuyển sang (zero vendor).
3. Full control: **WireGuard thuần** (mở port · quản lý khóa).

Ba máy cùng VPN → điện thoại thấy Hub nhà và Hub VPS như «cùng nhà». Phân biệt bằng QR 📱 từng máy.

## 6. Giấy phép / thương hiệu

- WireGuard / Tailscale / Headscale do **người dùng tự cài**, không bundle trong many-ai-cli → không bắt buộc notice license (nominative use tên được phép). `WireGuard®` v.v. là thương hiệu bên thứ ba.
- **Thư viện QR đi kèm phân phối** phải có notice. Chọn MIT/BSD/Apache-2.0/0BSD; giữ text license trong vendor (tránh GPL).

## 7. Xử lý sự cố

- Quét QR không mở → tunnel/VPN chưa lập. Làm tab 2/3 trước, rồi tab 1.
- SSH không vào → OpenSSH Server trên đích? Port 22 thông?
- 401 / token lỗi → token cũ (Hub restart rotate). Xuất QR mới.
- VPN lên nhưng Hub không mở → host/port URL khớp Hub? VPN dùng `127.0.0.1` hay `100.x`/`10.8.x`? IP VPN cần `allowed_hosts`.
- Push / voice / Add to Home Screen không được → secure context (§2.5). Dùng SSH tunnel hoặc HTTPS hóa VPN.
- QR/bookmark đột ngột hỏng (G5) → port Hub nhảy (47778…) → lệch URL. Xuất lại QR từ 📱.
- WireGuard thuần từ ngoài không vào (G4) → CGNAT nhà không mở port được. Dùng Tailscale/Headscale (không cần mở port).

## 8. Việc sau khi implement (TODO nguồn gốc)

- [ ] Chuyển file này vào `docs/` (public) — *đã ở `docs/`*.
- [ ] Thêm link dẫn README mục điện thoại/remote.
- [ ] Quyết định public `access-patterns.html` hay không.

### Gợi ý đoạn dẫn README

```
## Kết nối từ điện thoại / remote
Hub many-ai-cli vẫn bind 127.0.0.1; thao tác từ điện thoại qua SSH tunnel / VPN.
Nút 📱 góc phải hiện QR (URL / SSH / VPN).
Chi tiết: docs/manual_mobile-access.md (bản Việt: docs/manual_mobile-access.vi.md).
```
