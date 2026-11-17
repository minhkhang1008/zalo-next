# 47 - Sync V2: call logs and `cmd 591` dispose reason

Date: 2026-11-17. Owner exception recorded in `AGENTS.md` (Sync V2 only, no fresh capture).
Status: **unverified against a real phone** - offline fixtures only.

## Source of evidence

Upstream ctdozlite v0.1.2 (`src/history_sync.rs`: `parse_proto_metadata`, `format_sync_v2_call_text`,
`is_sync_v2_call_attach`; `src/socket.rs`: `send_sync_ack_delete`). Upstream cites Zalo PC's `app.asar`
`DisposeReason` enum for the ack values. No capture was taken for this change.

## 1. Call logs arrive as plain text without their attachment

A Sync V2 message (`parse_message`) carries `metadata` in field 8. Before this change field 8 was ignored,
so a phone-synced call log was stored as `webchat` text such as `0:18` or an empty string and the GUI could
not tell it from chat. Now field 8 is decoded: `metadata.field 1` = attachments, each with
`attach_type(1) action(3) params(4) title(5) description(8)` (strings). Quotes (`field 4`) and the other
attachment fields (ext info, href, thumb, ids) are not decoded yet.

A message is a call log when an attachment has `action` in `recommened.{calltime,misscall,calling,receivecall}`
(the upstream spelling `recommened` and `recommended` are both accepted), or `title == "sendBubbleMessage"`,
or a `description` starting with `Cuộc gọi` while the content is empty or a colon time (`m:ss`, `h:mm:ss`).

It is stored as `msg_type = chat.ecall`, `text` = canonical call text (`📞 Cuộc gọi thoại đến (74s)`,
`📹 Cuộc gọi video đi (0s) — Bạn đã hủy`, …) and `raw_json = {"call_bubble":true,"is_video":…,"duration":…}`.
The non-empty `raw_json` makes `MessageStore::upsert_message` rewrite an existing row, so **re-running the sync
upgrades call logs imported earlier as plain text**. The GUI call-card parser (`extract_call_card_view`) reads
the canonical text.

Direction: `params.isCaller` (bool / 0-1 / "true"), else the action (`calling` = caller, `receivecall` = callee),
else the sender. Duration: `params.duration|dur`, else the colon time in the content. Missed = action contains
`misscall`, duration 0, or no duration with empty content. `params.reason` picks the wording
(1 busy, 2 no answer, 3 declined, otherwise cancelled / missed).

## 2. `cmd 591` ack_delete reason is an integer

`ACK_DELETE_SYNC_SESSION` is sent when a session ends. Our build sent strings (`SYNC_FULL_COMPLETED`,
`BATCH_TIMEOUT`, `PC_USER_CANCELS_SYNC`). Upstream reports the server wants the integer enum and that a string
leaves the phone stuck on "Đang đồng bộ tin nhắn…":

| Value | Meaning | Used when |
| :-- | :-- | :-- |
| 0 | user cancel | `PHONE_SYNC_CANCEL` set |
| 1 | success | every pending batch imported |
| 2 | error | timeout or failure (replaces `BATCH_TIMEOUT`) |
| 3 | outdated | not emitted yet |

## Verification

Offline: `history_sync::tests::{attach_metadata_…, call_logs_become_ecall_rows_…, call_text_covers_…}`.
To confirm on a phone: run a sync from the phone, check call logs render as cards after re-sync and that the
phone leaves "Đang đồng bộ" when the PC session ends.
