# Kiến trúc hệ thống Zalo-next

Tài liệu mô tả cách Zalo-next chia luồng xử lý, tổ chức module, lưu dữ liệu và kết nối mạng.
Quy tắc khi sửa từng phần nằm trong [`docs/standards/`](standards/README.md).

---

## 1. Tổng quan

```text
┌──────────────────────────── TẦNG GIAO DIỆN ────────────────────────────┐
│  GUI: src/gui.rs + src/gui/*  (eframe/egui, vẽ lại theo sự kiện)       │
│  TUI: src/tui.rs + src/ui.rs  (ratatui/crossterm)                      │
└───────────────┬───────────────────────────────────▲────────────────────┘
                │ WorkerCmd                          │ GuiEvent
                │ (tokio mpsc, unbounded)            │ (std mpsc + request_repaint)
┌───────────────▼───────────────────────────────────┴────────────────────┐
│  WORKER: thread "zalo-worker", Tokio runtime 2 luồng (src/gui/worker.rs)│
│  • WebSocket zpw_ws (src/socket.rs): tin nhắn, sự kiện, báo hiệu gọi   │
│  • HTTP Zalo (session.rs, chat.rs, media.rs): gửi/nhận, lịch sử, media │
│  • Đồng bộ điện thoại (history_sync*), cuộc gọi (call.rs, audio.rs)    │
│  • Cập nhật (updater.rs), trợ lý AI (ai.rs), công cụ FFmpeg             │
│  • Việc nặng: spawn_blocking (SQLite, giải mã ảnh, giải nén, băm)       │
└───────────────┬───────────────────────────────────┬────────────────────┘
                ▼                                   ▼
┌─────────────────────────────┐     ┌──────────────────────────────────────┐
│ LƯU TRỮ (thư mục dữ liệu)   │     │ MẬT MÃ & PHIÊN                       │
│ zalo_history.db (SQLite)    │     │ ParamCipher v2, AES theo zpw_enk,    │
│ threads_cache.json, cache   │     │ signkey MD5, cookie jar zpw_sek,     │
│ session.json (0600)         │     │ is_trusted_zalo_url                  │
└─────────────────────────────┘     └──────────────────────────────────────┘
```

Nguyên tắc cốt lõi: **luồng giao diện không bao giờ chờ I/O**. Mọi tác vụ mạng, đĩa, SQLite, mật mã và
tiến trình con đi qua `WorkerCmd`; kết quả quay về dưới dạng `GuiEvent`.

---

## 2. Bản đồ module

### 2.1. Lõi giao thức và phiên

| Module | Vai trò |
|---|---|
| `session.rs` | `Session`, `session.json`, cookie, `ZPW_VER`/`ZPW_TYPE`, ký request, `is_trusted_zalo_url`, `is_trusted_zalo_media_url` |
| `login.rs`, `qr.rs` | Đăng nhập QR (giao diện). Đăng nhập mật khẩu chỉ còn ở TUI/CLI (`--phone`, `--pass`) |
| `param_cipher.rs`, `crypto.rs` | `ParamCipher v2`, AES-CBC/GCM, `signkey` |
| `endpoints.rs` | Bảng dịch vụ `SERVICE_MAP_V3`, host cố định, URL tải FFmpeg/WASM đã ghim |
| `socket.rs` | WebSocket `zpw_ws`: mã hoá/giải mã frame, mã lệnh (`501/521` push, `510/511/513–515/517/518` pull, `601–604` điều khiển) |
| `chat.rs` | API tin nhắn và hội thoại: gửi, trích dẫn, reaction, thu hồi, lịch sử, nhóm, GIF, nhãn |
| `message_model.rs`, `message_navigation.rs`, `reactions.rs`, `unread.rs` | Metadata tin nhắn không mất mát, nhảy tới tin được trích dẫn, tổng hợp reaction, trạng thái chưa đọc theo con trỏ đọc của máy chủ |

### 2.2. Dữ liệu cục bộ

| Module | Vai trò |
|---|---|
| `store.rs` | SQLite `messages`, `threads`; nhập lịch sử (`import_history_from_path`: không xoá, tin trùng `msg_id` có thể được cập nhật nội dung) |
| `productivity.rs` | Bản nháp, Đã lưu, nhắc hẹn riêng |
| `scheduler.rs` | Hàng đợi tin hẹn giờ |
| `data_dir.rs` | Chọn (`resolve`) và khởi tạo (`initialize`) thư mục dữ liệu, `chdir` vào đó, ghi tệp riêng tư `0600`, chuyển dữ liệu cũ |
| `account_manager.rs` | Nhiều tài khoản: `accounts/<uin>.json` (phiên), `<uin>_history.db`, `<uin>_threads.json` |
| `quick_messages.rs` | Tin nhắn nhanh (`/` trong ô soạn) |

