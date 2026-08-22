---
module: agent-loop
tags: [reflection-mode, update_fact, tool-contract, first-attempt-miss, retry-recovery, evidence-field, mika-agent]
problem_type: llm-contract-slip-with-self-recovery
category: prompt-engineering
---

# Reflection-mode `update_fact` misses the required `evidence` field on the first attempt (~47%), self-recovers on retry (~87%)

Investigation of mika#1770. The original ticket mika#1743 (2026-07-06) reported
"7+ stale commitments Mika cannot cancel" and hypothesised that the `id` field
was not exposed in `search_memory` output. That hypothesis was invalidated in
the mika#1743 closing comment. This investigation names the real wedge.

## What we measured

Direct SQL against `~/.mika/data/mika.db` for the last 30 days of `update_fact`
calls on the `mika` agent (the singleton personal-agent DB — well-known family
agents write into the shared per-DB path, not `~/.mika/agents/mika/data/`):

```sql
SELECT COUNT(*) as n,
       SUM(CASE WHEN success=0 THEN 1 ELSE 0 END) as failed,
       SUM(CASE WHEN success=1 THEN 1 ELSE 0 END) as ok
FROM tool_calls
WHERE agent_id='mika' AND tool_name='update_fact';
```

Result (2026-07-28 → 2026-08-17, N=17):

| metric | count | % |
|---|---:|---:|
| Total attempts | 17 | 100 |
| Failed (first attempt) | 8 | 47 |
| Succeeded | 9 | 53 |

**Every single failure carried the same output**, verbatim:

> `Reflection mode requires an evidence field citing specific conversation content.`

The failure fingerprint matches `crates/mika-agent/src/tools/mod.rs:281` —
`check_reflection_evidence()` returns this error when `ctx.is_reflection ==
true` and the `evidence` field is empty. The gate call site is
`crates/mika-agent/src/tools/update_fact.rs:60`.

## The retry pattern is the key finding

7 of the 8 failures self-recovered within the same reflection session, ~10
seconds after the initial error. Session `reflection-2026-07-28` on 2026-07-28
at 13:00:39 emitted seven parallel `update_fact` calls **without** an evidence
field (rows 1–7); all seven failed. The next assistant turn at 13:00:49–13:00:50
re-emitted the same seven calls **with** a synthesised evidence field
(concrete example on row 8):

```json
{
  "category": "commitment",
  "id": 22,
  "updates": {"status": "cancelled"},
  "evidence": "[2026-07-28T13:00:00Z] Reflection search found 3 identical \"check email tomorrow morning at 9am\" commitments (ids 22, 23, 25) — all pending. \"Tomorrow\" relative to creation date is months past. No actionable meaning."
}
```

All seven retries succeeded. The **effective** cancellation success rate on
that reflection turn was 100% — the mika#1743 report of "7+ stale commitments
Mika cannot cancel" was true at the time (2026-07-06) and is no longer true
(the same-shape retry landed 22 days later on 2026-07-28).

The **residual failure** is a single case: session `reflection-2026-08-17`,
attempt on commitment id=52 ("Vincent has minimum 5 months of runway…",
due 2026-12-03). Mika tried to mark it `completed`, hit the evidence gate, and
**did not retry** in that session. That single commitment stayed `pending`.

Current pending-commitment state for `agent_id='mika'`: **one row** (id=52).
Every other pending row in the DB belongs to another agent (mika-arch,
mika-dev, mika-prime, etc.) whose commitment-lifecycle wedge is out of scope
here.

## Class-by-class verdict (mika#1770 issue candidates A/B/C/D/E)

