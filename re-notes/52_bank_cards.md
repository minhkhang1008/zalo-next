# 52 - Saved bank accounts and "send bank card" (Gửi số tài khoản)

Date: 2026-11-28. Source: Zalo Web static bundles fetched without cookies (no login):
`lazy/2.21dd5856eb8f1d31f964.js` (API calls) and `lazy/web-startup.69ed8a04988794a39905.js` (service
`SEND_BANK_CARD_MODULE_KEY`, list mapping, validation). No capture yet; sending and editing live is left to the
owner.

## Host
`getZInstantMsgDomain()` = service map `zinstant_msg` (constant `ZINSTANT_MSG`), default
`https://zimsg.chat.zalo.me`. All calls carry `_getCommonParams()` and `encodeAES` params (post-login
`zpw_enk`), like the other chat APIs.

## Endpoints (cmd ids in parentheses)
- `GET /api/transfer/conf` (15309): bank catalogue. Response `banks[]`: `bin`, `logo`, `name`, `name_eng`,
  `short_name`, `search_key_word`.
- `GET /api/transfer/list?…&params=enc({page, limit})` (15298), `page >= 0`, `limit >= 1` (Zalo uses 10;
  `pagingLimit` server config). Response `data` is AES; decrypted: `{myBanks[], total, hasMore}` with
  `myBanks[]`: `id`, `account_id`, `bank_logo`, `bank_name`, `bank_number`, `bin`, `holder_name`,
  `is_default`, `default`, `is_public`, `public`, `created_at`, `updated_at`.
- `POST /api/transfer/create` body `params=enc({bin, bank_number, holder_name, language})` (15306) →
  `{account_id, bank_number, bin, holder_name, is_default, created_at, updated_at}`.
- `POST /api/transfer/update` `{account_id (number), bin, bank_number, holder_name, language}` (15307).
- `POST /api/transfer/delete` `{account_id (number), is_default, language}` (15308). Client rule: the default
  account cannot be deleted while others exist.
- `POST /api/transfer/card` `{binBank, numAccBank, nameAccBank, cliMsgId, tsMsg, destUid, destType}` (15220):
  sends the bank card into a conversation; the server creates the message. `cliMsgId` and `tsMsg` are both
  `getTimeNow()` (ms). `destType` 0 = user, 1 = group; for a group `destUid` is the conversation id without
  its leading `g`. Success: HTTP 200 and `error_code == 0`.
- `language` = current UI language name (`vi`).

## Validation (client side, Zalo Web)
- bank number: `^[A-Za-z0-9]{1,19}$`
- holder name: `^(?=.{5,50}$)[A-Za-z0-9]+(?: [A-Za-z0-9]+)*$`, upper-cased while typing (no diacritics).

## Received card
Rendered by Zalo as a zinstant (`chat.webcontent`) message laid out by the server; the exact fields of a
received bank card are NOT known yet (no sample). Pending: inspect one on Zalo Web.

## VietQR
Zalo-next draws the QR locally from bin + account number (NAPAS VietQR / EMVCo static payload:
`00 02 01`, `01 02 11`, `38` { `00` `A000000727`, `01` { `00` bin, `01` account }, `02` `QRIBFTTA` },
`53 03 704`, `58 02 VN`, `63 04` CRC-16/CCITT-FALSE over the payload including `6304`). No network.
