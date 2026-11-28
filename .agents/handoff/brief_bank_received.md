<task>
Render RECEIVED Zalo bank cards natively. Read re-notes/54_received_bank_cards.md (message shape, card HTML
fields, VietQR payload) and the redacted fixture tests/fixtures/zinstant_bankcard_pc.html. This branch already
has the send/manage feature (src/bank_cards.rs, src/gui/bank_cards_ui.rs incl. a VietQR modal and
`vietqr_payload`); reuse its QR modal and texture helper.

1. Parsing (src/chat.rs parse_chat_message): for `chat.webcontent` (or msgType 52) whose content.action is
   "zinstant.bankcard", keep `ChatMessage.bank_card_ref: Option<BankCardRef { data_url: String }>` taken from
   `params.pcItem.data_url` (params may be a JSON string), accepted only if it is https on host
   `zinst-stc.zadn.vn` (or another host passing `is_trusted_zalo_url`), and set the message text to
   "[Tài khoản ngân hàng]". Persist it in raw_json like `media_size` (src/store.rs) and merge it in
   `merge_message_metadata`.
2. Fetch + parse (src/bank_cards.rs + worker): `WorkerCmd::LoadReceivedBankCard { uin, key: msg id, url }` →
   GET with a client that has NO cookies/session headers, no redirects off Zalo hosts, 256 KiB cap, 15 s timeout;
   parse the HTML without a full HTML parser (small, robust string scanning is fine): bin (`id="bank_bin"`), bank
   name (`bank_info` … `normal_text`), number (`number_text`), logo URL (`_LOGO_` on res-zalo.zadn.vn), and the
   VietQR payload from the first `bankcard-action.zalo.me?action=transfer&content=` (HTML-unescape `&amp;`).
   Validate: number `^[A-Za-z0-9]{1,19}$`, payload starts with "000201", ≤ 512 chars, CRC-16/CCITT-FALSE matches
   (if it does not, fall back to `vietqr_payload(bin, number)`). Reply `GuiEvent::ReceivedBankCard { uin, key,
   result: Result<ReceivedBankCard { bin, bank_name, number, logo: Option<String>, qr_payload }, String> }`.
   Cache results in the GUI per message id for the session; fetch lazily only when the card is rendered.
3. UI (src/gui/messages.rs + a renderer in src/gui/bank_cards_ui.rs): a card like Zalo's (bank colour-ish
   header is optional; keep it simple and readable in light/dark): bank logo (only auto-load if
   `is_trusted_zalo_url`), bank name, big account number, and a small QR (texture from `qr_payload`, reuse the
   existing helper) on the right. Clicking the QR opens the existing VietQR modal (with enlarge). Right-click the
   card: "Sao chép số tài khoản", "Hiện mã VietQR". No transfer button. Loading and error states (error shows the
   preview text with "Thử lại"). Works for cards we sent too (same message type).
4. Tests: fixture parsing (number 0123456789, bin 546034, name, logo URL, payload), HTML-unescape, CRC check and
   fallback, untrusted data_url rejected, raw_json round trip.
</task>
<scope>
May edit: src/chat.rs, src/store.rs, src/bank_cards.rs, src/gui/bank_cards_ui.rs, src/gui/messages.rs,
src/gui/workspace.rs, src/gui/worker.rs, src/gui.rs
Must not touch: Cargo.*, docs/, re-notes/, tests/fixtures/, CHANGELOG.md, .agents/WORKLOG.md
</scope>
<constraints>
Follow AGENTS.md (§4.3 no cookies to the CDN fetch; §4.6 only Zalo hosts auto-load; §4.2; §4.9; §4.10 offline
tests using the fixture via include_str!). Never log account numbers. Do not commit. rustfmt only
src/bank_cards.rs and src/gui/bank_cards_ui.rs. No new clippy warnings in code you add.
</constraints>
<verification_loop>
Run: CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo check --message-format=short
Run: CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo test --locked --lib bank_cards
Run: CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo test --locked --lib chat
If a check fails, fix and re-run.
</verification_loop>
<completeness_contract>
Done means received bank cards render with copy / VietQR actions, the fetch is cookie-less and bounded, tests
pass, cargo check clean. If blocked, stop and say what is missing.
</completeness_contract>
<compact_output_contract>
Reply with: summary (≤10 lines), files touched, commands run with pass/fail counts, anything unverified.
</compact_output_contract>
