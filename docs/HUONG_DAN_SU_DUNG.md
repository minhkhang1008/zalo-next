# Hướng dẫn sử dụng Zalo-next

Tùy chọn dòng lệnh, cách dùng giao diện đồ hoạ (GUI) và phím tắt của giao diện terminal (TUI). Trạng thái từng tính năng: [TINH_NANG.md](TINH_NANG.md). Gặp lỗi: [KHAC_PHUC_SU_CO.md](KHAC_PHUC_SU_CO.md).

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
| `--profile <TÊN>` | Dùng thư mục con `profiles/<tên>` cho một tài khoản riêng (chỉ chữ, số, `-`, `_`). | `zalo-next --profile acc2` |
| `--check-update` | Kiểm tra bản mới trên GitHub Releases rồi thoát. | `zalo-next --check-update` |
| `--version`, `-V` | In phiên bản rồi thoát. | `zalo-next --version` |
| `--proxy <URL>` | Đi qua proxy HTTP(S) khi kết nối mạng. | `zalo-next --proxy http://127.0.0.1:8080` |
| `--logout` | Xóa phiên đăng nhập hiện tại (`session.json`) rồi thoát. | `zalo-next --logout` |
| `--phone <SỐ>` | Đăng nhập bằng mật khẩu (chỉ TUI/CLI; giao diện đồ hoạ chỉ có QR). | `zalo-next --tui --phone 0912345678` |
| `--pass <MK>` | Điền sẵn mật khẩu. | `zalo-next --tui --phone 0912345678 --pass ...` |
| `--dump`, `-d` | Lưu các gói phản hồi JSON ra thư mục `dump/` để phân tích (có thể chứa dữ liệu riêng tư, không chia sẻ). | `zalo-next --dump` |
| `--dump-raw` | Như `--dump` nhưng giữ dữ liệu thô chưa giải mã. | `zalo-next --dump-raw` |

---

Trên macOS, mọi cách chạy (bản `.app`, `cargo run`, TUI, CLI) đều dùng chung `~/Library/Application Support/ctdozlite`, kể cả khi thư mục làm việc là `/`, nên GUI và TUI luôn thấy cùng một phiên đăng nhập và cơ sở dữ liệu. Trên Windows, mọi cách chạy dùng chung `%LOCALAPPDATA%\ctdozlite` (nếu thiếu biến `LOCALAPPDATA` thì dùng thư mục làm việc). Trên Linux, dữ liệu nằm ở `$XDG_DATA_HOME/ctdozlite` (mặc định `~/.local/share/ctdozlite`); lần mở đầu tiên tự sao chép dữ liệu cũ từ thư mục làm việc sang (giữ bản gốc `*.bak`), kể cả thư mục `profiles/<tên>` khi chạy với `--profile` (chưa kiểm thử trên máy Linux thật). `--data-dir` ghi đè lựa chọn này (`--profile <tên>` tạo thư mục con `profiles/<tên>`); thư mục chỉ định và thư mục mặc định trên macOS có quyền `0700` trên Unix. Nếu thư mục không ghi được, chương trình báo lỗi trước khi mở dữ liệu.

`--version` không đụng tới dữ liệu. `--print-data-dir` chỉ tạo thư mục khi cần và thử ghi, không chuyển dữ liệu cũ.

