# Documentation, Decisions and Worklog

## 1. What goes where

| Content | Location | Language |
| :--- | :--- | :--- |
| Project rules for agents and humans | `AGENTS.md` (+ `CLAUDE.md` for Claude-only orchestration) | English |
| Engineering rules by topic | `docs/standards/` | English |
| Architectural decisions | `docs/adr/NNNN-<kebab>.md` | English (older ADRs are Vietnamese) |
| Domain terms and invariants `INV-N` | `GLOSSARY.md` | English |
| Product docs: install, usage, features, troubleshooting, architecture, protocol overview | `docs/*.md`, `README.md` | Vietnamese |
| Protocol evidence | `re-notes/NN_topic.md` | English or Vietnamese |
| Plans and backlogs | `tasks/` | English |
| User-visible change history | `CHANGELOG.md` (Keep a Changelog, ADR-0003) | Vietnamese |
| Per-task agent log | `.agents/WORKLOG.md` | English |

Don't create dated status reports in `docs/` (for example `PLAN_2026-…md`). Status belongs in the worklog,
plans in `tasks/`, and lasting facts in the product docs.

## 2. Keeping docs true

- Update docs in the **same change** that alters behaviour, settings, limits, CLI flags or data locations.
  `docs/TINH_NANG.md` holds the feature status (✅ 🟡 ⛔ 🚫). Change a status only with evidence, and keep
  "not verified live" notes until the owner confirms.
- Before documenting a fact, check it against the code (grep the constant, flag or function). Docs that describe
  intentions as facts caused real confusion before.
- Don't write a bare `@name` in Markdown that may reach GitHub. Use backticks.

## 3. ADRs

- One decision per file, numbered from `0001`, never renumbered. Index and template: `docs/adr/README.md`.
- Status is one of `proposed`, `accepted`, `superseded by ADR-NNNN`, `deprecated`. An accepted ADR is not
  rewritten: correct it with a new ADR or a dated note at the top.
- Write an ADR for: protocol or crypto strategy, schema direction, a new external network destination,
  agent/tooling policy, and anything that reverses an earlier ADR.
- If a proposal contradicts an accepted ADR, say so explicitly ("contradicts ADR-0004 because …").

## 4. Glossary and invariants

- Use `GLOSSARY.md` terms verbatim in code, tests, issues and ADRs, and avoid the listed synonyms.
- `INV-N` is declared once in `GLOSSARY.md`. The enforcing function mentions it in rustdoc (`# Invariants`),
  and at least one test names it. Ids are never reused.

## 5. Worklog

`.agents/WORKLOG.md` is append-only, newest at the bottom. Use the template at the top of the file.
- Read the last entries before starting work. Append one entry per task, including failed or partial ones.
- Every entry says what was verified and what was **not**.
- In orchestrated work, the orchestrator writes the entry and credits the agents involved. Workers don't
  write entries.
