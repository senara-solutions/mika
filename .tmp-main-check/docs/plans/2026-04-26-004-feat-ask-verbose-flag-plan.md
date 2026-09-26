---
title: "feat(ask): add --verbose flag with metadata trailer"
type: feat
status: active
date: 2026-04-26
issue: 823
---

# feat(ask): add --verbose flag with metadata trailer

## Overview

Add a `--verbose` CLI flag to `mika ask` that emits a `session_id: <uuid>` metadata trailer line after the normal response output. This enables downstream slash commands (`/mika-groom-ticket`, etc.) to capture the session ID from `mika ask` output without requiring `--format json`.

## Problem Frame

The `/mika-groom-ticket` two-pass pipeline needs the session ID from `mika ask` output to reference prior context. Using `--format json` would require JSON parsing that breaks the pipeline's text-oriented processing. A simple key-value trailer line on stdout is the minimal contract that downstream consumers can match by key name.

This is the CLI companion to mika-platform#52. mika-platform PR #54 is blocked on this shipping.

## Requirements Trace

- R1. Add `--verbose` boolean CLI flag to `mika ask` (scoped to `AskArgs`, not global)
- R2. When `--verbose` is set in text mode, emit `session_id: <uuid>` as a standalone line on stdout after all other output
- R3. No `---` markers or decorative framing around the trailer — bare `key: value` format
- R4. Downstream parsers match by `session_id:` key name, not by position or section markers
- R5. Do NOT require `--format json` for metadata access

## Scope Boundaries

- Only `mika ask` text-mode output is modified; JSON mode already has a structured envelope
- Team mode (`--team`) is out of scope — `--verbose` should conflict with `--team` like other solo-mode flags
- No changes to the agent loop, `AgentOutput`, or `AgentParams`

### Deferred to Separate Tasks

- Adding `session_id` to `AskJsonResponse` for JSON-mode parity: separate concern, not needed by the blocking consumer

## Context & Research

### Relevant Code and Patterns

- `crates/mika-cli/src/cli.rs` line 163–216: `AskArgs` struct — add `verbose: bool` field here
- `crates/mika-cli/src/commands/ask.rs` line 26: `run()` function signature — add `verbose: bool` parameter
- `crates/mika-cli/src/commands/ask.rs` lines 359–382: output section where trailer is emitted
- `crates/mika-cli/src/main.rs` lines 245–266: dispatch site passes `AskArgs` fields to `run()`
- `crates/mika-cli/src/commands/config.rs`: existing `--verbose` pattern on `config get/list` — bool field with `#[arg(long)]`

### Institutional Learnings

- **CLI log noise on stderr** (`docs/solutions/dev-loop/cli-log-noise-stderr-pollutes-piped-commands-2026-04-17.md`): `mika ask` stdout is consumed by pipes. However, the issue explicitly requests the trailer on stdout (not stderr) so downstream `$(mika ask --verbose ...)` captures it. The `--verbose` flag is opt-in, so non-verbose consumers are unaffected.
- **CLI flag scoping** (`docs/solutions/architecture-patterns/cli-flag-subcommand-scoping.md`): Add `--verbose` to `AskArgs` only, not as a global flag.
- **Strip internal tags** (`docs/solutions/ui-bugs/strip-internal-metadata-tags-from-display.md`): Emit trailer after `run_agent()` returns and response is printed — never from within the agent loop.

## Key Technical Decisions

- **stdout, not stderr:** The trailer goes to stdout because downstream consumers capture stdout via command substitution (`$(mika ask --verbose ...)`). The `--verbose` flag is opt-in so existing pipe workflows are unaffected.
- **`conflicts_with = "team"`:** Consistent with `--model`, `--task-id`, and other solo-mode flags. Team mode has its own metadata path (`team_run` in JSON).
- **Text mode only:** The trailer is only emitted in text mode. JSON mode already wraps everything in a structured envelope; adding `session_id` there is a separate concern.

## Implementation Units

