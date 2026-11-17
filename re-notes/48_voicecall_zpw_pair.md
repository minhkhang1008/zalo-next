# 48 - `voicecall` endpoints must use the PC `zpw_ver=693&zpw_type=24` pair

Date: 2026-11-17. Verified by the owner on a real phone (incoming video call), no packet capture.

## Symptom
Zalo mobile, when calling Zalo-next, showed that the callee's Zalo version "does not support video
calls". Voice calls worked, and outgoing calls from Zalo-next worked.

## Cause (inferred from the experiment)
`call.rs::voicecall_get` sent `zpw_ver=694&zpw_type=30` (the pair used by the rest of the API,
`type 30` = web) to `voicecall-wpa.chat.zalo.me/api/voicecall/*` (ringring, answer, conf, ...). The
official Zalo PC sends `693` / `24` on those endpoints (see `re-notes/07_voip_log.md` and the old
`ZCALL_ZPW` knob). The server evidently derives the callee's capabilities from that pair and relays
"no video" to the caller.

## Change
`voicecall_zpw()` defaults to `zpw_ver=693&zpw_type=24` for the voicecall endpoints only (the chat
API, login and the control long-poll keep `694` / `30`). `ZCALL_ZPW=ver:type` still overrides it.

## Result
With `ZCALL_ZPW=693:24` the phone accepted the video call. NOT yet fixed (tracked in
`tasks/video-call-quality.md`): Zalo-next does not display the phone's camera, and the phone sees
Zalo-next's video blurry and delayed.