**Tự chuyển dữ liệu cũ (macOS, Windows):** ở lần chạy đầu sau khi đổi quy tắc, nếu thư mục dữ liệu mới chưa có phiên/lịch sử nhưng thư mục làm việc có `session.json`, `zalo_history.db` hoặc `accounts/`, chương trình sao chép (quyền `0600`/`0700`) các tệp của Zalo-next (`session.json`, `zalo_history.db` kèm `-wal`/`-shm`, `accounts/`, `threads_cache.json`, `app_settings.json`, `theme.json`, `custom_stickers.json`, `autoreply_rules.json`, `active_account`, khóa Sync V2), cùng thư mục `profiles/`, kiểm tra kích thước rồi mới đưa tất cả vào thư mục mới một lượt; chỉ khi xong hết mới đổi tên bản gốc thành `*.bak` (không xóa). Lỗi giữa chừng thì hoàn tác, bản gốc giữ nguyên và lần chạy sau thử lại. Trên Windows, chương trình tìm dữ liệu cũ ở thư mục làm việc và thư mục chứa `zalo-next.exe` (ưu tiên nơi có `session.json`); nếu còn một cửa sổ Zalo-next khác đang chạy hoặc cơ sở dữ liệu cũ đang bị giữ, lần chạy này dùng tạm thư mục cũ và lần sau mới chuyển. Không bao giờ ghi đè tệp đã có ở thư mục mới, không đụng tới tệp khác, và không chạy khi dùng `--data-dir` (với `--profile` trên Windows, chỉ chuyển đúng hồ sơ đó từ `profiles/<tên>` cũ). Muốn quay lại, đổi tên `*.bak` về tên cũ và chạy với `--data-dir .`.

## 2. Giao diện đồ họa (GUI)

### 2.1. Đăng nhập
1. Chạy `./target/release/zalo-next --gui`.
2. Mở ứng dụng Zalo trên điện thoại, quét mã QR hiển thị trên màn hình và bấm **Đăng nhập**.
3. Sau khi xác nhận, phiên làm việc được lưu vào `session.json`. Ở những lần mở tiếp theo, ứng dụng tự nạp lại phiên này mà không cần quét lại mã QR.
4. Mã QR có hạn khoảng 3 phút; app hiện đếm ngược và tự lấy mã mới khi hết hạn. Nếu điện thoại báo
   **"không có kết nối với máy chủ"** khi quét: thoát hẳn Zalo trên điện thoại rồi mở lại, bấm
   **Tạo / Làm mới mã QR**, kiểm tra mạng (tắt VPN) rồi quét lại. Hướng dẫn này cũng có ngay dưới mã QR.

### 2.2. Nhắn tin & Gợi ý Sticker tự động
- **Tìm kiếm và lọc hội thoại:** Sử dụng ô tìm kiếm (`Esc` để xóa và thoát ô tìm kiếm) phía trên danh sách bên trái (hỗ trợ tìm tiếng Việt không dấu) hoặc lọc theo các tab *Tất cả*, *Cá nhân*, *Nhóm*.
- **Gợi ý nhãn dán khi đang gõ:**
  - Khi nhập từ khóa hoặc cụm từ ngắn trong ô chat (`xin chào`, `cảm ơn`, `ok`, `haha`, `buồn`, `khóc`, `nhớ`, `cố lên`, `mệt quá`, `đi chơi`, `trà sữa`...), ứng dụng tự động truy vấn API sticker của Zalo và hiển thị dải gợi ý phía trên khung soạn thảo.
  - Bấm trực tiếp vào sticker để gửi, hoặc dùng phím mũi tên `↑` để chọn lên thanh gợi ý, `←` / `→` để di chuyển, `Enter` để gửi và `Esc` để đóng.
  - Bấm biểu tượng kính lúp ở góc phải thanh gợi ý để mở toàn bộ kết quả tìm kiếm của từ khóa đó trong bảng Sticker Picker.