- [ ] **Unit 1: Add `--verbose` flag to `AskArgs` and thread through dispatch**

  **Goal:** Wire the `--verbose` boolean from CLI parsing through to the `ask::run()` handler.

  **Requirements:** R1

  **Dependencies:** None

  **Files:**
  - Modify: `crates/mika-cli/src/cli.rs`
  - Modify: `crates/mika-cli/src/commands/ask.rs`
  - Modify: `crates/mika-cli/src/main.rs`
  - Test: `crates/mika-cli/src/commands/ask.rs` (inline `#[cfg(test)] mod tests`)

  **Approach:**
  - Add `pub verbose: bool` with `#[arg(long, conflicts_with = "team")]` to `AskArgs` in `cli.rs`
  - Add `verbose: bool` parameter to `ask::run()` signature (already has `#[allow(clippy::too_many_arguments)]`)
  - Pass `args.verbose` in the dispatch site in `main.rs`

  **Patterns to follow:**
  - `crates/mika-cli/src/cli.rs` — existing `--model`, `--task-complete` flag definitions in `AskArgs`
  - `crates/mika-cli/src/commands/config.rs` — `verbose: bool` on config subcommands

  **Test scenarios:**
  - Happy path: `AskArgs` struct accepts `verbose` field without compilation errors (compile-time verification)

  **Verification:**
  - `cargo build -p mika-cli` succeeds
  - `cargo run --bin mika -- ask --help` shows `--verbose` in help output

- [ ] **Unit 2: Emit metadata trailer in text mode when `--verbose` is set**

  **Goal:** After printing the response text, emit `session_id: <uuid>` as a standalone line on stdout when `--verbose` is true and format is text.

  **Requirements:** R2, R3, R4, R5

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-cli/src/commands/ask.rs`
  - Test: `crates/mika-cli/src/commands/ask.rs` (inline `#[cfg(test)] mod tests`)

  **Approach:**
  - In the `OutputFormat::Text` branch (line ~359), after the existing `println!("{text}")` and after the pending callbacks notice, add: `if verbose { println!("session_id: {session_id}"); }`
  - The `session_id` variable is already in scope as a `String` — no plumbing needed
  - Position after the pending callbacks stderr notice so the trailer is always the last stdout line

  **Patterns to follow:**
  - `crates/mika-cli/src/commands/ask.rs` lines 359–371 — existing text output section

  **Test scenarios:**
  - Happy path: Unit test constructs scenario verifying `session_id: <uuid>` line format matches `session_id: ` prefix followed by a valid UUID string
  - Happy path: Verify trailer is not emitted when `verbose` is false (existing tests remain unchanged — `AskJsonResponse` serialization tests should still pass)
  - Edge case: When `output.text` is `None` (empty response fallback), the trailer is still emitted on stdout (the empty response fallback goes to stderr, so trailer can still appear on stdout)

  **Verification:**
  - `cargo test -p mika-cli` passes
  - All existing `AskJsonResponse` serialization tests unchanged

## System-Wide Impact

- **Interaction graph:** No callbacks, middleware, or agent loop changes. The trailer is appended after the agent loop completes.
- **Error propagation:** If the agent loop fails, `output?` propagates the error before reaching the trailer — no trailer emitted on failure, which is correct.
- **API surface parity:** JSON mode is explicitly out of scope per the issue. Text-mode trailer is the only contract.
- **Unchanged invariants:** `AskJsonResponse` serialization is unchanged. `--format json` output is unaffected. Team mode output is unaffected (`--verbose` conflicts with `--team`).

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Downstream parsers rely on trailer position rather than key name | Issue explicitly documents key-name matching as the contract (R4) |
| `too_many_arguments` lint on `run()` | Already suppressed with `#[allow(clippy::too_many_arguments)]` |

## Sources & References

- Related issue: #823
- Companion: mika-platform#52, mika-platform PR #54
- Related code: `crates/mika-cli/src/commands/ask.rs`, `crates/mika-cli/src/cli.rs`
