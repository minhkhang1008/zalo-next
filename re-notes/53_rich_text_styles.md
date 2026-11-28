# 53 - Zalo rich text ("Định dạng tin nhắn", textProperties)

Date: 2026-11-28. Source: Zalo Web static bundle `lazy/2.21dd5856eb8f1d31f964.js` (no login). Extends the
existing support in `chat.rs` (`extract_zalo_rtf_text_properties`, `format_incoming_zalo_rtf`), which only knew
`b,i,u,s,f_18,lst_1,lst_2` and showed incoming styles by turning them into Markdown.

## Wire format
Text messages (and quotes, mentions) carry `textProperties`:
`{"styles":[{"start":N,"len":M,"st":"b,i,..."}],"ver":0}`; `start`/`len` are JavaScript string offsets
(UTF-16 code units) into the plain `message` text.

Style tokens (`normalizeFormat` / `denormalizeFormat`, and the newer editor's `toWire/fromWire`):
- `b` bold, `i` italic, `u` underline, `s` strikethrough.
- `f_<px>` font size; sizes `SMALL 13`, `MEDIUM 15` (default, never sent), `LARGE 18`, `GIANT 20`.
- `c_<rrggbb>` text colour; the toolbar offers red `db342e`, orange `f27806`, yellow `f7b503`,
  green `15a85f` (module `ZOtC`); `000000` / `050a19` mean "default" and are never sent.
- `lst_1` bullet list, `lst_2` numbered list (applied to whole lines).
- `ind_<n>` indent in px, `n = level * 10`, level 1..5 (whole lines).
- The toolbar (`STR_FORMAT_TEXT`, composer button "chatbar-rtf") offers: bold, italic, underline,
  strikethrough, font size, colour, bullet / numbered list, increase / decrease indent, clear formatting.
