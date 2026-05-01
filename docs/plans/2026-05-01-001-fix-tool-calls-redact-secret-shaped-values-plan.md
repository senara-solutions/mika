---
title: "fix: Redact secret-shaped values in tool_calls persistence"
type: fix
status: active
date: 2026-05-01
---

# fix: Redact secret-shaped values in tool_calls persistence

## Plan Contract

**Retroactive archaeological-record contract.** Implementation Units 1–4 `[x]` complete in commits `7f7f11d` (Units 1–4 + plan + compound doc), `52284f6` (error_message scope extension — see F1 amendment in Key Decisions below), `db2191f` (compound doc). /ce:work dispatch is **SKIPPED** — implementation shipped via sprint dispatch (`mika ask --agent mika-dev "implement sprint: ..."` at 2026-04-30T20:07:55) prior to architect review, per the documented bypass gap in mika#919. Unit 5 deferred to mika#918 (see Implementation Units below). No `Co-authored-by` trailer required — mika-dev + claude-pilot authorship throughout, no community contributor whose attribution would otherwise be lost in a squash merge. Retroactive groom initiated 2026-05-01 once mika#919's grooming-bypass gap was identified post-dispatch; this plan now serves as the authoritative archaeological record so the Knowledge Graph indexes the rationale alongside the code.

**`[→]` deferred-with-forward-pointer convention.** Unit 5 below uses `[→ mika#918]` to mark a unit that is deferred to a sibling ticket rather than completed on this plan. Distinct from `[ ]` (open work to dispatch on this plan) and `[x]` (complete on this plan). The forward-pointer ticket carries Unit 5's actual delivery; the /mika pipeline reading this plan should treat `[→]` as a no-op (skip, do not dispatch).

## Overview

Add a secret-scrubbing layer at the `tool_calls` database persistence boundary so that secret-shaped values (API keys, tokens, PEM private keys, env var assignments containing secrets) are redacted before being written to SQLite. The LLM's in-memory tool output remains unscrubbed — only the durable copy in `tool_calls.input` and `tool_calls.output` is sanitized. A one-shot backfill migration scrubs existing rows.

## Problem Frame

`tool_calls.output` stores verbatim file content from any tool that returns file data (`read_agent_file`, exec handlers, MCP tools). When an agent reads a file containing secrets (e.g., `.env` with `MIKA_GITHUB_TOKEN=github_pat_...`), the secret persists in `mika.db` indefinitely and is served via the dashboard API. The broader secret discipline (`SecretString`, `scrub_mika_env_vars`, MCP `env_clear`) operates at process boundaries but does not cover the internal tool_calls persistence path.

Evidence: `tool_calls` row `461c76a1` (2026-04-13) contained a real GitHub PAT from `read_agent_file({path: ".env"})` — 17-day exposure window. See #903 audit.

## Requirements Trace

- R1. New `scrub_secrets(&str) -> Cow<'_, str>` function covering all listed secret patterns
- R2. `save_tool_call()` applies scrubber to both `input` and `output` before INSERT
- R3. The live tool-call result returned to the LLM remains unscrubbed
- R4. Unit tests: positive and negative cases for each pattern
- R5. Integration test: fixture `.env` through `read_agent_file` → assert DB redacted, assert live result intact
- R6. Backfill migration scrubs existing `tool_calls` rows (idempotent, schema v28→v29)
- R7. `ToolCallSummary` metadata path (`input_summary`, `output_summary`) also scrubbed

## Scope Boundaries

