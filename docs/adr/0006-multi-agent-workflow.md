# ADR-0006: Claude Code orchestrates, Codex implements and cross-reviews

- **Status:** accepted
- **Date:** 2026-11-27
- **Deciders:** repository owner

## Context

The owner works with several coding agents. Earlier sessions alternated Claude Code and Codex by hand, with
`.agents/WORKLOG.md` as shared memory. That caused some problems:
- fixes were reported as working when only unit tests had passed;
- the uncommitted work of one agent was hard for the next to judge;
- two instruction files (`AGENTS.md` and a separate `CLAUDE.md`) drifted apart.

The owner installed the Codex plugin for Claude Code (`codex@openai-codex`). It lets Claude hand tasks to Codex
(`task`, workspace-write or read-only sandbox) and request native or adversarial reviews.

Published experience with this pairing, mostly practitioner write-ups, agrees on the following:
- separate the writer and the reviewer, and keep the reviewer read-only;
- give the reviewer the diff and the spec, not the writer's self-summary;
- bound review loops (one or two passes);
- skip cross-review for tiny changes that have tests;
- keep the experimental stop-time review gate off, since it has no iteration cap and burns usage;
- keep one instruction file as the source of truth.

## Decision

1. **Roles:**
   - **Claude Code** (Opus 5.5) is the orchestrator. It plans, designs, talks to the owner, makes small
     edits, owns docs/CHANGELOG/worklog/commits, and reviews every Codex diff.
   - **Codex** is the worker (multi-file implementation, test suites, mechanical refactors, independent
     diagnosis) and the cross-reviewer of Claude's changes.
2. **Instructions:** `AGENTS.md` is the single source of truth, read by both agents. `CLAUDE.md` holds only
   Claude's orchestration playbook (delegation rules, brief template, review matrix). Claude Code loads both
   files, so `CLAUDE.md` does not import `AGENTS.md`.
3. **Autonomy:** Claude delegates and requests reviews **without asking** when a rule in `CLAUDE.md` matches,
   and reports afterwards. Verified, in-scope review findings may be fixed directly. Findings that change scope,
   protocol or data go to the owner. There are at most two review rounds.
4. **Safety:**
   - one writer per file at a time; parallel writers use separate git worktrees;
   - delegated Codex runs don't commit or write the worklog;
   - briefs never contain secrets;
   - a mandatory adversarial review applies to protocol, security, storage and concurrency changes;
   - the stop-time review gate stays off.
5. **Mechanics:** `scripts/codex.sh` finds the installed plugin companion, so Claude can call it from Bash
   (several plugin slash commands can only be typed by the owner).

## Consequences

- Positive:
  - cross-model review catches errors a single agent misses;
  - Claude's context stays small (Codex returns compact reports);
  - the rules hold no matter which agent the owner opens directly.
- Negative:
  - usage is metered on two providers;
  - a Codex job adds wall time;
  - background jobs need monitoring;
  - the plugin is young, so commands and flags may change, and `scripts/codex.sh` and `CLAUDE.md` must
    follow.
- Follow-ups:
  - revisit the delegation thresholds after a few weeks of real use;
  - record misfires in the worklog.

## Alternatives Considered

| Option | Why rejected |
| :--- | :--- |
| Single agent only | Loses independent review; the worklog shows repeated "fixed" claims that were not |
| Codex orchestrates, Claude works | The owner prefers Claude for planning and conversation; the plugin is built for Claude → Codex |
| Ask the owner before every hand-off | The owner chose automatic delegation by rules |
| Enable the stop-time review gate | Reviews every turn without an iteration cap; too slow and costly for daily use |
| Symlink `CLAUDE.md` → `AGENTS.md` | Claude would lose a place for orchestration-only instructions; both files are loaded anyway |