- **Bảng Sticker / Emoji / GIF:**
  - Bấm biểu tượng nhãn dán cạnh ô nhập tin nhắn để mở bảng nổi phía trên khung soạn thảo, kiểu Zalo Web, với ba tab **STICKER**, **EMOJI**, **GIF**.
  - **STICKER:** ô tìm kiếm tự tìm khi gõ (không cần Enter), mục *Gần đây* và phổ biến, duyệt từng bộ qua thanh bộ nhãn phía dưới, nút **+** mở kho để thêm bộ mới. Bấm vào nhãn dán để gửi.
  - **EMOJI:** bấm để chèn vào vị trí con trỏ, bảng vẫn mở để chèn tiếp (không tự gửi).
  - **GIF:** xem xu hướng hoặc tìm theo từ khóa, bấm vào GIF để gửi.
  - `Esc` hoặc bấm ra ngoài để đóng bảng. Mục *Gần đây* được ghi nhớ giữa các lần mở app.
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
  - Nhiều ảnh gửi cùng lúc hiển thị thành **album dạng lưới** như Zalo. Trong khung xem ảnh, nút sao chép là nút chia đôi: bấm để sao chép theo mặc định đã chọn (ảnh hoặc liên kết), bấm mũi tên để đổi.
  - **Xem lần lượt các ảnh:** khung xem ảnh chứa mọi ảnh đã tải của cuộc trò chuyện (ảnh trong album được tách riêng từng tấm). Dùng `↑`/`←` để xem ảnh cũ hơn, `↓`/`→` để xem ảnh mới hơn, hoặc bấm nút ▲/▼ và ảnh nhỏ ở dải bên phải (chia theo ngày: Hôm nay, Hôm qua, ngày/tháng). Lăn chuột trên ảnh để phóng to; mỗi khi chuyển ảnh, mức phóng to trở về vừa khung. Nút ở thanh trên cùng ẩn/hiện dải ảnh nhỏ.
  - Dán ảnh trực tiếp từ clipboard (`Ctrl+V` trên Windows/Linux, `⌘V` trên macOS): ảnh chụp màn hình, ảnh copy từ trình duyệt/Preview, hoặc tệp ảnh copy trong Finder. Ảnh hiện ở khung xem trước, nhập chú thích rồi nhấn `Enter` để gửi.
  - Ảnh được gửi ở chất lượng gốc (HD) theo mặc định; đổi tại **Cài đặt → Tin nhắn → Gửi ảnh chất lượng gốc (HD)**.
  - **Kéo-thả tệp** vào cửa sổ chat: khi kéo, khung chat hiện hai vùng — thả vào vùng tin nhắn (**Gửi nhanh**) để gửi ngay, hoặc thả vào ô soạn tin (**Xem trước khi gửi**) để đính kèm, xem lại, nhập chú thích rồi nhấn `Enter`. Vùng đang trỏ chuột được tô sáng. Trên Linux không đọc được vị trí chuột khi kéo nên tệp luôn được đính kèm để xem trước.

### 2.3. Gọi thoại và gọi video 1-1
- Trong cuộc trò chuyện cá nhân, bấm biểu tượng điện thoại hoặc máy quay ở góc phải thanh tiêu đề hội thoại để bắt đầu cuộc gọi.
- Khi có cuộc gọi đến, cửa sổ cuộc gọi hiện **luôn nằm trên cùng** kèm thông báo hệ thống "Cuộc gọi đến"; bấm **Nghe** để trả lời. Hiện app chưa hiển thị được camera của điện thoại, và hình Zalo-next gửi đi còn mờ (xem `tasks/video-call-quality.md`).
- Thiết bị thu âm (Mic) và máy ảnh (Camera) có thể chọn ngay trong cửa sổ cuộc gọi hoặc tại **Cài đặt → Cài đặt chung**.
- **Công cụ gọi video (FFmpeg):** FFmpeg dùng để thu camera, mã hoá/giải mã hình và phát video trong app. Nếu bấm **Gọi Video** hoặc **Bật Camera** khi máy chưa có FFmpeg, app tự mở Cài đặt chung và bắt đầu chuẩn bị (có tiến độ, nút dừng và thử lại); cũng có thể cài trước ở **Cài đặt → Cài đặt chung → Công cụ gọi video**. Cài xong là dùng được ngay, không cần khởi động lại.
  - macOS: tải FFmpeg 9.0.2 bản tĩnh vào thư mục dữ liệu của app (không cần Homebrew hay quyền quản trị), kiểm tra SHA-256 và chạy thử rồi mới dùng; không đụng tới FFmpeg của hệ thống. App vẫn nhận FFmpeg cài bằng Homebrew, kể cả khi mở app từ Finder.
  - Windows: cài qua WinGet (`Gyan.FFmpeg`). Linux (Debian/Ubuntu): `pkexec apt-get install ffmpeg`. Hai nền tảng này chưa được kiểm thử.

