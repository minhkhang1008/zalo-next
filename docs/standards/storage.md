# Storage Standards (SQLite and data directory)

## 1. Where data lives

The process `chdir`s into the data directory at startup (`src/data_dir.rs`), so relative paths such as
`zalo_history.db` resolve there.

| Platform / flag | Data directory |
| :--- | :--- |
| macOS (any launch mode, when `HOME` is set) | `~/Library/Application Support/ctdozlite`. The legacy name is kept on purpose; see AGENTS.md §7 |
| Windows / Linux | Current working directory |
| `--data-dir <dir>` / `--profile <name>` | That directory / `profiles/<name>` |

Main files: `session.json`, `zalo_history.db` (+ `-wal`/`-shm`), `threads_cache.json`, `app_settings.json`,
`theme.json`, `custom_stickers.json`, `autoreply_rules.json`, `ai_secret.json`, `sync_identity.json`,
`active_account`, `accounts/`, `stickers_cache/`, `media_cache/`, `.sync-runtime/`, `tools/ffmpeg`.
Private files are `0600` and directories `0700` (see [security.md](security.md)).

## 2. Schema (one database: `zalo_history.db`)

| Table | Owner module | Purpose |
| :--- | :--- | :--- |
| `messages(msg_id PK, thread_id, sender_id, sender_name, text, msg_type, ts, is_outgoing, raw_json)` + `idx_thread_ts(thread_id, ts)` | `store.rs` | Message history, idempotent by `msg_id` (INV-4) |
| `threads(thread_id PK, name, is_group, last_ts, last_read_msg_id)` | `store.rs` | Conversation list and read point |
| `drafts`, `saved_items`, `message_reminders` | `productivity.rs` | Per-conversation drafts, Saved items, local reminders |
| `scheduled_messages` | `scheduler.rs` | Scheduled sends |
| `local_ai_messages` | `ai.rs` | Assistant Q&A, never mixed into `messages` (INV-5) |

`PRAGMA user_version` is **shared** by every module: `store.rs` sets 1, and `productivity.rs` upgrades to 2.
Before you bump it, read all the `user_version` sites (`grep -rn user_version src`).

Upstream ctdozlite uses the same `user_version` numbers with different columns. The two schemas are
incompatible, so never open one app's DB with the other.

## 3. Rules

- **Add tables, don't overload columns.** New local-only data gets its own table (the pattern of ADR-0004).
  Don't add a "type" value to `messages` that every reader would then have to filter.
- **Migrations are forward-only and idempotent:**
  - each step runs in a transaction and bumps `user_version` in that same transaction;
  - a fresh DB and an existing DB must both end in the same shape;
  - test both paths (see `productivity.rs`, which tests migration from a legacy schema).
- **Never drop or rewrite user data in a migration** without a backup copy and the owner's approval.
- **Parameterise every query** (`params![…]`). Never format values into SQL. Escape `LIKE` wildcards when
  user text is used as a pattern (advanced search does this).
- **Bounded reads:** always `LIMIT` (search returns at most 200 results). History pagination goes through
  `idx_thread_ts`.
- **Batch writes in one transaction** (history sync, imports). No network, crypto or `.await` inside a
  transaction.
- **SQLite runs in the worker** or in `spawn_blocking`, never on the UI thread (see
  [concurrency.md](concurrency.md)).
- **Imports never delete.** Importing history and Sync V2 upsert by `msg_id`. They may refresh a matching
  row's text/type/raw JSON and thread metadata, but never remove rows.
- **Real deletes.** "Delete for me" or clearing a conversation removes rows. There is no hidden soft delete.
- Tests use `:memory:` or a private temp directory, never the repo-root or user database.

## 4. Known debt

- Connections are opened per operation in many places (`MessageStore::open`), and there is no shared WAL
  configuration. Don't add a new long-lived connection without considering concurrent writers (worker, scheduler,
  history sync).
- Schema bootstrap is spread across modules instead of living in one migration list. Keep new steps close to
  their owner module, and document the `user_version` they set.

## 5. Review checklist

- [ ] The migration is transactional and idempotent, with tests for a fresh DB and an existing DB.
- [ ] `user_version` changes are coordinated with the other modules.
- [ ] Queries are parameterised and bounded. Large writes are batched.
- [ ] No SQLite access from UI code.
