---
ticket: mika#843
branch: feat/843/ask-verbose-metadata-expansion
status: active
date: 2026-06-12
origin: https://github.com/senara-solutions/mika/issues/843
execution: code
---

# Plan: expand `mika ask --verbose` metadata envelope (mika#843)

## Problem frame

`mika ask --verbose` currently emits only `session_id` in its `MetadataEnvelope` (text trailer + JSON `metadata` object). Operators want more observability fields without scraping logs: what model ran, how long it took, what task/agent context applied, and what tokens were consumed.

The current scope was set by mika#824 (text-mode envelope) and mika#830 (JSON parity). CLAUDE.md documents the per-field gating convention:

> Today only `session_id` exists and it is `--verbose`-gated; future fields may ship gated or unconditional.

## Field-set commitment

Per architect first-pass guidance (session `641ce8fc-c885-4cee-96e1-dcc0c9d1c55f`), the candidate field list resolves to a committed minimum-viable set + cheap stretch, with deferrals explicit:

### Committed (Tier 1 — high value, definitively available):

| Field | Type | Source | Gating | Notes |
|-------|------|--------|--------|-------|
| `session_id` | `String` | already populated | `--verbose` | unchanged from current behavior |
| `model` | `String` | `ctx` settings after `override_model` | `--verbose` | the provider/model string actually used |
| `agent_id` | `String` | already known (passed as `agent_name`) | `--verbose` | confirms which agent handled the turn |
| `latency_ms` | `u64` | `Instant::now()` at start, diff at end | `--verbose` | wall-clock for the agent loop |
| `tokens` | `TokensMetadata` | `AgentOutput.usage` | `--verbose` | `{input, output, cache_read, cache_write}` from `LlmUsage` |

### Conditional (Tier 3 — already-known context, included when meaningful):

| Field | Type | Source | Gating | Notes |
|-------|------|--------|--------|-------|
| `task_id` | `Option<String>` | CLI flag value | unconditional when `--task-id` provided | already echoed in the top-level `task_id` field on success; this is a separate envelope-scoped echo for symmetry with `parent_task_id` |
| `parent_task_id` | `Option<String>` | CLI flag value | unconditional when `--parent-task-id` provided | confirms task-context threading |

### Deferred (out of scope):

| Field | Reason for deferral |
|-------|---------------------|
| `cost_usd` | Requires per-provider pricing tables — new infrastructure for a p3 CLI enhancement (YAGNI). Reopen as a sibling ticket if/when pricing is added centrally. |
| `tool_calls` (count or list) | `AgentOutput` does not currently surface tool-call summaries to the CLI return path. Plumbing would extend the type for one consumer. Defer until a second consumer needs it. |
| `skills_active` | Same as above — `AgentOutput` does not carry the active skill set. |
| `trace_id` | Already feature-gated on telemetry compile flag. Threading trace_id from `agent::run_agent` → CLI is a separate concern requiring its own propagation channel. Reopen when telemetry consumers want this. |

The committed field set delivers the operator value named in the issue ("what model ran, how long it took, what task context applied") without scope creep into pricing infrastructure or new plumbing channels.

## Scope boundaries

- Extend `MetadataEnvelope` struct with 6 new optional fields (5 verbose-gated, 2 unconditional-when-meaningful).
- Mirror in text-mode trailer (one `key: value` per line).
- Update `crates/mika-cli/CLAUDE.md` `mika ask` section to document the expanded envelope.
- **Out of scope:** cost_usd pricing infrastructure; tool_calls / skills_active threading from agent loop; trace_id propagation; restructuring text-mode trailer format; adding metadata without `--verbose`.

## Implementation Units

### U1 — Extend `MetadataEnvelope` struct

**Goal:** Add the 6 new fields with `skip_serializing_if = "Option::is_none"`.

**Files:**
- Modify: `crates/mika-cli/src/commands/ask.rs` (around lines 34-42 — `MetadataEnvelope` struct)

**Approach:**

