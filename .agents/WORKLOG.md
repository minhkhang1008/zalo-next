# Agent Work Log

Shared, append-only log for every AI agent (Claude, Codex, ...) working in this repo.
**Read the latest entries before starting. Append a new entry after every task.**
Newest entry at the bottom. Keep entries short and factual.

Entry template:

```
## YYYY-MM-DD — <short title>  (agent: <name/model>)
- Request: what the user asked
- Context / root cause: what was found (with evidence)
- Changes: files + what changed
- Verification: what was run and the result; what is NOT verified
- Status: done / partial / blocked — and any open follow-ups
```

---

## 2026-11-11 — Conversation sidebar (danh sách hội thoại) cannot be resized  (agent: Claude Code, claude-opus-5-5)

- Request: Sidebar still could not be resized after an earlier agent's fix, when running
  `cargo run --release -- --gui --dev --data-dir ~/.ctdozlite-personal`.
- Earlier attempt (uncommitted on `personal-hardened`): added `conversation_sidebar_width` state,
  fixed `width_range` to 200..=520, `ui.set_width(ui.available_width())` in the panel, and removed
  the tab counter / "Đã ghim (n)" label so the header could not force the panel wider. That did not
  address the real cause.
- Root cause: egui 0.29.1 `SidePanel` stores the *content* rect as its width every frame
  (`PanelState { rect }`, egui `containers/panel.rs`). In `src/gui/workspace.rs` the thread-list
  row width was measured **outside** the `ScrollArea` as `available_width() - 8`, but the app uses
  `ScrollStyle::solid()` (`src/gui/theme.rs`: bar_width 6 + inner margin 4 = 10px reserved). Rows
  were therefore 2px wider than the viewport; a vertical-only `ScrollArea` with
  `auto_shrink([false,false])` grows to fit, so the panel re-stored a width +2px larger on every
  repaint. After a drag, the sidebar crept back toward the 520px max, so resizing looked broken.
- Evidence: headless egui 0.29.1 repro (scratchpad, not in repo) — with the old measurement the
  stored width went 280 → 287 with no input and drifted 242 → 254 after a drag to 240; with the
  fix it stays at 280 and holds at 240 after the drag.
- Changes: `src/gui/workspace.rs` — `row_target_w` is now computed inside the
  `threads_list_scroll` `ScrollArea` closure (used by thread rows and message-search hit rows).
- Verification: `cargo check` passes. Headless repro confirms the behaviour. NOT yet verified by
  the user in the real GUI.
- Status: done, pending user test. Rule for future edits: inside a resizable `SidePanel`, never
  allocate widgets wider than the space measured in their own `ui` (especially inside a solid
  `ScrollArea`), or the panel will fight manual resizing.

## 2026-11-11 — Replace self-drawn new-message popup with native macOS notifications  (agent: Claude Code, claude-opus-5-5)

- Request: Disable the self-drawn in-app new-message notification; send every new incoming message
  as a native macOS notification (title = sender / group, body = preview) via
  `src/native_notifications.rs`; DailyNotifier helper in `.app`, `osascript` fallback when the helper
  binary is missing (e.g. `cargo run`), text passed safely; no notification for outgoing messages;
  request permission at startup.
- Context found:
  - The "toast" for new messages was `incoming_popup` (`IncomingPopupState`), a separate egui
    corner viewport with quick-reply, drawn in `render_toast_overlay` (`src/gui/modals.rs`).
    `show_toast` / `toast_notice` is a different thing: short action feedback ("Đã tải thêm…",
    "Đã gửi tự động…"). **It was kept on purpose**: it isn't a message notification, and sending it
    to the OS would flood Notification Center.
  - `native_notifications.rs` previously did authorization only (no delivery). `DailyNotifier.swift`
    `--notify` existed but only accepted the generic payload `"Zalo"/"You have a new message."`
    (privacy design). It now accepts sender/preview text because the owner asked for it.
- Changes:
  - `src/native_notifications.rs`: new `pub async fn notify(title, body)`. It flattens control
    characters, caps title at 120 / body at 240 chars, and falls back to "ctdozlite" / "Tin nhắn mới"
    when empty. If the helper binary exists → `--notify` with JSON on stdin (opaque random sha256 id,
    `sound:false` because the GUI plays its own chime); `notAuthorized` → `NotificationError::NotAuthorized`,
    no osascript bypass. Else → `/usr/bin/osascript -e 'on run argv' -e 'display notification (item 2 of argv)
    with title (item 1 of argv)' -e 'end run' -- <title> <body>`, so the text is argv, never code.
    The spawn/collect helpers were refactored to support stdin. Five new tests were added.
  - `src/gui/worker.rs`: `WorkerCmd::PostNativeNotification { title, body }`. Delivery is
    fire-and-forget on tokio; failures go to `logger::log_error` (dev mode only, variant name only).
  - `src/gui.rs`: where the popup used to be created (incoming, not muted, not outgoing, and thread ≠ active or
    window unfocused), it now sends `PostNativeNotification` if `settings.enable_notifications`.
    `IncomingPopupState` and the `incoming_popup` field were removed. A one-shot
    `request_notification_permission_at_startup()` (macOS only) sends `RequestNotificationAuthorization`
    on the first frame after the worker is up.
  - `src/gui/modals.rs`: the corner popup viewport and `get_desktop_bottom_right_pos` were removed from
    `render_toast_overlay` (only the action-feedback toast remains). The Settings note text was updated.
  - `native/macos/DailyNotifier.swift`: `Payload.validate()` now accepts non-empty single-line text
    (title ≤120, body ≤240 graphemes; rejects Unicode `Cc` control chars, allows emoji ZWJ) plus the
    64-hex id. The self-test was updated.
- Verification: `cargo check --message-format=short` OK; `cargo test --lib` 247 passed / 0 failed.
  The helper was rebuilt into the scratchpad (not `native/macos/build`) and `--self-test` PASSES. NOT verified:
  real notification delivery/permission prompt in the GUI (osascript path under `cargo run`, and the
  helper path inside a packaged `.app`); the user must test.
- Status: done, pending user GUI test. Follow-ups / caveats:
  - Under `cargo run`, notifications come from `osascript` (attributed to Script Editor); macOS may
    need Script Editor allowed in System Settings › Notifications. The startup permission request
    returns `HelperUnavailable` in that mode (no helper) — expected.
  - Brand-new threads (first message from a contact not yet in the list) still do not notify,
    same as the old popup did.
  - Removing the popup also removed its quick-reply box.
  - Packaged builds must rebuild DailyNotifier (`native/macos/build-native-helpers.sh`) to pick up the
    relaxed validation; an old helper will reject the payload (exit 2 → `InvalidResponse`).
  - `cargo fmt --check` still reports pre-existing diffs in untouched code (build.rs, call_probe.rs,
    gui.rs, modals.rs); I didn't change them.

## 2026-11-11 — Message bubble alignment, bare-URL links, link-share copy, large emoji  (agent: Claude Code, claude-opus-5-5)

- Request: (1) align incoming bubbles without avatar; (2) blue, hover-underlined, clickable URL when
  there is no preview; (3) "Copy tin nhắn" copies the original URL for link shares; (4) emoji-only
  messages rendered large / in colour.
- Root causes:
  1. In egui 0.29, `allocate_exact_size` (avatar) adds `item_spacing.x` after the widget, but
     `add_space(30.0)` does not. Follow-up bubbles therefore sat `item_spacing.x` further left.
  2. Every message containing `http(s)://` becomes a link card. When there was no thumb/title/desc,
     the card drew the URL as a bold title in normal text colour.
  3. The context menu copied `msg.text`, which for attached link shares holds title/desc preview text.
  4. egui ships only a monochrome emoji font. The app bundles only the 6 reaction PNGs (`assets/emojis`).
- Changes (`src/gui/messages.rs` only):
  - Placeholder is now `add_space(30.0 + item_spacing.x)`.
  - Link card: if it has no thumb, title (≠url/domain) or desc, it renders the URL as a label in
    `url_link_color` (#388BFD dark / #1E88E5 light). The label underlines on hover (hover state kept in
    egui temp data, one frame behind), opens the URL on click, and drops the domain line.
  - `render_markdown_inline_rich`: bare URLs (`http(s)://`, `www.`, `docs.google.com/`,
    `drive.google.com/` at a word boundary) are blue, underlined on hover and clickable. Scheme-less
    URLs get `https://`.
  - New `message_copy_text`: for messages with `link_url` (not photo/file/sticker) it copies the
    URL, or "comment\nurl". Otherwise it copies `msg.text` unchanged.
  - New `emoji_only_clusters` (ZWJ, skin-tone, VS16, flag aware, ≤8 clusters) and `render_emoji_only`
    at 36px. It uses the bundled colour PNG when the emoji is one of the 6 reactions, else the glyph.
    The width pre-measure mirrors this. `render_message_body` gained an `emoji_textures` param.
  - 3 new unit tests.
- Verification: `cargo check --message-format=short` clean; `cargo test --lib` 250 passed / 0 failed.
  NOT verified in the real GUI.
- Caveats:
  - Emoji other than the 6 reactions (e.g. 🐧) are still monochrome glyphs (now 36px). Full colour
    needs a colour emoji atlas (e.g. Twemoji PNGs) or CoreText rasterisation, which isn't done.
  - **Mistake:** I ran `cargo fmt -- src/gui/messages.rs`, which formats the WHOLE crate. Files that were
    clean before the task now carry formatting-only diffs. My attempt to `git checkout` them was blocked
    by the permission classifier; the owner should decide. Files that were already dirty
    (gui.rs, modals.rs, theme.rs, worker.rs, workspace.rs, store.rs, api_mcp.rs,
    native_notifications.rs, zproto_host.rs) were also reformatted on top of the owner's
    uncommitted edits. The changes are semantically neutral, but they make diffs noisy.
    Use `rustfmt --edition 2021 <file>` for single-file formatting.

## 2026-11-11 — Colour emoji from the system font (follow-up)  (agent: Claude Code, claude-opus-5-5)

- Request: finish colour emoji (e.g. 🐧, 💌); owner asked why emoji can't render as plain text.
- Why text can't do it: egui/epaint 0.29 rasterises glyphs via ab_glyph into a single-channel
  coverage atlas tinted by the text colour. Colour glyph tables (Apple `sbix`, Noto `CBDT`,
  Windows `COLR`) are never read. So egui text can only show monochrome emoji; colour emoji must
  be drawn as textures.
- Changes:
  - Deps: `rustybuzz 0.20` (shaping, shares the existing `ttf-parser 0.25`) and `memmap2 0.9`
    (already in the lock tree).
  - New `src/emoji_raster.rs`: memory-maps `/System/Library/Fonts/Apple Color Emoji.ttc`
    (Linux Noto paths as fallback; Windows unsupported → None). It shapes the cluster with
    rustybuzz so ZWJ sequences, flags and skin tones resolve to the combined glyph, takes the
    PNG strike via `glyph_raster_image(gid, 96)`, pads it to a square and caps it at 160px.
    No network and no bundled assets. Unsafe `Mmap::map` has a SAFETY note.
  - `WorkerCmd::RasterizeEmoji { key, cluster }` runs on `spawn_blocking` (INV-3) and replies with
    the existing `StickerImageReady` / `StickerImageFailed` events. Textures live in
    `sticker_textures` under `emoji_<hex codepoints>`. In `gui.rs`, a failed `emoji_` key stays in
    `loading_stickers` as a negative cache, so it is not re-requested every frame.
  - `messages.rs`: new `scan_emoji_clusters` (byte ranges); `emoji_only_clusters` is rebuilt on it.
    Emoji-only messages use system texture → bundled reaction PNG → glyph, at 36px.
    For inline text, `render_markdown_inline_rich` / `render_chat_markdown_block` now take
    `textures` + `loading`. `split_emoji_sections` turns each ready emoji into a transparent
    placeholder section (first codepoint only). The galley is laid out manually and
    `paint_emoji_marks` overdraws the texture at each placeholder glyph. Only Emoji_Presentation
    / ≥U+1F000 / VS16 clusters are converted, so `©`, `→`, `✓` stay text.
  - Tests: 4 in `emoji_raster` (real font: 🐧 💌 ❤️ 👍🏽 🇻🇳 👨‍👩‍👧, combined ≠ component) and 1 for
    section splitting.
- Verification: `cargo check` clean; `cargo test --lib` 255 passed / 0 failed. NOT verified visually
  in the GUI.
- Caveats: the first frame shows the monochrome glyph until the texture arrives. Selecting and copying
  label text yields only the first codepoint of a ZWJ sequence (the "Copy tin nhắn" menu still copies
  the full `msg.text`). Thread list previews, composer, quote snippets and reactions still use glyphs.
  Formatting this time used `rustfmt` on the touched files only.

## 2026-11-11 — @mention popover regressions + real group tagging  (agent: Claude Code, claude-opus-5-5)

- Request: (1) the @ features stopped working (seen with `--release --gui --dev --data-dir
  ~/.ctdozlite-personal`); (2) picking a member only inserted plain text, with no real tag.
- Root causes found (in code; NOT reproduced in the live GUI, no runtime logs available):
  - `extract_active_mention_query` stopped suggesting at more than 3 words, so typing a full
    Vietnamese name ("@Nguyễn Hoàng Hải Đăng", 4 words) closed the popover mid-name.
  - `GroupMembersLoaded` with an empty list (failed `getmg-v2`/`group/members`) was cached in
    `group_members_map` forever and never refetched. The popover then offered only "@All".
  - No send path ever carried `mentionInfo` (always `""` / `"[]"` / absent) → tags impossible.
  - `suggest_mentions` is `true` in the owner's `app_settings.json`, so settings were not the cause.
- Changes:
  - `chat.rs`: `MentionPick`, `build_mention_info` (UTF-16 pos/len incl. `@`, type 1 for `-1`,
    longest-name-first, word boundary, drops picks no longer in text), and
    `send_group_mention_message_with_client_id` → `/api/group/mention`. On a non-expiry error it
    falls back to `sendmsg`. Evidence and caveats are in `re-notes/38_group_mentions.md`
    (zca-js shape + our incoming `mentions` parser; NO capture).
  - `worker.rs`: `WorkerCmd::SendMessage` gained `mentions`. Group texts with tags skip
    `sendlink` and go through the mention sender. Callers in `modals.rs` pass `Vec::new()`.
  - `workspace.rs`: query limit raised to 6 words (`MENTION_QUERY_MAX_WORDS`).
    `mention_name_matches` helper added. An empty member list re-fetches every 20 s at most
    (`mention_member_retry_at`). Fallback candidates come from incoming senders' `uidFrom`
    (`mention_fallback_from_messages`). `insert_composer_mention` records the pick, sets the
    caret at the end, and dismisses the "Name " query so Enter sends instead of re-inserting.
    Picks are sent via `composer_mentions` and cleared when the draft is empty. The popover now
    returns `MentionCandidate`.
  - `gui.rs`: two new state fields.
  - Tests: 2 in `chat.rs`, 2 in `workspace.rs`.
- Verification: `cargo check` clean; `cargo test --lib` 259 passed / 0 failed. NOT verified: live
  send to Zalo (endpoint and response unconfirmed), popover behaviour in the GUI, and whether the
  owner's group member fetch actually fails at runtime.
- Formatting: `rustfmt` on touched leaf files only (chat.rs, workspace.rs, worker.rs, modals.rs,
  messages.rs). The crate-wide `cargo fmt` churn from earlier today is still in the tree.

## 2026-11-11 — Blue @tag text + @ popover keyboard navigation  (agent: Claude Code, claude-opus-5-5)

- Request: (1) render @tags blue in group messages (incoming and outgoing), using `mentions`
  metadata and falling back to text parsing; (2) ↑/↓/Tab/Enter navigation in the @ popover:
  Enter/Tab pick without sending or moving focus, clear highlight, caret after the name.
  (Owner confirmed group tag sending via `/api/group/mention` works → re-notes/38 evidence.)
- Root causes:
  - `render_markdown_inline_rich` only checked `@` when it sat exactly at the start of `rest`.
    `@` was not one of the "next marker" positions, so mid-text `@Name` was bulk-appended as
    plain text.
  - Popover keys were read AFTER the TextEdit had run: ↑/↓ moved the caret, and Tab moved focus
    (egui `Focus::begin_pass` reads Tab from raw input at frame start; TextEdit's filter has
    `tab: false`). Escape clears focus at frame start, so the old focus-gated Esc check failed.
