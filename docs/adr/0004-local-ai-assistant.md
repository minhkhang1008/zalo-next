# ADR-0004: Local-only AI assistant as a virtual participant

- **Status:** accepted (implemented 2026-11-17; pending live verification with a real provider)
- **Date:** 2026-11-17
- **Deciders:** repository owner

## Context

The owner wants a personal AI assistant: typing `@AI <question>` in any conversation yields an answer that looks like
a third participant, is visible only on this machine, can be copied / deleted / replied to, and (optionally) reads the
conversation to answer or summarise large group chats. Upstream ctdozlite v0.1.2 has an "LLM" settings block, but its
GUI never issues the request ("Kiểm tra kết nối" only sets a string, summarise uses a `StubProvider`), stores the API
key in plaintext `app_settings.json`, and silently falls back to other endpoints. See `docs/AI_TRO_LY.md` §8.

Constraints from this repository: INV-1/INV-2 (Zalo credentials only to Zalo hosts), INV-3 (no I/O on the UI thread),
the `is_trusted_zalo_url` allowlist, `docs/standards/storage.md` (add tables instead of overloading
columns) and `AGENTS.md` ("no automatic GET to message URLs", sensitive files `0600`).

## Decision

1. AI traffic uses a dedicated `reqwest::Client` in a new `ai` module: no cookie jar, no Zalo headers, redirects off,
   hard timeout and body cap, `https` only (loopback excepted). `is_trusted_zalo_url` is **not** widened.
2. AI messages are **local-only**: stored in a new table `local_ai_messages` and kept in the GUI in a map that is
   **separate from `Thread.messages`**, with an `ai_` id prefix, drawn as a third sender between the real messages
   by timestamp. Because no Zalo code path ever receives them, they cannot reach `messages`, `threads_cache.json`,
   history sync, seen receipts, unread counts, notifications, search, forward or any `WorkerCmd::Send*` path.
   Defence in depth: `send_quote_message_with_mentions` refuses `ai_` ids, and drafts never persist an AI reply.
3. `@AI` is intercepted in the composer before the send branch and becomes `WorkerCmd::AskAi`; it never reaches Zalo.
4. Sharing conversation text with the AI is **opt-in**, off by default, overridable per conversation, with a
   confirmation the first time for group chats. Only text is shared, never URLs, files or media.
5. The API key is not stored in `app_settings.json`: environment variable (`CTDOZ_API_KEY`), then a `0600` file
   `ai_secret.json` in the data directory. macOS Keychain storage is a follow-up (it must not put the key on a
   command line).
6. Context is built under a character budget (newest messages first kept), with map-reduce for summaries that exceed
   it. Message text is wrapped as untrusted data in the prompt.

## Consequences

- Positive: the feature cannot leak AI content to Zalo or Zalo credentials to the AI host by construction; no schema
  change to existing tables; behaviour is testable offline with a local fake server.
- Negative: the message list renderer merges two sources by timestamp; Assistant messages are not part of
  Zalo search/export and cannot be starred or reacted to.
- Follow-ups: Keychain storage, streaming, persona/prompt customisation, optional inclusion of AI text in search, cost warnings.

## Alternatives Considered

| Option | Why rejected |
| :--- | :--- |
| Store AI rows in `messages` with `msg_type = 'local.ai'` | Every existing reader (sync, search, persist, unread, export) would need to filter; one missed spot leaks or corrupts history. |
| Send the answer to Zalo as a real message | Defeats the "only I can see it" requirement and exposes the AI text to other people. |
| Reuse `Session.http` for the AI call | Would attach Zalo cookies to a third-party host (violates INV-2) or force widening the trusted-host allowlist. |
| Port upstream `llm.rs` as is | Its GUI path does not work and it stores the key in plaintext (see context). |
| Always share conversation context | Group chats contain other people's messages; sending them to a third party must be an explicit choice. |