### 2.3. Đồng bộ lịch sử điện thoại

| Module | Vai trò |
|---|---|
| `history_sync.rs` | Luồng chung, Sync V2 (iPhone): khoá, metadata, tải và giải nén lô (trần 256 MiB tải, 512 MiB giải nén) |
| `history_sync/android.rs` | Sao lưu Android (`.db.crypt`, ZDB + xz) |
| `zproto_host.rs` | Chạy `libzproto_wasm` của Zalo trong `wasmtime` để tạo khoá và giải mã Sync V2, không cần Node.js |
| `sync_runtime.rs` | Tải runtime WASM từ máy chủ Zalo khi thiếu: không cookie, tối đa 2 MiB, kiểm SHA-256 ghim sẵn, lưu `.sync-runtime/` |

Chi tiết và bằng chứng: `re-notes/42`–`44`, `47`.

### 2.4. Media, cuộc gọi, công cụ

| Module | Vai trò |
|---|---|
| `media.rs`, `media_resources.rs` | Ảnh, tệp, sticker, voice. Mọi lượt tải đều có giới hạn dung lượng; client có cookie chỉ dùng cho host Zalo |
| `doc_preview.rs` | Trích văn bản PDF/Office/text khi người dùng bấm Xem trước (chỉ trong RAM) |
| `call.rs`, `audio.rs`, `voip_log.rs` | Báo hiệu VoIP, RTP, âm thanh `cpal` + `opus`, nhật ký cuộc gọi |
| `voice.rs` | Thu âm tin nhắn thoại, đổi định dạng mẫu, AAC |
| `video_tools.rs`, `video_player.rs` | Dò/cài FFmpeg; phát video trong app (FFmpeg giải mã trên thread riêng) |
| `emoji_raster.rs` | Lấy bitmap emoji màu từ font hệ thống (chỉ chạy ở worker) |
| `native_notifications.rs` | Thông báo macOS qua helper `DailyNotifier`, dự phòng `osascript` |
| `updater.rs` | Tải, kiểm SHA-256 và cài bản mới (ADR-0002) |
| `ai.rs` | Client trợ lý `@AI` tách biệt hoàn toàn khỏi mạng Zalo (ADR-0004) |

### 2.5. Giao diện (`src/gui/`)

| Tệp | Vai trò |
|---|---|
| `gui.rs` | `ZaloGuiApp`, nhận `GuiEvent`, màn hình đăng nhập, thanh tiêu đề tự vẽ |
| `worker.rs` | `WorkerCmd`, `spawn_session_worker`, các vòng lặp nền, kiểm tra cập nhật |
| `workspace.rs` | Bố cục 3 cột: thanh điều hướng, danh sách hội thoại (kéo giãn được), khung chat, ngăn thông tin |
| `messages.rs` | Bong bóng tin, sticker, album, thẻ cuộc gọi, trích dẫn, reaction, menu chuột phải, dán ảnh |
| `modals.rs` | Cài đặt, cửa sổ cuộc gọi, xem ảnh/video, chia sẻ, tìm theo số điện thoại, Dev Inspector |
| `picker.rs`, `stickers.rs` | Bảng Sticker/Emoji/GIF; gợi ý sticker khi gõ |
| `album.rs`, `reactions_dialog.rs` | Album ảnh dạng lưới; hộp thoại Biểu cảm |
| `appearance.rs`, `theme.rs` | Tuỳ biến giao diện; `ThemePalette`, icon, font (Be Vietnam Pro đóng gói sẵn) |
| `ai_ui.rs` | Bong bóng và cài đặt trợ lý AI |
| `productivity.rs`, `doc_preview_ui.rs`, `voice_ui.rs`, `video_tools_ui.rs` | Giao diện bản nháp/Đã lưu/nhắc hẹn, xem trước tài liệu, ghi âm, cài FFmpeg |
| `tray.rs`, `background.rs` | Chạy nền khi đóng cửa sổ, icon menu bar/khay |
| `clipboard_write.rs`, `macos_clipboard.rs`, `macos_drag.rs` | Clipboard và kéo-thả theo hệ điều hành (objc2 trên macOS) |

Module cũ còn giữ: `api_mcp.rs` (máy chủ REST/MCP **đã tắt**, chỉ còn dùng phần quy tắc tự trả lời),
`feed.rs` (giao diện Feed đã gỡ, vài hàm vẫn được `modals.rs` dùng). Công cụ dò lỗi cho nhà phát triển nằm
trong `src/bin/`.

---

## 3. Luồng xử lý

### 3.1. Khởi động
1. `main.rs` đọc cờ dòng lệnh. `--version` và `--check-update` chạy rồi thoát mà không đụng tới dữ liệu.
   `--print-data-dir` chuẩn bị thư mục (tạo, thử ghi) nhưng không chuyển dữ liệu cũ, in đường dẫn rồi thoát.
