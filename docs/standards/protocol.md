# Zalo Protocol Standards

The protocol summary is in [`docs/GIAO_THUC_ZALO.md`](../GIAO_THUC_ZALO.md). The evidence lives in
[`re-notes/`](../../re-notes/). This file covers how to **change** protocol code safely.

## 1. Request path

1. Build params as the official web/PC client does (`zpw_ver=694&zpw_type=30`, from `ZPW_VER`/`ZPW_TYPE` in
   `session.rs`). Exception: the `voicecall` endpoints use the PC pair `693/24` (`call.rs::voicecall_zpw`,
   `re-notes/48`).
2. Encode them the way that endpoint expects:
   - signed plaintext query params: `reqqr`, `getServerInfo`;
   - `ParamCipher v2`: `getLoginInfo`, password login v2;
   - AES with `zpw_enk` for every call after login.
   The diagnostic `--v1` password login sends plaintext params and is CLI-only.
3. Add `signkey` where the endpoint requires it (login/bootstrap calls).
4. Send with the session cookie jar, only to a URL that passes `is_trusted_zalo_url`.
5. Decode the `{error_code, error_message, data}` envelope once, then branch on `error_code`.

Hosts: service hosts come from `SERVICE_MAP_V3` / constants in `src/endpoints.rs`. Add new hosts there. The
existing hard-coded hosts in `session.rs`/`chat.rs` are known debt (see AGENTS.md §4.4).

## 2. Error codes

| `error_code` | Meaning | Handling |
| :--- | :--- | :--- |
| `0` | Success | |
| `18012` | QR scanned, not yet confirmed | Keep polling; not an error for the user |
| `18022` | Bad `signkey` | Bug: wrong parameter set. Don't retry |
| `18060` | Params not encrypted as expected | Bug: the cipher path was bypassed. Don't retry |
| `114` | Invalid parameter | Bug: malformed payload. Don't retry |
| `600` / `-102` | `zpw_sek` missing or expired | Fail closed: ask for a QR re-login and keep history |
| `-69` | Seen on group/Cloud history paging | See `re-notes/` and the worklog before touching history paging |
| HTTP `429` | Rate limited (seen mainly from the Assistant host) | Back off and show a retry hint |

When you observe a new code, add it here and to `docs/KHAC_PHUC_SU_CO.md`.

**Retries:** only for network errors and `429`, with backoff (at most 4 attempts). Never retry auth failures,
protocol bugs, or non-idempotent sends without a stable client id. A client message id (`cliMsgId`) is created
once per send (by the worker) and reused for every retry. Reactions and quotes reference the original
`cliMsgId` and fall back to `ts` only when it is missing (`re-notes/35`).

## 3. WebSocket

- Command ids are named constants in `socket.rs` (`CMD_PUSH_MSG_1_1 = 501`, `CMD_PUSH_MSG_GROUP = 521`,
  pulls `510, 511, 513, 514, 515, 517, 518`, control `601–604`, …). Never inline a magic number.
- Unknown commands are never fatal. Today they are forwarded as a generic `WsEvent`; decrypt and inflate run
  inline in the read loop, so don't add heavy work there (move it to the consumer or `spawn_blocking`).
- Reconnect with backoff and re-authenticate before resubscribing.

## 4. Changing protocol behaviour

A change to params, signing, `zpw_ver`/`zpw_type`, frame ids, the cipher flow, or the meaning of a response
field needs **evidence**:

1. **Capture or source:** a Zalo Web/PC capture (redacted), or a reference into the official client bundle
   (file and function name, e.g. `_constructUrlParams`). A captured response's `error_code` counts as evidence.
2. **Note:** add or extend `re-notes/NN_topic.md` (next free number; numbers are never reused) with what was
   observed, the date, and what the code now does.
3. **Proof:** a fixture-based test (a redacted, synthetic response) or documented manual verification
   (account type, flow, observed `error_code`). Live checks need a real login by the owner. Agents never log in
   on the owner's behalf.
4. **Worklog:** state what is verified live and what is not.

**Sync V2 exception (owner, 2026-11-17):** when phone history sync is broken, agents may change Sync V2
decoding/acknowledgement without a fresh capture. Steps 2–4 still apply, and the worklog marks the change
**unverified against a real phone** until the owner confirms it. This exception does not cover login, signing,
`ParamCipher v2`, send/receive frames or calls.

**Porting from upstream** (only when the owner asks, see AGENTS.md §7): an upstream commit is a *reference*,
not evidence that the behaviour is correct. Treat the port as new protocol work under the same steps.

## 5. Review checklist

- [ ] New endpoint: host in `endpoints.rs`, standard cipher/sign path, trusted-URL check.
- [ ] Every new `error_code` branch is handled and documented.
- [ ] `re-notes/` updated, and a fixture test or manual verification recorded.
- [ ] No captured secret in fixtures, notes or commit messages.
