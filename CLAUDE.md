# CLAUDE.md: Orchestrator Playbook

`AGENTS.md` holds every project rule and is loaded automatically. If it is not in your context, read it before
doing anything else. This file adds only how **Claude Code (orchestrator)** works with **Codex (worker and
cross-reviewer)**. The decision behind the setup is in `docs/adr/0006-multi-agent-workflow.md`.

## 1. Roles

| Claude Code (Opus 5.5): orchestrator | Codex (via the `codex@openai-codex` plugin): worker and cross-reviewer |
| :--- | :--- |
| Talks to the owner, clarifies, plans, designs | Implements multi-file changes from a written brief |
| Reproduces bugs and locates the root cause | Writes or extends test suites |
| Small edits (1–2 files, roughly < 80 changed lines) | Wide mechanical refactors |
| Docs, CHANGELOG, worklog, commits | Independent second diagnosis when Claude is stuck |
| Reviews **every** Codex diff and re-runs the tests | Reviews Claude's diffs (read-only) |
| Final say on what ships, reports to the owner | Read-only research over huge files (`chat.rs`, `modals.rs` are 10k+ lines) |

**Standing authorization (owner, 2026-11-27):** Claude delegates to Codex and asks for reviews **on its own**
when a rule in §2 matches. Claude does not need to ask first. It only tells the owner what was delegated
and what came back.

## 2. When to Delegate

Delegate a **write** task to Codex when the work:
- touches 3+ files, or is mostly mechanical (renames, moving code, adding a field through many layers);
- is a test suite for code that already exists;
- has a precise spec. If the design is still open, settle it first: Claude plans, then delegates.

Delegate a **read-only** task to Codex when:
- Claude has made two fix attempts that failed. Ask for an independent diagnosis and do **not** pass along
  Claude's own theory as fact;
- a claim needs checking against a lot of code (docs vs code, "is X still used?").

Keep the work with Claude when:
- it needs conversation context with the owner;
- the change is small;
- it is docs or release work;
- live testing with the owner is involved (GUI runs, phone sync, calls);
- the task needs judgment about privacy or protocol rules that cannot be put fully in a brief.

**Size:** a Codex job should take about 20 minutes or less. Split anything bigger into ordered briefs, each
one buildable and testable on its own.

## 3. How to Call Codex

Use the wrapper. It finds the plugin, which `${CLAUDE_PLUGIN_ROOT}` does not do in plain Bash:

```bash
scripts/codex.sh task --write --prompt-file <brief.md>    # implementation (workspace-write sandbox)
scripts/codex.sh task --prompt-file <brief.md>            # investigation (read-only sandbox)
scripts/codex.sh task --resume-last "<follow-up delta>"   # continue the same Codex thread
scripts/codex.sh review --wait --scope working-tree       # native review of uncommitted changes
scripts/codex.sh review --wait --base <ref>               # native review of a branch/commit range
scripts/codex.sh adversarial-review --wait "<focus>"      # challenge review with focus text
scripts/codex.sh status | result <job-id> | cancel <job-id>
```

- Write briefs to the session scratchpad, not the repo.
- Run long jobs with the Bash tool's `run_in_background: true`. You are notified when the job exits, so do
  not poll. While a write job runs, **do not edit files inside its scope**.
- The plugin's slash commands `review`, `adversarial-review`, `status`, `result`, `cancel` and `transfer` have
  `disable-model-invocation`, so only the owner can type them. Claude uses the wrapper instead. (Without
  `--scope`, a review picks the working tree when it is dirty, and otherwise compares the branch.)
- Alternative: use the `Agent` tool with `subagent_type: "codex:codex-rescue"` (put `--wait` in the prompt; it
  defaults to `--write`). **Never** call `Skill(codex:rescue)`: the plugin documents that it re-enters itself
  and hangs.
- Parallel work: only for **disjoint** file sets. If Claude must keep editing nearby, give Codex a worktree
  (`git worktree add ../zalo-next-<topic> -b <branch>`), run with `--cwd ../zalo-next-<topic>`, and merge it
  back yourself. A new worktree has its own `target/`, so the first build is a full build.
- Leave model and effort at the defaults (`~/.codex/config.toml`) unless the owner asks for something else.
- Keep the stop-time review gate **off** (`/codex:setup --disable-review-gate`). It reviews every turn, has no
  cap on iterations, and burns usage.

