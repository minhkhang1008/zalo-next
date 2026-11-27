# ADR-0007: AI assistant reads links, attachments and older history

- **Status:** accepted (implemented 2026-11-27; not verified with a real provider)
- **Date:** 2026-11-27
- **Deciders:** repository owner
- **Supersedes:** ADR-0004 decision 4 ("only text is shared, never URLs, files or media")

## Context

The owner asked `@AI` to find a link shared in a group and it could not. The cause was a design choice in ADR-0004:
`ai::strip_urls` replaced every URL with `[liên kết]`, and the transcript was built from `ChatMessage.text` only,
so link cards (URL only in `link_url`), file names, photos and reply relationships never reached the model. Context
was also limited to the messages loaded in memory (often 20-200) and a 12 000-character budget, which is too
little for large groups. The owner asked for links, message metadata, deeper history (tools allowed), and images
and files to be passed to the model without filtering on our side by model capability.

## Decision

1. When sharing is on for a conversation, each transcript line carries the message id, time, sender, the reply
   target (`trả lời Sender: …`) and the attachment (`[Liên kết|Ảnh|Tệp|Video|Tin thoại|Nhãn dán: title — url]`).
   URLs are sent verbatim as text. Nothing fetches them: AGENTS.md §4.6 still holds.
2. Context comes from the in-memory messages merged with up to 600 rows of the local SQLite history, under a
   24 000-character budget, newest kept.
3. With sharing on, the model is offered read-only tools over **this conversation only**: `search_messages`,
   `list_links`, `get_recent_messages`, `get_messages_around`, `read_attachment`. The thread id is never a tool
   argument; it comes from the request. Rounds, calls, arguments and result sizes are capped, and results are
   wrapped as untrusted data. If the provider rejects `tools` (HTTP 400/415/422) the question is retried once
   without them.
4. Images and files are sent as content:
   - attachments the user adds to an `@AI` message in the composer (sent even when sharing is off: the user chose
     them for the AI);
   - with sharing on, the photo or file of the Zalo message being replied to, recent photos when the question is
     about images, and attachments the model asks for with `read_attachment`.
   Only Zalo-hosted media (`is_trusted_zalo_url`) and local files are downloaded, within the existing caps. Images
   are downscaled to 1024 px JPEG data URLs; documents go through `doc_preview` (PDF in the isolated helper). If
   the provider rejects images, the call is retried once without them. We do not filter by model name.
5. Group consent recorded under the old "text only" wording is not reused: the settings keys are versioned
   (`ai_group_confirmed_v2`, `ai_preload_consent_v2`), so each group and the pre-read feature ask again once with
   wording that names links, photos and files.

## Consequences

- Positive: `@AI` can quote links, answer "who said what in reply to whom", search older history, and describe
  photos and documents.
- Negative: more conversation data goes to the AI provider when sharing is on (URLs, which may be private signed
  links, and media content). Requests are larger and cost more; local models without vision or tool support fall
  back to text only.
- Follow-ups: live verification with a real provider (OpenAI-compatible tools and vision), streaming, a per-thread
  switch to share text but not media.

## Alternatives Considered

| Option | Why rejected |
| :--- | :--- |
| Keep stripping URLs, add a separate "find link" command | Does not cover the owner's other questions (who said what, older history, images). |
| Send media as remote URLs for the provider to fetch | Local providers cannot fetch, and signed Zalo URLs would be fetched by a third party; data URLs keep the fetch on our side. |
| Give tools access to every conversation | Would expose conversations the user never chose to share. |
