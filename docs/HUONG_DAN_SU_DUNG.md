# Hướng dẫn sử dụng Zalo-next

Tài liệu tóm tắt các tùy chọn dòng lệnh, thao tác trên giao diện đồ họa (GUI) và bảng phím tắt của giao diện terminal (TUI).

---

## 1. Tham số dòng lệnh (CLI Flags)

| Tham số | Ý nghĩa | Ví dụ |
|---|---|---|
| `--data-dir <PATH>` | Chọn thư mục lưu phiên, cache, log và cơ sở dữ liệu; đường dẫn tương đối tính từ thư mục khởi chạy. | `zalo-next --data-dir ./local-data --gui` |
| `--print-data-dir` | Chuẩn bị và in thư mục dữ liệu rồi thoát, không mở giao diện hay kết nối mạng. | `zalo-next --print-data-dir` |
| `--gui` | Chạy chế độ giao diện đồ họa (mặc định nếu không truyền tham số). | `zalo-next --gui` |
| `--tui` | Chạy chế độ giao diện dòng lệnh trong terminal. | `zalo-next --tui` |
| `--qr` | Yêu cầu tạo mã QR đăng nhập ngay khi khởi động TUI. | `zalo-next --tui --qr` |
| `--dev` | Bật thanh công cụ Dev Inspector để xem log mạng và payload giải mã. | `zalo-next --gui --dev` |
| `--logout` | Xóa phiên đăng nhập hiện tại (`session.json`) rồi thoát. | `zalo-next --logout` |
| `--phone <SỐ>` | Điền sẵn số điện thoại khi thử đăng nhập bằng mật khẩu. | `zalo-next --tui --phone 0912345678` |
| `--pass <MK>` | Điền sẵn mật khẩu. | `zalo-next --tui --phone 0912345678 --pass ...` |
| `--dump` | Lưu các gói phản hồi JSON ra thư mục `dump/` để phục vụ phân tích. | `zalo-next --dump` |

---

Trên macOS, mọi cách chạy (bản `.app`, `cargo run`, TUI, CLI) đều dùng chung `~/Library/Application Support/ctdozlite`, kể cả khi thư mục làm việc là `/`, nên GUI và TUI luôn thấy cùng một phiên đăng nhập và cơ sở dữ liệu. Windows và Linux giữ thư mục làm việc hiện tại. `--data-dir` ghi đè lựa chọn này (`--profile <tên>` tạo thư mục con `profiles/<tên>`); thư mục chỉ định và thư mục mặc định trên macOS có quyền `0700` trên Unix. Nếu thư mục không ghi được, chương trình báo lỗi trước khi mở dữ liệu.

`--version` và `--print-data-dir` không chuẩn bị hay di chuyển dữ liệu.

**Tự chuyển dữ liệu cũ (macOS):** ở lần chạy đầu sau khi đổi quy tắc, nếu thư mục dữ liệu mới chưa có phiên/lịch sử nhưng thư mục làm việc có `session.json`, `zalo_history.db` hoặc `accounts/`, chương trình sao chép (quyền `0600`/`0700`) các tệp của Zalo-next (`session.json`, `zalo_history.db` kèm `-wal`/`-shm`, `accounts/`, `threads_cache.json`, `app_settings.json`, `theme.json`, `custom_stickers.json`, `autoreply_rules.json`, `active_account`, khóa Sync V2), kiểm tra kích thước rồi đổi tên bản gốc thành `*.bak` (không xóa). Không bao giờ ghi đè tệp đã có ở thư mục mới, không đụng tới tệp khác, và không chạy khi dùng `--data-dir` / `--profile`. Muốn quay lại, đổi tên `*.bak` về tên cũ và chạy với `--data-dir .`.

## 2. Giao diện đồ họa (GUI)

### 2.1. Đăng nhập
1. Chạy `./target/release/zalo-next --gui`.
2. Mở ứng dụng Zalo trên điện thoại, quét mã QR hiển thị trên màn hình và bấm **Đăng nhập**.
3. Sau khi xác nhận, phiên làm việc được lưu vào `session.json`. Ở những lần mở tiếp theo, ứng dụng tự nạp lại phiên này mà không cần quét lại mã QR.
4. Mã QR có hạn khoảng 3 phút; app hiện đếm ngược và tự lấy mã mới khi hết hạn. Nếu điện thoại báo
   **"không có kết nối với máy chủ"** khi quét: thoát hẳn Zalo trên điện thoại rồi mở lại, bấm
   **Tạo / Làm mới mã QR**, kiểm tra mạng (tắt VPN) rồi quét lại. Hướng dẫn này cũng có ngay dưới mã QR.

