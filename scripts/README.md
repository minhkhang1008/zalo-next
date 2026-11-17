# Developer & Research Scripts

Thư mục này chứa các kịch bản hỗ trợ kiểm thử giao thức và phân tích lưu lượng mạng:

- `grab.py`: Script trích xuất và lọc các gói tin Zalo quan trọng từ file ghi lưu lượng `zalo.flow` (mitmproxy).
- `test_pc_status.py`: Script kiểm tra nhanh trạng thái phản hồi của endpoint xác nhận QR `pc-status`.
- `release_notes.py`: Sinh ghi chú GitHub Release ngắn từ `CHANGELOG.md` (CI gọi); tự bọc `@tên` trong dấu backtick để GitHub không gắn nhầm người dùng. Test: `python3 -m unittest tests/release_notes_test.py`.
- `install-macos.sh`: Trình cài đặt một lệnh cho macOS (kiểm tra SHA-256, hỏi trước khi cài).
- `build-app-icon.sh`: Dựng icon ứng dụng từ `Logo.png`.
- `decode_voip_log.py`, `sync_v2_helper.js`, `zproto_lib.js`: Công cụ giải mã log VoIP và hỗ trợ Sync V2.
