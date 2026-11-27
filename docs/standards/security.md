# Security Standards

Threat model: a desktop client holding a live Zalo session and the owner's private message history. The main
risks are credential leakage, private data leaving the machine, and untrusted content (messages, files,
downloads) being processed unsafely.

## 1. Secrets inventory

| Secret | Location | Rule |
| :--- | :--- | :--- |
| `zpw_sek`, `zpw_enk`, cookies, IMEI, user info | `session.json` | Never logged, committed or sent outside Zalo hosts (INV-2) |
| Sync V2 identity / RSA keys | `sync_identity.json`, `sync_rsa_identity.json` | `0600`; never committed |
| Assistant API key | `CTDOZ_API_KEY` env, or `ai_secret.json` (`0600`) | Never in `app_settings.json`, never on a command line, only sent to the configured Assistant host (INV-6) |
| Message history | `zalo_history.db`, `threads_cache.json`, caches | Private user data: must be `0600` (`threads_cache.json` is not yet, see §7); never used as a fixture |
| Captures | `dump/`, `test_lab/`, `*.flow`, `call_trace.log` | Gitignored; redact before quoting in `re-notes/` or issues |

If a secret leaks (commit, issue, screenshot, agent prompt), revoke the session from the Zalo phone app
(login history), remove the exposure, and record the incident in the worklog.

## 2. Network destinations

| Destination | Client | Credentials |
| :--- | :--- | :--- |
| Zalo API/WebSocket (`*.zalo.me`, `*.zaloapp.com`, `*.zadn.vn`, `*.zdn.vn`) | Session client | Zalo cookies, only after `is_trusted_zalo_url` passes (https/wss) |
| Zalo media CDNs (`dlmd.me`, `dlfl.vn`) | Cookie-free client (`is_trusted_zalo_media_url`) | **None** |
| GitHub Releases API for `minhkhang1008/zalo-next` | Updater | None. Verify SHA-256 and the signature/format before install (ADR-0002) |
| Sync V2 WASM runtime (`chat.zalo.me/libs/…wasm`, in `endpoints.rs`) | Dedicated fetch | None. 2 MiB cap, no redirects, pinned SHA-256 (`sync_runtime.rs`), stored `0600` in `.sync-runtime/` |
| FFmpeg 9.0.2 (`osxexperts.net` arm64, `evermeet.cx` Intel, redirect allowed to the `e.deolaha.ca` mirror; WinGet/apt on other OSes) | Dedicated fetch | None. Pinned SHA-256, 128 MiB ZIP / 192 MiB unpacked caps, only the `ffmpeg` entry is extracted, binary `0700` |
| Assistant (owner-configured OpenAI-compatible URL) | `ai` module client | Assistant key only. `https`, or `http` to loopback (`localhost`, `127.0.0.1`, `::1`); no redirects, no cookie jar, body cap, timeout |
| Call media (RTP/RTCP) | `call.rs` | Ephemeral UDP sockets during a call. No listening server |

Rules:
- **Never widen `is_trusted_zalo_url`.** A new non-Zalo destination gets its own cookie-free client, a row in
  this table, and owner approval.
- **Never auto-fetch URLs from incoming messages** (no automatic link previews, no auto-download of received
  files). Fetching happens only on an explicit user action.
- No telemetry, no external translation services, no scripting engines, no re-enabling the REST/MCP server.
- New network-capable or parsing crates need a one-line justification in the commit or PR.

## 3. Files and permissions

- Sensitive files are written with `data_dir::write_private_atomic` (`0600`, temp file + rename) or
  `write_private_file` (`0600`, in place). Directories use `ensure_private_dir` / `PrivateTempDir` (`0700`).
- Temporary decrypted databases and audio are deleted when the work ends, whether it succeeds or fails.
- File names from the network or from archives are never used as paths. Sanitise them and write only inside
  app-owned directories. Reject path traversal when unpacking.
- History sync keeps its caps: **256 MiB** download and **512 MiB** decompressed (`history_sync.rs`). Document
  preview: 10 MiB download, Office ≤ 2,000 entries / 8 MiB per entry / 32 MiB total, PDF in a child process
  with a 12 s timeout.

## 4. Cryptography

| Primitive | Use | Rule |
| :--- | :--- | :--- |
| AES-256-CBC, zero IV, PKCS#7 (`ParamCipher v2`) | Pre-login params | Protocol compatibility. Do not "improve" it, or the server rejects it (`18060`) |
| AES-CBC with `zpw_enk` (128- or 256-bit key) | Post-login params and responses | Same |
| AES-GCM (16-byte tag) | WebSocket frame payloads (`socket.rs`) | Protocol compatibility; never reuse a nonce in anything we encrypt |
| MD5 | `signkey`, `ParamCipher v2` key derivation, cache file names | Protocol and naming only. Never for integrity or authentication decisions of our own |
| SHA-256 | Updater, FFmpeg, WASM runtime | Mandatory before anything is executed or installed |
| Randomness | Ids, nonces, keys | OS CSPRNG (`getrandom`) |

Crypto changes need a known-answer test and protocol evidence (see [protocol.md](protocol.md)).

## 5. Untrusted content

Message text, names, file names, sticker and link payloads, raw JSON, received documents and archives:
- Render them as text. Never interpret them as markup, commands or paths.
- Bound lengths, counts, nesting depth and decompressed size.
- Wrap message text as **data** in Assistant prompts (prompt-injection defence, ADR-0004).

## 7. Known debt (found 2026-11-27, not yet fixed)

These go against the rules above. Fix each one as its own task, with an adversarial review:
- **Updater:** the SHA-256 digest is optional. In-place macOS replacement requires it, but the Windows
  portable `.exe` and Linux AppImage paths can install after format checks alone (`updater.rs`,
  `gui/worker.rs::apply_github_release_update`). Asset and checksum URLs come from release metadata and are not
  host-checked against `github.com/minhkhang1008/zalo-next`.
- **Legacy image/sticker loader** (`gui/worker.rs`, image fetch branch): uses the session HTTP client (cookie
  jar) for arbitrary `http(s)` image URLs without `is_trusted_zalo_url`, reads non-video bodies without a size
  cap, and caches into the shared `stickers_cache/`. The cookie jar's domain scoping keeps `zpw_sek` on Zalo
  hosts, but the fetch itself is unbounded and reaches non-Zalo hosts.
- **Link-card thumbnails** in incoming messages are fetched automatically on a cache miss
  (`gui/messages.rs`). That is an automatic request to a URL chosen by the sender.
- **`threads_cache.json`** and per-account thread snapshots are written with plain `std::fs::write`
  (`chat.rs`), so their mode depends on umask instead of being forced to `0600`.

## 8. Review checklist

- [ ] No secret in code, logs, errors, tests, fixtures, commit messages or agent briefs.
- [ ] Every credentialed connection checks `is_trusted_zalo_url`. Non-Zalo traffic uses a cookie-free client.
- [ ] New downloads have a size cap, a timeout, no redirects (or an allowlist), and a hash check if executed.
- [ ] New sensitive files are `0600` and written atomically.
- [ ] Run the `security-audit` skill (Claude) or an adversarial Codex review for changes in this area.
