# 51 - Zalo emoticons (text shortcuts) and contact cards

Date: 2026-11-28. Source: Zalo Web static bundles fetched without cookies (no login), plus the shape (keys
only, values not read) of `chat.recommended` messages in the owner's local cache. Display only: no request,
signing or frame changes.

## Emoticons
Zalo draws text shortcuts such as `:)`, `:-((`, `/-strong`, `_()_` (and `:name:` shortcodes) as its own emoji
art; the message text on the wire stays the shortcut (copying gives back `:)`).

- Table: `default-embed-render.2e39a6ff54b2ad98e7b1.js`, module `tDit` = `emojiRC`, 2735 keys → `"col row"`
  (1-based) in the sprite; module `owK0` = `emojiMapPos` gives the same cells as CSS percentages. 1440 keys are
  ASCII (145 shortcuts incl. upper-case variants, the rest `:name:`); the rest are Unicode emoji (not used by
  Zalo-next, which keeps the system emoji font for those). `src/zalo_emoticon_table.rs` is generated from
  the ASCII keys, lower-cased (1385 entries), zero-based.
- Sprite: `emojiImg: "assets/emoji.1e7786c93c8a0c1773f165e2de2fd129.png?v=20180604"`, served at
  `https://zalo-chat-static.zadn.vn/v1/assets/emoji.1e7786c93c8a0c1773f165e2de2fd129.png`, 3264 × 2624 px =
  51 × 41 tiles of 64 px, SHA-256 `39e30cb3dda96560788ff4af83bdfa48d0d550dd307b017e5bb435c1eb72c6dc`
  (checked: tile (41, 7) is the `:)` face). Bubble CSS: `background-size: 5100%`.
- Matching, module `xp5P` (`parse(e, t=-1, n, s)`): a trie over the map keys, input lower-cased per char;
  at each position take the longest key; a match counts only when the character before it is a space, a
  newline, the start of the text or the end of another match, and the same holds after it. (A macOS-only
  rule there disables shortcuts of length ≤ 2 in some web input contexts; Zalo-next follows the PC/mobile
  rendering and does not apply it.)

## Contact cards
`msgType "chat.recommended"`, `content.action = "recommened.user"` (Zalo's spelling): `title` = display name,
`thumb` = avatar URL (Zalo host), `params` = the contact's uid, `description` = JSON string
`{phone, qrCodeUrl, gUid}` (phone may be absent). Zalo Web's card (`bubble-contact-card`) shows avatar, name,
phone and Call / Message actions. `recommened.vip` is the OA variant (same fields). Other `recommened.*`
actions (`link`, `calltime`, `misscall`) are unrelated.

## Not verified
Live rendering against real chats.