- No encryption-at-rest for `tool_calls.output` (different threat model)
- No per-agent allowlists for which tools can record output
- No dashboard auth tightening on `/api/v1/traces/.../tool-calls`
- No retroactive credential rotation beyond the one already rotated

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/db.rs` line 5070: `Database::save_tool_call()` — the INSERT site. Already applies `truncate_utf8_safe()` to input/output. Scrubbing should happen before truncation.
- `crates/mika-agent/src/async_db.rs` line 1853: `AsyncDatabase::save_tool_call()` — async wrapper, clones strings. Scrubbing at the `Database` (sync) layer catches all callers.
- `crates/mika-agent/src/agent.rs` line 2432: Call site in `process_tool_calls()` passes raw `output.content` and serialized `input_json`.
- `crates/mika-agent/src/agent.rs` lines 2396, 2462: `ToolCallSummary` builds `input_summary` and `output_summary` via `truncate_summary()` — a secondary persistence path via `messages.metadata`.
- `crates/mika-agent/src/agent.rs` line 4033+: Established `LazyLock<Regex>` pattern for static compiled regexes.
- `crates/mika-agent/src/skills/executor.rs` line 39: `scrub_mika_env_vars()` — prefix-based env var scrubbing pattern.
- `crates/mika-common/src/config.rs`: `SecretString` with `[REDACTED]` convention.
- `regex` crate is already a workspace dependency used by `mika-agent`.

### Institutional Learnings

- `docs/solutions/security-issues/bash-set-x-leaks-secrets-in-trace-and-callback-2026-04-30.md` — sibling leak class (#903), documents the audit that discovered this issue.
- `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md` — documents the 50KB cap and fire-and-forget pattern for tool_call writes.
- `docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md` — prefix-based pattern matching for env var scrubbing.
- `docs/solutions/best-practices/secretstring-expose-at-boundary-pattern.md` — `[REDACTED]` convention.

## Key Technical Decisions

- **Scrub at `Database::save_tool_call()`, not at the call site**: Catches all callers (current and future) without relying on each caller to remember scrubbing. Applied before truncation so secrets are never written even partially.
- **Use `Cow<'_, str>` return type**: Avoids allocation when no secrets are found (the common case). Pattern established by `regex::Regex::replace_all`.
- **New module `crates/mika-agent/src/secret_scrubber.rs`**: Isolates the scrubbing logic for independent testing and reuse. Not in `db.rs` (already 12K+ lines) or `tools/mod.rs` (different concern).
- **`RegexSet` for fast rejection + individual `Regex` for replacement**: `RegexSet::is_match()` is a single DFA pass. Only when it matches do we iterate individual regexes. Optimizes the common case (no secrets in output).
- **Also scrub `ToolCallSummary` metadata**: The `input_summary` and `output_summary` in `messages.metadata` are a secondary leak path. Scrub at the summary construction site in `process_tool_calls()`.
- **Schema bump v28→v29 for backfill**: The backfill is a data-only migration (no DDL). Using the schema migration path ensures it runs exactly once, is idempotent, and follows the established pattern.
- **No scrub on `error_message`**: Error messages from tool failures should not contain secrets (they are error descriptions, not file content). Keeping them unscrubbed preserves debugging utility. If a future audit surfaces secrets in error messages, the scrubber is trivially applied.

  > **AMENDED by commit `52284f6`** (architect F1, ratified 2026-05-01): scrub IS applied to `error_message`. Rationale: HTTP client, webhook, and auth-layer error messages can embed request context including token values (e.g., bearer tokens reflected in URL parameters, API keys in auth headers re-emitted in error text, raw response bodies on auth failures). The original "error descriptions, not file content" rationale was under-argued — tool failure context is structurally capable of containing secrets. Scrubbing `error_message` is consistent with the defense-in-depth posture of Units 1–4 and matches the same INSERT-time chokepoint already established for input/output. Operator has reviewed and ratifies this scope extension.

- **Backfill migration shape — single-pass within one Immediate transaction** (architect F2 closure, 2026-05-01): `migrate_v28_to_v29()` opens a single `TransactionBehavior::Immediate` transaction, SELECTs all `tool_calls` rows where any of `input`/`output`/`error_message` are non-NULL into memory (`Vec<(id, input, output, error_message)>`), iterates in Rust applying `scrub_secrets()` to each field via `Cow`, and UPDATEs only rows where scrubbing produced an `Owned` value (`matches!(_, Some(Cow::Owned(_)))`) on at least one column. Schema-version bump and commit happen at the tail of the same transaction. **Not batched.** Rationale: SQLite's single-writer model means batching does not reduce lock contention; a single-pass migration with idempotent UPDATE-only-on-diff is correct for SQLite deployments. Large-instance latency risk is acknowledged and accepted — the migration runs at startup before the agent processes events, so the worst-case effect is a slower first boot, not user-facing latency. If a future deployment surfaces unacceptable startup-latency on a large `tool_calls` table, batching is a localized refactor of `migrate_v28_to_v29()` (the Rust loop, the data-only contract, and the idempotency story all stay the same).

