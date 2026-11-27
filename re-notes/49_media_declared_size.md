# 49 - Declared media size in `content.params` (photo / video placeholders)

Date: 2026-11-28. Source: Zalo Web static bundle, no capture, no login.

## Evidence
`https://zalo-chat-static.zadn.vn/v1/lazy/default-embed-render.2e39a6ff54b2ad98e7b1.js` (fetched
without cookies), function `_estimateSize(e, t = -1, n = 0)`:

```js
let t = JSON.parse(e.message.params), n = -1, c = -1;
switch (e.msgType) {
  case MSG_TEXT: if (t.subType != MSG_SUBTYPE_PHOTO && t.subType != MSG_SUBTYPE_GIF) break;
  case MSG_PHOTO: case MSG_PHOTO_2: case MSG_DOODLE: case MSG_GIF:
    n = e.width ? e.width : t.width; c = e.height ? e.height : t.height; break;
  case MSG_VIDEO: n = t.video_width; c = t.video_height;
}
// no size -> 368x368 box flagged `defaultSize`; else clamp to min 160x110, max 798x368
... return { height, width, defaultSize, rotation: t.rotation, ... }
```

So Zalo Web lays out the bubble before the image loads from `params.width`/`params.height`
(photos, GIFs, doodles, photo-subtype text) and `params.video_width`/`params.video_height`
(videos). The same fields appear in our existing fixture (`chat.rs` test with
`"params": "{\"width\":1280,\"height\":960}"`). Upstream ctdozlite PR #192 reads the same fields
(reference only).

## What Zalo-next does
- Parsing reads those fields into `ChatMessage::media_size` (each side 1..=20000, else ignored);
  photos we send ourselves take the size of the local file.
- The size is kept in the message's `raw_json` metadata blob in SQLite, so there is no schema
  change; rows saved before this change simply have no size and keep the old fixed placeholder.
- The placeholder uses the declared aspect ratio. Once the thumbnail is decoded, the declared size
  is used only when its aspect ratio is within 3% of the real one (rotation or a wrong value
  falls back to the thumbnail).
- No request, signing or frame changes: this only reads fields that already arrive.

## Not verified
Live rendering against real chats (owner's GUI run). `rotation` is not used yet.