### 2.2. Nhắn tin & Gợi ý Sticker tự động
- **Tìm kiếm và lọc hội thoại:** Sử dụng ô tìm kiếm phía trên danh sách bên trái (hỗ trợ tìm tiếng Việt không dấu) hoặc lọc theo các tab *Tất cả*, *Cá nhân*, *Nhóm*.
- **Gợi ý nhãn dán khi đang gõ:**
  - Khi nhập từ khóa hoặc cụm từ ngắn trong ô chat (`xin chào`, `cảm ơn`, `ok`, `haha`, `buồn`, `khóc`, `nhớ`, `cố lên`, `mệt quá`, `đi chơi`, `trà sữa`...), ứng dụng tự động truy vấn API sticker của Zalo và hiển thị dải gợi ý phía trên khung soạn thảo.
  - Bấm trực tiếp vào sticker để gửi, hoặc dùng phím mũi tên `↑` để chọn lên thanh gợi ý, `←` / `→` để di chuyển, `Enter` để gửi và `Esc` để đóng.
  - Bấm biểu tượng kính lúp ở góc phải thanh gợi ý để mở toàn bộ kết quả tìm kiếm của từ khóa đó trong bảng Sticker Picker.
- **Kho nhãn dán (Sticker Picker):**
  - Bấm biểu tượng nhãn dán cạnh ô nhập tin nhắn để mở bảng chọn.
  - Tab **Tìm kiếm** tự động tìm nhãn dán ngay khi gõ từ khóa mà không cần bấm Enter.
  - Tab **Kho nhãn** cho phép duyệt và ghim các bộ nhãn dán từ máy chủ Zalo.
- **Tương tác trên tin nhắn:**
  - Nhấp chuột phải vào bong bóng tin nhắn để thả cảm xúc (reaction), trả lời trích dẫn, chuyển tiếp, sao chép nội dung, ghim, đánh dấu sao, thu hồi tin nhắn với mọi người hoặc xóa ở phía cá nhân.
  - **Nhấp đúp** vào một tin nhắn để trả lời nhanh. Trong nhóm, ô soạn tin tự chèn `@Tên` người gửi ở đầu (có thể xoá như tag thường). Nhấn `Esc` hoặc nút `✕` để thoát chế độ trả lời.
  - Rê chuột vào một tin nhắn để hiện 3 nút: **Trả lời**, **Chia sẻ** và **…** (Copy, Ghim, Đánh dấu, Chọn nhiều tin nhắn, Xem chi tiết, Tuỳ chọn khác, Thu hồi, Xóa chỉ ở phía tôi).
  - **Chia sẻ:** tìm hoặc lọc theo tab / **Phân loại**, tick nhiều hội thoại, thêm lời nhắn nếu muốn, bấm **Chia sẻ**.
  - **Chọn nhiều tin nhắn:** bấm vào từng tin để chọn, rồi dùng thanh phía trên ô soạn tin để Sao chép / Chia sẻ / Xóa; `Esc` hoặc **Hủy** để thoát.
  - Rê chuột vào cảm xúc dưới tin nhắn để xem tên người đã thả; bấm vào để mở hộp thoại **Biểu cảm** (lọc theo từng loại cảm xúc).
- **Nhắc tên (@mention) trong nhóm:** gõ `@` để mở danh sách thành viên, dùng `↑`/`↓` để chọn, `Tab`/`Enter` để chèn, `Esc` để đóng. Tag hiển thị màu xanh và người được tag sẽ nhận thông báo.
- **Tìm và gửi GIF:** gõ `@gif` để xem GIF xu hướng, hoặc `@gif <từ khoá>` (ví dụ `@gif hello`) để tìm. Dùng phím mũi tên để chọn, `Enter` để gửi, `Esc` để đóng; bấm chuột vào GIF cũng gửi ngay. Bật/tắt ở **Cài đặt → Tiện ích → Gợi ý @ → Tìm kiếm GIF**.
- **Ảnh và tệp:**
  - Dán ảnh trực tiếp từ clipboard (`Ctrl+V` trên Windows/Linux, `⌘V` trên macOS): ảnh chụp màn hình, ảnh copy từ trình duyệt/Preview, hoặc tệp ảnh copy trong Finder. Ảnh hiện ở khung xem trước, nhập chú thích rồi nhấn `Enter` để gửi.
  - **Kéo-thả tệp** vào cửa sổ chat: khi kéo, khung chat hiện hai vùng — thả vào vùng tin nhắn (**Gửi nhanh**) để gửi ngay, hoặc thả vào ô soạn tin (**Xem trước khi gửi**) để đính kèm, xem lại, nhập chú thích rồi nhấn `Enter`. Vùng đang trỏ chuột được tô sáng.

