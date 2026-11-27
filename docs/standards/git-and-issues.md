# Git, Release and Issue Standards

## 1. Commits

- **Agents may commit locally without asking. Pushing, tagging, releasing, force-pushing, rewriting
  published history, and deleting branches need the owner's explicit OK** in the current conversation.
- Conventional Commits: `<type>(<scope>): <summary>`. The header is at most 72 characters, imperative, with no
  trailing period. The body explains *why* in short bullets.
  - Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `chore`, `ci`, `style`.
  - Scopes: `auth`, `net`, `chat`, `media`, `call`, `ui`, `store`, `crypto`, `session`, `sync`, `ai`,
    `updater`, `ci`, `docs`, `harness` (agent tooling).
- One logical, buildable change per commit. Never mix formatting sweeps with behaviour changes.
- **Stage explicit paths** (`git add src/a.rs src/b.rs`). Never use `git add -A` or `git commit -a` in this
  repo: the owner's uncommitted work and local files (`log.txt`, `skills/`, logs, data files) sit in the tree.
- Before committing, check that `git status` shows no secrets (`session*.json`, `*.db`, `ai_secret.json`, keys,
  captures).
- Agent-assisted commits may add a trailer such as `Co-authored-by:` when the harness provides one.

## 2. Branches

- Default: work on `main`.
- Use a branch for risky or multi-session work: `feature/…`, `fix/…`, `core/…` (protocol, schema, crypto),
  `docs/…`, `chore/…`. Rebase on `main` before merging, with no merge commits inside a branch.
- For parallel agent work, use git worktrees (`git worktree add ../zalo-next-<topic> -b <branch>`), one per
  writer.
- Fast-forwarding over a commit that untracks `native/macos/build/` deletes the built helpers on disk.
  Afterwards, run `bash native/macos/build-native-helpers.sh`.

## 3. Releases (owner approval required)

1. Bump `version` in `Cargo.toml`, then run `cargo check` to update `Cargo.lock`.
2. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [x.y.z] - YYYY-MM-DD` and add a new empty
   `## [Unreleased]` section.
3. `git commit -m "chore(release): vx.y.z"`, then `git tag -a vx.y.z -m "Zalo-next vx.y.z"`, then push `main`
   and the tag. The tag must match `Cargo.toml` and be on `main`, or CI stops.
4. `.github/workflows/ci-release.yml`:
   - runs tests on every push to `main` (docs-only changes are skipped);
   - builds and publishes macOS/Windows/Linux packages, `SHA256SUMS.txt`, `latest.json` and provenance
     attestations on a `v*` tag.
5. Release notes come from `scripts/release_notes.py`. It backticks bare `@name` mentions, because a bare
   `@AI` once pinged a real GitHub user. Never write a bare `@name` in a changelog, issue or PR.

## 4. Issues

Issues live in GitHub Issues on `minhkhang1008/zalo-next`. Use the `gh` CLI.

- Labels:
  - type: `type:feat`, `type:fix`, …
  - area: `area:auth|net|chat|media|call|ui|store|ci|harness`
  - priority: `priority:p0|p1|p2`
  - blocked: `status:blocked` plus a comment naming what is missing
- An actionable issue has:
  - a one-sentence outcome;
  - repro steps or acceptance criteria;
  - an area label;
  - evidence for protocol work (`re-notes/` path, `error_code`);
  - explicit non-goals when scope is easy to over-read.
- Small items and backlogs can live in `tasks/*.md` instead. Keep `tasks/todo.md` current when you finish one.
- Never paste `session.json`, cookies, `zpw_sek`/`zpw_enk`, IMEI, or unredacted captures into an issue, PR or
  commit.

## 5. Pull requests (when used)

Use `.github/PULL_REQUEST_TEMPLATE.md`: What / Why / How / Verification / Risk & Rollback. Protocol, schema,
crypto and release changes also link their ADR or `re-notes/` evidence and include a rollback note.