### 2.4. Tiện ích hằng ngày
- **Bản nháp:** mỗi hội thoại giữ bản nháp riêng, gồm cả trích dẫn và tag. Danh sách hội thoại hiện chữ *Nháp*. Bản nháp chỉ lưu trên máy và vẫn còn khi tắt app, đăng xuất hay đổi tài khoản.
- **Đã lưu:** menu tin nhắn → **Đánh dấu sao**. Mở **Đã lưu** ở thanh công cụ dưới cùng để thêm ghi chú, bỏ lưu hoặc nhảy tới đúng tin cũ.
- **Nhắc hẹn riêng:** menu tin → **Nhắc hẹn** → 15 phút / 1 giờ / ngày mai. Đến hạn app hiện thông báo; tính năng này không gửi gì vào hội thoại. App cần đang mở và đăng nhập; nhắc đã đến hạn trong lúc tắt app sẽ hiện khi mở lại.
- **Hẹn giờ gửi:** nút `🕒` cạnh ô soạn tin (khác với Nhắc hẹn).
- **Tin nhắn nhanh:** tạo ở **Cài đặt → Tin nhắn**; trong ô soạn gõ `/`, chọn bằng `↑`/`↓` rồi `Enter` để chèn nội dung và ảnh.
- **Tìm kiếm nâng cao** (chỉ trong lịch sử đã lưu trên máy, tối đa 200 kết quả mới nhất): ví dụ `"họp tuần" from:Minh has:file after:2026-11-01`. Hỗ trợ `from:`, `in:`, `has:image|file|voice|link`, `before:`, `after:`, `is:saved`/`is:starred`, `is:pinned`, cụm từ trong ngoặc kép và `#tag`. Bấm vào kết quả để nhảy tới tin, kể cả tin chưa nằm trong phần đang hiển thị.
- **Xem trước tài liệu:** bấm **Xem trước** trên tệp PDF/DOCX/XLSX/PPTX/TXT/CSV/MD/LOG/RTF. App chỉ trích văn bản (khoảng 20.000 ký tự, 50 hàng mỗi sheet), không dựng lại trang. PDF dạng ảnh scan cần OCR bên ngoài. Tệp chỉ được tải khi bạn bấm, tối đa 10 MiB, và không lưu ra đĩa.
- **Ghi âm:** chọn hội thoại → **Ghi âm** ở thanh dưới cùng, tối đa 5 phút. **Dừng và gửi** để gửi, **Hủy** để bỏ. Hội thoại nhận được cố định từ lúc bắt đầu ghi; bản ghi bị huỷ nếu bạn đổi tài khoản hoặc có cuộc gọi. App không tự gửi lại khi lỗi, để tránh gửi trùng.
- **Markdown:** khối mã có nút sao chép, bảng, danh sách. Tắt định dạng hoặc đổi kiểu `*một sao*` ở **Cài đặt → Tin nhắn**.

### 2.5. Đồng bộ tin nhắn từ điện thoại
1. Lần đầu, chọn **Android** hoặc **iPhone** ở **Cài đặt → Đồng bộ tin nhắn** (lựa chọn được lưu theo thư mục dữ liệu và có thể đổi lại).
2. Bấm đồng bộ rồi xác nhận yêu cầu trên điện thoại. Android dùng luồng sao lưu gốc; iPhone dùng Sync V2. Runtime Sync V2 được tải từ máy chủ của Zalo khi cần và được kiểm SHA-256, nên không cần cài Zalo PC.
3. Chỉ đồng bộ được một phần lịch sử 1-1 (xem [TINH_NANG.md](TINH_NANG.md)). Nhật ký cuộc gọi hiện thành thẻ cuộc gọi.
4. Muốn chuyển lịch sử từ máy hoặc thư mục khác: **Cài đặt → Đồng bộ tin nhắn → Nhập lịch sử**, chọn tệp `.db`/`.json`. Việc nhập không xoá gì; tin trùng có thể được cập nhật theo bản nhập.

