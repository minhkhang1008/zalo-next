# 55 - Conversation mute / unmute (`setmute` action codes)

- Endpoint (unchanged): `POST {profile}/api/social/profile/setmute`, command 11288, AES `params`
  `{toid, duration, action, startTime, muteType (1 user / 2 group), imei}`.
- Zalo Web bundle (`web-bundle/lazy-1.c72c0957b455ab70b73c.js`, `setMuteConversation(e,t,n,a,s=-1)`)
  passes `action` from its constants module (`web-bundle/default-embed-render.js`, exports
  `SET_MUTE` → `Ht=1`, `UNSET_MUTE` → `qt=3`). The unlock timer calls
  `muteConversation(id, UNSET_MUTE, isGroup)` with the default `duration = -1`.
- Bug (2026-11-28): we sent `action: 2` to unmute. The server ignored it; the `getmute` snapshot
  fetched right after still listed the thread, so the GUI muted it again ("Mở lại thông báo" did nothing).
- Fix: unmute sends `action: 3` (`chat::mute_action`, unit test `setmute_uses_zalo_web_action_codes`).
  Not yet verified against the live server.
