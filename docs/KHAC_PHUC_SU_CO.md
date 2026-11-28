# Xử lý sự cố

Khi gặp lỗi, chạy lại với `--dev` để xem log mạng trong Dev Inspector, mã lỗi thật thường nằm ở đó.

## Đăng nhập & phiên

**Bị đòi quét QR lại / báo lỗi `600` hoặc `-102`.** Phiên đã hết hạn hoặc bị thay bởi một lần đăng nhập
web khác (Zalo chỉ giữ một phiên web cho mỗi tài khoản). Đăng nhập lại bằng QR; lịch sử tin nhắn vẫn còn.

**Mở lại app phải đăng nhập lại dù chưa đăng xuất.** Kiểm tra bạn có đang mở app với một thư mục dữ
liệu khác lần trước không (trên macOS mọi cách chạy dùng `~/Library/Application Support/ctdozlite`, trên Windows là `%LOCALAPPDATA%\ctdozlite`, trừ khi bạn
truyền `--data-dir`/`--profile`; dữ liệu cũ trong thư mục làm việc được chuyển sang và để lại bản `*.bak`) và thư mục đó có quyền ghi không.

**Chạy hai tài khoản bị đăng xuất lẫn nhau.** Mỗi tài khoản cần một thư mục riêng:
`zalo-next --data-dir ~/.zalo-next-taikhoan2`.

**Đồng bộ từ điện thoại không nhận tin.** Kiểm tra đã chọn đúng **Android** hoặc **iPhone** trong Cài đặt và đã xác nhận trên điện thoại. Mỗi lần chỉ đồng bộ được một phần lịch sử 1-1. Chạy với `--dev` để xem lỗi ở Dev Inspector, rồi gửi kèm mã lỗi (đừng gửi `session.json`).

**Không thấy tin nhắn cũ.** Lịch sử nằm trong thư mục dữ liệu đang dùng. Nếu trước đó bạn chạy với thư
mục khác, nhập lại bằng **Cài đặt → Đồng bộ tin nhắn → Nhập lịch sử** và chọn tệp `zalo_history.db`
cũ (việc nhập chỉ gộp thêm, không xoá gì).

## macOS

**"Zalo-next.app không thể mở được".** App chưa được Apple công chứng: chuột phải → **Open**, hoặc chạy
`xattr -dr com.apple.quarantine /Applications/Zalo-next.app`.

**Không có thông báo tin nhắn mới.** Vào **System Settings → Notifications** và cho phép Zalo-next (hoặc
DailyNotifier) gửi thông báo.

## Biên dịch & hiển thị

**CMake báo `Compatibility with CMake < 3.5 has been removed`** khi build `audiopus_sys`/`opus-sys`:
đặt `export CMAKE_POLICY_VERSION_MINIMUM=3.5` trước `cargo build`.

**Thiếu font tiếng Việt trên Linux tối giản:** cài `fonts-noto-core` hoặc `fonts-dejavu-core`.

**Gọi video không có hình:** kiểm tra **Cài đặt → Cài đặt chung → Công cụ gọi video**. Nếu FFmpeg chưa sẵn sàng, bấm cài (app tự tải bản đã kiểm SHA-256). Kiểm tra thêm quyền Camera/Micro của Zalo-next trong **System Settings → Privacy & Security**. App chưa hiển thị được camera của điện thoại, và hình gửi đi còn mờ hoặc trễ. Đây là giới hạn đã biết.

**Cài FFmpeg báo sai hash hoặc chạy thử thất bại:** app giữ nguyên bản cũ và không kích hoạt bản lỗi. Thử lại khi mạng ổn định; nếu vẫn lỗi, cài bằng `brew install ffmpeg` (app tự nhận đường dẫn Homebrew).

**Điện thoại báo "chưa hỗ trợ gọi video":** cập nhật lên v0.4.0 trở lên (đã sửa cặp tham số `zpw_ver`/`zpw_type`).

**Không thấy cuộc gọi đến:** từ v0.5.0 cuộc gọi đến hiện thành một ô nhỏ ở góc trên bên phải màn hình (luôn nằm trên, không lấy focus) với Nghe máy / Nghe nhưng tắt mic / Không nghe máy; cửa sổ cuộc gọi chỉ mở sau khi nghe máy. Cuộc gọi video đến được trả lời với camera tắt nếu chưa bật **Cho phép gọi video** trong Cài đặt.

**Thông báo mất avatar / nền trong suốt khi chạy `cargo run`:** thiếu helper Swift đã dựng sẵn; chạy `bash native/macos/build-native-helpers.sh`.

**`@AI` báo lỗi kết nối:** kiểm tra Base URL (chỉ `https`, hoặc `http` tới `localhost`/`127.0.0.1`/`::1`), API key và model ở **Cài đặt → Trợ lý AI**, rồi bấm **Kiểm tra kết nối**. Mã `401` là sai key, `429` là bị giới hạn tốc độ hoặc hết hạn mức.

## Mã lỗi thường gặp từ máy chủ Zalo

| Mã | Ý nghĩa |
|---|---|
| `600` / `-102` | `zpw_sek` thiếu hoặc hết hạn → đăng nhập lại |
| `18012` | Đã quét QR nhưng chưa bấm xác nhận trên điện thoại |
| `18060` | Tham số chưa được mã hoá đúng (`ParamCipher v2`) |
| `18022` | Sai chữ ký `signkey` |
| `114` | Tham số không hợp lệ |
| `429` | Gọi API quá dày, chờ một lúc rồi thử lại |
