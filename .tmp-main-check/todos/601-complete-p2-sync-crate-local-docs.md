---
status: pending
priority: p2
issue_id: 601
tags: [code-review, documentation]
dependencies: []
---

# Sync crate-local docs with config simplification changes

## Problem Statement

The crate-local doc copies in `crates/mika-agent/docs/` still reference the deleted `config/default.toml` and `config/local.toml`. These are fallback docs for crates.io publishing and are compiled into the binary via `include_str!`.

## Findings

- `crates/mika-agent/docs/configuration.md` lines 105-106: cascade table lists deleted sources
- `crates/mika-agent/docs/configuration.md` line 396: references `config/default.toml`
- `crates/mika-agent/docs/deployment.md` line 60: states config copied to container
- `crates/mika-agent/docs/slash-commands.md` line 189: references `config/local.toml`

## Proposed Solutions

Run `scripts/sync-agent-docs.sh` or manually copy the updated `docs/` files to `crates/mika-agent/docs/`.

- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] No references to `config/default.toml` or `config/local.toml` in crate-local docs
- [ ] `scripts/sync-agent-docs.sh` runs clean
