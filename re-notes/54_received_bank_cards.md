# 54 - Received bank cards (zinstant.bankcard)

Date: 2026-11-28. Source: one real bank card in the owner's chat with Bo, observed on chat.zalo.me with the
owner's permission (message object read from the page, card data fetched from Zalo's static CDN without
cookies). Values below are redacted/synthetic except public bank data.

## Message
`msgType 52`, `originMsgType "chat.webcontent"`, `content.action = "zinstant.bankcard"`, preview text
`params.customMsg.msg.vi = "[Tài khoản ngân hàng]"`. `content.params` (JSON string) holds `item`, `pcItem`,
`bubbleItem`, each `{zinstantdata_id: "templateId=11845", content_time, checksum, data_type, version,
data_url}`. `pcItem.data_url` = `https://zinst-stc.zadn.vn/static/3m/11845/<date>/<token>?data=html`
(`item`/`bubbleItem` give a gzip binary layout of the same card). No bank fields are in the message itself.

## Card data (`pcItem.data_url`, HTML, ~20 KB)
- `<p id="bank_bin">546034</p>` — bank BIN.
- `#vw_card .bank_info p.normal_text` — bank display name ("Ngân hàng CAKE by VPBank").
- `#vw_card p.number_text` — account number.
- Logo / background: `https://res-zalo.zadn.vn/upload/media/.../<bin>_LOGO_....png`, `..._BG_....png`.
- QR image: `https://group-qr.zdn.vn/d/...` (Zalo-rendered QR).
- `https://bankcard-action.zalo.me?action=transfer&content=<VietQR payload>&...` and `action=save&content=...`:
  the `content` parameter is the full VietQR payload Zalo encodes in its QR, e.g.
  `000201 010211 38 54 { 00 10 A000000727, 01 24 { 00 06 <bin>, 01 10 <account> }, 02 08 QRIBFTTA }
  53 03 704 58 02 VN 80 30 { 00 12 com.vng.zalo, 01 10 ZABANKCARD } 63 04 <crc>` (Zalo adds field 80).
- Holder name: not present in this card (the template's script reads it from a node that may be empty).

## Zalo-next
Fetch `pcItem.data_url` only from `zinst-stc.zadn.vn` (Zalo host), no cookies, bounded; parse the fields above;
show a native card (logo if the URL is a Zalo host, bank name, number); QR drawn locally from the `content`
payload (falls back to building one from bin + number). Right-click: copy number / show VietQR; clicking the QR
enlarges it. No transfer button.