| Class | Verdict | Evidence |
|---|---|---|
| **A — LLM id-parse miss** | **Ruled out.** Every failed row carried a valid integer id (22, 23, 25, 18, 26, 27, 21, 52). Zero `'id' is required and must be a positive integer'` and zero `'Commitment with id N not found'` errors in the 30-day window. `crates/mika-agent/src/tools/search_memory.rs:242-253` already emits the `[commitment id=N status:...]` label prefix on the LIKE-fallback path (mika#1769 is in the tree). |
| **B — `update_fact` contract mismatch** | **Confirmed as primary cause.** Every failure output is the reflection-mode evidence gate. Every subsequent successful retry supplies the field. |
| **C — Missing recipe in prompt** | **Ruled out.** The reflection prompt at `crates/mika-agent/src/agent_loop/mod.rs:3825-3852` explicitly says: *"The evidence field MUST cite a specific conversation timestamp and quote."* The requirement is present; the LLM still misses it ~47% on first emission. This is prompt-erosion under multi-tool-parallel emission, not a documentation gap. |
| **D — Guard interference** | **Ruled out.** Zero guard firings on any of the 17 turns. The 8 failures are direct tool errors returned to the LLM; the assistant text on the corrective turn was accepted without any of the 11 post-condition guards firing. |
| **E — Truncation** | **Ruled out.** The `input` field on every row deserialises cleanly to JSON with a valid `id` at the expected position. `TOOL_METADATA_MAX = 4000` chars was never reached — inputs are all under 1 KB. |

## Why the first-attempt miss happens

The reflection prompt lists the evidence requirement inside a `## Rules`
block **below** the `## What to do` action list. When the LLM plans a
parallel batch of 7 `update_fact` calls at the top of a reflection turn
(dedup/consolidate HOUSEKEEPING step), it emits the schema-required fields
(`id`, `category`, `updates`) and forgets the reflection-mode-conditional
extra field. The evidence field is optional in the JSON schema
(`update_fact.rs:48-52`, `"required": ["id", "category", "updates"]`) — only
the runtime gate makes it mandatory in reflection mode. **The tool's own
declared contract does not tell the LLM that the field is required in this
mode.**

The gate then returns an error string, the LLM reads it, adds the evidence
field, and retries. Recovery is high (7/8 = 87%) because the error message is
specific and actionable. The unrecovered case (1/8) is the operational cost of
the miss + no-retry combination on the last turn of a session.

## Follow-up fix — proposed

Filed as a discrete follow-up ticket (per mika#1770's `## Not in scope`
clause). Two options, ranked:

**Option 1 (preferred) — Reflection-mode reinforcement in the tool description
+ prompt example.** Cheapest, best cost/benefit:

1. Extend the `evidence` field's description in `update_fact.rs:48-52` from
   the current *"Required in reflection mode: cite a specific conversation
   timestamp and quote as justification for this change"* to include a
   worked example the tokenizer will see attached to the tool call surface:

   ```
   REQUIRED IN REFLECTION MODE. Format: "[YYYY-MM-DDTHH:MM:SSZ] <one-sentence
   citation of the conversation content that justifies this change>". Missing
   or empty evidence in reflection mode ALWAYS returns an error — no
   exceptions. Example: "[2026-07-28T13:00:00Z] Reflection search found id=22
   duplicate of id=25, both pending, no actionable meaning."
   ```

2. Add an inline reminder to the reflection prompt (`agent_loop/mod.rs:3825`)
   **inside** the `## Available tools` block, right next to `update_fact`:

   ```
   - update_fact: Update commitment status (completed/cancelled). MUST include
     `evidence` field in reflection mode — no exceptions.
   ```

3. Sibling reinforcement for the two other reflection-gated tools
   (`store_fact.rs:72`, `update_core_memory.rs:126`) — same pattern.

**Option 2 (defense-in-depth if Option 1 misses < 5%) — Split the tool.**
Register a distinct `update_fact_reflection(id, category, updates, evidence)`
tool with `evidence` at position 4 and required in the JSON schema. Reflection
mode swaps `update_fact` for `update_fact_reflection` in the enabled-tool set.
Conversation-mode `update_fact` stays as-is. Removes the runtime-only
requirement from the LLM's decision surface entirely — the schema itself
enforces it.

Not chosen for v1 because Option 1's expected recovery (7/8 → 8/8 first-try
rate under prompt reinforcement) is likely sufficient and Option 2 doubles the
tool surface area for a bounded gain.

## Stale-commitment cleanup outcome

- 2026-07-28 batch of 7 stale commitments (mika#1743's original report):
  **cleared** — self-recovery via retry landed the cancels 22 days after the
  original report.
- 2026-08-17 residual (id=52, Vincent-runway assertion): **still pending**.
  Deferred to the follow-up fix ticket for a supervised retry once Option 1 is
  deployed — cancellation would be trivial post-fix.

## Discipline anchors

- **Evidence discipline.** Every classification decision above cites the
  concrete artifact (SQL row, log line, file:line). No "looks like class B"
  without a citation.
- **Read-only investigation.** No mutations to `~/.mika/agents/mika/` or its
  DB. The one remaining `pending` row is left untouched — the fix ticket owns
  the supervised retry.
- **Do not touch mika#1743.** That ticket is closed not-planned; the
  invalidated-premise history there is context, not a target.
