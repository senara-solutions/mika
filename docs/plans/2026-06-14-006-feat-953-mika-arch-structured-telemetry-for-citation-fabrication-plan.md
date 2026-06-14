# Plan: feat(mika-arch): structured telemetry for citation-fabrication detection

**Ticket:** mika issue#953
**Type:** enhancement (defense-in-depth observability)
**Branch:** `feat/953/mika-arch-structured-telemetry-for`
**Parent:** mika issue#952 (citation-fabrication prompt-level fix)

## Problem

mika-arch's citation-fabrication failure modes (#952) are addressed at the prompt level (verbatim-quote anchoring, session-id chain anchoring). However, when a fabrication *does* slip through — or when the operator or a second-pass review identifies one — there is no structured telemetry to record it. Guard firings in `agent.rs` use plain `warn!` calls without `target: "mika::otel"` or `event = "..."` structured fields, making them log-file-only (no Langfuse export, no SQL queryability).

The ticket asks for a `kg_arch_fabrication_detected` event (or equivalent) that fires on identified fabrication, with fields for session_id, instance type, and corrected content.

## Design Decisions

### D1: Two emission sites, not one

Fabrication detection occurs at two distinct points:

1. **Engine-level guards** — existing guard firings in `agent.rs` (positions 5, 5b, 6c, 6d) already detect fabrication classes at EndTurn. These currently emit plain `warn!` without structured event fields.

2. **Architect self-report** — mika-arch itself may detect during review that a prior finding or citation cannot be anchored. The skill prompts instruct "flag the inability to anchor" but there is no structured event for it.

**Decision:** Instrument both sites. The engine-level guards get `target: "mika::otel"` + `event = "..."` structured events (consistent with `kg_resolver_tick.complete` pattern). The architect self-report gets an `audit_events` row via the existing `log_audit_event()` path (queryable by agents/operators, consistent with `task_engine_reaper` pattern).

### D2: Event taxonomy — one event name per guard, not one generic event

Rather than a single `arch_fabrication_detected` event that multiplexes all fabrication classes via an `instance_type` field, use the existing guard labels as event names. This matches the codebase convention where each structured event is grep-friendly by name.

**Event names (engine-level):**

