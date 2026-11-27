# Coding Standards

## 1. Language and placement

- New code is **Rust**. For OS APIs, use `objc2` / `objc2-app-kit` (as in `src/gui/macos_clipboard.rs`) or
  `extern "system"` declarations in Rust. Do not add C, new Swift helpers or new script runtimes without the
  owner's agreement. The Swift helpers already in `native/macos/` are maintained only.
- Several files are very large (`gui/modals.rs` ~10k lines, `chat.rs` ~10k, `gui/workspace.rs` ~7.5k,
  `gui/messages.rs`, `gui/worker.rs`, `call.rs` ~5k each). Put new features in **new modules**
  (`src/gui/<feature>.rs`, `src/<feature>.rs`) and call them from the big files. Don't grow the big files.
- User-facing strings (labels, toasts, errors shown in the UI, CLI help) are **Vietnamese**. Identifiers,
  comments and log keys are **English**.

## 2. Comments

Code says *how*; comments say *why*.

| Kind | Form | When |
| :--- | :--- | :--- |
| Contract | `///` and `//!` | Public items and shared helpers. Add `# Errors` / `# Panics` / `# Safety` / `# Invariants` (`INV-N` from `GLOSSARY.md`) when they apply |
| Rationale | `//` prose | Non-obvious decisions, protocol quirks, observed server behaviour. Cite evidence: `// Observed 2026-11-17, re-notes/48_voicecall_zpw_pair.md` |
| Safety | `// SAFETY:` | Directly above **every** `unsafe` block, naming the invariant that makes it sound |
| Debt | `// TODO(<issue or topic>):` | Only for tracked gaps. A bare `// TODO: fix` is not allowed |

Not allowed:
- comments that repeat the code;
- commented-out code;
- "added by X / v2 rewrite" notes;
- banner art.

Older Vietnamese comments may stay, but translate the ones in any region you edit.

## 3. Errors

- Use `anyhow::Result` at module boundaries and `.context("…")` when an error crosses one. Add typed errors only
  where a caller branches on the cause (Zalo `error_code`s, see [protocol.md](protocol.md)).
- **No `unwrap()`/`expect()`** on network, JSON, SQLite, file, clipboard or user-input data. They are allowed
  in tests and for true compile-time invariants (with a message).
- Never swallow an error silently. `let _ = …` is fine for best-effort cleanup and for cases where the failure
  is already reported elsewhere; write a short comment when the reason is not obvious.
- Error text never contains secrets: no cookies, `zpw_sek`, `zpw_enk`, API keys, or `session.json` fragments.
- Every fallible `WorkerCmd` ends in a `GuiEvent` that the UI can show. The UI must never wait forever on a
  command that was dropped.

## 4. Logging

- Use `crate::logger` (`log_request`, `log_error`). Entries feed the Dev Inspector (`--dev`).
- Log ids (`thread_id`, `msg_id`), endpoints and `error_code`. Never log credentials, message bodies of other
  people, or phone numbers. Truncate raw payloads.
- Diagnostic files (`call_trace.log`, `voip.log`, `log.txt`) stay local and are never committed.

## 5. Formatting and lint

- Format **only the files you changed**: `rustfmt --edition 2021 <files>`. Never run `cargo fmt`. Many files
  are not rustfmt-clean, and a crate-wide format buries real changes under noise.
- `cargo clippy --all-targets` must report **no new warnings in touched files**. The crate does not pass
  `-D warnings` today. Clean-ups belong in a separate `style:`/`refactor:` change.
- Keep diffs focused. Don't mix reformatting or renames with behaviour changes.

## 6. Review checklist

- [ ] No `unwrap`/`expect` on external data. Errors carry context, without secrets.
- [ ] New UI strings are in Vietnamese. Comments are in English and explain why.
- [ ] Every `unsafe` block has a `// SAFETY:` comment.
- [ ] New feature code lives in its own module where practical.
- [ ] rustfmt was run on touched files only, with no new clippy warnings.
