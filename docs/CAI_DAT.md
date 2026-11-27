# Cài đặt & phát hành

## Cài bản dựng sẵn (macOS)

**Homebrew:**

```bash
brew tap minhkhang1008/tap
brew trust minhkhang1008/tap     # Homebrew yêu cầu bạn xác nhận tin tap bên thứ ba
brew install --cask zalo-next
```

Homebrew bắt buộc xác nhận tin cậy (`brew trust`) với mọi tap ngoài kho chính thức. Tap
[`minhkhang1008/homebrew-tap`](https://github.com/minhkhang1008/homebrew-tap) chỉ có một tệp cask: tải
`.dmg` từ GitHub Releases (Homebrew đối chiếu SHA-256), cài vào Applications và gỡ cờ cách ly cho app.

**Hoặc một lệnh, không bị Gatekeeper chặn** (script hỏi trước khi cài; muốn đọc trước thì tải về bằng
`curl -fsSLO <đường dẫn bên dưới>` rồi mở ra xem):

```bash
curl -fsSL https://raw.githubusercontent.com/minhkhang1008/zalo-next/main/scripts/install-macos.sh | bash
```

Script (`scripts/install-macos.sh`) tải bản mới nhất đúng chip, đối chiếu SHA-256 với
`SHA256SUMS.txt`, chép `Zalo-next.app` vào `/Applications`, gỡ cờ cách ly rồi mở app. Gatekeeper chỉ
chặn tệp có cờ `com.apple.quarantine` mà trình duyệt gắn vào; `curl` không gắn cờ này. Tuỳ chọn:
`CTDOZ_VERSION=0.2.1` (cài bản cụ thể), `CTDOZ_APP_DIR=~/Applications`, `CTDOZ_YES=1` (không hỏi),
`CTDOZ_NO_OPEN=1`. Kiểm tra nguồn gốc tệp: `gh attestation verify <tệp> --repo minhkhang1008/zalo-next`
(xem mục "An toàn & minh bạch" trong README).

Vì sao không ký "thật": muốn macOS mở app tải bằng trình duyệt mà không hỏi, app phải được ký
Developer ID **và** công chứng với Apple — cần Apple Developer Program (99 USD/năm). Từ macOS 15, mẹo
chuột phải → Open cũng không còn tác dụng với app chưa công chứng. Các bản cập nhật tải trong app
không mang cờ cách ly nên không bị chặn; chỉ lần cài đầu cần cách ở trên.

**Hoặc tải `.dmg` thủ công:**

Tải tại [Releases](https://github.com/minhkhang1008/zalo-next/releases):

| Máy | Tệp |
|---|---|
| Apple Silicon (M1–M4) | `zalo-next-<phiên bản>-macos-arm64.dmg` |
| Intel | `zalo-next-<phiên bản>-macos-x86_64.dmg` |
| Chạy được trên cả hai | `zalo-next-<phiên bản>-macos.dmg` |

Mở `.dmg`, kéo `Zalo-next.app` vào **Applications**. App chỉ được ký tạm (ad-hoc), chưa được Apple
công chứng, nên lần đầu mở macOS sẽ chặn. Vào **System Settings → Privacy & Security → Open Anyway**,
hoặc chạy:

```bash
xattr -dr com.apple.quarantine /Applications/Zalo-next.app
```

Muốn kiểm tra tệp tải về còn nguyên vẹn: tải kèm `SHA256SUMS.txt`, đặt cùng thư mục rồi chạy
`shasum -a 256 -c SHA256SUMS.txt --ignore-missing`.

Mỗi bản Release cũng có gói Windows (`.exe`) và Linux (`.deb`, `.rpm`, `.AppImage`, `.tar.gz`) do CI
dựng tự động, nhưng tôi chỉ kiểm thử thường xuyên trên macOS.

Sau khi cài, app tự kiểm tra bản mới khi mở và mỗi 30 phút. Khi có bản mới, bấm **Cập nhật ngay**:
app tải `.dmg`, kiểm tra SHA-256 và chữ ký, thay chính nó rồi tự mở lại. Nếu app không nằm ở thư mục
ghi được, app sẽ mở `.dmg` để bạn cài tay.

Gỡ cài đặt: xoá `Zalo-next.app`; muốn xoá cả dữ liệu thì xoá thêm
`~/Library/Application Support/ctdozlite`.

## Tự build từ mã nguồn

Cần: Rust 1.97+ (do `wasmtime` 50), Xcode Command Line Tools (`xcode-select --install`) và CMake (`brew install cmake`).

```bash
git clone https://github.com/minhkhang1008/zalo-next.git && cd zalo-next
export CMAKE_POLICY_VERSION_MINIMUM=3.5    # bắt buộc với CMake 4.x
cargo run --release -- --gui
```

Đóng gói `.dmg` trên máy (đúng kiến trúc của máy đang dùng; số phiên bản đọc từ `Cargo.toml`):

```bash
bash packaging/package-local.sh            # → dist/zalo-next-<phiên bản>-macos-<arch>.dmg
```

`.dmg` này đã kèm helper thông báo DailyNotifier và icon. Trên Windows dùng
`pwsh -File packaging\package-local.ps1`.

## Phát hành phiên bản mới

CI (`.github/workflows/ci-release.yml`) chỉ chạy test (~2 phút) khi push vào `main`, và bỏ qua hoàn
toàn nếu commit chỉ sửa tài liệu. Khi bạn đẩy một tag `v*`, CI dựng gói cho macOS/Windows/Linux
(~10–25 phút) rồi tự tạo bản Release; app đang cài sẽ tự nhận bản mới.

1. Nâng `version` trong `Cargo.toml` (ví dụ `0.2.0`), chạy `cargo check` để cập nhật `Cargo.lock`.
2. Trong `CHANGELOG.md`, đổi `## [Unreleased]` thành `## [0.2.0] - <ngày>` và thêm một mục
   `## [Unreleased]` trống ở trên.
3. Commit, gắn tag rồi đẩy lên. Tag phải trùng phiên bản trong `Cargo.toml` và nằm trên `main`,
   nếu không CI sẽ dừng:

   ```bash
   git commit -am "chore(release): v0.2.0"
   git push origin main
   git tag -a v0.2.0 -m "Zalo-next v0.2.0"
   git push origin v0.2.0
   ```

4. Theo dõi tiến trình ở tab **Actions** (hoặc `gh run watch`). Khi xong, bản Release có đủ các gói
   và `SHA256SUMS.txt`.

Có thể chạy lại thủ công: **Actions → CI & Cross-Platform Release → Run workflow**, bật `publish`.

## Công cụ gọi video (FFmpeg)

FFmpeg không nằm trong gói cài đặt. App chỉ tải về khi bạn cần (xem
[Hướng dẫn sử dụng §2.3](HUONG_DAN_SU_DUNG.md#23-gọi-thoại-và-gọi-video-1-1)). Trên macOS, app tải trực tiếp từ
các nhà phân phối được [ffmpeg.org](https://ffmpeg.org/download.html) dẫn link (cho phép chuyển hướng tới mirror
`e.deolaha.ca`), rồi đối chiếu với hash đã ghim
sẵn trong mã nguồn (`src/video_tools.rs`):

| Kiến trúc | Nguồn | SHA-256 của tệp `ffmpeg` sau giải nén |
|---|---|---|
| Apple Silicon | [OSXExperts](https://www.osxexperts.net/), `ffmpeg902arm.zip` | `6deb2860c3dc0c08e0f96bab119196c024103046fa3cc7ef72e3a77797d8553c` |
| Intel | [Evermeet](https://evermeet.cx/ffmpeg/), `ffmpeg-9.0.2.zip` | `a45b462cf91ed89148ae218c4577e30896485d7a6792c3673bcf5f823fa01b63` |

Giới hạn: tệp ZIP tối đa 128 MiB, sau giải nén tối đa 192 MiB. App chỉ đọc mục `ffmpeg` và không dùng tên đường
dẫn trong ZIP để ghi tệp; tệp chạy được đặt quyền `0700`. Nếu hash sai hoặc chạy thử thất bại, app không kích
hoạt bản đó. Trình tải dùng HTTP client riêng, không mang cookie hay token Zalo.