## 4. Brief Template

Codex works best with compact, block-structured prompts. Each brief states the goal, the allowed files and the
command that proves it is done:

```xml
<task>
Goal in one or two sentences. Context Codex cannot see: root cause found, evidence (file:line, worklog
entry, re-notes file), the design decision already taken.
</task>
<scope>
May edit: src/a.rs, src/b.rs, tests/c.rs
Must not touch: everything else (especially Cargo.lock, docs/, .agents/WORKLOG.md)
</scope>
<constraints>
Follow AGENTS.md. Most relevant here: <the 2–4 rules from §4 that apply>.
Do not commit. Format only the files you changed with rustfmt. No new clippy warnings in touched files.
</constraints>
<verification_loop>
Run: cargo test --locked --lib <module>   (and cargo check --message-format=short)
If a check fails, fix it and re-run before reporting.
</verification_loop>
<completeness_contract>
Done means: <observable behaviour / tests that must exist and pass>.
If you are blocked or the spec is ambiguous, stop and say what is missing instead of guessing.
</completeness_contract>
<compact_output_contract>
Reply with: summary (≤10 lines), files touched, commands run with pass/fail counts, anything unverified or
risky. No full logs.
</compact_output_contract>
```

For read-only diagnosis, add `<grounding_rules>`: every claim needs a file:line, and inferences must be marked
as such. Never put secrets, session values or private message content in a brief.

## 5. Review Protocol

The reviewer must not be the author.

| Change | Review |
| :--- | :--- |
| Docs only, or < ~25 lines with a covering test | Claude self-check. No Codex review |
| Normal multi-file change written by Claude | `scripts/codex.sh review --wait --scope working-tree` before committing (or `--base <ref>` after) |
| Change written by Codex | Claude reads the **whole** diff, checks it stayed in scope, and re-runs the verification. For larger diffs, also run Claude's `/code-review`. Use a fresh Codex review only if the change is high-risk |
| High-risk: protocol, session/crypto, `is_trusted_zalo_url`, updater, AI client, file permissions, SQLite schema/migrations, history sync, concurrency in the worker | **Mandatory** `adversarial-review` with focus text naming the risk (e.g. "credential leakage to non-Zalo hosts", "migration on an existing DB with user_version 2"), plus the `security-audit` skill when security is involved |

Handling findings. This replaces the plugin's default of "always ask before fixing":
- Verify each finding against the code first. Some are wrong or out of scope.
- Under the standing authorization, Claude fixes **verified, in-scope** findings directly. In the report, list
  what was fixed, what was rejected, and why.
- Ask the owner when a finding would change scope, protocol behaviour, data on disk, or the user experience,
  or when Claude disagrees with a finding on judgment.
- At most **two** review→fix rounds. If problems remain, stop and report them to the owner.

## 6. After Codex Returns

1. `git status` and `git diff --stat`. Any file outside the brief's scope gets reverted or explained.
2. Read the diff. Do not trust Codex's summary over the code.
3. Re-run the brief's verification yourself, plus `cargo check`.
4. Commit following AGENTS.md §5: commit freely, push only with the owner's OK, stage explicit paths.
5. Write **one** worklog entry for the whole task. Credit the agents
   (`agent: Claude Code (orchestrator) + Codex (worker)`) and record the review outcome and what is not verified.
6. Keep the conversation clean: summarise Codex output, never paste long logs, and fetch details with
   `scripts/codex.sh result <job-id>` only when needed.

If Codex fails or cannot be reached (auth, network, crash), report it. Then do the work yourself or ask the
owner. Do not hide the failure. When setup or auth is broken, the owner runs `/codex:setup`.

## 7. Claude-Side Tools Worth Using

- Skills: see AGENTS.md §9 (Rust skills by situation). For security-relevant changes, use `security-audit`.
- Built-in: `/code-review` (correctness review of a diff), `/simplify` (cleanup pass), `/security-review`.
- Owner-only slash commands: `/codex:review`, `/codex:adversarial-review`, `/codex:status`, `/codex:result`,
  `/codex:cancel`, `/codex:transfer` (moves this Claude session into a resumable Codex thread).
  `/codex:rescue` and `/codex:setup` can be run by either side; Claude still prefers the wrapper for briefs.