```rust
#[derive(serde::Serialize)]
struct MetadataEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<TokensMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_task_id: Option<String>,
}

#[derive(serde::Serialize)]
struct TokensMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_read: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_write: Option<u32>,
}
```

Constraint: top-level `metadata` field on `AskJsonResponse` already has `skip_serializing_if`; if all `MetadataEnvelope` fields are None the envelope is absent from JSON — non-verbose / non-task invocations remain byte-identical to current output (AC3 satisfied structurally).

**Test scenarios:**
- **Empty envelope omitted:** `MetadataEnvelope::default()` serializes to `{}`; wrapped in `Option`, non-Some value preserves omission.
- **Partial population:** envelope with only `session_id` set serializes as `{"session_id":"..."}` — other fields absent.
- **Full population:** envelope with all fields serializes complete JSON shape.

**Verification:** new unit tests in `ask::tests` cover the three shapes; `cargo build -p mika-cli` clean.

### U2 — Populate verbose-gated fields in `ask::run`

**Goal:** When `verbose` is true, populate `model`, `agent_id`, `latency_ms`, and `tokens` from runtime data.

**Files:**
- Modify: `crates/mika-cli/src/commands/ask.rs` (the `run` function, around the agent loop call site)

**Approach:**

1. Record start time: `let started = std::time::Instant::now();` immediately before `agent::run_agent(...)` is called.
2. After the agent loop returns, when building the metadata envelope:
   - `latency_ms = Some(started.elapsed().as_millis() as u64)` when `verbose`
   - `model = Some(<provider/model string from ctx>)` when `verbose` — implementer chooses the canonical source (settings resolved post-`override_model`); the value matches what would appear in `/model` output and `llm_calls.provider`/`llm_calls.model`
   - `agent_id = Some(agent_name.to_string())` when `verbose`
   - `tokens = output.usage.as_ref().map(|u| TokensMetadata { ... })` when `verbose`; map from `LlmUsage` fields; if `usage` is None, leave `tokens` as None (preserves omit-when-absent semantics)
3. Populate unconditional fields independent of `verbose`:
   - `task_id = task_id.map(|s| s.to_string())` — already-passed CLI param
   - `parent_task_id = parent_task_id.map(|s| s.to_string())` — already-passed CLI param

