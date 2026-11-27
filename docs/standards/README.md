# Engineering Standards

Rules for changing Zalo-next. They apply to humans and agents alike. `AGENTS.md` §4 lists the non-negotiables;
these files explain them and add detail for each area.

Each file separates **rules** (follow them in every change) from **known debt** (places where existing code
does not yet comply). Do not copy debt into new code. Do not fix debt as a side effect of an unrelated change
either: make it its own task.

| File | Read before touching |
| :--- | :--- |
| [coding.md](coding.md) | Any Rust code: comments, errors, logging, formatting, lint |
| [concurrency.md](concurrency.md) | `gui/worker.rs`, `GuiEvent`/`WorkerCmd`, threads, locks, background tasks |
| [storage.md](storage.md) | SQLite schema, queries, migrations, files in the data directory |
| [security.md](security.md) | Session, crypto, HTTP clients, downloads, updater, AI client, permissions |
| [protocol.md](protocol.md) | Zalo HTTP/WebSocket calls, params, signing, frame IDs, Sync V2 |
| [testing.md](testing.md) | Tests and fixtures |
| [git-and-issues.md](git-and-issues.md) | Commits, branches, releases, GitHub issues |
| [docs-and-decisions.md](docs-and-decisions.md) | Product docs, ADRs, glossary, worklog |
