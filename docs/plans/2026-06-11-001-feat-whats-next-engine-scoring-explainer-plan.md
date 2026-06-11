# Plan: What's Next Engine — Deterministic Scoring + LLM Explainer

**Ticket:** mika#1263  
**Layer:** 2 of Operational Partner Foundation  
**Depends on:** Layer 1 (mika#1262, merged — `operational_items` table at schema v39)

## Summary

Implement the What's Next engine: a deterministic priority scoring formula that ranks `OperationalItem`s, a status re-derivation pass that enforces the foundation's status taxonomy, an LLM explainer that narrates rankings without changing them, and four read surfaces (CLI `mika next`, dashboard inbox, daily brief, agent prompt injection).

The core principle: **the LLM explains, never decides.** The scoring formula is deterministic, calibratable, and debuggable. The LLM's role is downstream narration only.

## Phase 1 — Scoring Engine (`crates/mika-agent/src/operational/scoring.rs`)

### 1.1 Create `scoring.rs` module

New file at `crates/mika-agent/src/operational/scoring.rs`, exported from `mod.rs`.

**Functions:**

```rust
/// Compute composite priority score for a single item.
/// Returns a ScoreBreakdown with each term's contribution.
pub fn priority(item: &OperationalItem, now: DateTime<Utc>, db: &Database) -> ScoreBreakdown

/// Score and rank a list of items. Returns items sorted by priority DESC.
pub fn rank(items: &mut [OperationalItem], now: DateTime<Utc>, db: &Database) -> Vec<ScoredItem>
```

**`ScoreBreakdown` struct:**

```rust
pub struct ScoreBreakdown {
    pub total: f32,
    pub urgency: f32,
    pub commitment_weight: f32,
    pub user_importance: f32,
    pub stale_time: f32,
    pub dependency_risk: f32,
    pub confidence_penalty: f32,
}
```

**`ScoredItem` struct:**

```rust
pub struct ScoredItem {
    pub item: OperationalItem,
    pub breakdown: ScoreBreakdown,
}
```

### 1.2 Implement each scoring term

Per foundation doc §5, using constants from `calibration.rs`:

| Term | Implementation |
|------|----------------|
| `urgency` | Parse `item.due_at` as ISO 8601. `None` → 0.0. Past-due → `URGENCY_MAX` (100.0). Otherwise `URGENCY_MAX / (hours_until_due + 1.0)`. |
| `commitment_weight` | `item.kind == Commitment` → weight by `item.owner`: `User` → `COMMITMENT_WEIGHT_USER` (50.0), `Mika` → `COMMITMENT_WEIGHT_MIKA` (35.0), `Person(_)`/`Agent(_)` → `COMMITMENT_WEIGHT_THIRD_PARTY` (20.0). Non-commitments → 0.0. |
| `user_importance` | `item.user_importance.min(USER_IMPORTANCE_MAX)` (clamped at 50.0). |
| `stale_time` | Hours since `item.updated_at`. `log(hours + 1.0) * STALE_TIME_MULTIPLIER` capped at `STALE_TIME_CAP` (30.0). Uses `f32::ln()`. |
| `dependency_risk` | `db.count_blocked_items(&item.id)` × `DEPENDENCY_RISK_PER_BLOCKED` (10.0), capped at `DEPENDENCY_RISK_CAP` (40.0). Existing DB method. |
| `confidence_penalty` | `(1.0 - item.confidence) * CONFIDENCE_PENALTY_MULTIPLIER` (50.0). Subtracted from total. |

**Composite:** `urgency + commitment_weight + user_importance + stale_time + dependency_risk - confidence_penalty`.

### 1.3 Priority cache write-back

After scoring, call `db.update_operational_item_priority(id, total)` to persist the computed score. This keeps the `priority` column in sync for dashboard queries that sort by priority without re-computing.

### 1.4 Tests

- Unit tests for each term in isolation with synthetic `OperationalItem` fixtures.
- Deterministic ranking test: 5+ items with known field values, assert exact ordering.
- Edge cases: `due_at = None`, `confidence = 0.0`, `confidence = 1.0`, past-due item, commitment vs non-commitment, item with high dependency fan-out.
- Calibration suite: `#[cfg(test)] mod calibration_tests` with fixtures that produce expected rankings deterministically (no LLM in the loop).

## Phase 2 — Status Re-Derivation (`crates/mika-agent/src/operational/status.rs`)

### 2.1 Create `status.rs` module

Per foundation §4, status is **derivable, not authoritative** (except `Done` which is terminal). On every read, the engine re-derives status from item properties and updates the cache if it changed.

```rust
/// Re-derive the status of a non-Done item based on its current properties.
/// Returns the derived status. Does NOT write to DB — caller decides.
pub fn derive_status(item: &OperationalItem, now: DateTime<Utc>) -> OperationalStatus
```

**Derivation rules (precedence per Decision G: AtRisk > Now > Waiting > Delegated > Scheduled):**

1. **Done** → skip (terminal, excluded from re-derivation).
2. **AtRisk** — any of: `due_at` within 24h AND `updated_at` > 48h ago (stale near deadline); CI failure evidence; callback failure evidence. AtRisk overrides all non-terminal statuses.
3. **Now** — `due_at` is within the today window (next 24h) AND `blocked_by IS NULL`. Or: `due_at IS NULL` AND `blocked_by IS NULL` AND `status` was explicitly set to `Now`.
4. **Waiting** — `blocked_by IS NOT NULL`.
5. **Delegated** — `owner` is `Mika` or `Agent(_)` AND there's an active source task.
6. **Scheduled** — `due_at` is set AND beyond the today window.

### 2.2 Integrate re-derivation into the read path

`OperationalItem::query()` (in `query.rs`) calls `derive_status()` on each returned non-Done item. If the derived status differs from the cached status, issue a `db.update_operational_item_status()` call to update the cache. This ensures reads are always fresh.

**Important:** The re-derivation runs at read time, not write time. This means the `operational_items.status` column is a cache — the derivation rules are the authority.

### 2.3 `Blocked_by` cascade

When the query returns items, check for transitive unblocks: if a `Waiting` item's `blocked_by` ID points to an item that is now `Done`, the blocked item should re-derive (likely to `Now`). This is handled naturally by the derivation rules — `blocked_by IS NOT NULL` checks the ID, and if that blocker is Done, the write path should have cleared `blocked_by`. Add a helper:

```rust
/// Check if blockers are resolved and clear blocked_by if so.
pub fn resolve_cleared_blockers(items: &[OperationalItem], db: &Database) -> Result<()>
```

### 2.4 Tests

- Derive each status from fixture properties.
- Precedence: item with both AtRisk and Now conditions → AtRisk wins.
- Done exclusion: Done item with past-due `due_at` stays Done.
- Blocker cascade: Waiting item unblocks when blocker moves to Done.

## Phase 3 — LLM Explainer (`crates/mika-agent/src/operational/explainer.rs`)

### 3.1 Create `explainer.rs` module

The explainer takes a ranked list of `ScoredItem`s and produces human-facing prose explaining **why** each item is at its position. It does NOT change the ranking.

```rust
/// Generate narration for ranked items.
pub async fn explain_ranking(
    items: &[ScoredItem],
    llm_provider: &dyn LlmProvider,
    model: &str,
    max_items: usize,
) -> Result<Vec<ExplainedItem>>

pub struct ExplainedItem {
    pub item_id: String,
    pub rank: usize,
    pub rationale: String, // One-line human-facing explanation
}
```

### 3.2 Prompt design

The prompt receives the ranked list with score breakdowns and produces one-line rationales. Key prompt constraints:

- "You are narrating a pre-computed ranking. Do NOT suggest reordering."
- "Each rationale must reference the dominant scoring term (e.g., 'Past-due commitment to Sarah — urgency drove this to the top')."
- "Do NOT contradict the ranking. If an item is ranked #1, do not say it's low priority."
- Output format: JSON array of `{"item_id": "...", "rationale": "..."}` for reliable parsing.

### 3.3 Model routing

Per Decision H: route through the agent's default LLM provider/model. Cost is ~100 tokens per item — cheap enough for per-surface invocation. The explainer is a simple single-turn LLM call, not a multi-step agent loop.

### 3.4 Regression test: narration must not contradict ranking

A test that:
1. Creates a fixed ranked list with known #1 and #5 items.
2. Calls the explainer.
3. Asserts the rationale for #1 does NOT contain "low priority", "less important", etc.
4. Asserts the rationale for #5 does NOT contain "highest priority", "most urgent", etc.

This is a `MockLlmProvider` test — the mock returns a valid JSON response, and the test verifies parsing and contract enforcement. Real-provider variant gated behind `#[ignore]`.

### 3.5 Fallback

If the LLM call fails (timeout, rate limit, parse error), the surfaces degrade gracefully: they show the ranked list without rationales. The explainer returns `ExplainedItem` with an empty `rationale` string on failure, not an error.

## Phase 4 — Read Surfaces

### 4.1 CLI: `mika next` (`crates/mika-cli/src/commands/next.rs`)

New subcommand. Shows the top 5 operational items by priority with one-line rationale per item.

**Implementation:**
- Add `Next` variant to the CLI command enum in `cli.rs`.
- New `commands/next.rs` module.
- Query `operational_items` via the HTTP API (`GET /api/v1/operational-items?agent_id=<id>&sort=priority_desc&limit=5`).
- Call the explainer API endpoint (new: `GET /api/v1/operational-items/explained?agent_id=<id>&limit=5`) for rationales.
- Format: numbered list with kind emoji, title, status badge, and rationale.
- Supports `--format text|json` and `--agent <name>`.
- Feature-gated: returns "Operational partner mode is not enabled" when `MIKA_OPERATIONAL_PARTNER` is not set.

**Example output:**
```
What's Next:
1. [Task] Deploy v0.39 schema migration — urgency: past-due, drove to #1
   Status: Now | Due: 2026-06-10 | Priority: 185.3
2. [Commitment] Call Sarah re: partnership — commitment to user, approaching deadline
   Status: Scheduled | Due: 2026-06-12 | Priority: 142.1
3. [Blocker] Waiting on CI approval for mika#1200 — blocks 3 downstream items
   Status: Waiting | Priority: 98.5
...
```

### 4.2 Dashboard: Operational Inbox (`dashboard/`)

New dashboard page accessible via the sidebar.

**Implementation:**
- New route `/operational` in the dashboard router.
- New page component `OperationalInbox.tsx` consuming `GET /api/v1/operational-items`.
- Uses existing `@senara-solutions/ui` components: `StatusBadge` (map operational status to badge variant), `SelectFilter` (kind/status filters), `TimeRangeFilter`, `ListRow`, `Pagination`, `LoadingState`, `ErrorState`, `EmptyState`.
- Sortable by priority (default), created_at, due_at.
- Filterable by status (Now/Waiting/Delegated/Scheduled/AtRisk/Done) and kind.
- Each row shows: kind icon, title, status badge, owner, priority score, due_at, confidence.
- Priority column shows the score with a `TokenBudgetBar`-style visual (map 0-250 range to bar width).
- Live refresh via `LiveRefreshToggle` (reuses dashboard pattern).

**Server endpoint enhancement:**
- Extend `GET /api/v1/operational-items` to include `score_breakdown` in the response when `?include_breakdown=true` query param is set.
- New `GET /api/v1/operational-items/explained` endpoint for LLM-narrated rationales (lazy, on-demand — not computed on every list load).

### 4.3 Daily Brief Integration

The daily brief is a scheduled task that fires on the heartbeat/reminder cadence. Layer 2 adds operational items to it.

**Implementation:**
- In the heartbeat/daily-brief code path, query top 10 items by priority.
- Call the explainer for rationales.
- Format as a markdown section in the brief: `## What's Next` with the ranked list.
- Include an `## At Risk` section for any items with `status = AtRisk`.
- The brief is delivered via the existing `send_message` channel (Telegram, etc.).

**Location:** The daily brief logic lives in the heartbeat handler. Add a `build_operational_brief()` function in `operational/` that the heartbeat calls.

### 4.4 Agent Prompt Injection (`crates/mika-agent/src/prompt.rs`)

The load-bearing surface. Inject a `## Current Workload` section into the system prompt so every turn, the agent has the user's operational context.

**Injection point:** After core memory section (line ~495 in `prompt.rs`), before callback context.

**Implementation:**
- Add `operational_items: Option<Vec<OperationalItem>>` to `PromptContext`.
- In `build_system_prompt()`, after `write_core_memory_section()`, add:

```rust
if let Some(items) = ctx.operational_items {
    if !items.is_empty() {
        write_operational_section(&mut prompt, items);
    }
}
```

- `write_operational_section()` formats top 20 items (by pre-computed priority) as a concise list:

```
## Current Workload
<operational-context trust="data">
20 active items. Top priorities:
- [Now] Deploy v0.39 migration (priority: 185.3, due: 2026-06-10)
- [Scheduled] Call Sarah re: partnership (priority: 142.1, due: 2026-06-12)
- [Waiting] CI approval for mika#1200 (priority: 98.5, blocked)
- [AtRisk] Helm chart update stale 3d (priority: 87.2)
...
3 items at risk. 2 items waiting.
</operational-context>
```

- Feature-gated behind `settings.operational_partner`. When disabled, no injection.
- The items are queried from the DB at prompt-assembly time (same pattern as core memory).
- **No LLM explainer in prompt injection.** Per Decision H cost note: "Reassess if narrating ranking becomes a hot path (e.g., agent prompt injection at every turn)." The prompt surface shows raw scores and status — the agent can reference these to proactively surface context. Rationales are reserved for human-facing surfaces (CLI, daily brief).

**Caller changes:**
- `run_agent()` / `run_silent_agent()` / `run_team_agent()` — query operational items and pass to `PromptContext`. Gated by `settings.operational_partner`.
- For silent mode: include only top 5 items (smaller context budget).

## Phase 5 — Memory Rule Encoding

### 5.1 Feedback memory

Create `feedback_llm_explains_never_decides.md` in the memory system documenting the principle. This is a structural rule, not a preference.

### 5.2 Explainer prompt assertion

The explainer prompt must contain the literal string "do NOT suggest reordering" or equivalent. A `#[test]` asserts this invariant (same pattern as `summarization_prompt_enforces_factual_shape` in `compaction.rs`).

### 5.3 Regression test: ranking immutability

A test that calls `explain_ranking()` and verifies the returned `ExplainedItem` list preserves the input ordering exactly. The explainer must not reorder, insert, or remove items.

## Phase 6 — Wiring and Integration

### 6.1 Feature gate progression

The `MIKA_OPERATIONAL_PARTNER=1` flag gates all read surfaces. Writes remain always-on (Layer 1). The gate check happens at each surface's entry point:
- CLI: early return with guidance message.
- Dashboard: endpoint returns 404 (existing behavior).
- Agent prompt: section omitted.
- Daily brief: operational section omitted.

### 6.2 `AsyncDatabase` wrappers

Add `async fn score_and_rank_items(&self, agent_id: &str, limit: u32) -> Result<Vec<ScoredItem>>` to `AsyncDatabase` that:
1. Queries non-Done items for the agent.
2. Runs `derive_status()` on each, updating cache if changed.
3. Runs `priority()` scoring on each.
4. Sorts by priority DESC.
5. Returns top `limit` items with breakdowns.

This is the canonical read-path entry point for all surfaces.

### 6.3 HTTP API extensions

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `GET /api/v1/operational-items` | Existing | Add `include_breakdown=true` query param |
| `GET /api/v1/operational-items/explained` | New | Returns top-N items with LLM rationales |
| `GET /api/v1/operational-items/stats` | New | Summary counts by status + kind (for dashboard header) |

### 6.4 Documentation update

- Update `docs/architecture/operational-partner-frame.md` § Status taxonomy with implementation notes.
- Update `crates/mika-agent/CLAUDE.md` with the new modules and scoring details.
- Update `crates/mika-cli/CLAUDE.md` with the `mika next` command.

## Phase 7 — Testing Strategy

### 7.1 Unit tests (Phase 1-3)

- Scoring term isolation tests.
- Status derivation tests.
- Explainer prompt invariant test.
- Ranking immutability test.

### 7.2 Integration tests (`tests/eval/`)

- Full-stack eval scenario: synthetic items → score → rank → explain → verify output format.
- Agent prompt injection test: build prompt with operational items, assert `<operational-context>` block present.
- Feature gate test: with `operational_partner = false`, assert no operational section in prompt.

### 7.3 Dashboard tests

- API endpoint tests for new/extended endpoints.
- Component rendering tests for `OperationalInbox`.

## Implementation Order

1. **Phase 1** — Scoring engine (pure Rust, no LLM, no IO beyond DB reads). Foundation for everything else.
2. **Phase 2** — Status re-derivation. Integrates with Phase 1's read path.
3. **Phase 6.2** — `AsyncDatabase` wrapper. The canonical entry point.
4. **Phase 4.4** — Agent prompt injection. The load-bearing surface — proves the engine works end-to-end.
5. **Phase 4.1** — CLI `mika next`. Human-visible validation.
6. **Phase 3** — LLM explainer. Needed for CLI rationales and daily brief.
7. **Phase 4.3** — Daily brief integration. Uses explainer.
8. **Phase 4.2** — Dashboard inbox. Largest frontend surface, benefits from all prior work.
9. **Phase 5** — Memory rule encoding and regression tests.
10. **Phase 6.3-6.4** — API extensions and documentation.

## Decisions

- **No LLM in the ranking loop.** The scoring formula is the sole ranking authority. The LLM narrates after-the-fact.
- **Re-derivation at read time.** Status is a cache of derivable state. Every read re-derives and updates. This avoids complex event-driven status propagation.
- **Prompt injection without rationales.** The agent prompt surface shows raw scores, not LLM narration. Keeps the hot path cheap.
- **Graceful degradation.** All surfaces work without the explainer. Explainer failure = no rationales, not broken surfaces.
- **Score persistence.** The computed `priority` score is written back to the DB column. Dashboard sorting uses the cached score. The re-computation on read ensures freshness for the specific surface that triggered it.

## Open Questions

1. **Today window for Now/Scheduled threshold.** Foundation says "today window" without defining the exact hours. Propose 24 hours as the default, configurable via a constant in `calibration.rs`.
2. **AtRisk detection heuristics.** Beyond the stale+near-deadline case, what specific evidence patterns should trigger AtRisk? Propose starting with: (a) stale near deadline (updated_at > 48h ago + due_at within 24h), (b) callback failure on a Delegated item. Expand heuristics based on operational experience.
3. **Daily brief frequency.** Currently the heartbeat fires on a configurable cadence. The operational brief should fire once per day (morning). If the heartbeat cadence is more frequent, add a "last_brief_at" check to avoid spamming.
