<task>
Implement Zalo's official message formatting ("Định dạng tin nhắn") in both directions. Read
re-notes/53_rich_text_styles.md (style tokens, sizes, colours, offsets in UTF-16 units) and the existing code:
src/chat.rs `extract_zalo_rtf_text_properties` (Markdown → textProperties when sending) and
`format_incoming_zalo_rtf` (incoming textProperties → Markdown text, lossy: drops colour, size, indent).

Part A — show received formatting natively:
1. Parsing: keep the plain message text and store the styles: `ChatMessage.text_styles: Vec<TextStyle { start:
   usize, end: usize /* byte offsets into text */, bold, italic, underline, strike, size: Option<u8>,
   color: Option<[u8;3]>, list: Option<ListKind{Bullet,Numbered}>, indent: u8 }>` (or an equivalent compact
   form), converted from UTF-16 offsets safely (out-of-range spans dropped, no panics, cap 500 spans). Stop
   converting such messages to Markdown; keep `format_incoming_zalo_rtf` only where another caller needs it.
   Persist `text_styles` in the raw_json metadata like `media_size` (src/store.rs both directions) and merge in
   `merge_message_metadata`. Only accept sizes 13/15/18/20 (others → None) and colours as 6 hex digits.
2. Rendering (src/gui/messages.rs): when a message has `text_styles`, build its LayoutJob from the spans instead
   of the Markdown path: bold (use whatever the Markdown renderer uses for bold), italic, underline,
   strikethrough, font size relative to the bubble's base size (13→-2, 18→+3, 20→+5 px around the 15 px
   default), colour (in dark mode, keep the colour but ensure contrast if the renderer has a helper; otherwise use
   as is), lists (prefix each listed line with "• " or "1. ", "2. " … counting consecutive numbered lines),
   indent (leading 16 px per level via `leading_space`). Links and @mentions inside a formatted message must stay
   clickable/styled as today where the existing code allows it; emoji and Zalo emoticons must still be replaced
   (pass the job through the same emoji/emoticon step). Copy keeps the plain text.

Part B — compose with formatting:
3. Composer state (src/gui/workspace.rs + a new src/gui/rich_composer.rs): `composer_styles: Vec<TextStyle>`
   over `self.input_message` (byte offsets). Keep spans valid when the text changes: diff old/new text by common
   prefix/suffix and shift/trim spans; text typed at the end of a styled span inherits it (Zalo behaviour) while
   a "pending style" toggled with no selection applies to the next typed text. Clear styles when the composer is
   cleared/sent or the thread changes.
4. Toolbar: a composer button "Định dạng" (tooltip "Định dạng tin nhắn (Cmd/Ctrl+Shift+X)") toggles a toolbar
   row above the input with: B, I, U, S, size menu (Nhỏ 13 / Thường 15 / Lớn 18 / Rất lớn 20), colour menu
   (Mặc định, Đỏ db342e, Cam f27806, Vàng f7b503, Xanh lá 15a85f), bullet list, numbered list, indent +, indent −,
   "Xóa định dạng". Buttons apply to the current selection (read the TextEdit's cursor range from egui's
   TextEditState for the composer id); list/indent apply to the whole lines touched. Show active states for the
   caret position. Shortcuts Cmd/Ctrl+B, +I, +U work in the composer when the toolbar is on.
5. Rendering the composer: extend the existing composer layouter (`composer_mention_layout_job`) so styles show
   while typing (mentions still blue).
6. Sending (src/chat.rs and the send path): when `composer_styles` is non-empty, send the plain text with
   `textProperties` built from the spans (UTF-16 offsets, tokens exactly as re-notes/53, omit defaults, merge
   adjacent identical spans), and skip the Markdown conversion for that message. Otherwise keep today's Markdown
   path. Quotes and mentions keep working (the existing payload builders already accept textProperties).
   Drafts: store styles with the draft if the draft model has a JSON field you can extend without a schema
   migration; otherwise drop styles from drafts and note it.
</task>
<scope>
May edit: src/chat.rs, src/store.rs, src/gui/messages.rs, src/gui/workspace.rs, src/gui/rich_composer.rs (new),
src/gui.rs, src/productivity.rs (drafts, only without schema change)
Must not touch: Cargo.*, docs/, re-notes/, CHANGELOG.md, .agents/WORKLOG.md
</scope>
<constraints>
Follow AGENTS.md (§4.5: only the style tokens in re-notes/53; §4.9 no unwrap on wire data; §4.10 offline tests;
UI strings Vietnamese, comments English). Do not commit. rustfmt only the new file. No new clippy warnings in code
you add. Do not regress the Markdown path or existing tests.
</constraints>
<verification_loop>
Run: CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo check --message-format=short
Run: CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo test --locked --lib chat
Run: CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo test --locked --lib messages
Run: CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo test --locked --lib rich_composer
Tests: UTF-16 ↔ byte conversion with Vietnamese and emoji; all tokens parsed and re-emitted; defaults omitted;
span maintenance on insert/delete/replace; textProperties built from composer spans; incoming lists/indent
prefixes; raw_json round trip. If a check fails, fix and re-run.
</verification_loop>
<completeness_contract>
Done means received formatted messages render with bold/italic/underline/strike/size/colour/lists/indent, the
composer has the formatting toolbar and sends Zalo textProperties, tests pass, cargo check clean. If the TextEdit
selection cannot be read or something else blocks, stop and say what is missing.
</completeness_contract>
<compact_output_contract>
Reply with: summary (≤10 lines), files touched, commands run with pass/fail counts, anything unverified.
</compact_output_contract>
