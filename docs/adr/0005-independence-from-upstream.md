# ADR-0005: Zalo-next is independent of upstream ctdozlite

- **Status:** accepted
- **Date:** 2026-11-27
- **Deciders:** repository owner

## Context

Zalo-next started as a fork of [ctdozlite](https://github.com/ctdoteam1110/ctdozlite). Through November 2026,
upstream changes were reviewed and ported in batches (v0.1.1 on 2026-11-12/13, v0.1.2 on 2026-11-17). Those
reviews showed that wholesale merging does not work. A simulated merge conflicted in 18 files, and upstream
lacks this fork's rules:
- trusted-URL allowlist, download and decompression caps, `0600` files;
- disabled REST/MCP, private update source;
- macOS clipboard/IME/emoji fixes.

The two schemas also collide: the same `PRAGMA user_version` with different columns. The owner is now
migrating the project to stand on its own, with its own docs, standards and agent workflow.

## Decision

Zalo-next is developed independently. Upstream is a source of ideas, not a branch to track.
- The owner watches upstream, tests what looks useful, and decides what to bring over. **Agents never merge,
  cherry-pick or copy upstream files on their own initiative.**
- When the owner asks for a port, it is re-implemented against Zalo-next's code as new work. It must follow every
  rule in `AGENTS.md` §4, carry its own tests, and cite the upstream commit or file it follows (and add
  protocol evidence in `re-notes/` when the protocol is involved).
- Identifiers kept for user continuity: the bundle id `com.ctdoteam1110.ctdozlite`, the data directory
  `Application Support/ctdozlite`, and the `CTDOZ_*` environment variables. Changing them needs a migration
  plan and owner approval.
- The README keeps the attribution and the MIT/Apache-2.0 licence.

## Consequences

- Positive:
  - no merge churn;
  - fork protections cannot be lost by resolving a conflict "their way";
  - docs describe this project only.
- Negative:
  - upstream fixes arrive only when the owner notices them;
  - shared bugs may be fixed twice.
- Follow-ups: a possible future rename of the data directory, bundle id and env vars (needs a migration that
  keeps logins and OS permissions).

## Alternatives Considered

| Option | Why rejected |
| :--- | :--- |
| Keep merging upstream releases | Large conflicts, risk of losing fork-only security fixes, incompatible schema |
| Agents scan upstream and port automatically | The owner wants to test changes first; automatic ports bypass that judgment |
| Drop all mention of upstream | Attribution is required, and the kept legacy identifiers need an explanation |