### 2.3. Gọi thoại và gọi video 1-1
- Trong cuộc trò chuyện cá nhân, bấm biểu tượng điện thoại hoặc máy quay ở góc phải thanh tiêu đề hội thoại để bắt đầu cuộc gọi.
- Thiết bị thu âm (Mic) và máy ảnh (Camera) có thể chọn trực tiếp trong cửa sổ cuộc gọi hoặc tại mục **Cài đặt -> Cài đặt chung**.

---

## 3. Giao diện dòng lệnh (TUI)

Khởi chạy:
```bash
./target/release/zalo-next --tui --qr
```

| Phím tắt | Chức năng |
|---|---|
| `↑` / `↓` hoặc `k` / `j` | Di chuyển giữa các cuộc trò chuyện trong danh sách bên trái |
| `Enter` | Gửi tin nhắn đang soạn hoặc chuyển con trỏ vào ô nhập |
| Gõ ký tự bất kỳ | Tự động đưa con trỏ vào ô soạn thảo |
| `Esc` | Rời ô soạn thảo, quay lại danh sách hội thoại |
| `Tab` | Chuyển đổi qua lại giữa danh sách hội thoại và ô nhập |
| `r` | Đồng bộ lại danh bạ và tin nhắn mới |
| `h` | Tải thêm lịch sử tin nhắn của nhóm đang chọn |
| `x` | Đăng xuất và xóa `session.json` |
| `l` | Mở bảng xem log mạng (khi chạy kèm cờ `--dev`) |
| `q` | Thoát ứng dụng |

---

## 4. Tiện ích kiểm thử dòng lệnh (`test_fetch`)

Binary phụ `test_fetch` dùng để kiểm tra nhanh kết nối API trên phiên đã lưu mà không cần mở giao diện:

```bash
# Kiểm tra phiên và in danh sách hội thoại
cargo run --release --bin test_fetch

# Gửi thử tin nhắn cá nhân
cargo run --release --bin test_fetch -- --send <USER_ID> "Nội dung tin nhắn"

# Gửi thử tin nhắn nhóm
cargo run --release --bin test_fetch -- --send-group <GROUP_ID> "Nội dung tin nhắn"
```

## Trợ lý AI (`@AI`)

1. Vào **Cài đặt → Trợ lý AI**, bật trợ lý, nhập **Base URL** tương thích OpenAI (ví dụ `https://api.example.com/v1`;
   `http://localhost:11434/v1` cho Ollama), **API key** và **Model**, rồi bấm **Kiểm tra kết nối**.
2. Trong bất kỳ cuộc trò chuyện nào gõ `@AI <câu hỏi>` rồi Enter. AI trả lời bằng một bong bóng riêng như người thứ ba;
   **chỉ mình bạn thấy**, không gửi gì tới Zalo và người kia không biết.
3. Chuột phải vào tin của AI để **Sao chép**, **Trả lời** (tiếp tục hỏi AI), **Tạo lại câu trả lời** hoặc **Xóa**
   (chỉ xóa trên máy này).
4. **Chia sẻ nội dung cuộc trò chuyện**: mặc định tắt, AI chỉ thấy câu hỏi của bạn. Bật ở Cài đặt (mặc định chung) hoặc
   ngay ở khung soạn tin khi gõ `@AI` (từng cuộc trò chuyện) để AI đọc văn bản các tin gần đây và tóm tắt/gợi ý tốt hơn.
   Nhóm chat luôn hỏi xác nhận lần đầu vì có tin nhắn của người khác. Liên kết, ảnh và tệp không bao giờ được gửi.
5. API key nằm trong tệp riêng tư `ai_secret.json` (không nằm trong `app_settings.json`). Biến môi trường
   `CTDOZ_BASE_URL`, `CTDOZ_API_KEY`, `CTDOZ_MODEL` luôn ưu tiên hơn cài đặt.