2. `data_dir::resolve` chọn thư mục; `initialize` tạo thư mục (quyền `0700` với thư mục riêng tư trên Unix),
   chuyển dữ liệu cũ khi cần (macOS) rồi `chdir` vào đó.
3. Nạp `session.json`. Nếu còn hạn, GUI hiện ngay danh sách từ `threads_cache.json`. Worker kết nối
   WebSocket và đồng bộ phần còn thiếu từ máy chủ và SQLite.

### 3.2. Nhận tin
- WebSocket là kênh chính. Frame được giải mã ở worker, rồi tin được ghi SQLite (khử trùng theo `msg_id`) và
  gửi `GuiEvent` về giao diện.
- Song song đó, worker hẹn giờ HTTP polling mỗi 1,2 giây (bỏ qua khi đang có long-poll hoặc đã đăng xuất) để
  bù tin. Vòng polling chỉ gửi `ThreadsUpdated` khi có tin mới; đường làm mới toàn bộ cũng gửi sự kiện này.
- Số tin chưa đọc ưu tiên con trỏ đọc của máy chủ (`unread.rs`). Khi chưa có con trỏ, GUI đếm các tin đến
  đã tải.

### 3.3. Gửi tin
- Giao diện gửi `WorkerCmd`. Worker tạo `cliMsgId` một lần cho mỗi lần gửi, mã hoá params, gọi API rồi báo
  `MessageSent`; giao diện chèn bong bóng khi nhận sự kiện này.
- Reaction và trích dẫn tham chiếu `cliMsgId` của tin gốc (chỉ dùng `ts` khi thiếu). Gửi thất bại thì hiện lỗi, không tự gửi lại các thao tác
  không lặp an toàn (tệp, ghi âm).

### 3.4. Gợi ý sticker
Gõ trong ô soạn → debounce 145 ms (0 ms với emoji hoặc từ khoá đã cache) → `WorkerCmd::SuggestComposerStickers`
→ worker gọi song song `suggest/stickers` và `search` → nạp sẵn ảnh trong `stickers_cache/` rồi gửi
`GuiEvent::ComposerStickerSuggestions`. Ảnh còn thiếu được tải song song.

### 3.5. Chạy nền
Đóng cửa sổ chỉ ẩn app; worker vẫn nhận tin, thông báo và cuộc gọi. Khi cửa sổ có focus, heartbeat là 3 giây,
và khoá sắp xếp danh sách được cache để giảm CPU lúc rảnh.

---

## 4. Lưu trữ

| Nơi lưu | Nội dung |
|---|---|
| `zalo_history.db` | Bảng `messages`, `threads`, `drafts`, `saved_items`, `message_reminders`, `scheduled_messages`, `local_ai_messages`. Chi tiết schema: [`standards/storage.md`](standards/storage.md) |
| `threads_cache.json` | Ảnh chụp danh sách hội thoại để mở app tức thì |
| `session.json` | Phiên Zalo (bí mật, `0600`) |
| `app_settings.json`, `theme.json` | Cài đặt, giao diện, mục Gần đây của bảng sticker/emoji |
| `stickers_cache/`, `media_cache/` | Cache ảnh sticker và media |
| `ai_secret.json`, `sync_identity.json` | API key trợ lý, danh tính Sync V2 (bí mật, `0600`) |

Thư mục dữ liệu trên macOS là `~/Library/Application Support/ctdozlite`. Tên cũ được giữ để không mất đăng
nhập và quyền hệ thống (ADR-0005).

---

## 5. Kết nối mạng

| Đích | Mục đích | Mang thông tin đăng nhập Zalo? |
|---|---|---|
| `*.zalo.me`, `*.zaloapp.com`, `*.zadn.vn`, `*.zdn.vn` | API, WebSocket, tải lên | Có, chỉ sau khi `is_trusted_zalo_url` chấp nhận |
| `dlmd.me`, `dlfl.vn` | CDN ảnh/tệp/video của Zalo | Không (client không cookie) |
| `api.github.com`, `github.com` (repo này) | Kiểm tra và tải bản cập nhật | Không |
| `chat.zalo.me/libs/…wasm` | Runtime Sync V2 | Không |
| `osxexperts.net`, `evermeet.cx` (cho phép chuyển hướng tới mirror `e.deolaha.ca`) | Tải FFmpeg (macOS) | Không |
| Địa chỉ trợ lý AI do người dùng cấu hình | `@AI` | Không, chỉ API key của trợ lý |

App không chạy máy chủ cục bộ nào (REST/MCP đã tắt); cuộc gọi chỉ mở cổng UDP tạm cho RTP/RTCP. App không tự
tải các liên kết trong tin nhắn đến. Ảnh và ảnh thu nhỏ từ máy chủ ngoài Zalo chỉ được tải khi người dùng bấm
**Hiện ảnh**.
