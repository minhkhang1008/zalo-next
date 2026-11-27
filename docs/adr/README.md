# Architecture Decision Records

One decision per file: `NNNN-<kebab-name>.md`, numbered from `0001`, never renumbered. The rules are in
[`docs/standards/docs-and-decisions.md`](../standards/docs-and-decisions.md).

## Index

| ADR | Title | Status |
| :--- | :--- | :--- |
| [0001](0001-history-sync.md) | History sync and backup import (inherited from ctdozlite) | Partly superseded, see the note in the file |
| [0002](0002-self-updater.md) | Self-updater contract and `latest.json` (inherited) | Accepted; update source is this repo only |
| [0003](0003-changelog-policy.md) | Changelog and release notes (inherited) | Accepted |
| [0004](0004-local-ai-assistant.md) | Local-only AI assistant as a virtual participant | Accepted; not verified with a real provider |
| [0005](0005-independence-from-upstream.md) | Zalo-next is independent of upstream ctdozlite | Accepted |
| [0006](0006-multi-agent-workflow.md) | Claude Code orchestrates, Codex implements and cross-reviews | Accepted |

Domain language: [`GLOSSARY.md`](../../GLOSSARY.md).

## Template

```markdown
# ADR-NNNN: <Decision title>

- **Status:** proposed | accepted | superseded by ADR-NNNN | deprecated
- **Date:** YYYY-MM-DD
- **Deciders:** <names/roles>

## Context

Forces, constraints and evidence (capture note, `error_code`, observed behaviour).

## Decision

What we will do, in the present tense.

## Consequences

- Positive:
- Negative:
- Follow-ups:

## Alternatives Considered

| Option | Why rejected |
| :--- | :--- |
```