### 2.6. Trợ lý AI (`@AI`)

1. Vào **Cài đặt → Trợ lý AI**, bật trợ lý, nhập **Base URL** tương thích OpenAI (ví dụ `https://api.example.com/v1`;
   `http://localhost:11434/v1` cho Ollama), **API key** và **Model**, rồi bấm **Kiểm tra kết nối**.
2. Trong bất kỳ cuộc trò chuyện nào gõ `@AI <câu hỏi>` rồi Enter. AI trả lời bằng một bong bóng riêng như người thứ ba;
   **chỉ mình bạn thấy**, không gửi gì tới Zalo và người kia không biết.
3. Chuột phải vào tin của AI để **Sao chép**, **Trả lời** (tiếp tục hỏi AI), **Tạo lại câu trả lời** hoặc **Xóa**
   (chỉ xóa trên máy này).
4. **Chia sẻ nội dung cuộc trò chuyện**: mặc định tắt, AI chỉ thấy câu hỏi của bạn. Bật ở Cài đặt (mặc định chung) hoặc
   ngay ở khung soạn tin khi gõ `@AI` (từng cuộc trò chuyện). Khi bật, AI đọc các tin gần đây kèm liên kết, tên người gửi,
   tin được trả lời, ảnh và tệp, và tự tra cứu lịch sử cũ hơn của **chính cuộc trò chuyện đó** (ví dụ `@AI tìm link
   bảng tính hôm qua`). Nhóm chat luôn hỏi xác nhận lần đầu vì có tin nhắn của người khác.
   - Trả lời (Reply) một tin Zalo rồi gõ `@AI …`: AI đọc tin đó, kể cả ảnh hoặc tệp của nó.
   - Dán ảnh hoặc kéo tệp vào khung soạn tin rồi gõ `@AI …`: ảnh/tệp gửi cho AI (không gửi lên Zalo), kể cả khi tắt chia sẻ.
   - Chỉ ảnh/tệp trên máy chủ Zalo hoặc trên máy bạn mới được tải; PDF, Word, Excel, PowerPoint, văn bản và mã nguồn được
     đọc chữ. Model không nhận ảnh hoặc không hỗ trợ tra cứu thì app tự hỏi lại không kèm phần đó và ghi chú dưới câu trả lời.
5. API key nằm trong tệp riêng tư `ai_secret.json` (không nằm trong `app_settings.json`). Biến môi trường
   `CTDOZ_BASE_URL`, `CTDOZ_API_KEY`, `CTDOZ_MODEL` luôn ưu tiên hơn cài đặt.
6. **Đọc trước ngữ cảnh (tùy chọn, mặc định tắt):** ở Cài đặt → Trợ lý AI, đồng ý cho AI đọc, chọn các cuộc trò chuyện
   rồi bật công tắc. AI sẽ tóm tắt trước ở nền và giữ bản tóm tắt **chỉ trong bộ nhớ** (không lưu ra đĩa) để trả lời
   `@AI` nhanh hơn. Xem [AI_TRO_LY.md](AI_TRO_LY.md) mục 11.

### 2.7. Giao diện, chạy nền và cập nhật
- **Cài đặt → Giao diện:** chọn Sáng / Tối / Hệ thống, bảng màu gợi ý, màu chủ đạo, màu điểm nhấn và bong bóng tin nhắn tùy chỉnh (có xem trước), phông chữ, thu phóng, bật/tắt và tốc độ hiệu ứng; **Khôi phục mặc định** để quay lại ban đầu.
- **Chạy nền:** bấm nút đóng cửa sổ chỉ ẩn app, vẫn nhận tin nhắn, thông báo và cuộc gọi. Mở lại từ icon ở menu bar; thoát hẳn bằng menu đó hoặc `Ctrl+Q`.
- **Cập nhật:** app tự kiểm tra mỗi 30 phút; hoặc vào **Cài đặt → Cập nhật** để kiểm tra thủ công.

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
