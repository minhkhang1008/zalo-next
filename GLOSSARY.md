# Glossary

Ubiquitous language for `zalo-next`. Use these terms verbatim in issues, ADRs, tests and code identifiers; do not drift to synonyms.

> Purpose: keep protocol vocabulary, UI vocabulary and storage vocabulary aligned. Update it in the same change that introduces or renames a term (see `docs/standards/docs-and-decisions.md`).

## Session & Authentication

| Term | Definition | Avoid |
| :--- | :--- | :--- |
| **Session** | Authenticated Zalo identity persisted in `session.json`: cookies, `zpw_sek`, `zpw_enk`, IMEI, user info. | "login state", "auth blob" |
| **`zpw_sek`** | Server-issued session cookie proving an authenticated session. Attached to every `*.zaloapp.com` / `*.chat.zalo.me` request. | "session key" |
| **`zpw_enk`** | Base64-encoded encryption key used to derive the AES key for `params`. | "encryption key" (ambiguous) |
| **QR login** | Login by scanning a QR code, then long-polling `pc-status` until the phone confirms. | "scan login" |
| **Password login** | Login with phone number + password via `PasswordCipher`. TUI/CLI only (`--phone`/`--pass`; `--v1` is a plaintext diagnostic path); removed from the GUI. | "manual login" |

## Protocol

| Term | Definition | Avoid |
| :--- | :--- | :--- |
| **ParamCipher v2** | Request parameter encoding scheme (`src/param_cipher.rs`) required by the server. | "param encryption" |
| **`signkey`** | MD5-based signature over selected request parameters, bound to a timestamp/tag. | "checksum", "hash" |
| **Service map** | Server-provided `zpw_service_map_v3` mapping logical services (`chat`, `group`, `file`, …) to hosts; mirrored in `src/endpoints.rs`. | "host list" |
| **`error_code`** | Numeric status inside Zalo JSON responses. `0` means success. | "status" |
| **Push / Pull** | WebSocket message classes: `501/521` push, `510/511` pull, `601–604` control. | "socket message" |

## Chat Domain

| Term | Definition | Avoid |
| :--- | :--- | :--- |
| **Thread** | A conversation: 1:1 user thread or group thread. Identified by `thread_id`. | "chat", "conversation" (when the ID matters) |
| **Message** | One chat entry with `msg_id`, `thread_id`, sender, timestamp, type and raw JSON. | "text" |
| **Local message ID** | Client-generated ID used to reconcile optimistic UI rows with server echoes. | "temp id" |
| **Recall** | Server-side removal of a message for all participants. | "delete" (that is local-only) |
| **Delete for me** | Local-only removal mirrored into SQLite. | "recall" |

## Runtime

| Term | Definition | Avoid |
| :--- | :--- | :--- |
| **Worker** | Tokio runtime on the `zalo-worker` thread owning all network, disk and crypto work; receives `WorkerCmd`, answers with `GuiEvent`. | "backend", "service" |
| **UI thread** | The egui/ratatui event loop; must never block on I/O. | "frontend" |
| **Store** | Local SQLite database managed by `MessageStore`. | "cache" (cache is non-authoritative) |
| **Call** | VoIP session (audio/video) handled by `src/call.rs` and `src/audio.rs`. | "phone" |

## Assistant

| Term | Definition | Avoid |
| :--- | :--- | :--- |
| **Assistant** | The user-configured OpenAI-compatible model reached through the `ai` module (see ADR-0004). | "bot", "LLM provider" |
| **Local AI message** | A question or answer exchanged with the Assistant, stored in `local_ai_messages` with an `ai_` id and shown as a third participant. Exists only on this machine. | "AI chat message", "virtual message" |
| **Context sharing** | Per-conversation opt-in that lets the Assistant read recent conversation text. Off by default. | "AI access", "read mode" |

## Invariants

Domain invariants are tagged `INV-N` here and referenced verbatim from code and tests.

- **INV-1** — No Zalo API request leaves the process without the encoding that endpoint requires (signed params for `reqqr`/`getServerInfo`, `ParamCipher v2` for `getLoginInfo`/password login v2, `zpw_enk` AES after login) and a valid `signkey` where required. Non-Zalo traffic (updater, Assistant, FFmpeg, Sync V2 runtime) is out of scope and must carry no Zalo credentials (INV-6).
- **INV-2** — `session.json` is never logged, committed or transmitted outside Zalo endpoints.
- **INV-3** — The UI thread never performs blocking I/O; all such work runs in the worker.
- **INV-4** — A message row is idempotent by `msg_id`; re-delivery never duplicates history.
- **INV-5** — A Local AI message is never sent to a Zalo endpoint, written to `messages`/`threads_cache.json`, or used as a read-receipt / quote / forward target.
- **INV-6** — Zalo credentials are only sent to Zalo hosts and Assistant credentials only to the configured Assistant host; the two HTTP clients share no cookies, headers or redirects.