**Constraint:** the existing `--task-id` flag also populates the top-level `task_id` field on `AskJsonResponse` (per CLAUDE.md's documented behavior for `--task-complete`). The metadata-envelope echo is additive — non-task invocations leave both fields None.

**Test scenarios:**
- **Verbose with usage:** mock `AgentOutput { usage: Some(usage), ... }` + `verbose=true` → envelope has `model`, `agent_id`, `latency_ms`, `tokens`.
- **Verbose without usage:** `AgentOutput { usage: None, ... }` + `verbose=true` → envelope has `model`, `agent_id`, `latency_ms` populated; `tokens` is None and omitted from JSON.
- **Non-verbose with task context:** `verbose=false`, `task_id=Some("abc")` → envelope has only `task_id`, no other fields; JSON shows `{"task_id":"abc"}`.
- **Non-verbose, no task context:** envelope is fully empty → top-level `metadata` field omitted from JSON; output byte-identical to pre-#843 behavior.

**Verification:** `cargo test -p mika-cli ask::tests` covers each scenario; manual smoke test post-build confirms expected output.

### U3 — Mirror in text-mode trailer

**Goal:** Text-mode `--verbose` trailer renders the same fields (excluding fields that don't apply).

**Files:**
- Modify: `crates/mika-cli/src/commands/ask.rs` (text-mode print path)

**Approach:** Extend the existing text-mode trailer printing logic to emit one `key: value` line per populated field. Order:

```
<assistant response text>

session_id: <uuid>
model: <provider/model>
agent_id: <name>
latency_ms: <ms>
tokens.input: <n>
tokens.output: <n>
tokens.cache_read: <n>
tokens.cache_write: <n>
task_id: <id>
parent_task_id: <id>
```

Use flat `tokens.input` / `tokens.output` keys for grep-friendliness in text mode (vs JSON's nested object). Omit any line whose value is None.

The blank-line separator (already in place between response and trailer) is preserved.

**Test scenarios:**
- **Verbose with tokens:** trailer prints all 4 `tokens.*` lines + the 4 envelope fields.
- **Partial population:** only populated fields appear in trailer; lines are NOT printed for None values.
- **Non-verbose, no task context:** no trailer at all (existing behavior).

**Verification:** assert text-mode output line-by-line in `ask::tests`; manual smoke test.

### U4 — Docs update

**Goal:** `crates/mika-cli/CLAUDE.md` `mika ask` section documents the expanded envelope with field-by-field gating.

**Files:**
- Modify: `crates/mika-cli/CLAUDE.md` (the `mika ask` section + the **Metadata envelope semantics (JSON mode)** subsection)

**Approach:** Replace the current "Today only `session_id` exists..." paragraph with a field table mirroring this plan's Committed/Conditional sections (without the Deferred section — that lives in the closed-PR-and-merged-plan archive). Note that:
- Verbose-gated fields appear only when `--verbose` is set
- Unconditional fields (`task_id`, `parent_task_id`) appear when their CLI flag is provided, regardless of `--verbose`
- The `metadata` envelope is omitted entirely from JSON output when all fields are None

Preserve the existing per-field-gated framing as the doctrinal contract.

**Verification:** manual read.

## Dependencies / sequencing

- U1 → U2 (U2 populates fields U1 defines)
- U1 → U3 (U3 reads the same struct, prints text-mode)
- U2 and U3 are independent after U1 lands
- U4 (docs) ships in same PR; can be authored after U1-U3 stabilize

## Patterns to follow (cross-cutting)

- `crates/mika-cli/src/commands/ask.rs:34-42` — existing `MetadataEnvelope` struct pattern (`skip_serializing_if`, derive(Serialize))
- `crates/mika-cli/CLAUDE.md` `mika ask` § Metadata envelope semantics — existing doc contract
- mika#824, mika#830 PRs — prior art for envelope extensions

## Verification (top-level)

- `cargo test -p mika-cli` passes
- `cargo clippy -p mika-cli` clean
- `cargo fmt --all -- --check` clean
- Manual smoke test:
  - `mika ask --agent mika-test --verbose --format json "ping"` — JSON metadata has model, agent_id, latency_ms, tokens
  - `mika ask --agent mika-test --verbose "ping"` — text-mode trailer shows same fields
  - `mika ask --agent mika-test --format json "ping"` (no --verbose, no task) — byte-identical to current output (no `metadata` field)
  - `mika ask --agent mika-test --task-id abc-123 --format json "ping"` — JSON metadata has `task_id` but no other fields (unconditional path)

## Risk / known unknowns

- **`model` field source.** The plan assumes `model` is resolvable from the agent context after `override_model` resolves. Implementer verifies the canonical field — likely `ctx.settings.provider` + `ctx.settings.<provider>_model` or similar. If a clean accessor doesn't exist, defer `model` to a sibling ticket and ship the rest.
- **`AgentOutput.usage` field type.** Plan assumes `LlmUsage` carries the four token fields. If the type only has `input` / `output` (no cache fields), populate what's available — `cache_read`/`cache_write` stay None and are omitted from output.
- **Per-field gating discipline.** This plan introduces 2 unconditional fields (`task_id`, `parent_task_id`). Future contributors must continue the per-field decision — not blanket-`--verbose`-gate. The doc update in U4 keeps this contract visible.

## Out-of-scope (explicit)

- `cost_usd` — pricing infrastructure (YAGNI for p3 CLI work).
- `tool_calls` — would require `AgentOutput` plumbing extension.
- `skills_active` — same.
- `trace_id` — telemetry-side propagation concern.
- Text-mode trailer format restructuring (e.g., YAML, JSON) — separate concern.
- Adding metadata to `--format json` without `--verbose` (per body's out-of-scope).