- **False-positive calibration — placeholder values are deliberately redacted** (architect F6 sharpening, 2026-05-01): Placeholder values in `.env.example` like `MIKA_API_KEY=your_key_here` will be redacted by the `MIKA_*=value` patterns. False-positive cost (operator sees `<REDACTED>` in tool output where they expected the placeholder string) is materially lower than false-negative cost (real key persists unredacted in the database for the documented 17-day audit horizon). Value-side length and character-class validation could reduce false positives but adds complexity and risks new false-negative classes (e.g., a token format we haven't seen yet whose value looks "non-secret-shaped"). Deferred.

## Open Questions

### Resolved During Planning

- **Where to scrub — `Database` or `AsyncDatabase`?** At the `Database` (sync) level. This is the single funnel all writes go through, and it avoids needing to scrub the owned strings in `AsyncDatabase` before cloning them.
- **Should the backfill use `regex` in SQL or in Rust?** In Rust. SQLite's `REGEXP` requires loading an extension. The migration will SELECT rows, scrub in Rust, and UPDATE in batches within a transaction.
- **Should `error_message` be scrubbed?** No — error messages are tool failure descriptions, not file content. The blast radius is minimal and debugging value is high.

### Resolved Post-Implementation (architect first-pass closure, 2026-05-01)

- **Exact batch size for the backfill migration** — RESOLVED: not batched. Single-pass within one `TransactionBehavior::Immediate` transaction. See Key Decisions §"Backfill migration shape" for full rationale.

### Deferred to Implementation

- Whether the `RegexSet` pattern set needs tuning after seeing real-world false positive rates.

## Implementation Units

- [x] **Unit 1: Create `secret_scrubber` module with `scrub_secrets()` function**

  **Goal:** Provide a reusable function that redacts known secret-shaped patterns from arbitrary text.

  **Requirements:** R1

  **Dependencies:** None

  **Files:**
  - Create: `crates/mika-agent/src/secret_scrubber.rs`
  - Modify: `crates/mika-agent/src/lib.rs` (add `pub mod secret_scrubber`)

  **Approach:**
  - Define `SECRET_PATTERNS: &[(&str, &str)]` as `(pattern, replacement)` pairs covering: `github_pat_*`, `ghp_*`, `gho_*`, `ghs_*`, `ghu_*`, `sk-ant-(api|oat)*`, `sk-proj-*`, `sk-or-*`, `gsk_*`, `MIKA_*TOKEN/KEY/SECRET=value`, `GH_TOKEN=value`, `GH_APP_TOKEN=value`, PEM private key blocks.
  - Use `std::sync::LazyLock<regex::RegexSet>` for fast rejection and `LazyLock<Vec<regex::Regex>>` for individual replacements.
  - `scrub_secrets(input: &str) -> Cow<'_, str>` — returns `Borrowed` when no match (common case), `Owned` when scrubbed.
  - Replacement strings use the same `<REDACTED>` convention as existing code (e.g., `github_pat_<REDACTED>`, `MIKA_API_KEY=<REDACTED>`).

  **Patterns to follow:**
  - `LazyLock<Regex>` pattern from `agent.rs` line 4033
  - `[REDACTED]` convention from `mika-common/src/config.rs`

  **Test scenarios:**
  - Happy path: each pattern individually — input containing a real-shaped token → output has the token replaced with the correct `<REDACTED>` form
  - Happy path: multiple different secret types in one string → all replaced
  - Happy path: PEM private key block (multi-line) → entire block replaced with `<REDACTED-PRIVATE-KEY>`
  - Happy path: `MIKA_GITHUB_TOKEN="github_pat_11CBQ5..."` (the actual incident shape) → both the env var assignment and the token prefix are redacted
  - Edge case: no secrets in input → returns `Cow::Borrowed` (no allocation)
  - Edge case: text near but not matching patterns (e.g., `ghp` without trailing chars, `sk-ant-` without suffix) → left untouched
  - Edge case: `.env.example` with placeholder values like `MIKA_API_KEY=your_key_here` → `MIKA_API_KEY=<REDACTED>` (acceptable — defensive redaction of assignment values is correct)
  - Edge case: partial prefix at end of truncated string → no panic, graceful handling
  - Edge case: empty string → returns `Cow::Borrowed` empty string

  **Verification:**
  - `cargo test -p mika-agent secret_scrubber` passes
  - All pattern positive/negative cases covered

- [x] **Unit 2: Apply scrubber at `Database::save_tool_call()` persistence boundary**

  **Goal:** Ensure all tool_call input/output is scrubbed before INSERT.

  **Requirements:** R2, R3

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/db.rs` (`save_tool_call()`)

  **Approach:**
  - In `save_tool_call()`, apply `scrub_secrets()` to both `input` and `output` BEFORE the existing `truncate_utf8_safe()` call. Order: scrub → truncate → INSERT.
  - The function already takes `Option<&str>` — map through scrubber, converting `Cow` back to the truncation input.
  - No changes to the function signature — callers continue passing raw strings.

  **Patterns to follow:**
  - Existing `truncate_utf8_safe()` application pattern at lines 5090-5092

  **Test scenarios:**
  - Happy path: `save_tool_call` with output containing `github_pat_ABC123...` → SELECT the row back → output contains `github_pat_<REDACTED>`
  - Happy path: `save_tool_call` with input containing `MIKA_API_KEY=sk-proj-abc...` → SELECT back → input is redacted
  - Happy path: `save_tool_call` with clean output → output unchanged in DB
  - Integration: round-trip test — save with secrets, read back, verify redacted

  **Verification:**
  - Existing `cargo test -p mika-agent` passes (no regressions)
  - New tests confirm scrubbing at DB boundary

- [x] **Unit 3: Apply scrubber to `ToolCallSummary` metadata path**

  **Goal:** Prevent secret leakage through the secondary `messages.metadata` persistence path.

  **Requirements:** R7

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/agent.rs` (`process_tool_calls()` — `input_summary` and `output_summary` construction)

  **Approach:**
  - Apply `scrub_secrets()` to the `input_summary` string at line 2396 (after `truncate_summary`) and to `output_summary` at lines 2457-2462 (after `truncate_summary`).
  - Order: truncate → scrub. Since summaries are already short (200/300 chars), scrubbing after truncation is fine and ensures truncated secrets don't escape.

  **Patterns to follow:**
  - Existing `truncate_summary()` application at the same sites

  **Test scenarios:**
  - Happy path: `ToolCallSummary` with output_summary containing a token prefix → summary is redacted
  - Edge case: truncation cuts a token mid-pattern → scrubber handles gracefully (partial match may or may not fire, but no panic)

  **Verification:**
  - `cargo test -p mika-agent` passes

- [x] **Unit 4: Backfill migration v28→v29 — scrub existing `tool_calls` rows**

  **Goal:** Retroactively redact secrets from historical tool_call records.

  **Requirements:** R6

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/db.rs` (add `migrate_v28_to_v29()`, update `CURRENT_SCHEMA_VERSION` to 29, wire into migration chain)

  **Approach:**
  - `migrate_v28_to_v29()` follows the established migration pattern. SELECT all `tool_calls` rows where `input IS NOT NULL OR output IS NOT NULL`, apply `scrub_secrets()` to each, UPDATE only rows where the scrubbed value differs from the original (avoid no-op writes). Process in batches within a single transaction.
  - Idempotent: if no rows contain secrets, the migration is a no-op. If re-run (schema already at 29), the migration function's version guard skips it.
  - Bump `CURRENT_SCHEMA_VERSION` from 28 to 29.

  **Patterns to follow:**
  - `migrate_v27_to_v28()` at db.rs line 3444 (idempotency guard, transaction pattern)

  **Test scenarios:**
  - Happy path: DB at v28 with tool_calls rows containing secrets → after migration, rows are scrubbed, schema at v29
  - Happy path: DB at v28 with no secrets in tool_calls → migration completes, schema at v29, no rows modified
  - Edge case: DB already at v29 → migration skipped
  - Edge case: empty tool_calls table → migration completes without error

  **Verification:**
  - Migration test in the existing migration test suite
  - `CURRENT_SCHEMA_VERSION == 29`

- [→ mika#918] **Unit 5: Integration test — end-to-end secret redaction through `read_agent_file`** — DEFERRED

  **Status (architect F3 closure, 2026-05-01):** Unit 5 is **NOT shipped on this plan**. The EvalHarness integration test (R3+R5) was identified as missing during PR #915 review — mika-qa blocked PR #915 with `block[ac]` on this Unit. Vincent filed mika#918 the same day as a follow-up specifically for this delivery. mika#918 is the canonical dispatch target for Unit 5; this plan will not re-deliver it.

  /ce:work dispatch on this plan SKIPPED for Unit 5 — dispatch lives in mika#918's own plan-on-branch (`fix/918/...`). mika-qa's `block[ac]` on PR #915 clears when mika#918 lands; the two tickets ship in sequence (#908 → #918) with PR #915 merging once #918 unblocks the AC.

  **Goal:** Prove the full pipeline: tool returns secrets → DB has redacted values → live tool result retains real values.

  **Requirements:** R5, R3

  **Dependencies:** Units 1, 2

  **Files:**
  - Create: `crates/mika-agent/tests/eval/tool_call_secret_redaction.rs` (or add to existing eval test module)

  **Approach:**
  - Use `EvalHarness` with `MockLlmProvider` to simulate a `read_agent_file` call against a fixture `.env` file containing all secret pattern shapes.
  - Assert: the `tool_calls.output` DB row has all values redacted.
  - Assert: the `ToolOutput` returned to the LLM in the conversation contains the real values (verify via the mock provider's captured tool_result content).
  - Create a fixture `.env` file in a temp directory with representative secret shapes.

  **Patterns to follow:**
  - Existing eval tests in `crates/mika-agent/tests/eval/`
  - `EvalHarness` builder pattern

  **Test scenarios:**
  - Integration: fixture `.env` with `MIKA_GITHUB_TOKEN=github_pat_ABC...`, `MIKA_ANTHROPIC_API_KEY=sk-ant-api123...`, `GH_TOKEN=ghp_xyz...` → DB output redacted for all three, live output has real values
  - Integration: tool returning non-secret file content → DB output unchanged

  **Verification:**
  - `cargo test -p mika-agent --test eval tool_call_secret_redaction` passes

## System-Wide Impact

- **Interaction graph:** The scrubber runs inside `Database::save_tool_call()` which is called from `process_tool_calls()` in `agent.rs`. The `ToolCallSummary` scrubbing runs in the same function before metadata serialization. No callbacks or observers affected.
- **Error propagation:** Scrubbing is infallible (`regex::Regex::replace_all` never fails on valid UTF-8). No new error paths introduced.
- **State lifecycle risks:** None — scrubbing is a pure string transformation applied before the INSERT. No partial-write risk.
- **API surface parity:** The dashboard API at `/api/v1/traces/:trace_id/tool-calls` will now serve redacted values. This is the desired behavior. No API contract change needed.
- **Integration coverage:** The integration test (Unit 5) covers the full pipeline from tool execution through DB persistence.
- **Unchanged invariants:** `ToolOutput` returned to the LLM is NOT modified. The agent's ability to use tool results in subsequent turns is unaffected. The 50KB truncation cap remains unchanged. The `MIKA_STORE_TOOL_CALLS` toggle continues to gate all writes.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| False positives redacting non-secret strings that happen to match patterns | Patterns are designed to match known token prefixes with sufficient length requirements. `MIKA_*=` patterns only match known secret-shaped env var names (TOKEN, KEY, SECRET suffixes). `.env.example` placeholder values getting redacted is acceptable (defensive). |
| Performance impact of regex on every tool call | `RegexSet::is_match()` is a single DFA pass — fast rejection for the common case (no secrets). Individual regex replacement only runs when secrets are detected. |
| Backfill migration takes too long on large DBs | Batch processing within a transaction. The `tool_calls` table is pruned by retention policy, so row count is bounded. |
| New secret formats not covered by patterns | The pattern list is extensible. Document the pattern list in the module for future additions. Log a structured event when scrubbing fires so operators can audit coverage. |

## Future Work

- **`scrubber_fired` structured log event** (architect F5 sharpening): When `scrub_secrets()` returns `Owned`, emit a `scrubber_fired` structured log line with fields `tool_call_id`, `column` (one of `input`/`output`/`error_message`), `pattern_index` (which regex matched). Operators can then audit how often each pattern fires, which surfaces missed patterns when a known-leaking tool produces zero `scrubber_fired` lines. **Prioritization threshold:** first post-deploy report of a scrub miss that this observability would have caught. File a follow-up ticket once the threshold is hit. Until then, the patterns themselves are the contract — no telemetry needed for the happy path.

## Sources & References

- Related issue: #908
- Architect grooming session: `4dbf2391-b2e6-4a09-a8d3-db9881cac54d` (first-pass ITERATE 2026-05-01, F1–F6)
- Sibling follow-up: mika#918 (Unit 5 R3+R5 EvalHarness integration test, mika-qa `block[ac]` on PR #915)
- Sibling structural fix: mika#919 (operator-CLI dispatch grooming bypass — root cause of why this plan is being groomed retroactively)
- Sibling issue: #903 (bash set -x leak class)
- `docs/solutions/security-issues/bash-set-x-leaks-secrets-in-trace-and-callback-2026-04-30.md`
- `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md`
- `docs/solutions/best-practices/secretstring-expose-at-boundary-pattern.md`
