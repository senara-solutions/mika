---
date: 2026-10-13
type: docs
---

# Plan: move 1.3 MB out of the CLAUDE.md files so pilots survive turn 1

## Problem

Every pilot dispatched on this repo dies at turn 1 with a synthetic « Prompt is too long ». Claude Code loads `CLAUDE.md` whole into the prompt: 825 140 bytes, 767 KB of it in § Environment Variables, for a 390 108-token starting prompt. `crates/mika-agent/CLAUDE.md` (594 290 bytes) loads as soon as a session reads a file under `crates/mika-agent/`.

## Fix

- Move § Environment Variables verbatim to `docs/operator/environment-variables.md`, leaving a pointer in `CLAUDE.md`.
- Move the eight `crates/mika-agent/CLAUDE.md` sections above 15 KB verbatim to `docs/agent-internals/`, one file per section, keeping each title with a pointer.
- Add `crates/mika-common/tests/claude_md_size.rs`: a 100 000-byte cap on every `CLAUDE.md` in the repo, with a guard against an empty green and a compile-time negative control.
- Point `test_pr_push_guard.sh` V10 at the new runbook path.

No production code changes.

## Acceptance criteria

- AC1: Moved content is byte-identical to `origin/main` for the root section and the eight crate sections.
- AC2: `cargo test -p mika-common --test claude_md_size` is green, and red when either old `CLAUDE.md` is restored.
- AC3: `bash skills/bundled/_shared/tests/test_pr_push_guard.sh` passes.
- AC4: A pilot's starting prompt on this branch is well under the model window (measured 59 250 tokens, down from 390 108).
