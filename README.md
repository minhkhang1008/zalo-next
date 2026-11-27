# Zalo-next

**Zalo cho máy tính, viết bằng Rust: gọn, nhanh, riêng tư.**

Zalo-next là client Zalo desktop tự phát triển từ [ctdozlite](https://github.com/ctdoteam1110/ctdozlite),
tập trung vào **macOS**. Không Electron, không Chromium, không Node.js: một tệp thực thi duy nhất, ít RAM,
lịch sử tin nhắn lưu ngay trên máy.

[Tải bản mới nhất](https://github.com/minhkhang1008/zalo-next/releases/latest) ·
[Nhật ký thay đổi](CHANGELOG.md) · [Tính năng & giới hạn](docs/TINH_NANG.md) ·
[Xử lý sự cố](docs/KHAC_PHUC_SU_CO.md)

> [!NOTE]
> Dự án cá nhân, phi thương mại, không liên quan tới VNG hay Zalo. Bạn tự chịu trách nhiệm khi sử dụng.
> Zalo chỉ cho phép một phiên web cho mỗi tài khoản, nên đăng nhập Zalo-next sẽ đăng xuất Zalo Web.

---

# Dành cho người dùng

## Điểm nổi bật

| | |
|---|---|
| **Nhẹ và nhanh** | Ứng dụng gốc, không trình duyệt nhúng; chạy nền khi đóng cửa sổ, vẫn nhận tin và cuộc gọi. |
| **Trợ lý AI `@AI`** | Gõ `@AI <câu hỏi>` trong bất kỳ cuộc trò chuyện nào; câu trả lời **chỉ mình bạn thấy**, không gửi tới Zalo. Dùng AI của riêng bạn (gắn API tương thích OpenAI). |
| **Lịch sử không mất** | Tin nhắn lưu cục bộ bằng SQLite, giữ nguyên khi tắt app, đăng xuất hay đăng nhập lại; tìm được cả khi gõ không dấu. |
| **Nhắn tin đầy đủ** | Trả lời trích dẫn, reaction, tag `@`, chuyển tiếp, thu hồi, ghim, chọn nhiều tin, hẹn giờ gửi, bản nháp theo hội thoại. |
| **Sticker, Emoji, GIF** | Bảng chọn kiểu Zalo Web; gõ `@gif từ khoá` để tìm và gửi GIF. |
| **Ảnh & tệp** | Dán ảnh bằng `⌘V`, kéo-thả tệp, album ảnh dạng lưới, xem video ngay trong app. |
| **Gọi thoại & video** | Gọi 1-1, có cửa sổ cuộc gọi đến luôn nằm trên cùng. |
| **Giao diện tùy biến** | Sáng/tối/theo hệ thống, bảng màu, phông chữ, thu phóng, tốc độ hiệu ứng. |
| **Thông báo macOS** | Qua Notification Center, kèm avatar. |
| **Tự cập nhật** | Kiểm tra bản mới, xác minh SHA-256 rồi thay app tại chỗ. |

Danh sách đầy đủ kèm trạng thái từng tính năng: [Tính năng & giới hạn](docs/TINH_NANG.md).

## Cài đặt

**Cách 1: Homebrew**

```bash
brew tap minhkhang1008/tap
brew trust minhkhang1008/tap
brew install --cask zalo-next
```

**Cách 2: trình cài đặt một lệnh** (in rõ từng bước và hỏi trước khi cài)

```bash
curl -fsSL https://raw.githubusercontent.com/minhkhang1008/zalo-next/main/scripts/install-macos.sh | bash
```

Muốn đọc trước khi chạy: tải bằng `curl -fsSLO <cùng đường dẫn>`, mở ra xem rồi chạy `bash install-macos.sh`.

**Cách 3: tải `.dmg` thủ công**

Vào [Releases](https://github.com/minhkhang1008/zalo-next/releases), chọn tệp phù hợp:

| Máy | Tệp |
|---|---|
| Apple Silicon (M1–M4) | `zalo-next-<phiên bản>-macos-arm64.dmg` |
| Intel | `zalo-next-<phiên bản>-macos-x86_64.dmg` |
| Cả hai | `zalo-next-<phiên bản>-macos.dmg` |

Kéo app vào **Applications**. Lần đầu mở macOS sẽ chặn vì app chưa được Apple công chứng: vào
**System Settings → Privacy & Security → Open Anyway**, hoặc chạy
`xattr -dr com.apple.quarantine /Applications/Zalo-next.app`.

Windows và Linux có gói do CI dựng sẵn (`.exe`, `.deb`, `.rpm`, `.AppImage`, `.tar.gz`) nhưng chỉ được kiểm thử
thường xuyên trên macOS. Chi tiết: [Cài đặt & phát hành](docs/CAI_DAT.md).

## Bắt đầu sử dụng

1. Mở **Zalo-next**, quét mã QR bằng app Zalo trên điện thoại rồi xác nhận.
2. Lần sau app tự đăng nhập lại, không cần quét.
3. Muốn dùng trợ lý AI: **Cài đặt → Trợ lý AI**, nhập Base URL, API key, model rồi bấm **Kiểm tra kết nối**.

Cách dùng chi tiết (phím tắt, thao tác tin nhắn, cuộc gọi): [Hướng dẫn sử dụng](docs/HUONG_DAN_SU_DUNG.md).
Gặp lỗi: [Xử lý sự cố](docs/KHAC_PHUC_SU_CO.md).

## Quyền riêng tư

- Không mở cổng mạng cục bộ; REST API/MCP bị tắt có chủ đích.
- Token phiên chỉ được gửi tới tên miền của Zalo (`*.zalo.me`, `*.zaloapp.com`, `*.zadn.vn`, `*.zdn.vn`).
- `session.json` và cơ sở dữ liệu tin nhắn chỉ chủ máy đọc được (quyền `0600`).
- Không tự tải liên kết trong tin nhắn đến để tạo bản xem trước.
- Trợ lý AI mặc định **không** đọc nội dung cuộc trò chuyện; bạn bật riêng từng cuộc. API key lưu trong tệp riêng tư, không nằm trong cài đặt thường.

Dữ liệu nằm ở `~/Library/Application Support/ctdozlite` (giữ tên cũ để không mất đăng nhập khi đổi tên app).
**Không chia sẻ `session.json` cho bất kỳ ai.**

**Gỡ cài đặt:** kéo `Zalo-next.app` vào Thùng rác; muốn xóa cả dữ liệu thì xóa thêm thư mục trên.

## Vì sao tin tưởng được bản cài đặt

Zalo-next chưa được Apple công chứng (cần tài khoản 99 USD/năm), nên thay vì yêu cầu tắt bảo vệ của macOS,
mọi bước đều kiểm chứng được:

- **Mã nguồn mở:** cả ứng dụng lẫn [trình cài đặt](scripts/install-macos.sh) nằm trong repo này.
- **Build công khai:** tệp cài đặt do [GitHub Actions](https://github.com/minhkhang1008/zalo-next/actions) dựng từ mã nguồn.
- **Kiểm tra nguồn gốc:** mỗi bản phát hành có `SHA256SUMS.txt` và chứng nhận nguồn gốc của GitHub:
  `gh attestation verify zalo-next-<phiên bản>-macos-arm64.dmg --repo minhkhang1008/zalo-next`.
- **Trình cài đặt chỉ làm 5 việc:** tải `.dmg` từ Releases → đối chiếu SHA-256 → kiểm tra chữ ký → chép vào
  Applications → gỡ cờ "tải từ Internet" cho riêng app này. Không dịch vụ nền, không sửa cài đặt hệ thống,
  không tắt Gatekeeper, không gửi dữ liệu đi đâu.

---

# Dành cho người phát triển

## Công nghệ

| Lớp | Thành phần |
|---|---|
| Ngôn ngữ | Rust 1.97+, edition 2021 |
| Giao diện | `egui` / `eframe` (GUI), `ratatui` (TUI) |
| Mạng | Tokio worker: WebSocket `zpw_ws`, HTTP polling, `reqwest` |
| Lưu trữ | SQLite (`zalo_history.db`), `threads_cache.json` |
| Mật mã | `ParamCipher v2`, AES-CBC/GCM, chữ ký MD5 `signkey` |
| Cuộc gọi | VoIP signalling, `cpal` + `opus`, giải mã video bằng `ffmpeg` |

Kiến trúc hai tầng: giao diện chỉ vẽ và gửi `WorkerCmd`; mọi tác vụ mạng, đĩa và mật mã chạy trong worker và
trả về `WorkerEvent`/`GuiEvent`. Sơ đồ đầy đủ: [Kiến trúc hệ thống](docs/KIEN_TRUC_HE_THONG.md).

## Bản đồ mã nguồn

| Đường dẫn | Vai trò |
|---|---|
| `src/main.rs`, `src/lib.rs` | Điểm vào CLI, chọn GUI/TUI; thư viện dùng chung `zalo_rust` |
| `src/gui.rs`, `src/gui/` | Giao diện egui: workspace, tin nhắn, modal, sticker picker, giao diện, trợ lý AI |
| `src/gui/worker.rs` | Tokio runtime; ranh giới `WorkerCmd` / `WorkerEvent` |
| `src/chat.rs`, `src/socket.rs` | Gửi/nhận tin, WebSocket, polling |
| `src/session.rs`, `src/login.rs`, `src/qr.rs` | Phiên, đăng nhập QR, ký và gắn cookie cho request |
| `src/param_cipher.rs`, `src/crypto.rs` | `ParamCipher v2`, AES, `signkey` |
| `src/store.rs`, `src/history_sync.rs` | SQLite; đồng bộ lịch sử Android (`.db.crypt`) và iOS (Sync V2) |
| `src/call.rs`, `src/audio.rs`, `src/media.rs` | Cuộc gọi, âm thanh, ảnh/tệp/sticker/voice |
| `src/ai.rs` | Trợ lý `@AI` (client OpenAI-compatible tách biệt khỏi mạng Zalo) |
| `src/updater.rs` | Tự cập nhật, chỉ từ Releases của repo này |
| `src/endpoints.rs` | Bảng host dịch vụ Zalo; host mới phải khai báo ở đây |

## Build và chạy

```bash
git clone https://github.com/minhkhang1008/zalo-next.git && cd zalo-next
export CMAKE_POLICY_VERSION_MINIMUM=3.5     # nếu dùng CMake 4.x
cargo run --release -- --gui                # chạy thử
cargo test --locked                          # kiểm thử
bash packaging/package-local.sh              # đóng gói .dmg vào dist/
```

Cần Xcode Command Line Tools và CMake. Helper Swift thông báo nằm ở `native/macos/`; nếu thiếu, build lại bằng
`native/macos/build-native-helpers.sh`.

## Tham số dòng lệnh

| Lệnh | Tác dụng |
|---|---|
| `zalo-next --gui` | Giao diện đồ hoạ (mặc định) |
| `zalo-next --tui --qr` | Giao diện terminal, đăng nhập bằng QR |
| `zalo-next --data-dir <thư mục>` | Thư mục dữ liệu riêng (mỗi tài khoản một thư mục); `--profile <tên>` tạo thư mục con |
| `zalo-next --dev` | Dev Inspector: xem log mạng khi gỡ lỗi |
| `zalo-next --print-data-dir` | In thư mục dữ liệu rồi thoát |
| `zalo-next --check-update` | Kiểm tra bản mới trên GitHub rồi thoát |
| `zalo-next --version` | In phiên bản |

Danh sách đầy đủ: [Hướng dẫn sử dụng](docs/HUONG_DAN_SU_DUNG.md#1-tham-số-dòng-lệnh-cli-flags).

## Nguyên tắc bắt buộc

- Không commit bí mật: `session.json`, `*.db`, cookie, `zpw_sek`, `zpw_enk`, IMEI, bản ghi lưu lượng.
- Không chạy I/O, SQLite nặng hay mạng trên luồng vẽ UI; luôn đi qua worker.
- Host Zalo mới khai báo trong `src/endpoints.rs`. Request tới Zalo đi qua đường chuẩn: params → mã hoá (`ParamCipher v2` trước đăng nhập, `zpw_enk` sau đăng nhập) → `signkey` khi endpoint yêu cầu → cookie phiên.
- Chỉ format tệp đã sửa (`rustfmt`), không chạy `cargo fmt` cho cả crate; không thêm cảnh báo clippy mới.
- Kết nối mang token phiên phải qua `is_trusted_zalo_url`; auto-updater chỉ trỏ về repo này.
- Đổi hành vi giao thức (params, chữ ký, frame ID, cipher) cần bằng chứng trong [`re-notes/`](re-notes/) cùng test hoặc ghi chú kiểm chứng thủ công.
- Chú thích mã bằng tiếng Anh, chuỗi hiển thị cho người dùng bằng tiếng Việt.

## Tài liệu kỹ thuật

| Tài liệu | Nội dung |
|---|---|
| [AGENTS.md](AGENTS.md) | Hướng dẫn làm việc cho người và AI agent trong repo |
| [Kiến trúc hệ thống](docs/KIEN_TRUC_HE_THONG.md) | Luồng worker, module GUI, lưu trữ |
| [Giao thức Zalo](docs/GIAO_THUC_ZALO.md), [`re-notes/`](re-notes/) | Ghi chú phân tích giao thức và bằng chứng |
| [Trợ lý AI](docs/AI_TRO_LY.md), [ADR](docs/adr/README.md) | Thiết kế trợ lý `@AI` và các quyết định kiến trúc |
| [Quy chuẩn](docs/standards/README.md) | Code, đồng thời, SQLite, bảo mật, giao thức, test, git, tài liệu |
| [CLAUDE.md](CLAUDE.md), [ADR-0006](docs/adr/0006-multi-agent-workflow.md) | Claude Code điều phối, Codex triển khai và review chéo |
| [Mục lục tài liệu](docs/README.md) | Toàn bộ tài liệu trong `docs/` |
| [Thuật ngữ](GLOSSARY.md) | Ngôn ngữ chung cho issue, ADR, test và mã |
| [SESSION_SECURITY.md](SESSION_SECURITY.md) | Bảo vệ `session.json` |

Đóng góp: theo [quy chuẩn git](docs/standards/git-and-issues.md), giữ commit nhỏ, thêm mục vào
[CHANGELOG](CHANGELOG.md) cho mọi thay đổi người dùng thấy được.

## Ghi công & giấy phép

Xây dựng trên [ctdozlite](https://github.com/ctdoteam1110/ctdozlite) của nhóm ctdoteam1110 và các cộng tác viên.
Phát hành theo giấy phép **MIT hoặc Apache-2.0**, tùy bạn chọn
([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).

Định dạng giao thức được tìm hiểu qua phân tích client Zalo công khai để viết lại độc lập bằng Rust; repo
không phân phối lại mã nguồn hay tài nguyên của Zalo.
