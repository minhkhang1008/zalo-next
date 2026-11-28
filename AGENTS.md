# AGENTS.md

Operating guide for every agent (Claude Code, Codex, others) and every human working in this repository.
This file is the single source of truth for project rules. Claude Code also reads `CLAUDE.md`, which
only adds the orchestration playbook for Claude; it never overrides anything here.

> **Read first:** before starting, read the last 3–5 entries of [`.agents/WORKLOG.md`](.agents/WORKLOG.md)
> (`grep -n '^## ' .agents/WORKLOG.md | tail`). They hold recent root causes, unverified fixes and open
> follow-ups. After finishing **any** task, append one entry using the template at the top of that file.

## 1. Project

**Zalo-next** is a native Zalo desktop client written in Rust, maintained by one owner
(`minhkhang1008`) and focused on **macOS**. It uses an `egui` GUI, a `ratatui` TUI, a Tokio worker for all
network/disk/crypto work, and a local SQLite message store. There is no Node.js, Electron or Chromium.

- Repo: <https://github.com/minhkhang1008/zalo-next>. Crate `zalo-next`, library `zalo_rust`, edition 2021. Rust **1.97+** (required by the locked `wasmtime` 50).
- Origin: started as a fork of [ctdozlite](https://github.com/ctdoteam1110/ctdozlite) and has since diverged
  into an independent project. See §7 for how upstream changes are handled.
- Windows and Linux packages are built by CI but are **not** tested regularly. Treat them as best effort and
  never claim they work without evidence.
- Language: code, comments, `AGENTS.md`, `CLAUDE.md`, `docs/standards/`, ADRs, `GLOSSARY.md` and the worklog
  are in **English**. User-facing strings, `README.md` and the product docs in `docs/` are in **Vietnamese**.

## 2. Commands

| Task | Command | Notes |
| :--- | :--- | :--- |
| Quick check | `cargo check --message-format=short` | Default after each edit |
| Test (all) | `cargo test --locked` | ~470 tests; no network needed (the one live call test is `#[ignore]`). Python script tests: `python3 -m unittest tests/release_notes_test.py` |
| Test (one area) | `cargo test --locked --lib <module_or_name>` | Prefer this while iterating |
| Format | `rustfmt --edition 2021 <files you changed>` | **Never `cargo fmt`**: much of the crate is not rustfmt-clean, so a crate-wide format buries real diffs |
| Lint | `cargo clippy --all-targets --message-format=short` | `-D warnings` already fails on existing code. Rule: add **no new warnings in the files you touched**. Do not fix unrelated warnings |
| Run GUI (dev) | `cargo run -- --gui --dev` | `--dev` enables the Dev Inspector (network log) |
| Run TUI | `cargo run -- --tui --qr` | |
| Release binary | `cargo build --release --locked --bin zalo-next` | |
| Package `.dmg` | `bash packaging/package-local.sh` | Output in `dist/` |
| macOS helpers | `bash native/macos/build-native-helpers.sh` | Rebuild when notifications or previews regress (see §6) |

With CMake 4.x, export `CMAKE_POLICY_VERSION_MINIMUM=3.5` before building (`opus`/`audiopus_sys`).

## 3. Source Map

| Path | Responsibility |
| :--- | :--- |
| `src/main.rs` | CLI flags; selects GUI, TUI or one-shot commands (`--version`, `--check-update`, `--print-data-dir`) |
| `src/lib.rs` | Library surface `zalo_rust`, shared by binaries and tests |
| `src/gui.rs` | `ZaloGuiApp`, `GuiEvent` handling, login screen, window chrome |
| `src/gui/worker.rs` | Tokio runtime (2 threads); `WorkerCmd` (UI → worker, unbounded mpsc) and `GuiEvent` (worker → UI, `std::sync::mpsc`); updater check (`GITHUB_REPO`) |
| `src/gui/workspace.rs`, `messages.rs`, `modals.rs` | Layout and conversation list; message bubbles and context menus; settings, call window, viewers, dialogs |
| `src/gui/picker.rs`, `stickers.rs`, `album.rs`, `appearance.rs`, `theme.rs` | Sticker/Emoji/GIF panel, sticker suggestions, photo albums, appearance settings, palette and fonts |
| `src/gui/ai_ui.rs`, `src/ai.rs` | Local `@AI` assistant (ADR-0004) |
| `src/gui/tray.rs`, `background.rs`, `macos_*.rs`, `clipboard_write.rs` | Background mode, tray, macOS clipboard/drag (objc2 FFI) |
| `src/tui.rs`, `src/ui.rs` | ratatui client |
| `src/session.rs` | `session.json`, cookies, request signing, `is_trusted_zalo_url`, `ZPW_VER`/`ZPW_TYPE` |
| `src/login.rs`, `src/qr.rs` | QR login (password login exists only as a TUI/CLI path) |
| `src/chat.rs` | Message/thread API: send, quote, reaction, recall, history, groups |
| `src/socket.rs` | WebSocket `zpw_ws`, frame codec, command IDs |
| `src/param_cipher.rs`, `src/crypto.rs` | `ParamCipher v2`, AES-CBC/GCM, MD5 `signkey` |
| `src/endpoints.rs` | Service map and fixed hosts (Zalo and pinned third-party downloads) |
| `src/store.rs`, `src/productivity.rs`, `src/scheduler.rs` | SQLite: messages/threads; drafts/saved/reminders; scheduled sends |
| `src/history_sync.rs`, `src/history_sync/android.rs`, `src/zproto_host.rs`, `src/sync_runtime.rs` | Phone history sync: Android backup and Sync V2 (WASM runtime via `wasmtime`) |
| `src/call.rs`, `src/audio.rs`, `src/voice.rs`, `src/video_*.rs` | VoIP signalling, audio (cpal/opus), voice messages, FFmpeg tooling and playback |
| `src/media.rs`, `src/media_resources.rs`, `src/doc_preview.rs` | Attachments/images/stickers, bounded downloads, document text preview |
| `src/updater.rs`, `src/data_dir.rs`, `src/account_manager.rs` | Self-update (ADR-0002), data directory resolution and migration, accounts |
| `src/api_mcp.rs` | **Disabled** REST/MCP server. Only its auto-reply rule helpers are still used |
| `src/feed.rs` | Feed parsing. The Feed UI was removed; some helpers are still used by `modals.rs` |
| `src/bin/*` | Developer probes (`test_fetch`, `call_probe`, …), not shipped |
| `native/macos/` | Swift helpers (notifications, previews, history codec). Build output in `native/macos/build/` is gitignored |
| `re-notes/` | Protocol evidence notes (`NN_topic.md`). Required for protocol changes |
| `tasks/` | Open plans and backlogs (`todo.md`, feature task files) |
| `docs/` | Product docs (Vietnamese), `docs/standards/` (English rules), `docs/adr/` (decisions) |

## 4. Non-Negotiables

1. **No secrets in Git or in output.** `session.json`, `*.db`, cookies, `zpw_sek`, `zpw_enk`, IMEI,
   `ai_secret.json`, Sync V2 keys and raw captures never get committed, logged, pasted into issues, or sent to
   another agent or model. Check `git status` before each commit.
2. **No blocking work on the UI thread.** In the GUI, network, disk, SQLite, crypto and process spawning go
   through `WorkerCmd`. The UI only renders and sends commands. Every fallible command must emit a terminal
   `GuiEvent`. (The TUI runs its own store and poll task. Known GUI exceptions are listed in
   `docs/standards/concurrency.md` §4.)
3. **Zalo credentials go only to Zalo hosts.** Any connection that carries `zpw_sek`, `zpw_uin` or session
   cookies (HTTP, WebSocket, history sync, uploads) must pass `is_trusted_zalo_url`
   (`*.zalo.me`, `*.zaloapp.com`, `*.zadn.vn`, `*.zdn.vn`). Non-Zalo traffic (updater, AI, FFmpeg, Sync V2
   runtime) uses its own client with no cookies, and never widens that allowlist.
4. **Zalo API requests follow the protocol path the official client uses:** signed plaintext params
   (`reqqr`, `getServerInfo`), or `ParamCipher v2` (`getLoginInfo`, password login v2)
   during login; `zpw_enk` AES for everything after login; `signkey` where the endpoint requires it;
   session cookies. Put **new** hosts in
   `src/endpoints.rs`. Many existing hosts are still hard-coded in `session.rs` and `chat.rs`. That is known
   debt: don't add to it, and don't do drive-by moves either.
5. **Protocol changes need evidence.** Any change to params, signing, `zpw_ver`/`zpw_type`, frame IDs or the
   cipher flow needs a capture or bundle reference written down in `re-notes/`, plus a test or documented
   manual verification. See [`docs/standards/protocol.md`](docs/standards/protocol.md).
   - *Owner exception (2026-11-17), Sync V2 / phone history sync only:* agents may change Sync V2 decoding
     and acknowledgement behaviour without a fresh capture **when the current behaviour is broken**. The change
     must still be written in `re-notes/` (what changed, why, and the reference it follows), covered by an
     offline fixture test, and logged as **unverified against a real phone** until the owner confirms it.
     The exception does not cover login, signing, `ParamCipher v2`, send/receive frames or calls.
6. **Privacy.** Never auto-fetch URLs from incoming messages (no automatic link previews; images and thumbnails
   from non-Zalo hosts load only on click). The REST/MCP server stays disabled. Do not add telemetry, translation services or scripting engines.
7. **Self-updater** checks and downloads **only** from `minhkhang1008/zalo-next` (`GITHUB_REPO` in
   `src/gui/worker.rs`), must verify SHA-256 before installing, never sends Zalo cookies or tokens to GitHub,
   and never gets another update source.
8. **File permissions and memory caps.** Sensitive files (`session.json`, `zalo_history.db`, `ai_secret.json`,
   temporary decrypted DBs, Sync V2 identity) are `0o600` and their directories `0o700` on macOS/Unix. History
   sync keeps its caps (256 MiB download, 512 MiB decompressed). Document preview keeps its own caps.
9. **No `.unwrap()`/`.expect()` on data from the network, JSON, SQLite, files or user input.** Tests and real
   compile-time invariants are fine.
10. **Tests never touch the network, the real `session.json` or the real database.** Use temp dirs and
    `:memory:`. One test once wrote into the repo-root `zalo_history.db`; pass explicit paths.
11. **Rust only for new code.** Prefer objc2 or `extern "system"` FFI declared in Rust over new C, Swift or
    scripts. The existing Swift helpers in `native/macos/` are maintained, not extended, unless the owner agrees.
12. **Comments in English, user-facing strings in Vietnamese.** Comment rules are in
    [`docs/standards/coding.md`](docs/standards/coding.md).

## 5. Git

- **Commit freely, push only with approval.** Agents may create local commits on the current branch without
  asking: Conventional Commits, one logical change each, staging **explicit paths only** (never `git add -A`).
  **Pushing, tagging, releasing, force-pushing, rewriting published history, or deleting branches always needs
  the owner's explicit OK** in the current conversation. The one exception is the cleanup rule below.
- Work happens on `main` by default. Use a short-lived branch (`feature/…`, `fix/…`, `core/…`) for risky or
  multi-session work: protocol, schema, crypto, release packaging.
- **Clean up as soon as a piece of work is done** (owner, 2026-11-28; the disk is small and every worktree
  builds its own multi-GB `target/`). Keep worktrees to a minimum: prefer the current checkout, and add a
  worktree only when parallel work really needs one. When a branch an agent created is merged, the agent
  removes its worktree (`git worktree remove`) and deletes the local branch (`git branch -d`) right away,
  without asking. This covers only local branches the agent created and that are fully merged. Remote
  branches, unmerged branches and branches the agent did not create still need the owner's OK.
- Never commit: `log.txt`, `call_trace.log`, `voip.log`, `skills/`, anything under `target/`, `dist/`,
  `dump/`, `accounts/`, or data files in the repo root. `.gitignore` covers most of them, so check anyway.
- The owner often has large uncommitted edits. Don't stage, revert, stash or reformat files you did not change.
- An ff-merge or checkout that untracks `native/macos/build/` deletes the built helpers from disk. Afterwards,
  run `bash native/macos/build-native-helpers.sh` and check `ls native/macos/build`.
- Details: [`docs/standards/git-and-issues.md`](docs/standards/git-and-issues.md).

## 6. Definition of Done

A task is done when all of the following hold:

1. `cargo check` passes, and the tests for the touched modules pass (`cargo test --locked` before a commit
   that touches more than one module).
2. Touched files are formatted with `rustfmt` and add no new clippy warnings.
3. User-visible changes have a line under `## [Unreleased]` in `CHANGELOG.md`, and docs in `docs/` are updated
   in the same change when behaviour, settings or limits changed.
4. A worklog entry is appended. It states what was verified and, explicitly, what was **not** (live server
   behaviour, GUI visuals, phone sync, calls, Windows/Linux). Never report a fix as working on the strength of
   unit tests alone when the bug was observed live.

## 7. Upstream (ctdozlite)

- The owner watches ctdozlite and decides what is worth taking. **Agents never pull from upstream on their
  own initiative**: no merges, no cherry-picks, no wholesale file replacement.
- When the owner asks for a port, re-implement it against this repo's code. Keep every rule in §4, plus the
  fork-specific behaviour upstream lacks: trusted-URL checks, caps, `0600` files, private updater source,
  disabled REST/MCP, macOS clipboard/IME/emoji fixes, our versioning. Record the upstream commit or file it
  follows in the worklog (and in `re-notes/` for protocol work), and add tests as for any new change.
- Upstream schemas and data directories are **not** compatible with ours (same `user_version`, different
  columns). Never point an upstream build at our database.
- Kept on purpose: the bundle id `com.ctdoteam1110.ctdozlite`, the data directory
  `~/Library/Application Support/ctdozlite`, and the `CTDOZ_*` environment variables. Renaming any of them
  logs users out or drops OS permissions. That needs a migration plan and the owner's approval.

## 8. Working With Several Agents

The owner uses **Claude Code as orchestrator** and **Codex as worker and cross-reviewer**. The workflow is
recorded in [ADR-0006](docs/adr/0006-multi-agent-workflow.md), and Claude's playbook is in `CLAUDE.md`.
Rules that bind every agent:

- **One writer per file at a time.** While a delegated write task runs, nobody else edits the files in its scope.
- **When you are a delegated worker** (your prompt arrives as a brief from an orchestrator):
  - edit only the files the brief allows;
  - do not commit, push or append to the worklog (the orchestrator does that);
  - run the verification command the brief gives you;
  - finish with a short report: what changed and why, the files touched, the test commands and their result,
    and anything unverified or uncertain. No long logs.
- **When you are a reviewer:**
  - stay read-only;
  - ground every finding in a file and line, and order findings by severity;
  - mark anything you inferred without proof;
  - do not "fix while reviewing".
- **When you work directly for the owner** (no orchestrator): the normal rules apply, including commits (§5)
  and the worklog.
- Never pass secrets or private data to another agent (see §4.1). Redact to `zpw_sek=<redacted>`.

## 9. Skills

Repository skills are installed from [`actionbook/rust-skills`](https://github.com/actionbook/rust-skills).
Sources and hashes are pinned in `skills-lock.json`. Files live in `.agents/skills/` (Codex), with symlinks in
`.claude/skills/` (Claude Code). Both directories are gitignored, so on a fresh clone reinstall them from the
lock file with the `skills` CLI. **Repository rules beat skill advice.** For example, ignore a skill that says
to run `cargo fmt` or crate-wide `clippy -D warnings`.

Use these skills when the situation matches:

| Situation in this repo | Skill |
| :--- | :--- |
| Unsure which Rust skill applies | `rust-router` |
| Borrow-checker errors in egui closures and long GUI functions | `m01-ownership`, `m03-mutability` |
| `Arc`/`Rc`/`RefCell`, caches, texture handles | `m02-resource` |
| Generics and trait bounds, `dyn`, E0277/E0308 | `m04-zero-cost` |
| Worker, Tokio tasks, channels, `spawn_blocking`, locks, `Send`/`Sync` | `m07-concurrency` |
| Startup and shutdown, `Drop`, `OnceLock`, connection lifetimes | `m12-lifecycle` |
| `Result` design, removing `unwrap`, `anyhow` context | `m06-error-handling` |
| Mapping Zalo `error_code`s to retry, re-login or user message | `m13-domain-error` |
| New state machines (calls, sync, login), newtypes for ids | `m05-type-driven`, `m09-domain` |
| Idle CPU, repaint cost, history pagination, media cache | `m10-performance` |
| Choosing or upgrading crates, feature flags | `m11-ecosystem`, `rust-learner`, `rust-deps-visualizer` |
| Reviewing a diff for idioms and smells | `m15-anti-pattern`, `coding-guidelines` |
| Any change to session, crypto, updater, AI client, downloads, file permissions | `security-audit` |
| `unsafe`, objc2/AppKit FFI, `#[repr(C)]`, clipboard DIB | `unsafe-checker` |
| TUI (`ratatui`/`crossterm`) work | `domain-cli` |
| Large renames or moves, call graphs, finding definitions (need rust-analyzer) | `rust-refactor-helper`, `rust-call-graph`, `rust-code-navigator`, `rust-symbol-analyzer`, `rust-trait-explorer` |

Not relevant here (don't load them): `domain-fintech`, `domain-embedded`, `domain-iot`, `domain-ml`,
`domain-cloud-native`, `domain-web`, `rust-daily`, `rust-skill-creator`, `m14-mental-model`,
`meta-cognition-parallel`, and the internal `core-*` skills.

## 10. Documentation Map

| Need | Read |
| :--- | :--- |
| Doc index | [`docs/README.md`](docs/README.md) |
| Architecture, threads, storage, network destinations | [`docs/KIEN_TRUC_HE_THONG.md`](docs/KIEN_TRUC_HE_THONG.md) |
| Protocol summary and `re-notes/` index | [`docs/GIAO_THUC_ZALO.md`](docs/GIAO_THUC_ZALO.md) |
| Engineering rules by topic | [`docs/standards/`](docs/standards/README.md) |
| Decisions | [`docs/adr/`](docs/adr/README.md) |
| Domain terms and invariants (`INV-N`) | [`GLOSSARY.md`](GLOSSARY.md) |
| Features and their verification status | [`docs/TINH_NANG.md`](docs/TINH_NANG.md) |
| Open work | [`tasks/todo.md`](tasks/todo.md) |

Before changing code in an area, read the matching standard file:

| Area | Standard |
| :--- | :--- |
| Comments, errors, style | `coding.md` |
| Worker, channels, locks | `concurrency.md` |
| SQLite | `storage.md` |
| Secrets, crypto, network | `security.md` |
| Zalo protocol | `protocol.md` |
| Tests | `testing.md` |
