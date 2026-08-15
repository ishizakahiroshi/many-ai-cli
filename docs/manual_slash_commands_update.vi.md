# Quy trình cập nhật danh sách slash-command many-ai-cli

> Bản dịch tiếng Việt của [`manual_slash_commands_update.md`](manual_slash_commands_update.md).  
> Cập nhật nguồn: 2026-06-19 — skill bán tự động `slash-commands-update` và báo cáo freshness. Bản dịch: 2026-07-18.

> **⚠️ Bản dịch này là ảnh chụp tại một thời điểm và không còn được đồng bộ.**
>
> Kể từ 2026-08-15, các file `*.vi.md` trong repo này **không còn được cập nhật theo bản gốc**. Bản gốc [`manual_slash_commands_update.md`](manual_slash_commands_update.md) là chuẩn duy nhất — vui lòng đối chiếu bản gốc trước khi làm theo nội dung ở đây. **Bản gốc đã được cập nhật sau lần dịch này** (bản dịch: 2026-07-31 · bản gốc: 2026-08-11).
>
> Xin chân thành cảm ơn người đã đóng góp bản dịch. Chúng tôi vẫn hoan nghênh đóng góp dịch thuật; chỉ là không thể hứa giữ đồng bộ, nên bản được merge cũng sẽ là ảnh chụp có ghi ngày.
>
> *(EN: This translation is a dated snapshot and is no longer kept in sync with the original. Please refer to the source file linked above. The Vietnamese **UI** locale `web/src/i18n/vi.json` is a shipped feature and is still maintained.)*

Ghi chép vận hành: khi Claude Code / Codex CLI / GitHub Copilot CLI / Cursor Agent CLI thêm slash-command mới, cách phản ánh vào **slash-command picker** trên dashboard.

## Khuyến nghị: bán tự động bằng skill `slash-commands-update`

Trước khi theo dõi thủ công upstream, ưu tiên skill  
`D:\dev\workshop\skills\slash-commands-update` (đường dẫn môi trường tác giả). Skill hỗ trợ: phát hiện diff với upstream · đề xuất chuẩn hóa markdown · báo cáo cho người duyệt (quyết định accept/reject vẫn do người).

- Kích hoạt: «kiểm tra độ tươi slash-command», «slash-commands-update», …
- Chế độ:
  - `report` (mặc định): diff mọi provider → tạo `docs/local/slash-command-freshness_YYYY-MM-DD.md`. Provider được phát hiện động từ `resources/slash-commands/*.md` (kể cả `opencode.md` mới).
  - `apply`: chỉ áp diff có `decision = accepted` vào `resources/slash-commands/*.md` (`pending` / `unknown` / `deferred` bỏ qua).
  - `preflight`: cổng trước release (xem liên kết release).
- Script:
  - `scripts/freshness-report.ps1`: trích inventory local + khung báo cáo; `copilot help commands` diff tự động. Docs-based (codex / cursor-agent) và claude / opencode đặt `unknown` để tránh false positive — AI ở `report` xác nhận bằng WebFetch / claude-code-guide / thu thập máy thật. **Không commit / push**.
  - `scripts/freshness-preflight.ps1`: kiểm độ tươi báo cáo mới nhất (mặc định 7 ngày) và diff chưa quyết (`pending`); exit 0=ok / 2=cần xử lý / 3=stale·cần report.
- Hợp đồng chuẩn: skill `references/provider-sources.md` (source-of-truth theo provider · ràng buộc md · format báo cáo).

C1–C6 dưới đây là chi tiết các bước skill đi nội bộ (và chuẩn khi làm tay).

## Cơ chế (tiên quyết)

- Chuẩn danh sách:  
  `resources/slash-commands/claude.md` / `codex.md` / `copilot.md` / `cursor-agent.md` (bảng markdown).
- Hub lúc chạy **fetch + parse qua raw URL GitHub**. Nguồn mặc định trong `internal/config/config.go`:
  - `DefaultClaudeSlashCmdSource = https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/claude.md`
  - (tương tự codex / copilot / cursor-agent)
- **Không cần rebuild · không tốn token AI**: chỉ cập nhật md và push `main`. Không đụng binary.
- Cache theo provider **24h** (`slashCmdCacheTTL`).

## Quy trình

### C1. Thu thập danh sách lệnh hiện hành (nguồn có thẩm quyền)

