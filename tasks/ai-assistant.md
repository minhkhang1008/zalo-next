# Local AI assistant (`@AI`) — task breakdown

Design: [`docs/AI_TRO_LY.md`](../docs/AI_TRO_LY.md) · Decision: [ADR-0004](../docs/adr/0004-local-ai-assistant.md).
Status: **AI-1..AI-6 implemented 2026-11-17** except Keychain storage and streaming (see each task); offline tests pass, live verification with a real provider and Zalo account is pending. Each task below can be filed as a GitHub
Issue per `docs/standards/issue-tracker.md`; implement in order, one branch per task (`feature/ai-*`).
Terms follow `GLOSSARY.md` (Assistant, Local AI message, Context sharing; INV-5, INV-6).

Definition of done for every task: `cargo check`, relevant `cargo test`, no new clippy findings in touched files,
rustfmt on touched files only, `.agents/WORKLOG.md` entry, Vietnamese UI strings / English comments. Live
verification (real provider, real Zalo account) is tracked separately from offline tests.

## AI-1 — Configuration, client, connection test  (done; Keychain deferred, key file `0600` + env instead)

- New `src/ai/` module: `config` (base URL, model, key source), `client` (dedicated `reqwest::Client`, no cookie jar,
  redirects off, timeout, body cap, `https` or loopback only), `error` (typed errors → Vietnamese messages).
- API key: env `CTDOZ_API_KEY` → Keychain (Security framework; never on argv) → `0600` file. Never in
  `app_settings.json`, never logged.
- Worker commands `AiTestConnection`, `AiListModels` (`GET {base}/models`), results as `GuiEvent`s.
- Settings → "Trợ lý AI": Base URL, API key (masked), Model, Lưu, Kiểm tra kết nối (sends a real request).
- Tests: local fake HTTP server for 200/401/404/429/timeout/oversized/redirect; request carries no Zalo cookie or
  header (INV-6); key redaction; URL validation.
- Acceptance: test button reports a real result; key survives restart without appearing in `app_settings.json`.

## AI-2 — Local message model and storage  (done; kept outside `Thread.messages` instead of a `MessageKind`)

- `local_ai_messages` table + versioned migration (own `user_version` step; do not reuse numbers taken by
  `productivity.rs` / history heal). Account-scoped DB, `0600`.
- `ChatMessage` source flag (`MessageKind::LocalAi`) and `ai_` id prefix helpers; load/save/delete APIs in the store.
- Tests: round-trip, per-account isolation, delete thread removes rows, migration idempotent on old DBs.
- Acceptance: rows persist across restart; nothing is written to `messages`.

## AI-3 — Guards in existing code (INV-5)  (done by construction + quote/draft guards)

Make every spot in `docs/AI_TRO_LY.md` §3.2 ignore or specially handle Local AI messages: `SendSeen` last-message
id, `merge_thread_into` / `deduplicate_threads`, `persist_threads` + `threads_cache.json`, unread + native
notification + tray badge, thread preview/time, search, forward / multi-select / `SendQuote` / reactions,
`delete_thread`, history sync overwrite, scheduled send / auto-reply / MCP.

- Tests: one per guard, plus an end-to-end "AI row never reaches a `WorkerCmd::Send*`" check.
- Acceptance: with a Local AI message as the newest item, seen-receipt, unread count, preview and notifications are
  identical to the same thread without it.

## AI-4 — Composer `@AI`, rendering, message menu  (done)

- Intercept `@AI <text>` before the send branch (precedent: the `@gif` command); disabled assistant → plain text.
  Group mention popover must not hijack `@AI`.
- Render the Assistant as a third sender (private bubble style, "Chỉ mình bạn thấy", typing indicator).
- Message menu for Local AI messages: Sao chép, Trả lời, Xóa (local only), Tạo lại; no Thu hồi / Chuyển tiếp /
  React. Replying to an AI message never attaches a quote to Zalo.
- Worker `AskAi` → `GuiEvent::AiMessage` / `AiError` (Thử lại); no UI-thread I/O (INV-3).
- Acceptance: the "Rust là gì" example works in a 1-1 chat; the other person sees nothing (verify in Zalo Web).

## AI-5 — Context sharing, context builder, summaries  (done)

- Setting: global default (off) + per-conversation override; first-time confirmation for groups naming the host.
- Context builder: character budget, chronological, `[HH:mm] Tên: text`, text only (no URLs/attachments), untrusted
  data delimiters, history with the Assistant, "đã dùng N tin" note.
- Quick actions: Tóm tắt cuộc trò chuyện, Tóm tắt từ lần đọc cuối, Gợi ý câu trả lời; map-reduce for oversized
  ranges with progress + cancel.
- Tests: budget/ordering/truncation, URL stripping, prompt-injection fixture stays inside the data block, sharing
  off ⇒ no conversation text in the request body.
- Acceptance: summarising a 1,000-message group stays within budget and cites the covered time range.

## AI-6 — Streaming, polish, docs  (docs done; streaming not done)

- Optional SSE streaming via `Response::chunk()`, cancel on conversation change / quit.
- Error copy, empty states, accessibility of the new controls; update `docs/HUONG_DAN_SU_DUNG.md`,
  `docs/TINH_NANG.md`, CHANGELOG; flip ADR-0004 to `accepted` once live-verified.
- Open questions to settle first: see `docs/AI_TRO_LY.md` §10.
