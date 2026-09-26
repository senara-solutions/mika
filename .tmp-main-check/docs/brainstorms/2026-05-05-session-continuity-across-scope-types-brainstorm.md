# Session Continuity Across Scope Types

**Date:** 2026-05-05
**Status:** Decided (Option C — `task_messages` parallel narrative table)
**Arch peer review:** session `01963864-7c63-4242-a1ff-718941618f8a` (validated empirical claims, surfaced cold-start universal-fallback constraint, recommended ESCALATE for platform-direction call)


## Headline

mika's compaction primitive deletes messages **agent-scoped, not session-scoped or task-scoped**. The DELETE clause at `db.rs:5845` is `WHERE agent_id = ?` — when compaction fires for mika-dev, it removes messages from every mika-dev session in one transaction. 98.5% of message rows ever written are gone (IDs 13–27042 in the table, 406 rows surviving). What survives is **one flat prose summary per agent** in a `system-<agent>` session — no scope structure, no task partition, no way to extract "just the milestone#13 narrative" from it.

Any continuity fix that only tags at write time will lose its tagging within hours of the next compaction trigger. The continuity question is not just "where do we write?" but "what survives the deletion model?"

## Empirical evidence

### Compaction code path (verified `db.rs:5833–5862`)

```rust
pub fn replace_with_summary(
    &mut self,
    agent_id: &str,
    summary: &str,
    compacted_through_id: i64,
) -> Result<i64> {
    let system_session = self.get_or_create_system_session(agent_id)?;
    let tx = self.conn.transaction()?;
    // (1) Delete all non-summary messages for this AGENT up to threshold
    tx.execute(
        "DELETE FROM messages
         WHERE agent_id = ?1 AND role != 'summary' AND id <= ?2",
        params![agent_id, compacted_through_id],
    )?;
    // (2) Delete old summary
    tx.execute(
        "DELETE FROM messages WHERE agent_id = ?1 AND role = 'summary'",
        params![agent_id],
    )?;
    // (3) Insert one new summary row in system-{agent} session
    tx.execute(
        "INSERT INTO messages (session_id, agent_id, role, content, compacted_through_id)
         VALUES (?1, ?2, 'summary', ?3, ?4)",
        params![system_session, agent_id, summary, compacted_through_id],
    )?;
    ...
}
```

Trigger threshold (per `crates/mika-agent/CLAUDE.md`): 50 messages per agent, keep 20 most recent, summarize older.

### Empirical state (today's session)

- `messages` table: 406 rows, ID range 13–27042. **~98.5% deletion rate.**
- 6 `role='summary'` rows survive — one per agent — in `system-<agent>` sessions:

| Summary ID | Session | compacted_through_id | When |
|---|---|---|---|
| 27005 | system-mika-dev | 26954 | today 11:45:23 |
| 26987 | system-mika-relay | 26939 | today 11:38:51 |
| 26665 | system-mika-arch | 26372 | 2026-05-03 09:59 |
| 26468 | system-mika-qa | 25816 | 2026-05-02 19:56 |
| 22346 | system-mika-test | 19395 | 2026-04-26 17:30 |
| 22341 | system-mika | 19656 | 2026-04-26 17:28 |

- Verified earlier in this session that bf7ccb4d (sprint dispatch session, 00:22 UTC) had three messages with full content (IDs 26789–26791). Re-queried after compaction at 11:45: zero rows. Compacted via row 27005 (compacted_through_id=26954 > 26791).
- The session row in `sessions` persists; its narrative does not.

### What survives in the per-agent summary

**The structural fact:** the per-agent summary is one flat prose blob. system-mika-dev's row content begins *"## Conversation Summary ### mika#955 — fix(executor): validate required fields..."* — mixing ticket #955 work with everything else mika-dev did before the compaction trigger. There is **no structured way** to extract "just the milestone#13 narrative" from this summary. This is the structural argument for **partitioning summarization by scope**, not just tagging at write time. Tagging without partitioning means the tag is durable for ~50 messages, then evaporates.

### Multi-session fan-out for one milestone (live)

Trace of milestone#13 (`d9989b6d`, dispatched 11:15 today, in-progress):