- **Claude Code**: nhờ agent claude-code-guide «liệt kê đủ slash-command built-in» (không viết từ trí nhớ).
- **Codex CLI**: docs chính thức [Slash commands in Codex CLI](https://developers.openai.com/codex/cli/slash-commands).
- **GitHub Copilot CLI**: output máy thật `copilot help commands` làm chuẩn; lệch docs thì ưu tiên máy thật.
- **Cursor Agent CLI**: `/help` máy thật `cursor-agent`; không có máy thì docs [Slash commands | Cursor Docs](https://cursor.com/docs/cli/reference/slash-commands), ưu tiên máy thật nếu lệch.
- Loại lệnh đã xóa (ví dụ Claude `/vim`, `/pr-comments`).

### C2. Cập nhật file md

Mỗi dòng dạng:

`| \`/cmd\` | Mục đích (1 câu) | Khi nào dùng (1 câu) |`

Thứ tự: **ABC theo tên lệnh** (thống nhất giữa provider).

**Ràng buộc parser** (`internal/hub/slash_cmd_fetch.go` — `cleanDescMarkdown` / `tableRowRe`):

- **Cột lệnh chỉ tên lệnh, không đối số.** Viết `/effort [level|auto]` có `|` sẽ vỡ parse dòng bảng.
- **Mô tả không dùng `( )`.** `cleanDescMarkdown` xóa nội dung trong ngoặc tròn/vuông → mất trên dashboard.  
  Ví dụ mức: `Set the model effort level: low, medium, high, ...` (dấu hai chấm, không ngoặc).
- Không dùng `|` trong mô tả (xung đột cột).
- Markdown trang trí (`**bold**` / link / `` `code` ``) bị gỡ tự động — tránh thì an toàn hơn.

### C3. Kiểm tra local (tùy chọn · khuyến nghị)

Gọi parser trên file thật bằng test tạm trong `internal/hub/`:

```powershell
go test ./internal/hub/ -run <tên-test-tạm> -v
```

- Số lệnh parse khớp số dòng bắt đầu bằng `| \`/` → không sót.
- Soi mô tả `/effort` v.v. không bị cắt vì ngoặc.
- Xóa file test tạm sau khi xong (**không commit**).

### C4. Commit và đưa lên `main`

Nguồn phát là raw URL của **`main`**. Chỉ commit `develop` **không** đủ — cần merge push `main`.

```powershell
git add resources/slash-commands/claude.md resources/slash-commands/codex.md resources/slash-commands/copilot.md resources/slash-commands/cursor-agent.md
git commit -m "feat: refresh slash-command lists"

git checkout main
git merge develop --no-edit
git push origin main
git checkout develop
```

> Here-string commit PowerShell dùng `@'...'@`; đừng đưa `@'...'@` vào Bash tool (dấu `@` dính message). Sửa bằng `git commit --amend -F <file>` nếu cần.

### C5. Xác nhận nội dung phát

Sau push, mở raw URL:

```
https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/claude.md
https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/codex.md
https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/copilot.md
https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/cursor-agent.md
```

### C6. Force refresh trên dashboard

Hub cache 24h — muốn ngay thì force fetch tay:

- **Không** phải màn Settings. Ô «slash command sources» chỉ invalidate cache khi **đổi URL**; URL giữ nguyên thì Save **không** fetch lại.
- Đúng: hàng nút nhanh dưới ô nhập → **`/ ▾`** mở picker → góc phải **`⟳` (refresh)**. Gọi `POST /api/slash-commands?provider=...` bỏ cache.
- Picker theo **provider session active** (claude / codex / copilot / cursor-agent). Mở từng provider và bấm `⟳`.
- Timestamp trên picker thành «vừa xong» và lệnh mới xuất hiện → thành công.

## Liên quan

- Skill bán tự động: `.../skills/slash-commands-update/SKILL.md` (hợp đồng: `references/provider-sources.md`)
- Parser: `internal/hub/slash_cmd_fetch.go`
- API: `internal/hub/slash_handlers.go` (`handleSlashCommands`: GET=cache / POST=force)
- URL nguồn: `internal/config/config.go`
- UI picker: `web/src/app.js` / `web/src/index.html` (`slash-picker-refresh`)
- Release: [manual_release.md](manual_release.md) (kiểm diff resource phục vụ runtime)