- Changes:
  - `messages.rs`: `message_mention_texts` maps wire `mentions` / archive `meta.mentionsList`
    UTF-16 `pos/len` onto `msg.text` → exact `@Name` strings. It returns `Some(empty)` when the
    message is known to have no tags, and `None` → heuristic when entries are unmappable.
    `find_mention_start` (word-boundary `@`) was added to the marker scan. Metadata spans match
    longest-first. The old `@All`/`@Name` guess runs only without metadata. Style:
    `mention_link_color` (#0068FF light / #4DA3FF dark), no chip background or underline. The
    renderers gained a `mentions: Option<&[String]>` param.
  - `GuiEvent::MessageSent` gained `mentions`. The SendMessage worker path fills it with
    `build_mention_info`, and gui.rs stores it in the local outgoing message's
    `wire_ref.fields["mentions"]` so our own bubble highlights exactly. Other emitters
    (worker ×3, call.rs ×5) pass `Vec::new()`.
  - `workspace.rs`: keys are consumed before the TextEdit with `consume_key` (↓/↑ clamp,
    Tab/Enter pick, Enter's `\n` text event dropped). The TextEdits use
    `.lock_focus(popover open)` so Tab doesn't leave. Esc is handled without a focus gate and
    restores focus. Selection resets to 0 when the query changes (`mention_last_query`). The
    selected row gets a primary tint plus a left accent bar, and is scrolled into view.
  - Tests: +2 in messages.rs.
- Verification: `cargo check --message-format=short` clean; `cargo test --lib` 261 passed /
  0 failed. NOT verified in the live GUI. Caveat: if `group/mention` falls back to `sendmsg`, our
  local bubble still shows blue tags even though recipients get plain text.

## 2026-11-11 — Redesign GUI to match Zalo PC (chat.zalo.me) look  (agent: Claude Code, claude-opus-5-5)

- Request: inspect chat.zalo.me with BrowserSkill (owner's logged-in account, permission given to
  open chats) and make ctdozlite look like it.
- Evidence gathered (scratchpad only, nothing committed): computed CSS custom properties for dark
  mode, the `:root .light` token block from Zalo's public stylesheet, and DOM measurements of nav
  rail, conversation rows, header, bubbles and composer. Only colour values and metrics were reused;
  no Zalo CSS, icons, fonts or images were copied. The Lucide icon set was kept.
- Changes:
  - `theme.rs`: dark/light palettes replaced with Zalo tokens (dark: rail #121416, panels #22262B,
    in-bubble #282D33, out-bubble/selected #1F344D, text #DFE2E7/#9FACBC, accent #0068FF/#66A6FF;
    light: blue rail #005AE0, white panels, canvas #E5E7EB, out-bubble/selected #E5F1FF, text
    #081B3A/#5A6981). New tokens `nav_icon`, `nav_icon_active`, `nav_selected_bg`, `nav_hover_bg`
    (the first two can be overridden in theme.json). Widget rounding 8 → 6.
  - `workspace.rs`: nav rail 64px wide with 48px tiles, 6px corners, tinted selection tile (pill
    removed); sidebar search is a borderless 32px well with 5px corners and "Tìm kiếm" hint; tabs
    14px; conversation rows 70px card in a 78px slot, 48px avatar, 16/14/12px text, no left bar, no
    group icon prefix; chat header 68px with 48px avatar and 18px title; composer input 15px.
  - `messages.rs`: bubbles borderless with a 1px soft shadow (`bubble_shadow`), 6px corners, 12px
    padding, 15px body text, 12px time, 13px secondary-colour sender name, 40px incoming avatar.
  - `gui.rs`: default sidebar width 280 → 344.
- Verification: `cargo check` clean; `cargo test --lib` 261 passed / 0 failed. rustfmt run on the
  three touched gui files only. NOT verified visually in the running GUI (not launched).
- Status: done, pending the owner's visual check. Not done: Zalo's right-hand group info panel
  layout, the pinned-message bar and the separate "Đã nhận" status pill under the last bubble.
  Text truncation still uses character counts, so very wide glyphs may need tuning after the
  larger fonts.

## 2026-11-11 — Blue @tags inside the composer + first-use @ popover keys dead  (agent: Claude Code, claude-opus-5-5)

- Request: (1) picked @tags in the composer should be blue like in sent bubbles (they were white);
  (2) the first @ pick in a session only worked with the mouse; keyboard worked afterwards.
- Root cause (2), from code (not reproduced live): `gui.rs` IME pre-pass set `ime_active = true`
  on `ImeEvent::Enabled`, which macOS/winit sends when a TextEdit gains focus. Enabled has no
  preedit, so the flag stayed true until the first IME commit. While it was true, `is_ime_busy`
  skipped the popover's ↑/↓/Tab/Enter handling and the pre-pass stripped every Enter key. After
  the first commit (e.g. after a mouse pick) the flag cleared, which matches "works from the
  second time".
- Changes:
  - `gui.rs`: `ImeEvent::Enabled` no longer marks composing; only a non-empty Preedit does.
  - `workspace.rs`: `composer_mention_spans` (longest-first, word boundary, no overlap) and
    `composer_mention_layout_job`; both composer TextEdits use a `.layouter` that paints
    `composer_mentions` names in `mention_link_color`. `COMPOSER_FONT_SIZE` = 15. +1 test.
  - `messages.rs`: `mention_link_color` made `pub(super)`.
- Verification: `cargo check` clean; `cargo test --lib` 262 passed / 0 failed. NOT verified in the
  live GUI (Telex IME behaviour in particular). If keys still fail while a Telex preedit is open,
  that is intended: Enter then commits the IME text first.

## 2026-11-11 — Paste images from the clipboard on macOS (Cmd+V)  (agent: Claude Code, claude-opus-5-5)

- Request: allow pasting images straight from the clipboard. Upstream only had it working on Windows.
- Root cause: egui-winit 0.29.1 `on_keyboard_input` swallows `Cmd+V` (`is_paste_command`) and
  pushes `Event::Paste` only when the clipboard has non-empty text. With an image-only clipboard
  egui saw no input at all, so `is_clipboard_paste_triggered` never fired on macOS; the old
  osascript `«class PNGf»` reader was never reached. That reader also missed TIFF-only images and
  would have pasted the file *icon* for files copied in Finder.
- Changes:
  - `Cargo.toml`: macOS-only deps `objc2 0.5`, `block2 0.5`, `objc2-foundation 0.2`,
    `objc2-app-kit 0.2`. These are the same versions winit already pulls in, so no new crates
    (Cargo.lock only gained dependency-list entries).
  - New `src/gui/macos_clipboard.rs`: a local `NSEvent` key-down monitor (no Accessibility or
    Input Monitoring permission needed) records `Cmd+V` (no Ctrl/Option, not a repeat) with a
    timestamp and requests a repaint. `take_cmd_v()` consumes it once, with a 1.5 s staleness
    window. `read_image()` reads NSPasteboard in-process: (1) Finder file URLs with image
    extensions, ≤64 MB, never the icon TIFF; (2) `public.png`; (3) `public.tiff`. Formats the
    `image` crate can't decode (TIFF/HEIC/BMP/WebP) are re-encoded to PNG through
    `NSBitmapImageRep`. 2 unit tests.
  - `src/gui/messages.rs`: on macOS, `is_clipboard_paste_triggered` installs the monitor (once)
    and ORs in `take_cmd_v()`. The macOS branch of `read_clipboard_image_to_png` now calls
    `macos_clipboard::read_image()`. The osascript script and its test were removed, so there is
    no longer a subprocess on every text paste.
  - `src/gui.rs`: `#[cfg(target_os = "macos")] mod macos_clipboard;`.
- Verification: `cargo check` clean; clippy shows no warnings in the touched code;
  `cargo test --lib` 263 passed. A temporary live test (removed afterwards) put a PNG, a TIFF and
  a Finder file copy on the real pasteboard: all decoded to 37×21. A text-only clipboard returned
  None. NOT verified: the Cmd+V key monitor in the running GUI (needs a manual try in a chat).
- Status: done, pending the owner's GUI test. UI hint strings still say "Ctrl+V".

## 2026-11-11 — Zalo-style "who reacted" tooltip and dialog  (agent: Claude Code, claude-opus-5-5)

- Request: rework the UI/UX for viewing who reacted. The owner sent two Zalo PC screenshots as
  references: a blue hover tooltip naming the reactor ("Trí"), and a "Biểu cảm" modal with emoji
  tabs on the left and reactors on the right.
- Before: hovering the pill showed only emoji + counts. Clicking opened an `egui::Window`
  "Cảm xúc" (340×380) with horizontal tab buttons.
- Changes:
  - `messages.rs`: `render_chat_bubble` / `render_reactions` take
    `reactor_name: &dyn Fn(&str) -> String`. New `reaction_tooltip_lines` builds the tooltip text
    from the ledger only (`reactor_ids`, deduplicated, most-used emoji first, max 8 names, then
    "và N người khác", which also absorbs unexplained counts). With no ledger it shows
    "N lượt thả cảm xúc". New `paint_reaction_tooltip` paints the solid blue card (#1A3F94 dark /
    #0052CC light), 14px white text and a down arrow centred on the pill, clamped to the screen,
    on the Tooltip layer. +2 tests.
  - `workspace.rs`: the caller builds the resolver from the existing per-frame
    `member_name_by_uid`, mapping self to "Bạn" and a 1:1 peer to `thread_name`.
  - `reactions_dialog.rs`: rewritten as a modal: dimmed backdrop (click outside or Esc closes) and
    a 440×350 card (clamped to the window) with a 52px header "Biểu cảm" + X button and a divider.
    The 120px left column has 36px tab rows ("Tất cả N", then emoji N), the selected one tinted.
    The right column scrolls 56px rows: 40px avatar, ellipsized 15px name, and emoji + count
    right-aligned. Data and row-building logic are unchanged.
- Verification: `cargo check` clean; clippy reports nothing new in the touched code (one existing
  `sort_by` lint at reactions_dialog.rs:108 remains); `cargo test --lib` 265 passed. rustfmt was
  run on reactions_dialog.rs and messages.rs only. NOT verified visually in the running GUI.
- Status: done, pending the owner's visual check.

## 2026-11-11 — Double-click to reply with an auto @tag; Esc leaves reply mode  (agent: Claude Code, claude-opus-5-5)

- Request: double-clicking a message enters reply mode and tags the sender at the start of the
  composer as a visible `@Name` the user can delete. Esc exits reply mode.
- Context: double-click → `BubbleAction::Reply` already existed, but it only set `replying_to`.
  There was no Esc handling for reply mode. `SendQuote` sent no tags at all
  (re-notes/38 "Not covered").
- Changes:
  - `workspace.rs`:
    - `reply_mention_for` resolves the sender uid from `sidebar_sender_id`, a numeric `sender`,
      or a unique member-name match. The name comes from `member_name_by_uid`. It never tags
      us. +1 test.
    - `start_reply` drops any previous auto tag and prefixes `@Name ` (only in groups and only
      for others' messages). The tag is added to `composer_mentions`, so it renders blue and is
      sent as a real tag. It also dismisses the @ popover for that query and puts the caret at
      the end.
    - `cancel_reply` / `remove_reply_auto_mention` strip the auto tag only if it is still
      untouched at the start.
    - Used by: double-click/menu Reply, the banner X, thread switch, and Esc. Esc is consumed
      after the @ popover's Esc and skipped while IME is composing or any modal is open
      (image viewer, reactions, forward, settings, schedule, phone lookup, profile menu).
    - Sending clears `reply_auto_mention` and passes `composer_mentions` to `SendQuote`.
  - `gui.rs`: `reply_auto_mention` state. `QuoteSent` carries `mentions` and stores them in
    `wire_ref.fields["mentions"]` so our own reply bubble highlights the tag.
  - `worker.rs`: `SendQuote { mentions }` calls the new `chat::send_quote_message_with_mentions`.
  - `chat.rs`: `send_quote_message_with_mentions`; `send_quote_message_with_client_id` delegates
    with `&[]`, so the TUI and old callers are unchanged. A group quote adds `mentionInfo` only
    when tags resolve. The group fallback now goes through
    `send_group_mention_message_with_client_id`, which equals plain sendmsg when there are no tags.
  - `re-notes/38_group_mentions.md`: documents quote + mentionInfo (zca-js evidence, NOT
    capture-verified) and the manual check.
- Verification: `cargo check` clean; `cargo test --lib` 266 passed; clippy reports nothing new
  in the touched code. NOT verified live: the GUI behaviour, or whether Zalo honours
  `mentionInfo` on `group/quote`.
- Status: done, pending the owner's live test (see the re-notes/38 manual check).

## 2026-11-11 — Drag & drop files: "Gửi nhanh" vs "Xem trước khi gửi" zones  (agent: Claude Code, claude-opus-5-5)

- Request (with a Zalo web screenshot as reference): while dragging a file over the app, show
  two clearly drawn drop zones and highlight the one under the cursor. Dropping on the message
  area sends immediately; dropping on the composer attaches the file to the draft for review.
- Before: any drop anywhere sent every file at once (handler inside the composer frame), with
  no hover UI.
- Key finding: during an AppKit drag session the window receives no mouse-move events, and
  winit 0.30 only forwards `HoveredFile` with no position, so egui's pointer stays frozen. New
  `src/gui/macos_drag.rs::cursor_in_window` reads `NSEvent.mouseLocation` and maps it into
  egui points: it flips the y axis using the primary `NSScreen` height, subtracts the
  viewport's `inner_rect`, and corrects for egui zoom (`pixels_per_point / native`). Other
  OSes use `pointer.latest_pos()`. Cargo features added: foundation `NSGeometry`, app-kit
  `NSScreen` (same crate versions).
- Changes (`workspace.rs`, `gui.rs`):
  - After the composer, a `4b` block: while `hovered_files` is non-empty,
    `drop_zone_rects(messages inner_rect, composer rect)` gives the quick zone over the message
    area and the preview zone over the composer (min 96px). `paint_drop_zones` draws on the
    Foreground layer: a covered background, a dashed border, title + hint. The active zone gets
    a primary tint, primary border and primary title. Repaint runs every 16ms while hovering.
    `drop_zone_last` is the fallback when no position is known.
  - `drop_zone_at`: only the message area means QuickSend; anywhere else (composer, sidebar,
    unknown) means Preview, so an imprecise drop never sends.
  - `handle_dropped_files`: directories are skipped with a toast. QuickSend sends one
    `PickAndSendAttachment` per file. Preview pushes `PendingDropFile` entries (dedup, max 30);
    size and a 112px thumbnail (png/jpg/gif ≤40 MB) are read on a `std::thread` and returned
    via the `drop_thumb_tx/rx` mpsc channel.
  - `render_pending_drop_files` strip (above the scheduled banner): a header showing the count
    plus "Hủy tất cả", and horizontally scrolling 210×52 cards (thumbnail or icon, ellipsized
    name, size / "Đang đọc…", ✕). `picker_reserve` +96 while the strip is non-empty.
  - Send: `can_send` also counts pending drops. Enter/send sends the pasted image first, then
    the dropped files; the typed text is the caption of the first file only.
  - +1 test (zone split / default).
- Verification: `cargo check` clean; `cargo test --lib` 267 passed; clippy reports nothing new
  in the touched code. rustfmt was run on workspace.rs and macos_drag.rs. NOT verified live:
  hover highlighting during a real Finder drag (especially the coordinate mapping on
  multi-monitor setups or with a non-1.0 UI zoom), and the GUI look.
- Status: done, pending the owner's live test.

## 2026-11-11 — Investigation: "Tìm kiếm GIF" setting does nothing  (agent: Claude Code, claude-opus-5-5)

- Request: Settings → "Gợi ý @" lists name suggestions, GIF search and sticker search, but in
  practice only @-tagging works. Check GIF search (wanted as a real feature).
- Findings:
  - `AppSettings::enable_gif_search` is only stored and drawn as a toggle
    (`modals.rs` ~2594). No code reads it, and the app has no GIF search, picker or send path.
  - `enable_sticker_keyword_search` ("Tìm sticker") only AND-gates the contextual sticker
    suggestions together with `suggest_stickers_while_typing` (`stickers.rs:1809`). There is
    no "@sticker"-style command.
  - Protocol evidence available offline: Zalo config has
    `apiGifDomain = https://sticker-wpa.<domain>`,
    `gif:{search_by_kw_limit:50, get_trending_limit:50, chat_box_gif_trending_limit:50}`,
    `maxGifSize: 3` (MB, overridable by `settings.sharefile.max_size_gif`), message types
    `MSG_GIF` / `CLI_MSG_TYPE_GIF` / `MSG_SUBTYPE_GIF` (photo-like `params` with
    width/height), and the `chat.gif` msgType (23), which the store already parses.
    `chat.rs` sticker suggest sends `gif:1` but only reads `sugg_sticker`.
  - The captured `web-bundle/` does NOT contain the GIF search or GIF send request code
    (lazy chunk not captured): no `/api/...gif...` paths at all.
- Changes: none (investigation only).
- Status: blocked on protocol evidence. Implementing search/send would mean guessing endpoints,
  which AGENTS.md forbids. Proposed next step (needs owner OK): capture on chat.zalo.me the
  request made when searching GIFs, and the one made when sending a GIF (e.g. to My
  Documents); record them in re-notes/39; then implement.

## 2026-11-11 — History lost after restart / re-login; phone sync "has no effect"  (agent: Claude Code, claude-opus-5-5)

- Request: after quitting, reopening and logging in again, all old messages were gone, unlike
  Zalo web; data sync seemed to do nothing. (Also: owner approved capturing the GIF protocol on
  chat.zalo.me; see Status.)
- Root cause (code + on-disk evidence): history was split between two databases.
  - Writes: live messages go to `zalo_history.db` (`PersistThreads`, group/cloud sync).
  - Reads: startup (`main.rs`), the worker reloads after login, workspace refresh, and phone
    sync (`history_sync.rs:472/1193`, writes too) all used `store::db_path_for_account(uin)`,
    i.e. the parked `accounts/<uin>_history.db`, which is refreshed only by
    `save_current_account_snapshot`.
  - Logout snapshots the live DB, then deletes it. Login never restored the parked copy, so a
    fresh, empty live DB was created, and the next snapshot overwrote the good parked copy.
    Phone-synced messages written to the parked DB were also clobbered by the next snapshot.
  - On disk: `~/Library/Application Support/ctdozlite` live 1909 msgs vs parked 1824 (47 threads,
    2026-11-02..11). `~/.ctdozlite-personal` live 422 vs parked 421 (22 threads); that data dir
    was only created at 21:44 today, so the older history lives in the default dir, not the
    personal one.
- Changes:
  - `store.rs`: `db_path_for_account` always returns `zalo_history.db` (the active account's DB).
  - `account_manager.rs`: `activate_account_data(uin)` with an `active_account` owner marker.
    Live files owned by another account are parked first. A missing live DB is restored from
    `accounts/<uin>_*`. Legacy layout (no marker, both exist): the parked DB is merged into
    live via `import_history_from_path` (INSERT OR IGNORE, so no loss). `switch_to_account`
    writes the marker. +3 tests (temp dirs).
  - `main.rs`: `activate_logged_in_account` runs before the threads cache / DB are read (GUI and
    TUI paths). `worker.rs`: activation runs before the snapshot on QR and password login.
- Caveat: the one-time legacy merge upserts thread rows from the older parked copy, and
  `last_read_msg_id` may move back, so a few threads can show stale unread badges once.
- Verification: `cargo test --locked` all pass (270 lib); clippy reports nothing new in the
  touched code. rustfmt was run on main.rs and account_manager.rs. NOT verified live: a real
  logout → login cycle, and phone sync (`sync_phone_messages`) end to end (needs the phone).
- GIF (still blocked): no browser automation was available in this session (no Claude in Chrome /
  built-in browser tools), so nothing was captured. Asked the owner to save the lazy JS chunk
  that loads when opening the GIF panel on chat.zalo.me into `web-bundle/`.

## 2026-11-11 — GIF search & send via the composer `@gif` command  (agent: Claude Code, claude-opus-5-5)

- Request: make Settings → "Tìm kiếm GIF" real; the owner approved capturing the protocol on
  chat.zalo.me with BrowserSkill (Main Chrome) and sending one test GIF to "My Documents".
- Evidence (re-notes/39_gif_search_send.md, capture-verified):
  - `GET {sticker}/api/message/giphy/trending` params `{offset,limit,imei}`.
  - `GET {sticker}/api/message/giphy/search` params `{keyword,offset,limit}`.
  - `POST {file}/api/{message|group}/gif` with an EMPTY body; params in the query string:
    `{thumb,width,height,msg:"",type:0,href,contentId,[visibility],src,toid|grid,clientId:"<str>"}`.
  - Results arrive as `data.data_list[{id, original{url,w,h}, normal{url,w,h}}]` on
    `zgif-v2.zdn.vn`. src 7 = composer `@gif` command.
  - The test send returned error_code 0 and showed "Bạn: GIF" in My Documents. The source chunk
    was saved as `web-bundle/lazy-1.c72c0957b455ab70b73c.js` (16 MB).
- Changes:
  - `chat.rs`: `GifItem`, `parse_gif_list` (drops untrusted hosts / id-less items),
    `gif_trending`, `gif_search`, `gif_send_body`, `send_gif_with_client_id` (local echo is a
    photo-style `ChatMessage`, text `[GIF]`). Hosts come from `s.domains.*` with an
    `endpoints::service` fallback. +2 tests.
  - `worker.rs`: `WorkerCmd::SearchGifs{query,seq}` (empty → trending, limit 30) and
    `WorkerCmd::SendGif` (→ `AttachmentSent` / `SendFailed`). `gui.rs`: `GuiEvent::GifResults`
    (stale seq ignored) plus gif_* state.
  - `workspace.rs`: `gif_command_query` (`@gif` + whitespace, case-insensitive), 350 ms
    debounced `update_gif_search`, a `render_gif_popover` 4-column grid above the composer
    (thumbnails via `FetchStickerImage`, key `thumb_<md5>`), keys ←→↑↓ / Enter (Enter
    swallowed while there are no results, so "@gif…" is never sent as text) / Esc (dismiss for
    that text). Click sends. Mention suggestions are off in GIF mode. Gated by
    `settings.enable_gif_search`. +2 tests.
  - `messages.rs`: `[GIF]` is not shown as a photo caption. `modals.rs`: setting hint now says
    "Gõ @gif + từ khóa…".
- Verification: `cargo test --locked` all pass (274 lib); clippy reports nothing new in the
  touched code. A read-only live probe (temporary example, deleted afterwards) could NOT run:
  both saved sessions (`~/.ctdozlite-personal` and Application Support) return 600 even for
  get_friends, i.e. they are expired. So search/send from ctdozlite itself is NOT verified live;
  only the wire format is (from the web capture plus the source).
- Status: done, pending a live test after re-login.

## 2026-11-11 — Turn off Claude co-author attribution in commits/PRs  (agent: Claude Code, claude-opus-5-5)

- Request: Stop adding "Co-Authored-By: Claude" to GitHub commits.
- Changes: user-global `~/.claude/settings.json` (outside the repo) — added
  `"attribution": {"commit": "", "pr": "", "sessionUrl": false}`. No repo files changed besides this log.
- Verification: `jq -e '.attribution'` parses the file and shows the block. Commits already in
  history are NOT rewritten (only new ones are affected).
- Status: done.

## 2026-11-11 — Release all fork work to `main`, delete `personal-hardened`  (agent: Claude Code, claude-opus-5-5)

- Request: commit and push everything to `main` of github.com/minhkhang1008/ctdozlite, delete
  the `personal-hardened` branch, and make the docs professional.
- Pre-commit audit: scanned the diff and the new files for UIN/IMEI/tokens/emails/personal paths;
  none found. A captured Giphy id in a test was swapped for a dummy. The local-only files are
  now git-ignored: `.claude/`, `.agents/skills/` (machine symlinks), `active_account` (holds the
  UIN), and `web-bundle/lazy-*.js` (16 MB Zalo JS, referenced by URL in re-notes/39 and not
  redistributed, per the README disclaimer). `.agents/WORKLOG.md` is tracked as AGENTS.md
  requires.
- Docs:
  - CHANGELOG `[Unreleased]` (Security/Added/Changed/Fixed).
  - README fork notice and module tree (new files; updater and REST/MCP marked disabled).
  - TINH_NANG matrix: ⌘V paste ✅; GIF, mentions, reply-by-double-click, reactions viewer,
    drag & drop zones; updater and REST/MCP 🚫; the Cyrillic "Авто-Reply" typo fixed.
  - HUONG_DAN_SU_DUNG §2.2 interactions. re-notes/39 now says where the chunk lives.
- Commits: chore(harness) worklog/ignore → feat(ui) code batch → docs. `main` is
  fast-forwarded from `personal-hardened` (which carries the security commit 1c691ae).
- Verification: `cargo build --locked` and `cargo test --locked` pass. Not split per feature:
  the features share workspace.rs / gui.rs / worker.rs hunks, so per-feature commits would
  not build on their own.

## 2026-11-11 — Rebrand docs as ctdozlite-next; drop upstream-team material  (agent: Claude Code, claude-opus-5-5)

- Request: README and other .md files belong to the original team; ctdozlite-next is the owner's
  personal build and needs simple docs. Owner has enabled GitHub Actions.
- Removed (upstream-only): `ctdozlite_landing/` (site, contributors, issue feeds); `docs/i18n/`;
  `docs/KE_HOACH_PRODUCTION.md`; `docs/DA_BIET.md` (auto-generated from upstream issues);
  the mdBook site (`docs/README.md`, `docs/SUMMARY.md`, `book.toml`, `.github/workflows/docs.yml`,
  `scripts/build-docs.sh`, `scripts/gen-known-issues.sh`); `.github/ISSUE_TEMPLATE/`; and
  `docs/assets/demo-windows.png`.
- Rewritten short: README (fork intro, differences, install, quick use, attribution + licence
  kept), `docs/CAI_DAT.md` (macOS install, local build, release-by-tag steps),
  `docs/TINH_NANG.md` (no upstream issue column), `docs/KHAC_PHUC_SU_CO.md`.
- Edited: HUONG_DAN (binary name `zalo-rust` → `ctdozlite`); AGENTS.md (fork repo URL, correct
  `--bin ctdozlite`); CHANGELOG (note that ≤0.1.0 is upstream history); ADR 0002 marked
  superseded; issue-tracker standard now points at the fork; DESIGN.md landing reference
  removed. `Cargo.toml` homepage/repository → fork; the Settings "Tìm hiểu" link → fork docs.
- Kept on purpose: upstream copyright in `build.rs` Windows metadata and the licences; the
  engineering docs (standards, ADRs, re-notes, KIEN_TRUC, GIAO_THUC, DESIGN, GLOSSARY,
  SESSION_SECURITY); the binary/bundle name `ctdozlite` (renaming it would move user data dirs).
- CI: the release step's hardcoded v0.0.7 title/body (upstream links, auto-update claims) was
  replaced by a generic `ctdozlite-next vX.Y.Z` with links built from `github.repository`.
  YAML validated with Ruby.
- Verification: `cargo build --locked` and `cargo test --locked` pass.

## 2026-11-12 — Release ctdozlite-next v0.2.0  (agent: Claude Code, claude-opus-5-5)

- Request: do the release (the owner enabled Actions and pushed the docs/CI commits).
- Steps: a dry run of CI on `main` (2879f98) built every platform. Two Ubuntu jobs first failed
  with apt "Temporary failure resolving" (runner network, 31 min at 1.3 KB/s); a rerun of only
  the failed jobs passed. Then `chore(release): v0.2.0` (7d66bee): version 0.2.0 in
  Cargo.toml/lock, CHANGELOG `[0.2.0] - 2026-11-12`. Tag `v0.2.0` was pushed. Run 43064924907
  passed every job and published "ctdozlite-next v0.2.0" with 14 assets.
- Verification: downloaded `ctdozlite-0.2.0-macos-arm64.dmg`. SHA256SUMS OK;
  `codesign --verify --deep --strict` OK (ad-hoc); Info.plist 0.2.0; arm64 Mach-O;
  `--version` prints v0.2.0. The release body links resolve to the renamed repo.
- Note: the owner renamed the GitHub repo to `minhkhang1008/ctdozlite-next` (GitHub redirects the
  old name). Local `origin` still uses the old URL (changing it was blocked, so it is left for the
  owner). Docs/Cargo/in-app links still say `minhkhang1008/ctdozlite`; update them in a follow-up.

## 2026-11-12 — Rename to ctdozlite-next, re-enable updater, hover bar, share dialog, reply layout, darker theme  (agent: Claude Code, claude-opus-5-5)

- Requests: (1) header "ctdozlite" → "ctdozlite-next"; (2) reply banner pushes the composer
  off-screen; (3) re-enable auto update and rename "Zalo Lite"; (4) Zalo hover bar (Trả lời /
  Chia sẻ / "…" menu as in the owner's screenshot); (5) Zalo-style share dialog; (6) darker
  background, gray bubbles. Plus a question about release turnaround.
- BrowserSkill: the chat.zalo.me agent tab kept losing "active tab" to the owner's own Zalo tab
  ("Bạn đang mở Zalo trên một Tab khác"). Stopped rather than fight the owner's session; worked
  from the owner's screenshots of the "…" menu and the share dialog.
- Changes:
  - `lib.rs` `APP_DISPLAY_NAME = "ctdozlite-next"` is used for the window/notification titles, the
    header, the Settings card, the macOS notification fallback and `--version`. Info.plist
    CFBundleName/DisplayName follow it. The binary, bundle id, data dir and eframe storage key
    stay `ctdozlite`, so data and notification permissions carry over. The integration test
    expectation was updated.
  - Updater: removed `AUTO_UPDATER_ENABLED`; `GITHUB_REPO = minhkhang1008/ctdozlite-next`.
    Restored the startup check (`gui.rs`) and the 30-min periodic check (`worker.rs`); the update
    notification goes through `PostNativeNotification` (`updater::new_update_notification`).
    New macOS in-place install in `updater.rs` (`current_app_bundle`,
    `install_macos_app_from_dmg`, `relaunch_app_bundle_after_exit`): it runs only when running
    from a .app and the DMG SHA-256 was supplied/verified. Steps: hdiutil attach -readonly
    (detached on drop) → codesign --verify --deep --strict → ditto to a staging .app → rename swap
    with a backup and rollback → strip quarantine. Relaunch waits for this PID to exit. Any
    failure falls back to the old "open the DMG" path. AGENTS.md rule #1 was rewritten: REST/MCP
    stays off; the updater may only use the fork repo.
  - Reply banner: `msg_area_height` now uses the measured height of everything below the list
    (`chat_bottom_stack_h`, measured from the scroll inner bottom to the composer bottom on the
    previous frame, with a repaint on change) instead of the 48px estimate. The reply preview is
    flattened to one truncated line.
  - `messages.rs` `hover_action_bar`: 28px circles (quote, forward, three dots), left of outgoing
    and right of incoming bubbles, bottom-aligned. Shown when hovering the bubble or bar (same
    layer, inside the clip) or while its menu is open. "…" opens the existing menu in
    `compact` mode (no emoji row, no reply/share; separator after Copy; "Tuỳ chọn khác ›" flyout
    holding reminder / open photo / save file via the new `message_extra_menu_items`). The
    outside-click close ignores the flyout and the "…" toggle. Shared `menu_frame` /
    `style_menu_ui`. Right-click keeps the full menu.
  - Share dialog (`modals.rs::render_forward_dialog`) rewritten: modal card with "Chia sẻ" + ✕,
    accent-insensitive search, tabs Gần đây / Nhóm trò chuyện / Bạn bè, a checkbox list with
    avatars (posting-restricted groups greyed), a preview box (link / photo / file / sticker /
    text), an optional note, Hủy / Chia sẻ (n). `WorkerCmd::Forward` gained `note`, sent after a
    successful forward in the same task (keeps order) and echoed as `MessageSent`.
  - Theme (dark): canvas #121417, sidebar #1B1E22, input #101215, border #2E3237, in-bubble gray
    #30343A, out-bubble blue-gray #343D4A.
  - Docs: README, TINH_NANG, CAI_DAT (auto update), ADR 0002 re-activated, CHANGELOG
    [Unreleased]; every `minhkhang1008/ctdozlite` link → `-next` (repo was renamed).
- Verification: `cargo build --locked` and `cargo test --locked` all pass; clippy reports nothing
  new in the touched code (the remaining warnings predate this work). NOT verified in the
  running GUI (no live session available here): hover bar, menus, share dialog, reply layout,
  colours. The macOS in-place update is untested end to end; it needs a newer release (v0.2.1+)
  installed over v0.2.x.

## 2026-11-12 — Faster CI; BrowserSkill pass: multi-select, message details, share "Phân loại", "Copy link"  (agent: Claude Code, claude-opus-5-5)

- Request: apply the CI speed-up, re-run BrowserSkill and do what was skipped last time.
- CI (`ci-release.yml`): the four installer jobs now run only for `v*` tags, workflow_dispatch,
  or PRs labelled `ci:full-package`. A push to main runs just `test`. `paths-ignore`
  (`**.md`, `docs/**`, `.agents/**`, `re-notes/**`, `LICENSE-*`) skips docs-only pushes and PRs
  (path filters do not apply to tags). YAML validated.
- BrowserSkill (My Documents only, nothing sent or deleted): this time the tab activated. Observed:
  - Hover bar = quote / forward / "Thêm" (bottom-aligned, left of own bubble), plus a reaction
    row.
  - The "…" menu on an own link message in My Documents = Copy link / Đánh dấu tin nhắn /
    Chọn nhiều tin nhắn / Xóa. It opens upward near the bottom. In groups it also has Ghim,
    Xem chi tiết and Tuỳ chọn khác (owner's screenshot).
  - "Chọn nhiều tin nhắn" shows a bar above the composer: "[n] Đã chọn · Sao chép · Chia sẻ ·
    Xóa (red) · Hủy". Cancelled afterwards.
  - The share dialog's "Phân loại" is a checkbox dropdown of the account's conversation labels
    with colour tags. Closed with Hủy.
- Code:
  - `chat.rs`: `ConvLabel`, `parse_conv_labels`, `fetch_conv_labels`
    (`GET label-wpa/api/convlabel/get`, read-only; source evidence in re-notes/40). +1 test.
  - `worker.rs` `FetchConvLabels` → `GuiEvent::ConvLabelsLoaded`, fetched lazily when the share
    dialog first opens.
  - Share dialog: a "Phân loại ▾" dropdown filter (ARGB/RGB colours parsed safely);
    `forward_more` lets one share carry several messages (the note only follows the last
    message per target).
  - `messages.rs`: `BubbleAction::{StartMultiSelect, ToggleSelect, ShowDetails}`;
    `multi_select_overlay` (temp-data keys `MULTI_SELECT_ACTIVE/KEYS` set by the workspace before
    the list; when active, each bubble is a toggle with tint, border and check badge, and
    hover/menus/double-click are skipped). Menus gained "Chọn nhiều tin nhắn" and
    "Xem chi tiết". Copy is labelled "Copy link" for link messages.
  - `workspace.rs`: `open_share_dialog`, `multi_selected_messages`, `render_multi_select_bar`
    (Sao chép joins `message_copy_text`, Chia sẻ opens the dialog with all selected, Xóa sends
    DeleteForMe for each, Hủy). Esc exits; switching thread clears the selection.
    `modals.rs::render_message_details`: sender, local full time, type, delivery state,
    reactions, id + copy.
  - Docs: CHANGELOG, TINH_NANG, HUONG_DAN, CAI_DAT (CI timing), re-notes/40.
- Verification: `cargo test --locked` all pass (275 lib); no clippy warnings inside the new
  functions. rustfmt was run on messages/modals/workspace/chat only. NOT verified in the running
  GUI. `convlabel/get` is not live-tested (no valid local session); the parser follows the
  source and zca-js.

## 2026-11-12 — Light-mode chrome, QR auto-refresh + help, no phone login, start maximized, sidebar labels, drop .mailmap  (agent: Claude Code, claude-opus-5-5)

- Requests: (1) remove phone login; (2) QR: phone sometimes says "không có kết nối với máy chủ";
  guide the user, and auto-refresh/expire like Zalo PC; (3) header still says "ctdozlite"; (4) is
  .mailmap needed?; (5) light mode: title bar text/icons, version pill and info panel unreadable;
  (6) open full screen; (7) implement "Phân loại" fully.
- Findings:
  - (3) The owner's screenshots show "Native v0.1.0", an old binary. Current code renders
    `APP_DISPLAY_NAME` and the version from Cargo.toml (0.2.0). Verified by launching a fresh build
    with a temp data dir: the title shows "ctdozlite-next v0.2.0". A second Zalo icon in the Dock
    was the owner's running old copy.
  - (5) Light `bg_app` is Zalo blue, but the title bar used `text_*` colours; the drawer footer
    used `bg_app` as a panel fill.
  - (2) `wait_for_scan` / `confirm` errors were silently dropped, so an expired QR stayed on
    screen forever.
  - (6) `ViewportBuilder::with_maximized(true)` with `decorations(false)` made the window never
    appear on macOS (reproduced: process running, no window). Fixed by sending
    `ViewportCommand::Maximized(true)` on the first frame (verified: window opens maximized).
- Changes:
  - theme.rs: chrome tokens (`chrome_fg`, `chrome_fg_muted`, `chrome_hover`, `chrome_pill_bg`,
    `chrome_pill_fg`) are white-on-blue in light and match the old look in dark. The title bar
    uses them; the pill now reads `v0.2.0`. The drawer footer uses `bg_hover`.
    `label_color()` helper (RGB/ARGB).
  - gui.rs: the login screen is QR only (LoginTab/phone/password state removed; the worker
    `LoginPassword` and the CLI `--phone/--pass` remain). QR countdown from
    `qr_login.qr_timeout` (minus 5 s), auto `request_new_qr` on expiry, `GuiEvent::QrExpired`
    (from worker scan/confirm errors) triggers auto refresh, capped at 3 failures within 20 s
    each. Collapsible help: quit/reopen Zalo, refresh QR, check network/VPN, confirm in time.
  - Sidebar labels: server labels replace the hard-coded local categories. Colour tag on rows,
    "Phân loại ▾" filter next to the tabs (refetches on open), and a read-only label list in the
    right-click menu. `SetLabel` action removed; `settings.thread_labels` is left unused for
    settings compatibility. Labels are fetched at startup (if logged in) and on LoginSuccess.
  - Removed `.mailmap` (only merges upstream contributor identities in git log/shortlog).
  - Docs: TINH_NANG, HUONG_DAN (QR help), CHANGELOG, re-notes/40 (why there is no write path).
- Verification: `cargo test --locked` all pass; no clippy warnings in the new code. Launched a
  debug build in light mode with a temp data dir: the title bar is readable, there is no phone
  tab, the countdown and help link render, and the window starts maximized. NOT verified: a
  logged-in session (sidebar labels, the drawer in light mode) and real QR expiry auto-refresh
  end to end.

## 2026-11-12 — Update dialog shows the actual changes, not just links  (agent: Claude Code, claude-opus-5-5)

- Request: the in-app "Cập nhật phần mềm" card listed only CHANGELOG/CAI_DAT links; show a short
  list of the changes instead.
- Cause: the CI release body was a fixed pointer text, and the app lists every `- ` bullet of the
  release body (`extract_concise_changelog`).
- Changes:
  - New `scripts/release_notes.py <ver> [CHANGELOG] [owner/repo]`: takes the `## [ver]` section
    and emits one short line per bullet (the bold title, minus "(`src/…`)" refs), with headings
    mapped to Vietnamese and a non-bullet "Chi tiết / Cài đặt" line at the end.
  - `ci-release.yml` release job: sparse-checks out CHANGELOG + script, builds
    `release_notes.md`, uses `body_path` with `generate_release_notes: false` (GitHub's commit
    list leaked "by @user in https://…" bullets into the dialog).
  - `modals.rs`: `short_changelog_title` trims long "**Title:** text" bullets. The extractor
    skips link-only bullets ("Có gì mới:", "Cài đặt:") and GitHub auto commit lines. +1 test.
  - The published v0.2.0 release body was regenerated from the tagged CHANGELOG and updated
    with `gh release edit`, so the installed app shows the list now.
- Verification: the script output was checked for 0.2.0; workflow YAML validated; the new test
  passes along with the full `cargo test --locked`.

## 2026-11-12 — Release ctdozlite-next v0.2.1  (agent: Claude Code, claude-opus-5-5)

- Request: tag and release (the owner pushed the five feature commits plus a README edit, f2f2df4).
- Before tagging, the CI on main for f2f2df4 passed (tests only, 1m36s, thanks to the new
  workflow).
- `chore(release): v0.2.1` (8347d1a): Cargo.toml/lock 0.2.1, CHANGELOG `[0.2.1] - 2026-11-12`.
  The "Changed" bullets were rewritten as `**Title:** detail` so `scripts/release_notes.py`
  yields short lines. Tag v0.2.1 was pushed; run 43078053535 passed every job (~9 min) and
  published "ctdozlite-next v0.2.1" with 14 assets and a body generated from the CHANGELOG.
- Verification: downloaded the arm64 DMG. SHA256SUMS OK; codesign --deep --strict OK;
  CFBundleShortVersionString 0.2.1; CFBundleDisplayName ctdozlite-next;
  `--version` → "ctdozlite-next v0.2.1"; `--check-update` reached the fork's releases and reported
  it is the latest.
- Note: v0.2.0 has no updater, so the owner installs v0.2.1 by hand once. From v0.2.1 on the in-app
  updater handles it (the macOS in-place swap is first exercised by v0.2.2).

## 2026-11-12 — Free way to install on macOS without the Gatekeeper block  (agent: Claude Code, claude-opus-5-5)

- Request: no paid Apple Developer account; find a free way so users do not have to approve the app
  in Privacy & Security. Owner pasted a reference listing (1) a curl installer, (2) a Homebrew tap
  with postflight `xattr -cr`, (3) a `.command` fixer inside the DMG.
- Assessment: (1) works because Gatekeeper only blocks files carrying `com.apple.quarantine`, which
  browsers add and curl does not. (2) works for a personal tap. (3) is not viable: a `.command` file
  inside a browser-downloaded DMG is itself quarantined and unsigned, so Gatekeeper blocks it too.
  CI already ad-hoc signs with `--deep`, which avoids "app is damaged".
- Changes:
  - `scripts/install-macos.sh`: finds the latest version via the `/releases/latest` redirect (no
    API rate limit), or `CTDOZ_VERSION`. Picks the arm64/x86_64 DMG, checks SHA-256 against
    SHA256SUMS.txt, mounts read-only, quits only an app running from the target path, then
    `ditto` + `xattr -cr` (sudo only if the dir is not writable) and opens the app.
    `CTDOZ_APP_DIR` / `CTDOZ_NO_OPEN` exist for testing. Variables are braced before Unicode text:
    the system bash 3.2 otherwise reads "$APP_DIR…" as a different variable (hit during the test).
  - `packaging/homebrew/ctdozlite-next.rb`: per-arch cask with the v0.2.1 SHA256s, postflight
    `xattr -cr`, `auto_updates true`, zap of the data dir. Not published (needs the owner to
    create `minhkhang1008/homebrew-tap`).
  - README / CAI_DAT: installer first, explanation of why, manual DMG path
    (`xattr -cr` / Open Anyway). `release_notes.py` adds the one-liner to future release bodies
    (in a code block, which the in-app dialog ignores). CHANGELOG [Unreleased].
- Verification: ran the installer against v0.2.1 into a temp dir with CTDOZ_NO_OPEN. Download and
  SHA-256 OK, the app was installed with 0 quarantine attributes, codesign verify OK, version
  0.2.1. `ruby -c` on the cask is OK; brew install is not tested (no tap). The one-liner URL only
  works once `scripts/install-macos.sh` is on main.

## 2026-11-12 — Make the free macOS install look and be trustworthy; Homebrew tap  (agent: Claude Code, claude-opus-5-5)

- Request: go ahead (tap + install path), but design it so users do not suspect malware.
- Changes:
  - `scripts/install-macos.sh` rewritten for transparency. A Vietnamese header lists exactly the 5
    steps and what it does NOT do. It prints version, source URL and target, and numbers each step
    [1/5]…[5/5]. It verifies SHA-256 and `codesign --verify --deep --strict`, asks Y/n via /dev/tty
    (works under `curl | bash`; `CTDOZ_YES=1` skips), uses sudo only if needed with an explanation,
    and removes only `com.apple.quarantine` from this app (`xattr -dr`, not `-cr`). It ends with
    uninstall instructions. Tested into a temp dir: OK, 0 quarantine xattrs.
  - CI release job: `actions/attest-build-provenance@v2` over the installer files
    (`id-token`/`attestations: write`), so `gh attestation verify <file> --repo …` works from
    v0.2.2. A new optional "Update Homebrew tap" step bumps version + per-arch SHA-256 in the tap
    and pushes, gated on secret `HOMEBREW_TAP_TOKEN` (the sed bump was tested on a copy).
  - Created public repo `minhkhang1008/homebrew-tap` (owner approved) with an English README
    explaining exactly what the cask does, and `Casks/ctdozlite-next.rb`. Current Homebrew
    required `depends_on macos: :big_sur` (not the string form) and `postflight_steps { run
    "/usr/bin/xattr", args: ["-cr", "{{appdir}}/ctdozlite.app"] }` (old `postflight` is deprecated;
    `run` args expand `{{appdir}}`, checked in Homebrew's install_steps.rb). `brew info` is clean;
    `brew fetch` downloaded and matched SHA-256. The tap was untapped/untrusted afterwards.
    `brew style` shows only generic Sorbet/frozen-string-literal sigil notes.
  - Homebrew now refuses third-party taps until `brew trust <tap>`; the docs show
    `brew tap … && brew trust … && brew install --cask ctdozlite-next` and explain why.
  - README "An toàn & minh bạch" section (open source, public CI build, SHA256SUMS, attestation from
    v0.2.2, exactly what the installer does, uninstall) plus a "read first, then run" option.
    CAI_DAT, release_notes.py footer and CHANGELOG updated.
- Not done: brew install itself (it would replace the owner's /Applications app);
  `HOMEBREW_TAP_TOKEN` must be created by the owner.

## 2026-11-12 — Shared phone sync action, history bootstrap, remembered theme (agent: Codex)
- Request: make rail sync behave like Settings phone sync; investigate Group/Cloud `-69`; default to light and remember the last theme after reopening.
- Context / root cause: both phone-sync buttons already emitted the same worker command in this checkout, so a command mismatch was not reproduced. Quick theme toggles changed only runtime state. Group/Cloud paging always requested old mode, even without a SQLite cursor, and substituted `9223372036854775808` for zero; checked-in Zalo bundle `getCM` defaults to recent mode and cursor `0`. Screenshot confirms `-69` rejection, not its exact server-side cause.
- Changes: shared `start_phone_sync`; rail opens Settings sync progress, existing Settings button retained. All theme controls update persisted settings, default light; settings snapshots write in worker order off the UI thread. History starts recent when no cursor, then pages old; errors omit URLs, report incomplete sync, and failed threads receive a 400 ms interval. Added protocol evidence/manual checks in `re-notes/42_history_sync_entrypoints.md` and Unreleased changelog.
- Verification: reproduced failing default-theme and initial-history-mode tests before fixing. `cargo test --locked --lib gui::` 36 passed; `cargo test --locked --lib history_sync::tests` 9 passed; `cargo test --locked --lib chat::tests` 25 passed. GUI tests include writing/reopening theme settings in a temporary file. `cargo check --message-format=short` exit 0; `git diff --check` exit 0. Formatted touched Rust files only. No live Zalo requests, phone confirmation, GUI relaunch, installed-app replacement or release performed; server-side `-69` resolution remains unverified.
- Status: done for source changes and offline checks; live phone/server verification pending. No commit or push.

## 2026-11-12 — Review upstream v0.1.1 for selective integration (agent: Codex)
- Request: check the 66 upstream commits and v0.1.1 for changes worth merging.
- Context / root cause: fetched upstream main and v0.1.1, both `7ed3408`; local HEAD `cb2cd66`, common base `3d9e690`. Upstream has 257 commits total, 66 exclusive to upstream; fork has 17 exclusive commits. Existing uncommitted sync/theme changes preserved.
- Changes: added `docs/UPSTREAM_REVIEW_2026-11-12.md` with candidates, dependencies, source SHAs, security/privacy concerns and integration order. No application code changed in this task.
- Verification: merge-tree simulation exit 1, 18 conflicted files (committed HEAD only; no real merge). Patch applicability: `efe708d` exit 0; `5e6b98b`, `1203b97`, `1889923`, Linux IME commits exit 1/context differences. Reviewed source for sync key permissions, destructive key test, unbounded Office XML extraction, updater source, REST/MCP, and missing recent-first history fix upstream. No upstream builds/tests/live requests performed. GitHub page checked; code conclusions use fetched Git objects.
- Status: done for review; recommend selective ports starting with clamp/WASM discovery and passive ringing. No merge, cherry-pick, commit or push performed.

## 2026-11-12 — First upstream integration batch, video-tool installation and prioritised Sync V2 (agent: Codex)
- Request: begin implementing upstream improvements sequentially, add automatic video dependencies; owner selected phone Sync V2 as the next priority after fixes/video tools.
- Context / root cause: FFmpeg was probed once through PATH (Finder could miss Homebrew; a failed probe stayed cached forever). Video controls did device I/O in rendering paths. Narrow link/call cards retained unsafe clamp bounds. Incoming calls probed relay before acceptance. Actual upstream WASM reproduced identical consecutive generated public keys; metadata glue also passed 13 arguments to an 11-argument export.
- Changes: ported narrow clamps and app-bundle WASM search; passive ringing from `efe708d` with an acceptance claim, target-call checks and orphan-probe cleanup. Added worker-only FFmpeg detection/installation/refresh/cancellation and native settings controls; video action starts setup automatically when missing. macOS uses app-local, pinned SHA-256-verified 9.0.2 builds, Windows WinGet, Debian/Ubuntu pkexec+apt-get. All media spawns use the discovered absolute path; UI reads cached devices/readiness. No system FFmpeg was installed/replaced during this task.
- Sync V2: corrected WASM typed-array entropy and metadata ABI; private atomically persisted identity, fresh ephemeral keys, cryptographic base62 IDs, manual empty tempKey, real znoise partitions, one 590 request/no competing V1/592 and 591 cleanup. Shared ordered batch handling waits for metadata and pending imports, validates request/session, maps real thread IDs and persists idempotently in an atomic batch. Malformed lengths fail closed; counts reflect committed insertions. Account-file changes wait for phone sync/decoding, and stale account completion events are ignored. Kept URL trust, 256 MiB download / 512 MiB decompression caps, fork-only updater and disabled REST/MCP. No upstream raw-key logging or destructive key-file test ported.
- Verification: final relevant module totals 116 passed, 0 failed, 1 pre-existing ignored live-call test: GUI 39, chat 26, history_sync 13, call 19 (+1 ignored), video_tools 5, param_cipher 8, zproto_host 6. Incremental runs included reproduced failing clamp/default-theme/entropy tests before fixes. WASM tests ran with an actual upstream module extracted to `/tmp`, not silently skipped; key uniqueness, agreement and metadata ABI passed. Offline installer tests cover activation/hash rejection/private modes/cancellation; headless UI covers both themes/loading/ready/narrow widths. Downloaded ARM64 FFmpeg hash matched publisher and real synthetic H.264 encode/decode passed (64x64 RGB, 12,288 decoded bytes). Intel hash computed from publisher HTTPS download; binary not executed on ARM hardware. Cargo check and git diff --check pass. Impeccable detector reported only existing color advisories outside changed controls.
- Not verified: real phone confirmation/znoise response/import; real voice/video call; installed GUI app relaunch; Windows/Linux installers; release packaging or installed-app replacement. Snapshot WASM stays outside Git. Notes: `re-notes/43_sync_v2_lifecycle.md`, `docs/VIDEO_TOOLS.md`, `tasks/plan.md`, `tasks/todo.md`.
- Status: first four source slices complete with offline/runtime-component checks; live phone/call checks pending. Drafts/Saved/reminders/search and document-preview/voice/Markdown remain later slices. Preserved prior uncommitted work; no commit, push or release.

## 2026-11-12 — Explain the Zalo PC dependency of phone Sync V2 (agent: Codex)
- Request: why syncing from a phone asks for Zalo PC.
- Context: verified `ZprotoHost::load` and `find_wasm_path`: the client needs `libzproto_wasm_bg.*.wasm` for local key generation/decryption and currently discovers it in an installed Zalo PC, bundled resources, a developer copy or an explicit path. The phone remains the requested message source; the Zalo PC application itself need not run.
- Changes: explanation only; no application behavior changed. Acknowledged that automatic provisioning/packaging of this dependency remains incomplete.
- Verification: source inspection; no tests or live phone actions needed for this explanation.
- Status: explanation complete; standalone provisioning and live phone verification remain open.

## 2026-11-12 — Manual acceptance checklist for the current integration batch (agent: Codex)
- Request: explain what still needs testing.
- Changes: provided prioritised manual scenarios for actual phone-message import, cancellation/retry, separate Group/Cloud paging, FFmpeg detection/installation, incoming/outgoing voice/video, theme persistence, narrow UI and account isolation. Distinguished the missing standalone WASM dependency from the phone-transfer test itself.
- Verification: checked current worklog, task status and video-tool verification boundaries. No application changes or live calls/phone requests performed.
- Status: checklist delivered; manual acceptance must use a build containing these source changes. Current installed app has not been replaced by this work.

## 2026-11-13 — Separate manual phone sync and restore native Android selection (agent: Codex)
- Request: main-screen sync finished without a phone prompt/history, retry failed on missing zproto WASM; owner confirmed Android and asked to continue.
- Context / root cause: automatic server refresh shared the manual command/running/cancel state and phone completion events, so it could prevent a manual click from sending a request. The preceding integration selected V2 for every manual attempt. Existing native backup import also contained a hardcoded sender account, swallowed SQLite/parser failures and invented Maps text. These are source findings; the actual phone/server response was not captured.
- Changes: distinct RefreshMessageQueues command/state/account-scoped event; manual intent yields background refresh and never receives its phone completion. Settings persist Android/iPhone (default Android), both buttons retain one shared action. Android selects one native GET pull with empty manual temp_key and actual device name, without WASM; phone requests wait for committed import, reject/busy/timeout remain errors, cancel/retry/account changes wait for decoding and clear request keys. Both transports share typed account/key-gated, single-receipt handling. New history_sync/android.rs keeps native RSA/CBC/XZ/SQLite work in the blocking pool, bounded/private extraction, real sender mapping through the standard profile znoise GET, atomic/idempotent writes and hydration of new conversations. Removed fallback second POST and literal znoise host. Incomplete phone requests no longer say completed; V2 timeout also fails instead of falling through to cloud success. Updated changelog/features/protocol notes/tasks; no runtime binary or JS/WASM downloaded into the app.
- Evidence: checked-in lazy-1.c72c0957b455ab70b73c.js static pullMobileMsg uses file GET command 12000 with the existing payload; static getNoisedUids uses profile GET command 12390. Existing native crypto/container contract retained. New re-notes/44_android_phone_sync.md records changes and manual checks.
- Verification: completion regression assertion failed before the fix. Relevant modules: history_sync 19 passed (6 native Android tests including synthetic encrypted XZ/SQLite, mapping/hydration/idempotency/rollback/account gating/rejection), GUI 40 passed, chat 26 passed: 85 passed total. Re-ran the 6 Android tests after tightening timeout cancellation; all passed. Final cargo check --message-format=short exit 0; git diff --check exit 0. Preserved previous uncommitted changes; no commit/push.
- Status: source fix and offline checks complete. Not verified: real Android prompt/approval, current phone backup schema/live znoise mapping, actual old-message/media coverage, cancellation/account switching against a real transfer, or installed GUI replacement/relaunch. iPhone V2 still needs its existing local WASM runtime; standalone provisioning remains incomplete. No release build/install or authenticated live phone request performed.

## 2026-11-13 — Require an explicit phone platform instead of defaulting to Android (agent: Codex)
- Request: clarify Android default because the owner's relatives also use iPhones.
- Context / root cause: the prior GUI setting selected Android for every installation with no saved choice, based only on the owner's phone. That was an inappropriate product-wide assumption; iPhone support must remain selectable.
- Changes: unset platform is now None; the first sync opens Settings and requests Android/iPhone without sending a transfer. Both explicit choices are remembered and can be changed; existing string preferences remain compatible. Removed the enum's implicit default, updated changelog/protocol clarification. Existing Android and iPhone worker paths retained.
- Verification: new unset-preference assertion failed before the correction; cargo test --locked --lib gui::tests passed 7 tests, including default/roundtrip of both choices. cargo check --message-format=short and git diff --check exit 0.
- Status: source correction complete; no installed-app change, commit or push. Actual phone transfers remain unverified; iPhone Sync V2 still requires the existing local WASM runtime and is not claimed complete.

## 2026-11-13 — Finish remaining source implementation groups in the integration plan (agent: Codex)
- Request: complete every remaining item in the existing plan, including the standalone iPhone/Sync V2 dependency; owner asked to continue after interruptions.
- Context / root cause: tasks/plan.md still deferred drafts/Saved/reminders/advanced search and document preview/voice/Markdown. Sync V2 discovery did not provision a missing runtime. Upstream UI mixed SQLite work with rendering and scanned parked account data; the port therefore uses worker-owned local operations instead of copying that UI. Upstream PDF stream decompression is unbounded in-process, motivating a separate parser process.
- Changes — runtime: new sync_runtime prepares the pinned public Zalo module before a phone request, credential-free with app User-Agent/public Referer, no redirects, 2 MiB cap, SHA-256, actual-host key generation and private atomic cache activation. A default urllib request returned 403; the production public headers returned 200/287049 bytes with SHA-256 398a2016ede72d1e604a6ac13bc7c62f2c44a43f0c1d7b996e968427380fc739. Preserve explicit overrides; unknown modules from another Zalo installation are not modified and do not replace the known runtime. No WASM blob redistributed or put in Git.
- Changes — productivity: new productivity module + native GUI controls for per-thread drafts (quote/mention/auto-mention), Saved notes, local reminders and bound-parameter advanced SQLite search. Transactional additive schema migration 2 preserves upstream-compatible text drafts/notes/reminders; test fixtures remain private. Saved flag writes are atomic. Worker terminal events are account-scoped, stale search results are generation-filtered and search is debounced; old-message lookup loads bounded context for exact-reference navigation. Reminder ticks claim once and never send into the conversation. Draft writes precede quit acknowledgement/account changes; ordered PersistThreads removes the new mutation/snapshot race. Removed the directly related UI-thread account snapshot call; Logout already owns that operation.
- Changes — documents/Markdown: explicit preview for supported PDF/Office/text attachments only, allowlisted bounded downloads, Office item/count/aggregate actual-read caps and text limits. PDF uses the same Rust executable's early internal helper, bounded stdin/stdout, a deadline and Unix CPU/data limits; no session initialization in helper. Added pinned quick-xml/pdf-extract dependencies. Markdown ports pure code/table/list parsing and copy/plain-text/star preferences while retaining the fork's mention/emoji/font/quote rendering.
- Changes — voice: new native capture thread with advertised F32/I16/U16 input, phase-preserving mono 16 kHz resampling, five-minute cap, explicit stop/send/cancel and fixed destination. Managed FFmpeg encodes private AAC ADTS; a process guard, cancellation token, generation/account checks and cleanup-on-drop cover normal error/stale-result paths. Output is owner-only before encoding starts. Voice upload retains the standard Session cipher/sign/trust path and upstream 12060/12063 voice/upload contract. Incoming voice takes precedence over generic fileSize metadata and normalizes even short millisecond duration; missing duration is not invented. No live message or call was sent by the agent.
- Verification: 151 relevant module tests passed, 0 failed, 1 existing live-call test ignored: productivity 7, document 9, voice 4, GUI 44, store 10, chat 27, history 19, runtime 1, zproto 6, video tools 5, call 19 (+1 ignored). Relevant reruns followed new changes only. Actual pinned WASM was supplied via CTDOZLITE_ZPROTO_WASM and activated in a private fixture; this was not a silent missing-module skip. Actual publisher-hash-verified ARM64 FFmpeg encoded/decoded synthetic audio and checked temporary permissions/cleanup. File-based temporary DB fixture proves quit acknowledgement follows draft persistence and a reopened connection retains quote/mention metadata. Synthetic PDF/Office, archive expansion, SQL literal/injection/boolean/date bounds, atomic rollback and exact old-message context tests passed. Final cargo check --message-format=short exit 0, no warnings; git diff --check exit 0. No real session/history files used as test inputs.
- Documentation: tasks plan/todo updated; docs/HOAN_THIEN_PLAN_2026-11-13.md describes entry points, limits and acceptance checks; re-notes/45_remaining_integration.md records protocol/source evidence. Feature table/changelog and earlier sync-note follow-ups updated. Preserved all pre-existing uncommitted work; no commit, push, release or installed-app replacement.
- Status: all agreed source implementation groups delivered. Still explicitly unverified: actual Android/iPhone confirmation/import/current phone backup schema, real Zalo voice/video send/receive, microphone permission/device behavior, native GUI relaunch/layout of the new controls, PDF helper through a packaged executable, and Windows/Linux runtime/install paths. Tasks requiring these devices/packages stay unchecked; offline codec/parser tests do not establish live interoperability. Existing app process/binary has not been replaced.

## 2026-11-13 — Inline playback for received Zalo videos (agent: Codex)
- Request: distinguish received videos from generic files and play them directly in the native app, with careful loading and memory handling.
- Context / root cause: `parse_chat_message` classified `chat.video` payloads that contained `fileName`/`fileSize` as generic `Tập tin Zalo` before its video fallback ran, so the UI exposed an opaque storage key without an extension. The current native egui client had image textures but no message-video playback path. Public Zalo help documents Photos/Videos separately from Files, but does not publish payload, buffering or codec internals.
- Changes: video payloads now preserve video kind, usable filename/extension and trusted thumbnail/source URLs. Message bubbles show a bounded 16:9 poster and explicit play affordance; clicking opens one native modal player. The worker downloads only after the click into an account-isolated private cache, reports progress, supports cancellation and validates the Zalo allowlist. A managed local FFmpeg process decodes a fixed 960x540 RGBA stream and native audio; only one video is active, only one unconsumed frame can cross to the UI, audio is capped at two seconds, and close/replacement kills and reaps both processes. Preview encoded bytes, decoded image allocation, original-video cache and timeouts are bounded. Added parser/UI/resource/decoder tests, protocol note, docs and changelog entries.
- Verification: reproduced the parser failure first (`Tập tin Zalo` instead of `Video Zalo`), then fixed it. `cargo check --message-format=short` passed. Focused video tests: 11 passed. Full `cargo test --locked`: library 334 passed / 2 ignored / 0 failed; integration suites 6 + 9 + 14 + 20 passed; doc tests passed. A real local FFmpeg synthetic MP4 produced exactly one 2,073,600-byte RGBA frame and 384,000 bytes of float audio. Full Clippy remains red on the existing dirty worktree with many unrelated warnings; its two findings in the new player were fixed and check/tests rerun. `git diff --check` is clean for this task's files but still reports the pre-existing `AGENTS.md:98` trailing space. NOT verified: an authenticated live Zalo video, expired source URL, HEVC/vertical media, native GUI interaction, packaged app playback, or Windows/Linux behavior.
- Status: source implementation and offline verification complete; live Zalo/packaged-GUI acceptance remains pending. Existing uncommitted work was preserved; no commit, push, release or installed-app replacement.

## 2026-11-13 — Finish live-regression batch left by Codex: Android sync errors, Group/Cloud -69, video/voice CDN, notification avatar, phone lookup (agent: Claude Code, claude-sonnet-5-5)
- Request: continue Codex's interrupted work on 7 live issues (Android sync failure, Group/Cloud error -69, video shown as file, voice must play in-app, remove 4 bottom buttons + broken recording, blank notification logo, phone lookup code 212).
- Context / root cause (from Codex evidence + checks): real videos use hosts `*-zvc.dlmd.me` / `video-stal-*.dlmd.me` and a generic wire type, so the earlier fixture-based fix missed them; `getrecentv2`/`getoldv2` sent `src` 1/3 while the checked-in Zalo PC bundle defaults to -1 (strong -69 candidate, not live-verified). Android failure text was a fixed string that hid the failing step. Codex's notification avatar was cached at a RELATIVE path while `helper_payload` only accepts absolute paths, so the attachment was silently dropped.
- Changes: `src/history_sync/android.rs` — `AndroidStage` enum + `android_failure_message` (user sees the failing step: URL / key / download+HTTP status / decode / identity mapping / DB write, never URLs or content); backup URL may now be on a media CDN via cookie-free client (cookies still only to core Zalo domains). `src/gui/worker.rs` — `notification_avatar_dir()` now absolute. `src/chat.rs` — `phone_lookup_error_message` (212/216 explained; no protocol change, bundle params match). `src/gui.rs` — `#[allow(dead_code)]` on dormant `voice_ui` (no entry point after bottom bar removal). Kept Codex's changes: src=-1 for cloud history, `is_trusted_zalo_media_url`, in-app audio playback, video kind detection, DailyNotifier attachment/icon, bottom toolbar removal.
- Verification: `cargo test --locked` 341 lib passed / 2 ignored / 0 failed + integration 6+9+14+20; `cargo check --all-targets` clean; DailyNotifier built via build-helper.sh into scratchpad, `--self-test` PASS, AppIcon.icns present. New tests: android failure message, absolute avatar dir, phone lookup 212/216. `cargo clippy -D warnings` already fails crate-wide with ~105 pre-existing lints (unchanged by this work).
- NOT verified: real phone/Android backup import (the actual failing stage is unknown until the next run shows the new message), live Group/Cloud paging with src=-1, live CDN video/voice playback, notification showing the sender avatar in Notification Center, phone-lookup 212 root cause (server answer; likely unregistered/hidden number). Voice recording stop/cancel bug undiagnosed — hypothesis: VoiceStop queued behind a long-running command in the worker loop; feature is now unreachable.
- Status: partial — source complete and offline-verified; needs a rebuilt app + live retest. No commit/push/release.

## 2026-11-13 — Second live-test round: -69, Android 404, toast, video bubble, fType, search button, forward order (agent: Claude Code, claude-sonnet-5-5)
- Request: owner's live results — notification still shows blank logo + avatar; search-by-phone button shifts left while typing; video/voice UI should follow Zalo (inspect Cloud in browser); Android sync HTTP 404 not visible in --dev; Group/Cloud still -69 on getoldv2; dropped video shows as "Thư mục" on phone and bubbles drift right; forward list should be recent-first.
- Root causes / evidence: (1) `fType == 2` is "folder" in the Zalo bundle (`STR_DOWNLOAD_FOLDER_WEB`); `send_attachment_ext` set 2 for videos. (2) Video bubble width was measured from the title (min 135) but the poster card drew `clamp(220,340)`, overflowing the frame by a per-message amount. (3) `ui.put(edit_rect)` rewinds the horizontal layout cursor, so the add-friend button moved left by the 26px clear-button gap while typing. (4) -69 in the bundle: cloud API controller caches -69 per (cursor,isOld,conv) as "stop retry"; message search treats -69 as "no more" — not a global failure. (5) macOS native banners always draw the app icon; avatar can only be an attachment, so a self-drawn toast is needed.
- Changes: `chat.rs` fType=1 (`ASYNC_FILE_TYPE_FILE`); `gui/messages.rs` `video_card_width`/`is_video_card` shared by measure+draw; `gui/workspace.rs` add-friend button pinned to search box rect; `gui/modals.rs` forward targets sorted by `forward_recency_ms`; `history_sync.rs` -69 on an OLD page ends that thread's history (first/recent page -69 stays an error); `history_sync/android.rs` dev-log of download host+path+status (query redacted); `native/macos/DailyNotifier.swift` new `--toast` mode (avatar circle or lettered fallback, stacking, 6s auto-dismiss, click opens app, rounded shadow mask); `native_notifications.rs` tries toast first, native `--notify` fallback.
- Verification: `cargo test --locked` 345 lib passed / 0 failed + integration 6+9+14+20; `cargo check --all-targets` clean; helper built, `--self-test` PASS; toast rendered and screenshotted with avatar, lettered group fallback and stacked toasts. New tests: -69 end-of-history, fType, video card width, forward recency, redacted download target.
- NOT verified: live -69 behaviour (treating it as end-of-history is evidence-based but unproven; if it is really a rate limit, history may be incomplete), Android 404 cause (host/path now visible in --dev), real video playback layout, search button after fix in the running app, real toast click-to-open. Video sent as file (asyncfile); Zalo's dedicated `/api/message/video` flow not implemented. NOT done: video/voice player UI redesign — Agent Chrome had no connected browser, so Zalo's real UI could not be inspected.
- Status: partial. No commit/push.

## 2026-11-13 — Zalo Cloud-matched video and voice playback UI (agent: Codex)
- Request: use BrowserSkill with Chrome profile `f8e97645` to inspect the Zalo Cloud media UI that the previous agent could not access, then implement the omitted video/voice UX work.
- Context / root cause: live Zalo Cloud showed video posters preserving source aspect ratio and opening a dark viewer with play/pause, elapsed/total time, seek and volume. Voice messages use one compact bubble with a circular play/pause control, waveform, elapsed time and played/unplayed colors. The Rust UI instead forced every poster to 16:9, rendered voice as a label plus ordinary button, and the video window exposed no playback controls or duration.
- Changes: `src/gui/messages.rs` now preserves loaded poster ratios, adds a duration badge, and renders voice playback as a waveform card with live progress. `src/gui.rs`, `src/gui/modals.rs`, `src/gui/workspace.rs`, `src/gui/worker.rs` and `src/video_player.rs` now provide a dark full-screen viewer with play/pause/replay, current/total time, seek and live volume; seeking restarts bounded FFmpeg decoders at the requested offset, duration is probed from the private cached file, stale generations are ignored, and voice elapsed state is read through a short mutex guard. `CHANGELOG.md` and `docs/TINH_NANG.md` describe the current boundary.
- Verification: BrowserSkill session `takm` used the requested signed-in profile and inspected actual video/voice states in My Documents, then closed cleanly. TDD RED first failed on missing layout/waveform/time helpers; focused media tests then passed (16). `cargo fmt --all -- --check`, `cargo check --message-format=short`, and `cargo build --release --locked --bin ctdozlite` pass. Full `cargo test --locked` passes outside the filesystem sandbox: 349 passed, 0 failed, 2 intentionally ignored, plus all integration binaries (6 + 9 + 14 + 20 tests). The first sandbox run had one expected loopback-bind denial, which passed outside sandbox.
- Status: source and release binary complete; live Zalo Cloud was observed, but the rebuilt native app has not yet been exercised against a real received portrait video/voice message, so visual/playback acceptance in ctdozlite-next remains pending. No commit, push, installed-app replacement or Zalo message send performed.

## 2026-11-14 — Fix live voice controls and video viewer geometry (agent: Codex)
- Request: remove the redundant voice-loading toast, vertically center the active pause control, and fix video playback that had audio but no visible image while the seek bar extended outside the app.
- Context / root cause: a real cached Zalo `.media` file was probed as AAC + H.264 (1280x720, 16.15 s), produced a visible frame, and the exact player FFmpeg filter emitted one 960x540 RGBA frame, so download and decoding were healthy. The viewer nested centered horizontal layouts and derived control widths from the remaining cursor width; this displaced/covered the frame and let the seek bar consume the screen. The pause state used a font glyph with baseline-dependent vertical placement.
- Changes: removed the voice-loading toast in `src/gui/workspace.rs`; replaced the voice play/pause glyph with centered vector geometry in `src/gui/messages.rs`; rebuilt `render_video_player_modal` in `src/gui/modals.rs` around bounded absolute rectangles, direct aspect-fit texture painting, and constrained play/seek/time/volume controls. Added pure layout/aspect-fit helpers and regression tests. Updated `CHANGELOG.md`.
- Verification: TDD RED first failed because the new pause/layout helpers did not exist. Focused video tests passed 18/18; `cargo fmt --all -- --check` and `cargo check --message-format=short` passed. Full `cargo test --locked -q` passed: library 351 passed / 2 ignored / 0 failed, integration 6 + 9 + 14 + 20 passed. `cargo build --release --locked --bin ctdozlite` passed; scoped `git diff --check` passed. NOT verified: the corrected layout in a relaunched native app against the user's received video; the currently running PID still uses the older binary.
- Status: source and release binary complete; owner should quit the current app, relaunch `target/release/ctdozlite`, and visually retest voice/video. Existing unrelated uncommitted changes were preserved; no app process was stopped and no commit/push/install was performed.

## 2026-11-14 — Prepare v0.3.0 release snapshot (agent: Codex)
- Request: after confirming the corrected voice and video playback works in the relaunched app, commit the accumulated implementation, push `main`, and publish a new version tag.
- Context / root cause: the tree contains backward-compatible user-visible additions since v0.2.1 (phone sync/runtime work, productivity/document/voice features, inline video/voice playback and notification/UI fixes), so SemVer requires a minor release rather than a patch. `main` and `origin/main` were even after fetching tags. The stray `ppp` line in `AGENTS.md` was excluded. Ignored local `session.json` and SQLite databases remain untracked.
- Changes: bumped the crate and lockfile to 0.3.0; moved the curated Unreleased notes to `2026-11-14`; prepared one integrated release snapshot because the accumulated GUI/worker/protocol changes overlap across files. User explicitly authorized commit, push and tag creation.
- Verification: live acceptance by the owner covers the final voice/video regression. Final `cargo fmt --all -- --check`, `cargo check --all-targets --message-format=short`, full `cargo test --locked -q` (351 passed / 2 ignored plus integration 6 + 9 + 14 + 20) and `cargo build --release --locked --bin ctdozlite` passed at version 0.3.0. Release-note extraction produced the expected v0.3.0 section; `git diff --check` passed. Crate-wide strict Clippy still has the previously documented legacy lint backlog and was not weakened.
- Status: v0.3.0 release snapshot is verified and ready for the explicitly authorized `main` push plus annotated `v0.3.0` tag; GitHub Actions will perform cross-platform packaging after the tag reaches origin.

## 2026-11-14 — Fix v0.3.0 macOS packaging test and add landing page (agent: Claude Code, claude-sonnet-5-5)
- Request: check the v0.3.0 release, then build a landing site for ctdozlite-next modelled on ctdozlite.ctdo.net but with its own design.
- Root cause: `tests/native_packaging.py` synthetic bundle lacked `DailyNotifier.app/.../AppIcon.icns`, which `validate-native-bundle.py` requires, so the macOS job failed and "Publish GitHub Release" was skipped. Fixed the fixture, pushed `main`, force-moved tag `v0.3.0` (no release existed yet; owner approved).
- Changes: `tests/native_packaging.py`; landing page `site/` was drafted then deleted at owner's request (never committed).
- Verification: icon assertion gone locally; full arch matrix only runs in CI (local build is arm64-only). The CI run on the tag was still in progress.
- Status: partial. No site remains.

## 2026-11-14 — Run in background when the window is closed (agent: Claude Code, claude-sonnet-5-5)
- Request: pressing X exited the app, so message/call notifications stopped; keep the app running.
- Changes: new `src/gui/background.rs` (`hide_app` = macOS `NSApp hide:`, else minimise; `show_app`
  = unhide + activate + focus). `src/gui.rs`: X button and `close_requested` now `CancelClose` +
  hide (quit only via Ctrl+Q or updater `allow_quit`); incoming call calls `show_app`; a 500ms
  `request_repaint_after` keeps `check_events` draining worker events while hidden.
  `Cargo.toml`: objc2-app-kit features `NSApplication`, `NSResponder`, `NSRunningApplication`.
- Verification: `cargo check` passes. NOT verified at runtime: that egui keeps updating while the
  app is hidden (if notifications stop after hiding, move event draining off the UI loop), and
  that Dock click restores the window. No tray icon / "quit" menu yet; Ctrl+Q is the only quit.
- Status: partial, pending user test. No commit.

## 2026-11-14 — Tray icon for background mode (agent: Claude Code, claude-sonnet-5-5)
- Request: X should hide the window but leave a tray icon (Windows "^" area) with the app still receiving messages/calls; same on other OSes.
- Changes: new `src/gui/tray.rs` (tray-icon 0.26, Windows+macOS only; menu "Mở Zalo"/"Thoát", left click restores; events set atomics consumed in `update`). `background.rs`: with tray → `Visible(false)`; `show_app` also `Visible(true)`. `gui.rs`: `_tray` field, tray show/quit handling. `Cargo.toml`/`Cargo.lock`: tray-icon dep (win/macOS only).
- Verification: `cargo check --locked` on macOS only. NOT verified: Windows build/behaviour, that egui keeps updating while hidden on Windows, macOS menu-bar icon. Linux has no tray (needs GTK/appindicator libs) and falls back to minimise.
- Status: partial, pending user test. No commit.

## 2026-11-14 — Release v0.3.1: build warnings + background mode (agent: Claude Code, claude-sonnet-5-5)
- Request: fix the small v0.3.0 build warnings, then publish v0.3.1 with the background-run feature.
- Changes: removed redundant `package.homepage`; `src/chat.rs` `fetch_update` → CAS loop (`try_update` needs Rust 1.95, MSRV is 1.75); `.github/workflows/ci-release.yml` actions bumped to Node 24 lines (checkout@v5, upload-artifact@v6, download-artifact@v7, attest-build-provenance@v4, action-gh-release@v3); version 0.3.1; CHANGELOG. The "ubuntu-latest → Ubuntu 26" and "macOS arm64 queue" annotations are informational, left alone.
- Verification: `cargo check --locked`, `cargo test --locked` (351 pass). `cargo clippy -D warnings` already fails with 121 pre-existing errors (not touched). CI action major bumps NOT verified until the tag workflow runs.
- Status: tagged v0.3.1 and pushed (see git log).
## 2026-11-14 — Tin nhắn nhanh (zlite#2)  (agent: Codex, GPT-5)

- Request: Implement Kanzo task `zlite#2`: remove quick replies attached to rejecting calls and add configurable slash-command quick messages with optional images.
- Context / root cause: the existing feature was coupled to the incoming-call rejection overlay and could automatically send a fixed text. The requested flow instead stores reusable messages in Settings and inserts a selected `/shortcut` result into the composer without sending it.
- Changes: added the bounded, validated quick-message model and private managed-image storage in `src/quick_messages.rs`; added Settings management/editor UI and persisted settings in `src/gui.rs` and `src/gui/modals.rs`; added background image import/delete commands in `src/gui/worker.rs`; added slash filtering, keyboard/mouse selection, and draft insertion in `src/gui/workspace.rs`; removed the call-rejection quick-reply UI; updated `src/lib.rs`, `CHANGELOG.md`, and `docs/TINH_NANG.md`.
- Verification: `cargo fmt --all -- --check`, `git diff --check`, and `cargo check --message-format=short` pass. `cargo test --locked quick_messages --lib` passes (13/13). Full `cargo test --locked` passes (413 passed across unit/integration suites, 2 repository-declared ignored, 0 failed). NOT visually verified in a running native GUI and not committed or pushed.
- Status: done, pending owner GUI review. Kanzo status update may still require connector write approval.

## 2026-11-14 — Release v0.3.2 with quick messages  (agent: Codex, GPT-5)

- Request: after the owner confirmed the feature works in the native GUI, commit and push the implementation, tag it, and publish `v0.3.2`.
- Context / root cause: `main` and `origin/main` were even at `v0.3.1`; `v0.3.2` did not exist locally or remotely. The release workflow requires the tag, `Cargo.toml`, `Cargo.lock`, and a versioned CHANGELOG section to agree.
- Changes: bumped the crate and lockfile to `0.3.2`; moved the curated quick-message notes from Unreleased into the `0.3.2` section. The release includes configurable slash-command quick messages with an optional managed image and removes call-rejection quick replies.
- Verification: owner confirmed the feature works in the native GUI. `cargo fmt --all -- --check`, `git diff --check`, `cargo metadata --locked --no-deps`, release-note extraction, `cargo check --locked --all-targets --message-format=short`, full `cargo test --locked` (413 passed, 2 repository-declared ignored, 0 failed), and `cargo build --release --locked --bin ctdozlite` all pass at version `0.3.2`.
- Status: release snapshot verified; owner explicitly authorized commit, push, tag, and publication. GitHub Actions will build and publish the cross-platform artifacts after `v0.3.2` reaches origin.

## 2026-11-17 — Zalo Web parity batch: folder picker, search Esc, image copy menu, video fallback, photo albums, cliMsgId (agent: Claude Code, claude-sonnet-5-5)
- Request: nine items (react not reaching server, album layout, quote reply sent as plain text, image copy menu, video fallback buttons, download-folder picker, incoming calls, Esc/nav closes search, Zalo Web sticker panel). Owner asked to study Zalo Web via BrowserSkill (My Documents) and to code in Rust only (C only if impossible).
- Done (compiles, `cargo test --lib` 369 passed; NOT verified visually/live):
  - Item 6 root cause: `pick_dl_dir` called `pick_native_file_dialog(false)` (a *file* picker) and took the file's parent, and ran it on the UI thread. Added `pick_native_folder_dialog()` (macOS `choose folder`, Windows `SHBrowseForFolderW` via Rust FFI, zenity/kdialog) run on a helper thread (`dl_dir_pick_rx`). Windows/Linux branches not compiled on this Mac.
  - Item 8: Esc in the sidebar search clears it and drops focus (skipped while IME busy); clicking the "Tin nhắn" nav icon clears it too.
  - Item 4: image viewer copy is a split button: click = saved default (`image_copy_default`: image|link), caret menu = copy image / copy link / choose default. Image copy reads `stickers_cache/<key>.png` (raw CDN bytes despite the extension), re-encodes to PNG, writes the clipboard on a helper thread (`gui/clipboard_write.rs`: NSPasteboard, Windows CF_DIB+PNG via FFI, wl-copy/xclip).
  - Item 5: failed video player shows "Sao chép URL video" and "Tải video về" (`VideoPlayerModal.failed`, reuses `WorkerCmd::DownloadFile`).
  - Item 2: new `gui/album.rs`. Evidence: `threads_cache.json` album photos carry `content.params` `is_group_layout:1`, `group_layout_id`, `id_in_group`, `total_item_in_group`, `width`, `height` (and `extraData.groupMediaMsg.groupLayoutId`). Zalo Web row split measured by sending mixed-aspect test photos to My Documents: n=2→[2], 3→[3], 4→[2,2], 5→[2,3], 6→[2,2,2], 7→[2,3,2], 10→[2,3,2,3]; every row is justified to full width at equal height, ~2px gaps, HD badge top-left. Implemented as a row-aspect DP (target 2.4) with the alternating-2/3 tie-break; tests pin those splits. Consecutive same-sender photos with the same layout id collapse into the head message's bubble (`album_runs`); reply/react/forward on an album act on its first photo. Photos sent one by one are unchanged. 22-photo albums seen on the phone show 4 per row; that case was not measured on Zalo Web (my DP gives ~3), so large albums may differ.
  - Items 1 and 3 (fix applied, NOT verified live): `send_reaction` used `msg.ts` as `cMsgID` and `send_quote_message` used `ts` as `qmsgCliId`; live capture shows `cliMsgId` 1794398852870 vs `ts` 1794398854583 for the same photo, so the server could not match the original. Now both use the real client id (`wire_cli_msg_id`, falls back to ts when absent); quote owner uses the wire sender uid (group display names made it "0"). The silent plain-text fallback in `send_quote_message_with_mentions` still exists but now logs the reason (`logger::log_error`, dev mode).
- Live test blocked: session in `~/Library/Application Support/ctdozlite/session.json` had `expired_at` 1794900602 (past), `session_status()` = Expired, server error 600. Temporary probe + session copy were deleted. Needs a fresh login to verify items 1 and 3.
- Not done yet: item 7 (incoming call UI/accept + missed-call after phone answer), item 9 (Zalo Web sticker/emoji/GIF panel). Both need design decisions/captures; see conversation.
- Notes: Zalo Web test messages (albums of 2..10 solid-colour images) were left in My Documents. `cargo clippy -D warnings` already fails on pre-existing code; not touched.

## 2026-11-17 — Launch debug GUI for live reaction/reply verification (agent: Codex)
- Request: open the app so the owner can log in and test the unverified reaction and quote-reply fixes.
- Context / root cause: the previously saved session is expired; live verification requires an interactive QR login.
- Changes: no source changes; started BrowserSkill against the explicitly requested Main instance `f8e97645`, then launched `cargo run --locked -- --gui --dev` and kept the GUI process running.
- Verification: BrowserSkill connected successfully (session `trkj`); the Rust build completed and the native GUI process remains active. Login and live server behavior are not yet verified.
- Follow-up: the owner completed QR login and the native session is now valid. Zalo permits only one of Zalo Web or ctdozlite-next to be logged in at a time, so the BrowserSkill Zalo Web session was closed without logging in; future Web inspection must happen separately after exiting the native app.
- Live evidence: the debug log recorded `POST /api/group/quote` returning `error_code: 0` / `Successful`. Phone-side quote rendering is still awaiting owner confirmation; reaction has not yet been confirmed.
- Live verification: the owner confirmed on the phone that (1) the ctdozlite-next reaction appeared as exactly one real heart and (2) the reply displayed the correct quoted-message frame. This verifies both server propagation and official-client rendering.
- Status: done — items 1 and 3 are live-verified. The debug GUI remains running; no additional source changes, commit, or push were made.

## 2026-11-17 — Item 9: Zalo Web style Sticker / Emoji / GIF panel (agent: Claude Code, claude-sonnet-5-5)
- Request: resume the interrupted Codex session and implement item 9 (the sticker/emoji/GIF panel as on Zalo Web PC). Reference: owner's screenshot plus the Codex session's measurements (panel ~344x424, tabs STICKER/EMOJI/GIF, expand button, search pill, "Gần đây" section, bottom pack bar with clock/packs/arrows/"+"; emoji inserts without sending, sticker/GIF send).
- Changes: new `src/gui/picker.rs` (floating `egui::Area` panel anchored above the composer; replaces the old inline sticker drawer `render_sticker_picker` and its 268 px layout reserve, both removed from `stickers.rs`/`workspace.rs`). Sticker tab: search pill (debounced `SearchStickers`), recent + popular, per-pack view, store ("+"), scrollable pack bar. Emoji tab: ~340 unicode emoji in 6 groups drawn as colour textures via `RasterizeEmoji`; a click inserts at the caret and keeps the panel open. GIF tab: trending/search grid reusing `gif_results`/`SearchGifs`, send via `SendGif` with new `chat::GIF_SRC_CHAT_BAR` (=1, from re-notes/39 `SGG_SRC_*`). Recents persist in `AppSettings` (`recent_stickers`, `recent_emojis`). Esc or a press outside the panel closes it. `messages::system_emoji_texture` is now `pub(super)`.
- Verification: `cargo check`, `cargo test --locked --lib` (373 passed, 4 new in `picker.rs`: recents, caret insertion with multibyte/ZWJ, emoji list uniqueness, grid columns). No clippy findings in `picker.rs`. rustfmt run on `picker.rs` only.
- NOT verified: anything visual or live (no GUI run). Guesses that differ from or go beyond Zalo Web: expand button just enlarges the panel (Web moves it into the right info column); emoji tab has no search box; sticker/GIF send closes the panel (Web behaviour not measured); GIF cells are a fixed 2-column 4:3 crop, not Web's layout; `src` 1 for the GIF box comes from the bundle constants, not a capture of the panel send.
- Not done: item 7 (incoming call UI / missed call). Nothing committed.
- Status: partial, pending owner GUI review.

## 2026-11-17 — Migrate from ctdozlite v0.1.2, rename to Zalo-next, repo cleanup  (agent: Claude Code, claude-sonnet-5-5)
- Request: evaluate upstream v0.1.2 and migrate what is better (call, idle CPU, appearance, AI assistant idea, macOS data dir/CI), rename project + app to "Zalo-next" with the new Logo.png (icon sized like other Dock icons), untrack the zstd build dir.
- Findings: upstream `call.rs`/`audio.rs` are equal or weaker than ours (nothing ported); the Sync V2 `due_at` error is a schema collision on a *shared* DB (upstream `saved_items` has `due_at`, ours `productivity.rs` does not; both also use `PRAGMA user_version = 2`) — ours is self-consistent, never point upstream at our DB. `ThreadsUpdated` is only sent when polling returned new messages (`retain(!messages.is_empty())`), so upstream's poll fingerprint is not needed here. Old icon `app_icon.png` was 256 px full-bleed (no margin) and also drives the runtime Dock icon via `ViewportCommand::Icon`.
- Changes (stacked branches, one commit each, nothing pushed): `chore/ignore-native-build` (untrack 654 files, `.gitattributes`), `chore/rename-zalo-next` (display name, `zalo-next` package/bin, `Zalo-next.app` + `Contents/MacOS/Zalo-next`, `setProcessName`, release assets `zalo-next-*`, deb/rpm Replaces/Obsoletes, cask `zalo-next`, latest.json now points at this repo, `scripts/build-app-icon.sh` + 1024/512 padded icon + .ico; bundle ids and `Application Support/ctdozlite` kept on purpose), `feature/appearance-settings` (upstream palette, font/scale/animation/colour presets, cached palette, "system" theme), `feature/call-cards` (legacy `sendBubbleMessage` / cancelled / missed / hours; bare `mm:ss` trusted only with call metadata), `feature/idle-cpu` (heartbeat 3 s when focused, cached sort keys), `feature/macos-data-dir` (Application Support for every macOS run + non-destructive CWD migration), `docs/ai-assistant-design` (docs/AI_TRO_LY.md, ADR-0004, tasks/ai-assistant.md, INV-5/6).
- Verification: `cargo test --locked` all green (386 lib + integration); each commit's staged tree compiled in isolation (user's uncommitted WIP in chat.rs/gui.rs/modals.rs/workspace.rs/messages.rs/stickers.rs/macos_clipboard.rs was left unstaged). `tests/native_packaging.py` architecture-matrix cases fail locally only because `native/macos/build` helpers are arm64-only (test expects CI's fat binaries); rename-related cases pass.
- NOT verified: anything visual or live (new icon in Dock next to Word/PowerPoint, menu reading "Zalo-next", Appearance page, system-theme follow, migration on the real data dir), the idle-CPU gain (no before/after measurement), Sync V2 call-log import (needs the proto attach parser from upstream `parse_proto_metadata`/`format_sync_v2_call_text` plus a capture, per AGENTS protocol rule), the Sync V2 ack `reason` string-vs-int question, `.deb/.rpm/.dmg/Windows` packaging (CI only), Homebrew cask move (tap repo not touched).
- Status: partial — pending owner review, push, and repo/cask rename decisions. `.agents/WORKLOG.md` still contains earlier uncommitted entries from other agents, so this entry is not committed.

## 2026-11-17 — Review/commit WIP, Sync V2 call logs + ack, repo rename, local @AI assistant  (agent: Claude Code, claude-sonnet-5-5)
- Request: owner renamed the repo to `zalo-next`; review and commit the other agents' uncommitted work; authorised an AGENTS.md exception for Sync V2; push to main; implement the AI assistant for a full local test.
- WIP review: album/picker/clipboard/reaction-quote ids were complete (builds, 386 tests, clippy-clean new files, fmt ok, no I/O on the UI thread) → committed `ec163b4`. Not done by that agent and still open: incoming-call UI (item 7). Windows/Linux folder-picker and clipboard FFI were never compiled on this Mac.
- Sync V2 (owner exception, `AGENTS.md`, `re-notes/47_sync_v2_call_logs.md`): decode message metadata (field 8) → call attachments become `chat.ecall` rows with canonical text (re-sync upgrades old plain "0:18" rows); `cmd 591` ack reason is now an integer (0 cancel, 1 success, 2 error) per upstream/`app.asar`. Offline fixtures only; NOT verified on a phone.
- Repo rename: every `minhkhang1008/ctdozlite-next` reference → `minhkhang1008/zalo-next` (updater `GITHUB_REPO`, install script, cask, CI manifest, docs); `origin` URL updated.
- Assistant (`src/ai.rs`, `src/gui/ai_ui.rs`, ADR-0004 accepted): dedicated HTTP client (no cookies/redirects, https or loopback), key from `CTDOZ_API_KEY` or `ai_secret.json` (0600), `local_ai_messages` table, GUI map outside `Thread.messages` (INV-5 by construction), quote/draft guards, context budget + map-reduce summaries, per-thread sharing opt-in with first-time group confirmation. Tests: parsing/URL/prompt/storage units, fake-HTTP tests (bearer, no cookie/Zalo headers, 401/429/redirect/network), worker end-to-end with temp SQLite, headless bubble render. 459 tests pass.
- NOT verified: any real provider, the visible UI (settings tab layout, bubbles, composer hint, confirmation dialog), keyboard flow in groups, Windows/Linux. Not done: streaming, Keychain storage, AI messages in search, persona/name customisation.
- Test hygiene note: an early version of the worker test wrote two rows into the repo-root `zalo_history.db` (relative path); the rows were deleted and `run_ai_ask` now takes a `db_path`.
- Status: done pending owner's local test; pushed to `main`.

## 2026-11-17 — Incoming calls fixed, @AI pre-read, release v0.4.0  (agent: Claude Code, claude-sonnet-5-5)
- Incoming call UI: the event reached the GUI but the call window (a separate OS viewport) opened behind the main window that `show_app` was focusing → now always-on-top + focused while ringing. Log instrumentation (`[call] GuiTx.send / check_events / GUI received`) kept. Verified live by the owner.
- Phone said Zalo-next "does not support video calls": `voicecall` endpoints now send the PC pair `zpw_ver=693&zpw_type=24` by default (`call.rs::voicecall_zpw`, `ZCALL_ZPW` override kept; `re-notes/48`). Verified live (call accepted). Still open: phone camera not shown in Zalo-next, our video blurry/delayed → `tasks/video-call-quality.md`.
- @AI pre-read (`docs/AI_TRO_LY.md` §11), notification regression root cause (helper binaries deleted by the ff-merge, rebuilt), release 0.4.0 (CHANGELOG, Cargo.toml/lock).
- Verification: 464 tests pass, release build ok. NOT verified: Windows/Linux packages (CI), pre-read with a real model, Homebrew cask auto-update at release.

## 2026-11-17 — Stray `@ai` mention in v0.4.0 release, README split, docs refresh  (agent: Claude Code, claude-sonnet-5-5)
- Request: the v0.4.0 release page showed GitHub user Andrey Sitnik; fix it, refresh the md files, make README professional with a clear user / developer split.
- Root cause: release notes (from `scripts/release_notes.py`) contained a bare `@AI`; GitHub treats it as a mention of the real account `ai` (also notifies them). CHANGELOG line 18 had the same bare mention.
- Changes: backticked `@AI` in CHANGELOG; `release_notes.py` now wraps any bare `@name` outside code spans (`neutralize_mentions`) with `tests/release_notes_test.py`; edited the live v0.4.0 release body (`gh release edit`) from the regenerated notes. README rewritten in two parts (users: highlights, install, getting started, privacy, trust; developers: stack, source map, build, CLI, rules, docs index). Refreshed `HUONG_DAN_SU_DUNG` (CLI flags `--profile/--check-update/--version/--proxy/--dump-raw`, new sticker/emoji/GIF panel, albums, calls, appearance/background, `@AI` moved into section 2.4), `TINH_NANG`, `KIEN_TRUC_HE_THONG` (picker/album/appearance/ai_ui/tray modules), `KHAC_PHUC_SU_CO`, `scripts/README`, ADR index (0004 accepted), `tasks/todo.md`.
- Verification: unittest for release notes passes; regenerated release body has no bare `@`. Docs were checked against `main.rs`/`modals.rs`/CHANGELOG, not by running the app.
- Status: done; nothing committed or pushed (owner to review). `log.txt` untouched.

## 2026-11-27 — Codex plugin setup check (`/codex:setup`)  (agent: Claude Code, claude-sonnet-5-5)
- Request: check that the local Codex CLI is ready for the `codex@openai-codex` plugin.
- Context: companion `setup --json` reports ready (node v26.12.1, npm 11.20.0, codex-cli 0.172.0, ChatGPT login verified, review gate disabled).
- Changes: none apart from this entry; no repo code touched.
- Verification: setup script output only; no Codex task or review was run.
- Status: done.

## 2026-11-27 — Rewrite AGENTS.md and docs/, set up Claude → Codex orchestration  (agent: Claude Code, claude-opus-5-5 (orchestrator) + Codex (read-only fact-check))
- Request: owner is migrating Zalo-next off ctdozlite. Scan the workspace, fully rewrite `AGENTS.md` and `docs/`,
  document the installed skills for agents, research cross-agent orchestration, and set up Claude Code
  (orchestrator) + Codex (worker and cross-reviewer).
- Owner decisions (asked in session): consolidate the old docs; upstream is used only when the owner asks for a
  port; agents may commit locally on their own, but pushing needs approval; Claude delegates to Codex
  automatically by rule, with the stop-time review gate off.
- Changes:
  - `AGENTS.md`: rewritten in English as the single source of truth. Covers commands (rustfmt on touched files
    only, no new clippy warnings), source map, non-negotiables scoped to reality, git policy, definition of done,
    upstream policy, multi-agent rules, and skills by situation.
  - `CLAUDE.md` (new): orchestrator playbook (delegation rules, brief template, review matrix, handling of
    findings). `scripts/codex.sh` (new): finds the Codex plugin companion so it can be called from Bash.
  - `docs/standards/`: 9 inherited files replaced by 8 repo-specific ones plus a README, each with a
    "known debt" section.
  - ADRs: notes added to 0001–0003; new ADR-0005 (independence from upstream) and ADR-0006 (multi-agent
    workflow).
  - Product docs: `KIEN_TRUC_HE_THONG`, `GIAO_THUC_ZALO` and `docs/README.md` rewritten. The dated
    `HOAN_THIEN_PLAN`, `UPSTREAM_REVIEW` and `VIDEO_TOOLS` were removed; their content went into
    `HUONG_DAN_SU_DUNG` (§2.3–2.5), `CAI_DAT`, `KHAC_PHUC_SU_CO` and `TINH_NANG`.
  - Link and wording fixes in `README.md`, `GLOSSARY.md` (INV-1 scope), the PR template, and `tasks/`.
- Verification:
  - every factual claim was grep-checked while writing;
  - Codex then fact-checked all the new docs read-only (`scripts/codex.sh task`, ~10 min, about 620 claim
    instances). It found about 28 wrong claims, all verified and fixed. Examples: Rust floor is **1.97** (wasmtime
    50), not 1.75; `reqqr`/`getServerInfo` send signed plaintext; imports can update rows; the updater hash is
    optional on some paths;
  - the relative-link check found no broken links;
  - the `scripts/codex.sh` failure path was tested (exit 127).
  No Rust code was changed, and no build or tests were run.
- New code issues found (documented in `docs/standards/security.md` §7, NOT fixed):
  - the updater's SHA-256 is optional for the Windows/Linux portable installs, and asset URLs are not
    host-checked;
  - the legacy image/sticker loader uses the session (cookie) client for any `http(s)` URL, with no size cap;
  - link-card thumbnails in incoming messages are fetched automatically;
  - `threads_cache.json` is not forced to `0600`;
  - `workspace.rs` deletes conversations via SQLite on the UI thread.
- Not done or open:
  - `.claude/` is gitignored, so no project `.claude/settings.json` was added;
  - the untracked `skills/system/run-workflow` (a `run_workflow` tool this harness lacks) was left alone,
    pending the owner;
  - `log.txt` untouched.
- Status: done; committed locally, not pushed.

## 2026-11-27 — Track shared Claude config, remove unknown skill folder  (agent: Claude Code, claude-opus-5-5)
- Request: owner asked to un-ignore project Claude config, and gave the agent discretion over the unknown
  `skills/system/run-workflow`.
- Changes:
  - `.gitignore`: `.claude/` → `.claude/*`, with `!.claude/settings.json` and `!.claude/agents/`.
    `settings.local.json` and the `skills/` symlinks stay ignored.
  - New `.claude/settings.json`:
    - allows `scripts/codex.sh`, `cargo check/test/clippy/build`, `rustfmt`, and read-only git;
    - denies Read of session/account/AI-key/Sync V2 identity/`*.db` files, both in the repo root and in
      `~/Library/Application Support/ctdozlite`.
  - `CLAUDE.md` §3 mentions the file.
  - Deleted the untracked `skills/` folder. It held one `run-workflow` SKILL.md (2026-11-24, unknown origin)
    describing a `run_workflow` tool this harness lacks; nothing referenced it.
- Verification: the JSON parses; `git check-ignore` confirms `settings.json` is tracked while
  `settings.local.json` and `skills` stay ignored. The deny rules were not exercised live, and a session restart
  may be needed for Claude Code to pick up the new project settings.
- Status: done; committed locally, not pushed.

## 2026-11-27 — Security fixes from security.md §7  (agent: Claude Code, claude-opus-5-5 (orchestrator) + Codex (worker, adversarial reviewer))
- Request: owner approved fixing the code issues documented in `docs/standards/security.md` §7.
- Workflow (first real use of CLAUDE.md): brief A (updater) → Codex write → Claude diff review + tests → commit;
  brief B (media) the same way; Codex adversarial review found 5 more issues, all verified; brief C (follow-up)
  → Codex, with 2 scope expansions approved; Claude fixed one regression itself; a final adversarial review
  (round 2) found 3 more, of which Claude fixed one and the rest were recorded as debt (2-round cap).
- Changes:
  - `a42ab7d` updater: SHA-256 is mandatory on every path; `VerifiedRelease` type; asset/checksum URLs pinned to
    `github.com/minhkhang1008/zalo-next/releases/download/`; https-only, no cookies, at most 3 redirects.
  - `ab040f1` media:
    - the legacy image loader uses the session client only for `is_trusted_zalo_url`, and the cookie-free
      client otherwise;
    - bodies are capped;
    - link thumbnails from non-Zalo hosts need a click;
    - thread snapshots are written `0600` atomically.
  - `676e3a6`:
    - the consent gate now covers photos, albums, video posters, the shared-media drawer and forward previews
      (shared `MessageThumbnail`);
    - failed thumbnails are no longer re-requested every frame (`failed_message_thumbnails`, cleared on account
      switch);
    - github.com redirect hops stay inside the repo;
    - `zpw_sek` is seeded with `Secure`;
    - snapshot write failures are logged;
    - Claude kept the user's own `local://` sent images auto-loading (Codex's version required a click).
  - Last commit: `hd_` originals get a 64 MiB cap instead of 16 MiB, fixing a regression the round-2 review
    found. `security.md` §7 now lists what was fixed and what is still open.
- Verification:
  - `cargo test --locked` all green: 424 lib + 7 + 9 + 14 + 20 integration, run outside the Codex sandbox,
    which denies loopback binds;
  - no clippy warnings on changed lines (checked against the diff ranges).
  - NOT verified: GUI look of the "Hiện ảnh" placeholders, live Zalo photo/sticker/thumbnail loading (risk:
    Zalo CDN URLs that redirect, or `http://` Zalo images, now need a click or fail), a real update via GitHub
    (redirect chain to `release-assets.githubusercontent.com`), and Windows/Linux.
- Open (in `security.md` §7):
  - redirect hops on automatic session-client image loads;
  - an album cell can't open the original after its thumbnail failed.
- Status: committed locally, not pushed.

## 2026-11-27 — Windows colour emoji (COLR renderer) and Windows parity fixes  (agent: Claude Code, claude-opus-5-5 (orchestrator) + Codex (read-only OS audit, 2 review rounds))
- Request: colour emoji still plain on Windows (fixed on macOS earlier); then find and fix other OS gaps.
- Root cause: `emoji_raster` only read embedded PNG strikes (`sbix`/`CBDT`). Windows' Segoe UI Emoji is a
  `COLR` vector font (v0 on Windows 10, v1 on 11), so `glyph_raster_image` returned nothing and every emoji
  fell back to egui's monochrome glyph. No call site was macOS-gated; only the font reader was.
- Changes:
  - New `src/emoji_raster/colr.rs`: COLR v0+v1 renderer on `ttf_parser::colr::Painter`. Outlines are
    flattened and filled with `ab_glyph_rasterizer` (already in the tree via egui; now a direct dep, no new
    crate), with segments clamped to the canvas so the row-major accumulator cannot wrap. Solid/linear/
    radial (two-point conical)/sweep paints per pixel, gradients interpolated in linear light (OpenType
    CPAL spec), sweep angles with the +1.0 bias, all Porter-Duff and W3C blend modes for PaintComposite.
    Canvas capped at 8×em (em ≤ 160 px) and ≤8 colour glyphs per cluster (untrusted message text).
  - `emoji_raster.rs`: bitmap first, else paint every shaped glyph (so unsupported sequences/flags render as
    Windows does); Segoe path from `%WINDIR%` via new `src/system_fonts.rs` (read into memory, not
    mmapped, since the env var could point anywhere), also used by `theme.rs` for the Windows UI/CJK fonts. Flag assertions tolerate Segoe's monochrome flags (would have broken Windows CI).
  - From the Codex audit (verified against code first): Windows drag-drop reads the live cursor with
    `GetCursorPos`; Linux drops always go to preview (stale pointer could quick-send); quick send now needs the
    last hover frame and the live cursor to agree. Windows screenshot uses `ShellExecuteW` (no PowerShell
    console flash) and reports launch failure; it still asks for Ctrl+V (see review).
    Attachment names that are Windows device names (`CON`, `nul.txt`, `COM1`, `COM²`…) get a `_` prefix.
    macOS hover/menu labels say Cmd+V / Cmd+F.
- Verification: `cargo test --locked` all green (434 lib + integration). No clippy warnings on changed lines.
  COLR checked by eye on Noto-COLRv1 (v1: gradients, composites, transforms) and Twemoji Mozilla (v0):
  `ZALO_COLR_TEST_FONT=<font.ttf> ZALO_COLR_TEST_OUT=<dir> cargo test --locked --lib rasterizes_colr_font_from_env`
  writes `colr_sheet.png`. Windows-only blocks (drag cursor, ShellExecute helper, screenshot wait) were
  type-checked on macOS by temporarily flipping their cfg; they were never compiled for or run on Windows.
- NOT verified: real Segoe UI Emoji on Windows 10/11 (no copy here), any Windows/Linux build or GUI run,
  drop-zone behaviour under mixed-DPI monitors.
- Review (`codex review --scope working-tree`, 2 rounds, cap reached):
  - round 1 (9 findings): fixed canvas/glyph cap, flag test that would fail on Windows CI, linear-light
    gradient interpolation, sweep-angle bias, drop race (hover+live must agree), superscript device names,
    changelog wording; kept two for round 2.
  - round 2 (6): dropped the Windows auto-attach of snips (could attach an unrelated clipboard image, or land
    in another chat if the user switched), clamped the COLR size, Segoe read into memory instead of mmap.
  - Remaining, accepted: on Windows a file can still quick-send if the user releases over the composer and
    moves back into the messages within one frame (~16 ms) — winit drops the native drop position.
- Open (audit, need owner decisions): Windows data dir is the launch directory (move to %LOCALAPPDATA% with
  migration?); Windows updater relaunch/installer flow; no Windows/Linux desktop notifications; release EXE
  has no console for `--version`/TUI; PDF helper lacks Job Object limits; no ACL hardening on Windows; FFmpeg
  only via WinGet; Linux AppImage self-update path, tray, apt-only FFmpeg; Windows CI only on tags/labels.
  Inferred only (need a live Windows session): image-only Ctrl+V quick taps missed; Telex Space recovery.
- Status: committed locally, not pushed.

## 2026-11-27 — Windows data dir, toast notifications, updater relaunch  (agent: Claude Code, claude-opus-5-5 (orchestrator) + Codex (worker ×4, adversarial reviewer ×4))
- Request: owner picked three items from the Windows audit (data dir to %LOCALAPPDATA%, updater flow, Windows
  notifications) and approved pushing `fix/windows-parity` to run the Windows CI job.
- Changes (branch `fix/windows-parity`, Codex wrote from Claude's briefs, Claude read every diff and re-ran tests):
  - `data_dir.rs`: `resolve` takes a `Platform` (pure); Windows base `%LOCALAPPDATA%\ctdozlite` (cwd fallback if
    unset). Migration (also macOS) now stages every item in `<target>/.migration-staging`, size-checks, publishes in
    one pass, then renames sources to `.bak`; failure rolls back and retries next launch. Windows: sources ranked
    session > accounts > db (cwd before exe dir), `profiles/` carried, per-profile migration for `--profile`, waits
    ≤30 s for other same-name processes (ToolHelp) and probes the DB with `share_mode(0)`, else uses the legacy dir
    for that run.
  - `native_notifications.rs`: WinRT toast via `windows` 0.54 (already locked by cpal; feature edge only). Fixed
    XML template + DOM `SetInnerText`/`SetAttribute`, avatar only as a local `file:///` (UNC rejected), AUMID
    `com.ctdoteam1110.ctdozlite` registered in HKCU and on the Inno shortcuts, ≤2 in-flight calls, stale toasts
    dropped. `status()` maps ToastNotifier.Setting.
  - Updater: transactional Windows swap with rollback (`try_install_windows_portable_release`), relaunch with
    hidden `--wait-for-pid` (stripped, never accumulates); the new exe exits if the old one outlives 60 s;
    swap failure returns a release-page error instead of running the portable exe from Downloads; `.exe.old`
    removed only after storage is selected.
- Verification: `cargo test --locked` green here (453 lib + integration). Windows CI (`workflow_dispatch` on the
  branch) green on runs 46008274032 and 46010264987: compiles the WinRT/ToolHelp/kernel32 code, runs the Windows
  data-dir and toast unit tests, and the Segoe UI Emoji test (now asserts the font is found on Windows).
- Reviews: adversarial ×3 (one per area) + round 2 on the fixes; `security-audit` skill in guidance mode found no
  boundary violation (toast text only via DOM, no remote image, updater keeps SHA-256/pinned URLs).
  Fixed in round 1: non-atomic migration, DB-probe race, empty-DB shadowing, stranded profiles, wait timeout
  starting a 2nd instance, early `.exe.old` delete, toast pool exhaustion.
  Open after the 2-round cap (owner decision): crash mid-publish then launch from another dir can reuse a partial
  target; macOS opens an empty target after a rolled-back migration (blocks retry); two simultaneous first
  launches can both fall back to the legacy dir; a destination `profiles/` created first blocks importing sibling
  legacy profiles; relaunch after update forwards a relative `--data-dir` that then resolves against the data dir.
  Not fixed: portable builds may not show toasts (HKCU AUMID without a Start-menu shortcut; Microsoft documents
  the shortcut).
- NOT verified: anything at runtime on Windows — a real update from v0.4.0, migration on a real profile, toasts
  appearing (setup and portable), Windows 10 vs 11.
- Status: committed on `fix/windows-parity`, pushed (branch only). Not merged to main, no release.

## 2026-11-27 — `@AI`: links, metadata, retrieval tools, images and files  (agent: Claude Code, claude-opus-5-5 (orchestrator, wrote steps 2-3) + Codex (worker, step 1) + Claude `/code-review` (reviewer))
- Request: `@AI` could not find a URL shared in the conversation; owner asked for richer context (sender, time, id,
  replies), deeper history / tools for large groups, and images + files passed to the model without filtering by
  model capability.
- Root cause: by design (ADR-0004 decision 4) `ai::strip_urls` replaced every URL with `[liên kết]`, and
  `transcript_for` / `run_ai_preload` read only `ChatMessage.text` / `StoredMessage.text`, dropping `link_url`,
  titles, file names, reply fields and ids (stored only in `raw_json`). Context was only the in-memory messages
  (often 20-200) under 12 000 chars. Two related bugs: `@AI …` with a pasted image/dropped file was sent **to Zalo**
  with the prompt as caption (`ai_intercepts_send` bailed on pending attachments), and replying to a Zalo message
  with `@AI` dropped that message (`filter(is_ai_id)`).
- Changes (branch `feature/ai-context`, ADR-0007 supersedes ADR-0004 decision 4):
  - Step 1 (Codex): rich `TranscriptLine` (id, ts, reply_to, attachment), verbatim URLs, shared converters for
    `ChatMessage`/`StoredMessage`, Zalo reply target as `<tin_duoc_tra_loi nguon="zalo">`, merge with ≤600 stored
    rows, budget 24 000 chars, consent keys versioned (`ai_group_confirmed_v2`, `ai_preload_consent_v2`), consent
    check moved into `ai_dispatch` (covers Retry/Regenerate).
  - Steps 2-3 (Claude; Codex hit its usage limit mid-brief): multi-part `Turn` (images, tool_calls, tool results),
    `chat_request`/`extract_reply`, tools `search_messages`, `list_links`, `get_recent_messages`,
    `get_messages_around`, `read_attachment` scoped to `req.thread_id` (never a parameter), ≤4 tool rounds, ≤8
    executions, 8 000-char results, final round flattened to text; retry without tools (400/415/422, first call)
    then without images (400/413/415/422). Attachments by reference (`AttachmentRef`): composer files (sent even with
    sharing off), reply-target photo/file and ≤3 recent photos when the prompt is about images (sharing on).
    Worker resolves them: downloads only from Zalo media CDNs (`zdn.vn`/`zadn.vn` with session, `dlfl.vn`/`dlmd.me`
    cookie-free; never `*.zalo.me` API hosts), bounded by existing caps, images → 1024 px JPEG data URLs (decode
    limits), documents via `doc_preview` (PDF isolated). Store: `messages_before`, `message_in_thread`,
    `messages_around` (thread-scoped). Search/link matching in Rust (SQLite LOWER is ASCII-only).
  - Docs: ADR-0007, ADR index, ADR-0004 status, AI_TRO_LY.md (§12 + corrections), HUONG_DAN_SU_DUNG, TINH_NANG,
    KIEN_TRUC network table, CHANGELOG.
- Verification: `cargo test --locked` green (482 lib + integration); new offline tests for URL preservation,
  stored-row conversion, merge, tool parsing/caps/loop/fallbacks, thread scoping (store + worker against a temp DB),
  image downscale, media host policy, local attachment resolution, composer intercept. Clippy: no new warnings in
  touched files. Review: Codex adversarial review was not possible (usage limit until 2026-11-28); Claude
  `/code-review high` on the branch found 6 important + 4 nits — all fixed (search limit/Unicode, photos counted as
  links, session cookies on sender-chosen Zalo API URLs, Regenerate losing the reply target, final tool round
  failing, fallback order, cancel losing composer attachments, stale comment, this entry). No second review round.
- After review: AI media downloads use a dedicated client with the session's cookie jar and **redirects off** (the
  session client follows redirects, so a CDN URL could have bounced to a Zalo API host), plus a final-URL check.
  Real hosts seen in `re-notes/`/fixtures: photos on `*.zdn.vn`/`*.zadn.vn`, files on `*.dlfl.vn`/`*.dlmd.me`.
- NOT verified: any real provider (tool calling and vision only tested against fake local servers), GUI behaviour
  (composer intercept with attachments, consent dialog restore), real Zalo CDN downloads (whether zdn.vn/zadn.vn
  photo/file URLs need the session or work cookie-free), large real groups.
- Status: committed on `feature/ai-context`, not pushed or merged. Follow-up: mandatory Codex adversarial review
  (AI client is high-risk) once Codex quota resets; live test with a real provider.

## 2026-11-27 — Clickable links in `@AI` answers; in-app video loading  (agent: Claude Code, claude-opus-5-5)
- Request: URLs in AI answers could not be clicked; in-app video mostly failed with "Không tải được video từ Zalo."
- Root causes: (1) `render_ai_bubble` called `Response::context_menu` on the bubble frame after its labels; egui's
  `context_interaction` re-registers that rect with `Sense::click()` last, so it sat on top of the link labels and
  took the click. URLs inside `code`/**bold** or glued to punctuation were also never treated as links.
  (2) Inferred, not captured: video URLs live on `*.dlmd.me`, fetched with `credential_free_client`, which had
  redirects off, so a CDN 3xx became `MediaError::Http(3xx)`; browsers follow it.
- Changes: `ai_ui.rs` registers the right-click target (last frame's rect) before the content; `messages.rs`
  `embedded_url` fallback makes a line clickable when it contains a URL anywhere; `media_resources.rs`
  `credential_free_client` follows ≤5 redirects to https Zalo media hosts only (`media_redirect_allowed`) and uses
  the ZaloPC user agent; the video arm logs `MediaError` + host (no query) and shows the HTTP code.
- Verification: cargo check, `media_redirects_stay_on_zalo_media_hosts`; release build launched for the owner.
- Follow-up (owner saw HTTP 301): log showed `*.dlmd.me` → 301 `video-stal-N.mdchat.me` → 302
  `video-stal-N-te-…mdchat.me`. `mdchat.me` added to `is_trusted_zalo_media_url`; videos/voice use
  `playback_client` (= credential-free client, redirects only to Zalo media hosts). Owner confirmed video plays.
- Originally NOT verified: that redirects were the real video cause (no capture) — if videos still fail, the log line
  `video download` names the error; link clicks need the owner's GUI check.

## 2026-11-28 — Image viewer: step through a conversation's photos like Zalo Web  (agent: Claude Code, claude-opus-5-5 (orchestrator, wrote it) + Codex (reviewer, 2 rounds))
- Request: implement Zalo Web's multi-photo viewer UX (observed by Claude on chat.zalo.me, My Documents, 2026-11-27:
  conversation-wide photo list with album members flattened, vertical day-grouped thumbnail strip on the right, ▲/▼
  buttons, ↑/← older and ↓/→ newer with no wrap, instant swap, wheel = zoom, zoom resets per photo; Zalo mounts one
  full-size image and flashes an empty stage while it loads). Kept with Claude rather than delegated: GUI judgment work
  that needs the owner's live check.
- Changes: new `src/gui/photo_gallery.rs` (photo collection with the bubbles' own rules, lookup by message id then
  thumbnail key then URL, day labels in UTC+7, `may_auto_fetch`, virtualised strip renderer with a cached row layout).
  `ImageViewerModal` (gui.rs) gains the photo list, index and texture bookkeeping; `ImageViewerModal::single` for
  cover/feed photos. `workspace.rs`: `open_photo_viewer`, `show_viewer_photo`, `load_viewer_originals`,
  `release_hd_textures`, `close_image_viewer`; chat bubbles, albums and the info drawer open the gallery
  (`BubbleAction::OpenImagePopup` now carries `msg_id`). `modals.rs`: keys, side column, wheel zoom only over the
  stage, thumbnail placeholder while the original loads (Zalo shows a blank), "tải ảnh gốc"/retry states, focus taken
  from the composer once. `messages.rs`: `message_sender_label`, `photo_open_target`, `photo_title_label` extracted.
  `worker.rs`: `hd_` textures capped at 4096 px (GPU texture limit; copy/save still use the original bytes on disk).
  `gui.rs`: `discard_textures` drops downloads released before they arrive; viewer closed on logout/account switch.
- Loading policy: originals of the current photo ±1 load only after a 180 ms pause on a photo (a held arrow key only
  shows thumbnails); the viewer keeps at most those three `hd_` textures and releases the rest plus strip-loaded
  thumbnails on close. Neighbours preload only from trusted Zalo media hosts or our own sent `local://` files.
- Review: round 1 (10 findings) fixed: GPU size cap, `local://` provenance, eviction of in-flight originals, wrong
  message for a shared thumbnail, failure state, strip virtualisation, original-load prompt, focus, moved strip out of
  modals.rs. Round 2 (9) fixed: close on account switch, debounce, strip thumbnails released on close, stage-click
  ownership, per-message thumbnail gate, prompt after an unsupported URL, cached layout. Rejected: (a) redirect
  allowlist for session-client `FetchStickerImage` downloads — pre-existing for every auto-loaded Zalo thumbnail, not
  new to this change, worth its own task; (b) sorting photos by normalised timestamp — the gallery follows the chat's
  own message order; a mixed s/ms ordering bug would be in thread loading, not here. Known limit: the strip can still
  hold many 720 px thumbnails while open in a very photo-heavy chat (released on close).
- Verification: cargo check; `cargo test --locked` 494 passed (11 new `photo_gallery` tests); rustfmt on clean files,
  no rustfmt drift added to messages.rs/gui.rs; no clippy warnings on changed lines.
- NOT verified: anything visual or interactive (layout, keys, strip scrolling, zoom reset, placeholder, toggle) — needs
  the owner's GUI run; Windows/Linux.
- Status: done pending the owner's live check.

## 2026-11-28 — Upstream ctdozlite v0.1.3–v0.1.4 review: what is worth porting  (agent: Claude Code, claude-opus-5-5)
- Request: read the ctdozlite changelog from v0.1.3 up, compare with Zalo-next, list new features and worthy ports.
  Read-only; nothing ported (AGENTS.md §7: the owner decides).
- Sources: GitHub release notes v0.1.3 and v0.1.4 (2026-11-27); upstream main has one commit after v0.1.4 (docs only).
  PR bodies read for #154, #161, #191.
- Findings with evidence (ours):
  - `src/socket.rs:502-506`: outbound frames sent before `authed` are taken off the channel and dropped, while
    `send_ws_frame` already returned true. Sync V2 590/592 sent right after launch is lost silently (same bug as
    upstream's "stuck at 10%" fix).
  - `src/main.rs:48` `#[tokio::main]` multi-thread runtime stays idle in GUI mode next to the worker's own runtime
    (`worker.rs:641`); upstream removed theirs (-39% threads).
  - Photo placeholder is a fixed 280x150 (`messages.rs:2796`); width/height from params are not kept, so bubbles
    jump when thumbnails load (upstream `media_size`, PR #192, schema change).
  - `render_hover_reaction_bar` is a stub returning None (`messages.rs:4794`): no quick-heart button.
  - Reaction banners: `historical: false` (`reactions.rs:586`) and offline pull uses `lastId: 1`
    (`socket.rs:401`): likely the same replay-on-launch bug as upstream #154 (not reproduced).
  - Mention fallback heuristic absorbs up to 5 words (`messages.rs:1975-1990`), as in upstream #161.
  - Not present: App Lock PIN, storage manager/relocation, group poll cards/voting (msgType 26 only named),
    single-instance guard on macOS/Linux, Linux XDG dir and tray, Open Sans.
  - Already have / skip: crash.log (in data dir; cwd is set before the hook), UTF-16 mention spans, reaction/quote
    ids (0.4.0), photo browser (own implementation), colour emoji (COLR renderer; Twemoji not needed), Ctrl+V image.
- NOT checked: #199 mute, #196 Mini App image, outgoing recall, multi-account snapshot isolation, #156 double-click
  deadlock, #191 audio by extension (we read `fileExt` at `chat.rs:5942`, not compared in depth), Settings-open stall.
- Status: done (report to owner); ports await the owner's choice.

## 2026-11-28 — Upstream port, package 1: pre-auth WS queue, no idle GUI runtime, mention guess, Linux XDG  (agent: Claude Code, claude-opus-5-5 (orchestrator, wrote it) + Codex (adversarial reviewer, 1 round))
- Request: owner approved porting the v0.1.3–v0.1.4 items except the quick-heart button; plan in
  `~/.claude/plans/i-accept-all-except-cuddly-scone-*.md`. Re-implemented, nothing copied from upstream.
- Changes:
  - `socket.rs`: `PreAuthQueue` (32 frames, per connection) holds frames sent before `AUTHEN`, flushed after the
    queue pulls; `OutboundRegistration` withdraws the global sender when the connection ends (only its own);
    the connection retires on `tx.closed()` and does not publish when its consumer is already gone.
  - `main.rs`: no `#[tokio::main]`; a current-thread runtime runs `--check-update` and `video_tools::refresh`,
    the TUI builds a multi-thread runtime (`run_tui_mode`). GUI-thread code checked for runtime use: none.
  - `gui/messages.rs`: `guessed_mention_name_len` (following words need an uppercase start, max 4 words; the plan
    said 2, raised to 4 so Vietnamese full names still match).
  - `data_dir.rs`: `Platform::Linux`, `linux_default_dir` (`$XDG_DATA_HOME/ctdozlite`, else
    `~/.local/share/ctdozlite`), CWD migration as on macOS; a `--profile` run migrates `./profiles/<name>`.
- Review (Codex adversarial, 1 round): fixed (1) a retired socket could publish its sender after an account switch
  and flush the new account's 590 (now retires on `tx.closed()`); (3) `--profile` skipped migration (now migrates
  its own folder). Not fixed (2): `send_ws_frame` still means "accepted", not "written", so a 590 discarded by a
  connection that drops before auth still waits for the phone timeout (pre-existing; was always dropped before).
- Verification: cargo check; `cargo test --locked` 502 passed; new tests for the queue, the registration, the
  mention guess, Linux resolve and profile migration; `--print-data-dir` smoke run (startup runtime).
- NOT verified: live sync right after launch on a real phone; a GUI run without the main-thread runtime (call,
  sync, proxy) — needs the owner's run; the Linux `initialize` branch is not compiled on this Mac (CI only).
- Status: package 1 done; package 2 (single instance, media size) next.

## 2026-11-28 — Upstream port, packages 2–3: single instance, media size, storage page, Open Sans  (agent: Claude Code, claude-opus-5-5 (orchestrator, wrote single instance / font / fixes) + Codex (worker: media size, storage core, storage page; reviewer: 1 review + 1 adversarial round))
- Changes:
  - `single_instance.rs` (new): `File::try_lock` on `ctdozlite.lock` (std, no libc; same name as upstream so the
    two apps exclude each other), owner-only `ctdozlite.sock` served on its own thread (`show` → tray-style flag
    + repaint), relative socket fallback for long paths, fail-open. Taken before `--logout`; an updater relaunch
    (`--wait-for-pid`) waits up to 60 s. Linux AppImage relaunch now passes `--wait-for-pid` (worker.rs).
  - Media size (Codex): `ChatMessage::media_size` from `params.width/height` / `video_width/video_height`
    (re-notes/49, Zalo Web `_estimateSize`), kept in `raw_json` (no schema change); `photo_display_size` shared by
    placeholder, image and bubble; video card min 120x90.
  - Storage (Codex core + page, Claude rewrite): `storage_usage.rs` scan/clear by category; Codex's raw
    openat/unlinkat FFI with hand-coded flag constants replaced by std-only checks (no `unsafe`); Settings → Bộ nhớ
    page with worker commands (blocking pool, one at a time, terminal events).
  - Open Sans Regular bundled (googlefonts/opensans@6cbc400d, sha256
    c53aceea2dcf5b4098099c0c4d0a061d17e178a049317b42a422b1a9f7f8eb59, OFL text alongside).
- Review: Codex review (7 findings) fixed: socket bind off the GUI thread, long socket path, Windows dead_code,
  video hit area, Linux relaunch race; the storage_usage "untracked file" finding was already committed.
  Adversarial review of storage (7): fixed download folder inside the data dir, pasted composer images,
  `.privatepart`/uncommitted media, Windows hard-link truncation, socket path resolution on the GUI thread.
  Rejected: (a) handle-relative (openat) deletion against a parent swapped mid-clear — needs write access to the
  owner-only (0700) data dir, so it is outside the threat model and not worth the unsafe FFI; (b) AppImage
  `current_exe()` points inside the read-only mount — pre-existing updater issue, logged as a follow-up.
  Open for the owner: Open Sans and Be Vietnam Pro licences are not shipped in release artifacts (OFL wants the
  licence with the font) — package the `*-OFL.txt` files or show them in-app.
- Verification: `cargo test --locked` 528 passed; clippy clean on touched lines; rustfmt on clean files.
- NOT verified: anything visual (storage page, photo/video layout, font), second launch bringing the window up,
  updater relaunch on Linux/Windows, Windows log removal, live media sizes from real chats.
- Status: done; next: group polls (`feature/group-polls` worktree) and App Lock (`feature/app-lock` worktree).

## 2026-11-28 — Upstream port, package 4: group polls and App Lock on branches (not merged)  (agent: Claude Code, claude-opus-5-5 (orchestrator, wrote app_lock core + macOS FFI + notification gate + fixes) + Codex (worker: poll API, poll card, lock UI))
- Branches (worktrees, nothing merged, nothing pushed):
  - `feature/group-polls` (`../zalo-next-polls`): d45eaed protocol + worker, d59019c poll cards. Evidence
    re-notes/50 (Zalo Web lazy chunk 2: `getPollDetail`/`vote` use `encodeAES` = zpw_enk, NOT ParamCipher v2 as
    upstream wrote; host `getGroupPollDomain` = service map `group_poll`). Votes: no redirects, no retries, no
    failover except HTTP 404; untrusted hosts dropped from candidates and re-checked per URL. Claude fix: poll
    commands ran inline in the worker loop (blocked every command during the request) → spawned FIFO tasks.
  - `feature/app-lock` (`../zalo-next-applock`): d4e2c64 core (`app_lock.rs`, pbkdf2 0.12 crate added, PBKDF2-SHA256
    600k, constant-time compare, cooldown 30 s doubling to 15 min after 5 tries, sleep = wall vs monotonic jump),
    `gui/macos_auth.rs` (LAContext deviceOwnerAuthentication via objc2 0.5, context kept alive until reply);
    0ce566d all OS notifications go through `notify_with_attachment`, which posts fixed text with no avatar while
    locked (Codex found the incoming-call notification in call.rs bypassed the worker); 45dbba4 lock screen gate
    in `update()` before any content, Cmd/Ctrl+L, idle/sleep auto-lock, ringing-call banner without caller,
    settings in Quyền riêng tư. Claude fix: passcode hashing awaited inline in the worker loop → spawned.
- Review: Codex hit its usage limit (until 05:52) during the poll adversarial review and before an App Lock
  review, so the MANDATORY adversarial reviews for both branches are still pending. Claude did a focused
  security-audit pass instead (guidance mode): no confirmed issue. Notes: the window title still shows the unread
  count while locked (count only, no content); `evaluatePolicy:localizedReason:reply:` argument encoding is only
  checked at runtime in debug builds (the `canEvaluatePolicy` call is covered by a test); App Lock is a UI lock,
  anyone who can read the data dir can read or edit `app_lock.json` and the DB (documented in the UI).
- Verification: polls branch `cargo test --locked` 540 → (+UI) passing; app-lock branch 538 passing; the
  non-macOS lock UI branch was compile-checked on this Mac by temporarily renaming its cfg; not built for
  Windows/Linux.
- NOT verified: any live poll detail/vote (owner must vote on a real group), Touch ID/password prompt, lock
  screen, auto-lock after sleep, ringing call while locked, passcode flow on Windows/Linux.
- Status: waiting for Codex adversarial reviews and the owner's live checks before merging either branch.

## 2026-11-28 — Owner test build, review fixes, contact cards and Zalo emoticons  (agent: Claude Code, claude-opus-5-5 (orchestrator) + Codex (worker, reviewer))
- Request: build an app to test everything; owner picks nothing on licences (Claude chose an in-app viewer:
  Settings → Giao diện → Giấy phép phông chữ, 4aba180 on main). Then: contact cards and Zalo text emoticons.
- Branches: `test/upstream-port` (checked out in the main dir) merges main + `feature/group-polls` +
  `feature/app-lock` + `feature/emoticons-contact-card`. Nothing merged into main, nothing pushed.
- Owner-found bugs fixed on the test branch: Settings tabs laid out in one row (8361b80; the storage page's
  ScrollArea dropped the vertical layout); albums older than the 20 cached messages split after restart
  (2ad4d62; pre-existing: album params and mention spans now kept in raw_json). Both still need to reach main.
- Mandatory adversarial reviews (Codex was over its limit earlier, rerun after reset): App Lock 6 findings fixed
  (b8b7b35: TUI refuses a locked data dir, macOS fails closed on an unreadable config, notifications re-check the
  lock, only a connected call defers locking, settings count once saved, 15 s sleep jump); polls 5 findings fixed
  (24dae54: session generation for queued votes, voter privacy, 1 MiB/size caps, ack refetch, stale cards).
  Claude also fixed a Windows-only type error in the toast code (not compiled here).
- New: re-notes/51 (Zalo Web bundle: emojiRC table, sprite URL + SHA-256, xp5P matching rule; contact card
  `recommened.user` fields). Emoticons e578f0b (cookie-less pinned sprite download, tile textures, selection
  copies the shortcut); contact cards 2d56ab6; review fixes 5677cf8 (redirect-checked avatar client for all
  `ava_` loads, Nhắn tin uses the sidebar open path, recall marker, `+` kept, cross-section boundaries,
  URL in endpoints.rs).
- Verification: `cargo test --locked` 586 passed on the test branch; dmg built
  (dist/zalo-next-0.4.0-macos-arm64.dmg).
- NOT verified: everything live (owner testing): polls voting, App Lock prompts/sleep/calls, contact card
  actions, emoticon rendering and selection copy, storage clearing, account switching after the cookie-jar
  change, Windows/Linux builds.

## 2026-11-28 — Bank accounts: save and send ("Gửi số tài khoản")  (agent: Claude Code, claude-opus-5-5 (orchestrator, finished it) + Codex (worker, ran out of quota at the end))
- Request: Zalo's saved bank accounts + send bank card; no transfer button; right-click copy number / show
  VietQR, click QR to enlarge. Owner chose the Zalo-server list (shared with the phone).
- Evidence: re-notes/52 (Zalo Web lazy chunks: /api/transfer/{conf,list,create,update,delete,card} on the
  `zinstant_msg` host, fallback zimsg.chat.zalo.me; params, response fields, validation regexes; VietQR payload
  built locally).
- Branch `feature/bank-cards` (2c4de8b): src/bank_cards.rs (client with no redirects/retries, failover only on
  404, bounded responses, no logging of numbers/names), `zinstant_msg` service-map slot restricted to trusted
  HTTPS origins, worker commands with account-scoped terminal events, src/gui/bank_cards_ui.rs (composer button,
  list, add/edit with searchable bank picker, delete with Zalo's default rule, right-click copy / VietQR, QR
  enlarge). Codex hit its usage limit during the final verification; Claude re-ran check/tests/fmt/clippy and
  reviewed the diff. Merged into test/upstream-port (31708de; conflicts with polls/app-lock in gui.rs,
  worker.rs, session.rs resolved by hand: one http_client_builder(proxy) for both).
- Verification: test branch `cargo test --locked` 595 passed; dmg rebuilt.
- NOT verified: anything live (list/add/edit/delete/send against Zalo, how the phone shows the card, logos),
  received bank cards are NOT rendered yet (zinstant layout, no sample; waits for the owner's BrowserSkill).
- Pending: mandatory adversarial review of feature/bank-cards (Codex quota resets 16:08).

## 2026-11-28 — Owner feedback round 2 (partial) and session handoff  (agent: Claude Code, claude-opus-5-5)
- Done on `feature/owner-feedback-2` (../zalo-next-fb, not yet merged into test/upstream-port): voice/video
  incoming call detection by extendData.callType (d668ced), fresh-only notifications (9e9c714), mark all read +
  chat.delete handling + blank rows hidden (43fda1a), contact card without call button (89396bc). 596 tests pass.
- App Lock: Touch ID prompt only on click, app activated before asking (56fdd35, merged into test branch).
- Zalo Web inspected via BrowserSkill (Bo's chat only, nothing sent): received bank card = zinstant.bankcard,
  data at zinst-stc.zadn.vn (re-notes/54, redacted fixture). Formatting toolbar layout confirmed.
- NOT done / pending: received bank card rendering, add-account redesign, call popup + optional video,
  phone-number cards, rich text, bank-cards adversarial review. Full plan in
  `.agents/handoff/HANDOFF-2026-11-28.md` (briefs saved next to it).