| Sessions involved | Channel | Role |
|---|---|---|
| 85ffcc04 | cli | original dispatch (created milestone parent + 8 children, ended 11:16:55) |
| 778e7977 | cli | second wave 9 minutes later (created 5 more children + first callback for issue#652) |
| callback-93a153fe | system | first callback for issue#652 — `parent_session_id`→778e7977 |
| 78b8c396 | github | webhook event (PR creation for issue#652) |
| callback-ea97b77a | system | retry callback — `parent_session_id`→78b8c396 (different parent than first callback) |
| a158ac81, a3b5e7e9, 50629176, 5736a297, 2b0d9c9e, 90e2b8b1 | github | further webhook events |

16+ sessions for one milestone in <1hr. The work-item tree (`parent_task_id` chain) correctly spans them all and is the only true continuity primitive. `parent_session_id` is unreliable: it points at *whatever session was active at callback dispatch time*, not at the scope's dispatch session — visible above where the two callbacks for issue#652 have different `parent_session_id` values. **All three options below route narrative via `task_id` walk to scope root, not via `parent_session_id` chain — this is why.**

## Deployed-surface inventory

### `messages` schema (v30)

| Column | Type | Notes |
|---|---|---|
| `id` | INTEGER PK | |
| `session_id` | TEXT NOT NULL | Channel-scoped |
| `agent_id` | TEXT NOT NULL | Compaction's deletion key |
| `role` | TEXT NOT NULL | user / assistant / summary / system |
| `content` | TEXT NOT NULL | |
| `metadata` | TEXT (JSON) | Tool-call summaries (capped 4000 chars per #744) |
| `trace_id` | TEXT | Per-turn correlation |
| `compacted_through_id` | INTEGER | On `role='summary'` rows: highest ID folded in |
| `created_at` | TEXT (ISO 8601) | |
| `internal` | INTEGER NOT NULL DEFAULT 0 | Schema v22 — DEPLOYED |

### `messages.internal` — deployed semantics

Per `crates/mika-agent/CLAUDE.md` schema notes: *"Agent-to-agent message visibility. TUI inbox mode filters internal messages at the DB level. Set by `delegate_task` tool and by `mika ask --task-id` relay sessions (without `--task-complete`). `AgentParams.internal` threads the flag through `run_loop` to all message save paths."*

**Write sites (verified `grep`):**
- `agent.rs:1798, 1892, 2247, 2303` — `run_loop` persists `params.internal` per turn.
- `agent.rs:2224, 2279, 2313` — deadline-fallback paths propagate it.
- `db.rs:5714` — `INSERT INTO messages (..., internal) VALUES (..., internal as i64)` — sole DB write.
- Set to `1` by: `delegate_task` (orchestrator → delegate); `mika ask --task-id <X>` without `--task-complete` (relay sessions, tool-permission-grant turns).

**Read sites:**
- `db.rs:5721` — `SESSION_MESSAGE_COLUMNS` includes `m.internal`.
- `db.rs:5755` — `load_recent_messages(exclude_internal: bool)` adds `AND m.internal = 0` when caller requests user-facing-only.

**Reviewer note for arch:** Extending `internal=1` from *"agent-to-agent message, hidden from human inbox view"* to also encode *"structured callback summary for the dispatch session"* is a real semantic conflation, not a clean reuse. Two orthogonal concerns on one boolean. Worth weighing explicitly against introducing a separate marker column or a different mechanism entirely.

### `tasks` table — work-item structure (v30)

- `parent_task_id` (chain) + `type IN ('issue','milestone','project')` (scope) — **the only structurally-correct continuity primitive today.**
- `created_by_session` records *one* originating session, not all sessions that contributed.
- Walking `parent_task_id` to scope root works for state. State ≠ narrative.

## Constraints from precedent

- `tasks.metadata` is the canonical engine-writes-async-result surface (mika#376, `try_extract_callback_metadata` in `dispatcher.rs`). Solution doc at `mika/docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md` is explicit: *"prompt-only enforcement doesn't work for critical persistence."*
- History-rebuild reads from `messages` rows; once compacted, the rebuild sees only the per-agent flat summary in `system-<agent>` instead of raw history. Any change to compaction's partition logic must preserve the prompt-context window assembly (last-20 raw rows + the summary). Note: `detect_fabricated_action_claim` at `agent.rs:4092` is a pure regex on the *current turn's* assistant text — does not scan history — so it's not coupled to compaction. Other post-conditions that DO scan history (e.g., `pr_reviews_posted` session-scope dedup) operate on the in-memory `AppState` map and are unaffected by message-table compaction.
- `messages` rows older than 50 (per agent) are not durable. Any solution operating on raw rows after that threshold is fighting the architecture.

## Universal: untagged-row fallback

All three options below condition behavior on `task_id IS NOT NULL` for at least some code path. Every option must specify what happens to messages without a known scope ancestor at write time — cron-fired tasks, webhook handlers without issue→task lookup, channel chatter, and pre-migration messages all generate untagged rows. The fallback rule is option-specific (see each option's block) but the constraint is universal. Arch's grooming pass should treat untagged-row handling as a first-class part of the chosen option's contract, not an edge case.

## Three options for arch — what's actually being chosen

The choice between A, B, and C is also a choice about whether compaction stays a single primitive or whether narrative storage becomes orthogonal to compaction. **This is an architectural-shape question, not a cost-tradeoff question.** All three options solve "milestone narrative survives compaction." They differ in what they imply for *future* scope-aware features (per-channel retrospection, per-skill audit, per-trace-id replay) — features that today would all hit the same agent-scoped-DELETE wall.

### Option A — Per-scope summarization

Schema: extend `messages` to optionally tag `task_id`; modify `replace_with_summary` to partition by scope at compaction time; produce one summary per scope-root instead of one summary per agent.

Architectural shape: **compaction stays a single primitive but learns about scope.** The partition function is hardcoded to `task_id` (scope root). Future scope-aware features (per-channel etc.) would need additional partition functions baked into compaction.

Untagged-row fallback: untagged rows continue to compact into the per-agent flat summary as today. Compaction now produces N+1 summaries per cycle (one per active scope plus the per-agent catch-all). The per-agent summary primitive persists indefinitely alongside per-scope summaries.

Tradeoff: reuses existing infrastructure; preserves task narrative natively; compaction code path materially changes; subsequent scope dimensions require touching compaction again.

### Option B — Preserve task-tagged from deletion

Schema: add `messages.task_id`; modify compaction's DELETE to `WHERE agent_id = ? AND role != 'summary' AND task_id IS NULL AND id <= ?`. Tagged messages survive forever.

Architectural shape: **compaction stays a single primitive; tagged messages opt out of it.** Simplest schema change.

Untagged-row fallback: the DELETE clause's `task_id IS NULL` predicate already handles this — untagged rows are the deletion target. Compaction behavior for untagged rows is unchanged from today.

Tradeoff: tagged-message rows accumulate unboundedly per agent; storage and scan performance degrade over agent lifetime; no answer for "scope summarization" — just full-fidelity retention of everything tagged.

### Option C — Parallel non-compacted table for narrative (chosen)

Schema: introduce `task_messages (id, task_id, agent_id, session_id, role, content, metadata, created_at)` — append-only, never compacted. `messages` stays as-is for channel narrative. History-rebuild for task-mode reads from `task_messages`; channel-mode reads from `messages`.

Architectural shape: **narrative storage becomes orthogonal to compaction.** Channel narrative and task narrative are structurally distinct surfaces with their own lifetime models. Future scope-aware features can either join `task_messages` against task structure or introduce their own parallel surface without touching compaction.

**Write-side contract — load-bearing for grooming:** task-tagged rows are written to **both** `messages` (preserving channel-narrative) and `task_messages` (enabling task-narrative). Untagged rows are written to `messages` only and don't appear in `task_messages`. The double-write decision is the implementation choice C carries that A and B don't:
- **Both** is the default and matches "channel-narrative is unchanged, task-narrative is additive."
- Doubles the write cost on async events where `task_id` is known.
- Creates a consistency-recovery question if one write succeeds and the other fails — needs an explicit recovery rule (single transaction, idempotent retry, or eventual reconciliation). Grooming pass must address.
- Alternatives (`task_messages` only — breaks channel-narrative reads; `messages` only with `task_id` joined — converges to A/B) are explicitly rejected.

Untagged-row fallback: rows with no `task_id` at write time are written to `messages` only. Channel-narrative reads them as today. They never appear in `task_messages` — the absence is the fallback.

Tradeoff: clean separation of concerns; new table + new write sites + double-write cost on tagged events; the most invasive change but the only one that decouples narrative storage from compaction's deletion model entirely. Storage cost is bounded — `tool_calls` (15.5K rows / 23.6 MB / ~500–1000/day) and `llm_calls` (112K rows / 24.9 MB) already establish the platform's pattern for unbounded structured surfaces; `task_messages` joins this existing class. Multi-year runway before any storage frontier matters.

### Reviewer's lean

**Vincent's call: C, accepting the migration cost.** The reasoning chain (validated by storage data and platform-direction analysis):

1. **Q1 — more scope dimensions coming?** Yes. `type IN ('issue','milestone','project')` is already three; per-skill audit, per-channel retrospection, per-trace-id replay are foreseeable. A as one-off partition function gets re-opened every time.
2. **Q2 — bounded vs unbounded retention?** Unbounded is fine. `tool_calls` and `llm_calls` already establish the precedent. Storage frontier is wide open (~60 MB total DB; multi-year runway).
3. **Q3 — task vs channel narrative read structurally different?** Yes. Cross-agent-cross-session graph (task) vs single-thread temporal (channel) is a genuine structural difference; one-table-with-partition-flag (A) forces every query to know which mode it's in.

A→C-trajectory is defensible (ship A now, migrate to C later) but has hidden cost: A's compaction-partition logic is harder to back out than to add. A→C means living with both during migration, paying double-write costs and dual-read complexity twice. C-direct accepts the migration cost once.

**B is off the table.** It's a stopgap that doesn't address summarization, accumulates rows unboundedly without the structural separation that justifies the unboundedness, and requires a second decision round when scope-scoped summarization becomes necessary anyway.

### Decision premise (load-bearing)

The C choice rests on the assumption that more scope dimensions are coming (per-skill audit, per-channel retrospection, per-trace-id replay). That assumption is grounded in the variant-tooling/skill-marketplace platform direction. **If that direction reverses** (skill marketplace stays internal-only, observability stays operator-only, no per-trace-id replay materializes), Q1's answer changes and A becomes sufficient. Low-probability path, but the assumption is worth flagging for future-self if platform direction shifts.

## Out of scope (still)

- Re-tagging historical messages (98.5% deleted; operationally impossible).
- Cross-agent task threading (mika-qa's review messages tagging the same scope as mika-dev's dispatch).
- Operator dashboard for task-narrative views.
- The `sprint:` keyword silent-routing UX bug (deferred follow-up; not yet filed as a ticket; tracked in operator handoff outside this brief's scope).