| Guard position | Current `warn!` | New structured event name |
|---|---|---|
| 5 — fabricated action claim (#308) | plain warn | `guard.fabricated_action_claim` |
| 5b — dev-groom fabrication (#1133) | plain warn | `guard.dev_groom_fabrication` |
| 6c — asserted unavailability (#862) | plain warn | `guard.asserted_unavailability` |
| 6d — assert grounded (#1331) | plain warn | `guard.assert_grounded` |
| 4c — callback state claim (#716) | plain warn | `guard.callback_state_claim` |

**Event name (architect self-report):**

| Source | Event name |
|---|---|
| audit_events row | `arch_citation_unanchored` |

### D3: audit_events for SQL-queryable catalog, tracing for OTLP export

The ticket asks for a "fabrication catalog queryable via SQL or log filter." Two complementary paths:

- **Tracing events** (`target: "mika::otel"`, `event = "guard.*"`) — exported to Langfuse via OTLP when telemetry is enabled, always written to log file. Queryable via `grep` or `jq`.
- **audit_events rows** — queryable via SQL (`SELECT * FROM audit_events WHERE tool_name = 'fabrication_guard'`). Written for the architect self-report path; engine guards do NOT write audit_events rows (they are re-prompt guards, not tool calls — the guard itself is the corrective action, not a state mutation that needs audit tracking).

### D4: Field set per the acceptance criteria

Each structured event carries:

| Field | Source | Notes |
|---|---|---|
| `trace_id` | `run_loop` scope | Already available in guard context |
| `agent_id` | `run_loop` scope | Already available |
| `session_id` | `run_loop` scope | Needs threading from `AgentParams` or `ToolContext` |
| `guard` | guard label constant | Event discriminator (e.g., `"asserted_unavailability"`) |
| `step` | loop counter | Already in scope at every guard site |
| `detail` | guard-specific | Tool name for 6c, verb+url for 5, claim text for 6d, etc. |

The `corrected_content` field from the ticket is not directly capturable — the guard re-prompts the LLM and the corrected response appears on the *next* turn. The guard event records what was *detected*, not what was corrected. This is the right shape: the detection event is the signal; the correction is the next turn's response, already persisted in `messages`.

### D5: No schema migration

No new tables or columns. Engine guards emit tracing events (zero DB writes). The architect self-report uses the existing `audit_events` table with `tool_name = 'fabrication_guard'`.

### D6: Scope boundary — engine guards only, no new detection logic

This ticket instruments *existing* detection logic with structured telemetry. It does NOT add new fabrication-detection heuristics. The detection code in `evidence/guards.rs` is unchanged. The calibration scenario `run_citation_discipline()` in `calibration/roles/mika_arch.rs` is test-time only and out of scope.

## Implementation Units

### Unit 1: Thread `session_id` to guard sites in `agent.rs`

**File:** `crates/mika-agent/src/agent.rs`

The guard sites in `run_loop` have access to `trace_id` (from `AgentParams`) and `step` (loop counter), but `session_id` is not in direct scope — it's on `AgentParams` which is destructured before `run_loop`. Thread `session_id: &str` as an additional parameter to `run_loop`.

**Changes:**
- Add `session_id: &str` parameter to `run_loop()` signature (line ~640)
- Pass `params.session_id` at the three `run_loop` call sites: `run_conversation_agent` (~line 2902), `run_silent_agent` (~line 3100), `run_team_agent` (~line 3300)
- Verify `AgentParams.session_id` exists — it does (`session_id: String` field)

### Unit 2: Upgrade guard `warn!` calls to structured events

**File:** `crates/mika-agent/src/agent.rs`

For each of the five guard firing sites, replace the plain `warn!` with a structured event that includes `target: "mika::otel"` and `event = "guard.<name>"`. The existing warn message text becomes the `message` parameter.

**Guard 5 — fabricated action claim (line ~1372):**
```rust
// Before:
warn!(step, verb, url, label = mode.label(), "Fabricated action ...");

// After:
warn!(
    target: "mika::otel",
    trace_id = %trace_id,
    agent_id = %agent_id,
    session_id = %session_id,
    step,
    verb,
    url,
    label = mode.label(),
    event = "guard.fabricated_action_claim",
    "Fabricated action claim with GitHub URL but zero tool calls"
);
```

**Guard 5b — dev-groom fabrication (line ~1436):**
```rust
warn!(
    target: "mika::otel",
    trace_id = %trace_id,
    agent_id = %agent_id,
    session_id = %session_id,
    step,
    label = mode.label(),
    event = "guard.dev_groom_fabrication",
    "dev-groom fabrication guard: response claims Verdict without dispatch"
);
```

**Guard 4c — callback state claim (line ~1330):**
```rust
warn!(
    target: "mika::otel",
    trace_id = %trace_id,
    agent_id = %agent_id,
    session_id = %session_id,
    step,
    claim = %claim_fragment,
    label = mode.label(),
    event = "guard.callback_state_claim",
    "Callback state claim without verification tool call"
);
```

**Guard 6c — asserted unavailability (line ~1615):**
```rust
warn!(
    target: "mika::otel",
    trace_id = %trace_id,
    agent_id = %agent_id,
    session_id = %session_id,
    step,
    tool = %tool_name,
    intent_guard = ASSERTED_UNAVAILABILITY_LABEL,
    label = mode.label(),
    event = "guard.asserted_unavailability",
    "Asserted-unavailability guard fired — re-prompting"
);
```

**Guard 6d — assert grounded (line ~1661):**
```rust
warn!(
    target: "mika::otel",
    trace_id = %trace_id,
    agent_id = %agent_id,
    session_id = %session_id,
    step,
    resource_type = claim.resource_type,
    resource_ref = %claim.resource_ref,
    claim = %claim.claim_text,
    intent_guard = ASSERT_GROUNDED_LABEL,
    label = mode.label(),
    event = "guard.assert_grounded",
    "Assert-grounded guard fired — re-prompting"
);
```

**Pattern note:** Use `target: "mika::otel"` so the OTLP exporter picks these up when telemetry is enabled. The `event = "guard.*"` field is the grep/jq discriminator. All existing log-file consumers continue to see the warn line.

### Unit 3: Architect self-report via audit_events

**File:** `crates/mika-agent/src/tools/send_message.rs` or a new lightweight helper

When mika-arch emits a message containing the phrase "unable to anchor" or "cannot be retrieved via a fresh tool call" (the verbatim-quote anchoring failure language from the skill prompts), the engine writes an `audit_events` row:

```
tool_name = "fabrication_guard"
target_key = "arch_citation_unanchored"
after_value = <truncated assistant text, max 500 chars>
reasoning = "mika-arch self-reported inability to anchor a citation"
```

**Implementation approach:** Add a post-EndTurn check in the conversation-mode path of `agent.rs`, after the guard chain but before the response is persisted. The check:

1. Only fires when `is_verdict_producer` is true (mika-arch skills loaded)
2. Scans the accepted assistant text for anchoring-failure phrases
3. Writes one `audit_events` row via `async_db.with_db()`

**Detection phrases (from skill prompts):**
- `"unable to anchor"`
- `"flag the inability to anchor"`
- `"cannot be retrieved via a fresh tool call"`
- `"could not retrieve"`

**False-positive mitigation:** Only fires for agents with verdict-producer skills (mika-arch). The phrases are specific enough that false positives from general conversation are unlikely. If a phrase appears in quoted plan content that mika-arch is reviewing, the event is a low-cost false positive — the catalog is for investigation, not automated action.

### Unit 4: Tests

**File:** `crates/mika-agent/src/agent.rs` (inline `#[cfg(test)]` module)

Add tests verifying the structured event fields are present. Since `tracing` events are not easily captured in unit tests without a subscriber, test the detection predicates (already covered in `evidence/guards.rs` tests) and add integration-level verification via the eval harness.

**New eval scenario:** `tests/eval/grounding_regressions/` — add a scenario that exercises one guard (e.g., fabricated action claim) and asserts the structured event fires. This uses `MockLlmProvider` with a canned fabrication response.

**Existing test coverage:** The `evidence/guards.rs` module already has comprehensive tests for all detection predicates. No changes needed there.

### Unit 5: CLAUDE.md documentation update

**File:** `crates/mika-agent/CLAUDE.md`

Add a subsection under Observability documenting the new structured guard events:

- Event names and when they fire
- Fields and their meaning
- How to query (grep pattern for log files, SQL for audit_events)
- Signal interpretation (companion to the existing Signal A–J documentation in root CLAUDE.md)

**File:** `CLAUDE.md` (root)

Add Signal K to the post-restart safety check section:

```
- **Signal K — guard fabrication telemetry (#953).** `grep 'guard\.' server.log | jq '.event'` — 
  any hits indicate a fabrication-class guard fired during an agent turn. The `session_id` and 
  `trace_id` fields enable drill-down to the specific turn. Sustained hits from the same agent 
  indicate the prompt-level defenses (#952) need reinforcement for that agent's failure mode.
```

## Verification

1. `cargo test -p mika-agent` — all existing tests pass (no guard behavior changed)
2. `cargo clippy` — clean
3. `grep "guard\." crates/mika-agent/src/agent.rs` — confirms structured event names present at all five guard sites
4. New eval scenario passes with `MockLlmProvider`
5. Manual: deploy, trigger a fabrication guard (e.g., send a message that triggers asserted-unavailability), verify `grep guard.asserted_unavailability server.log` returns a structured JSON event with all expected fields

## Out of Scope

- New fabrication-detection heuristics (separate ticket if needed)
- Changes to `evidence/guards.rs` detection logic
- Changes to mika-arch skill prompts (owned by mika#952)
- Dashboard UI for the fabrication catalog (future enhancement)
- Calibration scenario changes (`calibration/roles/mika_arch.rs`)
