# Giao thức Zalo Web/PC trong Zalo-next

Tóm tắt những gì Zalo-next đang dùng. Bằng chứng gốc (capture, trích bundle, ngày quan sát) nằm trong
[`re-notes/`](../re-notes/). Quy trình khi **sửa** giao thức: [`standards/protocol.md`](standards/protocol.md).

---

## 1. Tham số chung

| Tham số | Giá trị | Nguồn |
|---|---|---|
| `zpw_ver` / `zpw_type` | `694` / `30` (Zalo Web) | `ZPW_VER`, `ZPW_TYPE` trong `src/session.rs` |
| Ngoại lệ `voicecall` | `693` / `24` (cặp của Zalo PC) để điện thoại nhận cuộc gọi video | `call.rs::voicecall_zpw`, `re-notes/48`; ghi đè bằng biến `ZCALL_ZPW` |

Request sau đăng nhập gắn cookie `zpw_sek` và header `Origin: https://chat.zalo.me`.

## 2. Máy chủ

Host dịch vụ lấy theo `zpw_service_map_v3` của phiên web và được ghi lại trong `src/endpoints.rs`
(`SERVICE_MAP_V3`). Phiên web/QR dùng các host `*.chat.zalo.me`; bảng mặc định cũ của Zalo PC
(`*.zaloapp.com`) không còn đúng với loại phiên này.

| Khoá | Host |
|---|---|
| `profile`, `friend` | `tt-profile-wpa.chat.zalo.me`, `tt-friend-wpa.chat.zalo.me` |
| `conversation`, `chat`, `group` | `tt-convers-wpa.chat.zalo.me`, `tt-chat3-wpa.chat.zalo.me`, `tt-group-wpa.chat.zalo.me` |
| `sticker` (cả GIF qua Giphy) | `tt-sticker-wpa.chat.zalo.me` |
| `label`, `reaction`, `alias`, `emoji_status` | `label-wpa…`, `reaction…`, `tt-alias-wpa…`, `emoji-status…` (`.chat.zalo.me`) |
| `file`, `zcloud` | `tt-files-wpa.chat.zalo.me`, `zcld.chat.zalo.me` |
| `voice_call` | `voicecall-wpa.chat.zalo.me` |
| Đăng nhập | `wpa.zaloapp.com` (`AUTH`), `wpa.chat.zalo.me` (`WPA_CHAT`) |
| Board ghim | `groupboard-wpa.chat.zalo.me`, `friendboard-wpa.chat.zalo.me`, `board-wpa.chat.zalo.me` |

Media nhận được tải từ CDN `dlmd.me` / `dlfl.vn` bằng client **không cookie** (`is_trusted_zalo_media_url`).

## 3. Chữ ký `signkey`

Dùng cho các lệnh đăng nhập và khởi tạo:

```text
signkey = md5("zsecure" + tag + concat(params[k] for k in sorted(keys)))
```

`tag` là tên lệnh viết thường, bỏ gạch ngang (`reqqr`, `getlogininfo`, `getserverinfo`, …). Tham số phụ
(`ref`, `nretry`) được nối vào URL sau khi tính `signkey`. Sai chữ ký thì máy chủ trả `18022`.

## 4. Mã hoá tham số

**Trước đăng nhập: `ParamCipher v2`** (`src/param_cipher.rs`)
1. Khoá tĩnh `PARAM_CIPHER_KEY`; `zcid` = AES-256-CBC(`"{type},{imei},{firstLaunchTime}"`, IV 0), dạng hex in hoa.
2. `zcid_ext`: 10 ký tự hex ngẫu nhiên.
3. `encrypt_key` (32 ký tự) dẫn xuất từ các vị trí chẵn/lẻ của `MD5(zcid_ext)` và `zcid`.
4. JSON payload → AES-256-CBC (`encrypt_key`, IV 0, PKCS7) → Base64 vào `params`. Sai thì máy chủ trả `18060`.

**Sau đăng nhập: `zpw_enk`**: khoá nhận từ `getLoginInfo`, dùng AES-128-CBC (IV 0, PKCS7) để mã hoá
`params` và giải mã trường `data` của phản hồi.

WebSocket dùng thêm AES-GCM cho payload frame (`socket.rs`).

## 5. WebSocket `zpw_ws`

| Mã lệnh | Ý nghĩa |
|---|---|
| `1` | Xác thực (`CMD_AUTHEN`) |
| `2`, `4` | Ping / giữ kết nối |
| `501`, `521` | Đẩy tin 1-1 / nhóm |
| `510`, `511`, `513`–`518` | Kéo tin bù |
| `533`, `534` | Báo có tin / tin bị lỡ |
| `601`–`604` | Điều khiển (bao gồm kéo sự kiện VoIP `604`) |

Danh sách đầy đủ là các hằng `CMD_*` trong `src/socket.rs`.

## 6. Phiên

- **`zpw_sek`**: cookie phiên nhận sau khi xác nhận QR. Chỉ được gửi tới host vượt qua `is_trusted_zalo_url`.
- **`zpw_enk`**: khoá dữ liệu phiên (xem mục 4).
- Phiên hết hạn hoặc bị thay bởi một phiên web khác thì máy chủ trả `600` / `-102`. Phải đăng nhập lại, nhưng
  lịch sử vẫn giữ nguyên. Mỗi tài khoản Zalo chỉ có một phiên web cùng lúc.

Bảng mã lỗi đầy đủ: [`KHAC_PHUC_SU_CO.md`](KHAC_PHUC_SU_CO.md).

## 7. Mục lục `re-notes/`

| Ghi chú | Chủ đề |
|---|---|
| `01_login`, `capture` | Giao thức đăng nhập Zalo PC, cách bắt request |
| `02_chat_data`, `04_session_fix`, `05_message_store` | Luồng dữ liệu hội thoại, sửa phiên, kho SQLite |
| `06_call_media`, `07_voip_log`, `08_video_call`, `48_voicecall_zpw_pair` | Cuộc gọi thoại/video |
| `09_feed_and_zalo_video` | Feed và Zalo Video (giao diện Feed đã gỡ) |
| `35_exact_message_identity` | Định danh tin (`cliMsgId` và `ts`) cho reaction/trích dẫn |
| `37_group_posting_restriction`, `38_group_mentions` | Quyền đăng trong nhóm, tag `@` |
| `39_gif_search_send`, `40_conversation_labels` | GIF, nhãn hội thoại (chỉ đọc) |
| `42`–`45`, `47_sync_v2_call_logs` | Đồng bộ lịch sử: điểm vào, vòng đời Sync V2, Android, nhật ký cuộc gọi |
| `46_inline_video_playback` | Phát video đính kèm trong app |
| `50_group_poll`, `51_zalo_emoticons_and_contact_cards` | Bình chọn nhóm, biểu tượng cảm xúc Zalo, danh thiếp |
| `52_bank_cards`, `54_received_bank_cards` | Số tài khoản ngân hàng: gửi/quản lý, thẻ nhận được |
| `53_rich_text_styles` | Định dạng tin nhắn (`textProperties`) |
| `55_mute_action` | Tắt/mở lại thông báo hội thoại (`setmute`, mã action) |
| `03_progress_audit` | Đánh giá tiến độ cũ (lịch sử) |

Ghi chú mới lấy số kế tiếp (`56_…`). Không dùng lại số cũ.
