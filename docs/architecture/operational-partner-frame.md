# Operational Partner Foundation

Tracking ticket: [mika#1261](https://github.com/senara-solutions/mika/issues/1261). Project: [Operational partner foundation](https://github.com/orgs/senara-solutions/projects/2). Companion: [`mika/os/FOUNDATION.md`](../../os/FOUNDATION.md).

---

## 1. Frame

Today's Mika is an **assistant**: it waits for instructions, decides what tool to call, returns a response. The architecture says *"agent loop + tools."*

Target Mika is an **operational partner**: it maintains the user's structured operational state, detects what changed, ranks what matters, and surfaces the next action. The architecture says *"operational state machine + assistant interface."*

This document captures the model that makes the second architecture possible. Three layers derive from it:

- **Layer 1 — Task Ledger** ([mika#1262](https://github.com/senara-solutions/mika/issues/1262)) — canonical `OperationalItem` schema + write paths from every operationally-relevant subsystem.
- **Layer 2 — What's Next engine** ([mika#1263](https://github.com/senara-solutions/mika/issues/1263)) — deterministic scoring + LLM-as-explainer for ranking the ledger.
- **Layer 3 — Domain refactor** ([mika#1259](https://github.com/senara-solutions/mika/issues/1259)) — split `crates/mika-agent/src/agent.rs` (10k lines) + `crates/mika-agent/src/db.rs` (17k lines) into modules whose boundaries are derived from the operational model in §6 below.

**Pattern:** if Mika learns something operationally relevant, it MUST become structured state. Conversation history is not the operational truth.

---

## 2. The seven canonical types

Every operationally relevant fact in Mika's world is one of these seven kinds. Each has a row in the `operational_items` table (§3); the `kind` enum discriminates.

| Kind | Definition | Examples |
|---|---|---|
| **Goal** | What the user wants to accomplish over a long horizon | "Ship Mika Cloud v0", "Lose 10 pounds before summer", "Hire a senior eng by Q3" |
| **Task** | A concrete unit of work with a clear done-state | "Implement the foundation doc", "Reply to Aisha's email", "Run `make deploy` after #1259 lands" |
| **Commitment** | Something promised — by user, by Mika, or by a third party to either | "I told mom I'd call her Saturday", "Mika committed to summarize the meeting notes by EOD", "Sarah committed to send the contract draft Tuesday" |
| **Decision** | Something waiting on user judgment, with options + trade-offs | "Stamp markers on FOUNDATION.md", "Hire Aisha or keep interviewing", "Run Cloud deploy in eu-west-3 or eu-central-1" |
| **Blocker** | A named reason progress is stopped on something else | "Waiting on Sarah's contract draft", "AWS account verification pending", "Need decision on -march= before Layer 3 starts" |
| **Evidence** | Knowledge that supports the current state of another item | "PR#1264 merged, confirms hygiene docs landed", "Last meeting with Aisha had positive tone", "Customer X churn risk per support ticket #4521" |
| **NextAction** | The one thing that should happen next for an upstream item | "Stamp marker H (-march=) on FOUNDATION.md", "Reply to Aisha's email by 2pm", "Run `make deploy` after mika#1259 lands" |

Each item has exactly one kind. Relationships between items (a Blocker blocks a Task; an Evidence supports a Commitment; a NextAction belongs to a Goal) live in the `evidence_refs`, `blocked_by`, and `next_action` fields of `OperationalItem` (§3).

---

## 3. The `OperationalItem` unified surface

```rust
pub struct OperationalItem {
    pub id: String,                       // UUID
    pub kind: OperationalKind,            // discriminant — the 7 types
    pub title: String,                    // short human-readable
    pub status: OperationalStatus,        // the 6-status taxonomy (§4)
    pub owner: Owner,
    pub priority: f32,                    // computed by What's Next (§5), cached here
    pub user_importance: f32,             // 0.0–50.0 — operator-stamped or inferred per §5; default 0.0
    pub due_at: Option<DateTime<Utc>>,
    pub blocked_by: Option<String>,       // OperationalItem.id reference
    pub next_action: Option<String>,      // OperationalItem.id reference (NextAction kind)
    pub evidence_refs: Vec<EvidenceRef>,
    pub confidence: f32,                  // 0.0–1.0 — Mika's grounding strength
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub enum OperationalKind { Goal, Task, Commitment, Decision, Blocker, Evidence, NextAction }

pub enum OperationalStatus { Now, Waiting, Delegated, Scheduled, AtRisk, Done }

pub enum Owner { User, Mika, Person(String), Agent(String) }

pub struct EvidenceRef {
    pub kind: EvidenceRefKind,
    pub id: String,
}

pub enum EvidenceRefKind {
    OperationalItem,  // another row in this table
    Message,          // mika.messages.id
    ToolCall,         // mika.tool_calls.id
    GithubIssue,      // owner/repo#N
    GithubPr,         // owner/repo#N
    File,             // path:line
    External,         // free-form URL/identifier
}
```

> **DECISION — E (`Owner` enum granularity):** Ship the four variants as defined above (`User / Mika / Person(String) / Agent(String)`). `String` payloads carry name; semantic categorization (role, agent kind) lands as needed via a separate `Owner::category()` derivation rather than enum-payload expansion. This avoids the failure mode where every new owner question becomes an enum migration.

### Augmentation, not replacement

The `operational_items` table **augments** existing tables — it does not replace them. Existing tables stay source-of-truth for their domain:

- `mika.messages` keeps conversation history
- `mika.tasks` keeps the scheduler's view (per-agent dispatch queue)
- `mika.tool_calls`, `mika.llm_calls` keep telemetry
- `mika.sessions` keeps session boundaries
- GitHub issues stay GitHub issues
- KG entities stay KG entities

`OperationalItem` is the **operational query layer**. Reads go here; writes go to the source-of-truth table first, then to `OperationalItem` via write paths (§7). This is the architectural escape from the "join across 5 tables to ask what's happening" pattern that the mika#1251 investigation made painful.

> **DECISION — A (schema migration path):** Start the `operational_items` table fresh from this PR forward. The operational model is forward-looking; backfilling 6+ months of conversation history into Goal/Task/Commitment categories is a research project, not a migration. New writes populate from day 1. An optional backfill job lands as a separate ticket if it turns out to be valuable.

---

## 4. Status taxonomy

Every `OperationalItem` has exactly one status from this set:

| Status | Meaning | Derivable from |
|---|---|---|
| **Now** | Needs attention today (or sooner) | `due_at <= now() + today_window` AND `blocked_by IS NULL` |
| **Waiting** | Blocked on someone or something | `blocked_by IS NOT NULL` |
| **Delegated** | Mika or another agent is working on it | `owner = Mika OR Agent(_)` AND active task exists |
| **Scheduled** | Not relevant until later | `due_at > now() + today_window` |
| **AtRisk** | Deadline approaching, silence detected, failed callback, stale | Computed by What's Next (§5) |
| **Done** | Closed with evidence | terminal state |

Status is **derivable, not authoritative** — with one exception: **Done is terminal.** Once an item is written as Done, it stays Done until explicitly re-opened by an operator or a Mika action with an audit-trail Evidence link. Derivation rules do not reopen closed items. Every other status is re-derived on every read by the What's Next engine (§5); if the derived status differs from the cached `status` field, the cache is updated.

**Terminal-state alignment with source-of-truth tables.** When an `OperationalItem` is linked to a source-of-truth row (`mika.tasks`, GitHub Issue/PR, etc.), its `Done` state is slaved to the source's terminal state. If the source has a terminal-state lock (e.g., `mika.tasks.status` cannot revert from `completed` per the schema constraint), the `OperationalItem.Done` lock is defended by the source's lock and inherits it transitively. If the source allows un-closure (e.g., GitHub Issue reopened), an explicit `OperationalItem` re-open write — with an Evidence row pointing at the source's reopen event — is required to move it back out of Done. Layer 1 implementation enforces this alignment per write path.

This means status transitions don't need explicit writes from subsystems — they happen automatically when underlying conditions change. For example: when a Blocker item closes (its own status moves to Done), every Waiting item whose `blocked_by` field pointed to that Blocker re-derives to Now; when an item's `due_at` enters the today window, Scheduled re-derives to Now; when a callback fails on a Delegated item, the failure signal re-derives it to AtRisk.

> **DECISION — G (status derivation precedence):** AtRisk overrides all **non-terminal** statuses. The riskiest state must surface even if other conditions would otherwise hide it. **Done is terminal and excluded from re-derivation** — a Done item whose `due_at` has passed stays Done; closing an item is an explicit write, not a derivation outcome. Precedence order over non-terminal statuses: `AtRisk > Now > Waiting > Delegated > Scheduled`. `Done` is outside this order entirely.

---

## 5. The What's Next scoring formula

Priority is **deterministic first, narrated second**.

```
priority = urgency
         + commitment_weight
         + user_importance
         + stale_time
         + dependency_risk
         - confidence_penalty
```

Each term defined:

| Term | Range | Computation |
|---|---|---|
| `urgency` | 0.0 — 100.0 | Three cases: `0.0` if `due_at = None` (no time pressure); `100.0` if `hours_until_due_at <= 0.0` (past-due, clamped at max); otherwise `100.0 / (hours_until_due_at + 1.0)`. |
| `commitment_weight` | 0.0 — 50.0 | `kind = Commitment` adds weight by owner: User-promise > Mika-promise > Third-party-promise |
| `user_importance` | 0.0 — 50.0 | Operator-stamped or inferred from prior signals (mentions, response speed). Persisted per-item; default 0 |
| `stale_time` | 0.0 — 30.0 | `log(hours_since_last_touch + 1.0) * 5.0`. Old untouched items rise gradually |
| `dependency_risk` | 0.0 — 40.0 | If this item has > 0 items where `blocked_by = this.id`, add `count_blocked * 10.0` (capped at 40.0) |
| `confidence_penalty` | 0.0 — 50.0 | `(1.0 - item.confidence) * 50.0`. Low-confidence items get **down**-ranked |

Resulting `priority` typically lands 0–250. No normalization needed — relative ordering is what matters.

### Why deterministic-first

Today's Mika ranks via LLM judgment in conversation context. That's high variance, hard to debug, and prone to recency bias (the last topic mentioned wins). The deterministic formula produces a stable ordering with **explainable contributions per term**. The LLM's role is downstream: explain the ranking, refine the wording, handle the edge cases the formula misses — but never decide the ranking.

This is the abstract version of the fabrication-guard concerns documented in [mika#1254](https://github.com/senara-solutions/mika/issues/1254). The `confidence_penalty` term is the explicit hook: if Mika doesn't have grounding for an item, that item ranks lower, and the LLM's narration of its rank is honest about the gap.

> **DECISION — B (confidence units):** Continuous `f32` in [0.0, 1.0]. Buckets are easier for humans to assign but lose information when arithmetic happens (the penalty multiplication needs a real number). Mika derives buckets for display; underlying storage stays continuous.

> **DECISION — C (term weight calibration):** The constants above (50.0 commitment cap, 30.0 stale cap, etc.) ship as named constants in `crates/mika-agent/src/operational/calibration.rs` so adjustment is one PR, not a refactor. Real weights come from operator stamping over weeks. Calibration is an ongoing operational concern, not a v1 gate.

> **DECISION — H (LLM-explainer model + cost budget):** The narration step is per-item, ~50 input + 50 output tokens. Route through `mika-dev`'s default model. At ~$0.50 per million tokens (Sonnet-class) and ~100 tokens per item, a daily brief with 10 items costs ~$0.0005 — cheap. Reassess if narrating ranking becomes a hot path (e.g., agent prompt injection at every turn).

---

## 6. Domain boundaries (for Layer 3 refactor)

The mega-files refactor at [mika#1259](https://github.com/senara-solutions/mika/issues/1259) splits `agent.rs` (10k) and `db.rs` (17k) into modules whose responsibilities map 1:1 to operational concerns. Each module owns its slice of `OperationalItem` reads and writes:

| Module | Operational responsibility | Owns reads/writes for |
|---|---|---|
| `task_state/` | Task lifecycle: created → in_progress → blocked → done. Status transition rules. | `kind = Task`, `status` field transitions |
| `commitments/` | Promise tracking, follow-ups, due-date reminders | `kind = Commitment` |
| `planning/` | Plan-doc invariants, dispatch-readiness predicates, agent-loop policy | Cross-cutting reads; no exclusive writes |
| `agent_loop/` | The iteration itself: retrieve-context → build-prompt → LLM → match stop_reason → execute tools | Writes `kind = Evidence` on tool execution |
| `tool_execution/` | Tool dispatch, MCP integration, exec handlers, dispatch gates | Writes `kind = Evidence` on each tool call; owns the loader-engine parity check from [mika#1253](https://github.com/senara-solutions/mika/issues/1253) |
| `memory/` | Core memory, structured facts, search, KG bridges | Reads `OperationalItem` to inform retrieval; writes Evidence on persistence ops |
| `notifications/` | Outbound messages, webhook callbacks, channel-specific formatting | Reads What's Next ranking to decide what to surface |
| `evidence/` | Grounding-rule enforcement, fabrication-guard predicates, tool-call audit trail | Owns the predicates from [mika#1254](https://github.com/senara-solutions/mika/issues/1254); writes `kind = Evidence` with provenance |
| `dashboard_queries/` | Read-side aggregation for dashboard surfaces | Reads only; `OperationalItem` is the canonical join target |

Layer 3 lands **after** Layer 1 — the boundary definitions are derived from the kinds + statuses + scoring above, not invented during refactor.

---

## 7. Write paths inventory

For every operationally-relevant subsystem, the rule is: source-of-truth table first, then `OperationalItem` write.

| Subsystem | Source-of-truth write | `OperationalItem` write |
|---|---|---|
| User chat message expressing short-horizon intent | `mika.messages` | Create `kind = Task` with title from user text, owner = User, status = Now |
| User chat message expressing long-horizon intent ("I want to ship Mika Cloud") | `mika.messages` | Create `kind = Goal`, owner = User, status = Now (or Scheduled if a deadline is named). Distinguished from Task by classifier prompt: time horizon > weeks AND no concrete done-state |
| Reminder set by user | `mika.tasks` (scheduler) | Create `kind = Task` with `due_at`, owner = User, status = Scheduled |
| User chat message naming a promise ("I told mom I'd call her Saturday") | `mika.messages` | Create `kind = Commitment`, owner = User, `due_at` = parsed date if any, status = Scheduled/Now per due_at |
| Mika makes a promise in a turn ("I'll have the summary by EOD") | `mika.messages` | Create `kind = Commitment`, owner = Mika, status = Scheduled/Now per due_at |
| User chat message describing a choice they need to make | `mika.messages` | Create `kind = Decision`, owner = User, status = Now |
| mika-arch surfaces a DECISION NEEDED marker during grooming | `mika.tool_calls` | Create `kind = Decision`, owner = User, status = Now, evidence_refs ← arch session ID |
| Mika detects mid-turn that an item can't proceed without X | `mika.messages` (assistant turn) | Create `kind = Blocker` with title naming the dependency, status = Waiting; update the blocked item's `blocked_by` field |
| What's Next engine emits a next-action recommendation for a top-N item | (read-only computation) | Create `kind = NextAction` (ephemeral, may be replaced on next computation), owner = whoever should act, status = Now, parent item's `next_action` field updated |
| GitHub webhook: issue labeled `ready` | (event source, no write) | Create `kind = Task`, owner = Mika or named agent, status = Now |
| GitHub webhook: PR review submitted | `mika.messages` | Update `OperationalItem.status` (Now → Waiting on operator response) |
| GitHub webhook: CI status | (no write) | Update item linked to PR: status = AtRisk on red, Now on green |
| Skill: auto-groom dispatched | `mika.tasks` (sub-task) | Update parent `OperationalItem`: status = Delegated, owner = mika-dev |
| Skill: dev-pilot completed | `mika.tasks` (status update) | Update `OperationalItem`: status = Done if PR merged + CI green; status = AtRisk for any non-clean-Done outcome (PR opened-but-blocked, PR with red CI, exited-without-PR, pipeline_incomplete). Layer 1 implementation enumerates the full failure-mode matrix; the foundation summarizes |
| Team run started | `mika.tasks` | `OperationalItem.owner` = team, status = Delegated |
| Callback (claude-pilot completion) | `mika.tasks` | Status transition per callback result |
| Daily brief generated | (read-only) | Reads top `OperationalItem`s by `priority` |
| Dashboard surfaces | (read-only) | Reads filtered items by status + priority |
| Calendar/email integration (future) | external | Writes Commitments on incoming meetings/emails |

> **DECISION — D (write atomicity):** Single-transaction write of source-of-truth + `OperationalItem`. Strong consistency, blocks on DB. The async-eventual model is an optimization for later if write latency becomes a problem; correctness is the v1 priority. [mika#1258](https://github.com/senara-solutions/mika/issues/1258) (async_db backpressure) is relevant here — if DB-as-actor lands as #1258's fix, transactional writes route through the actor cleanly. **Sequencing implication:** Layer 1 ([mika#1262](https://github.com/senara-solutions/mika/issues/1262)) and mika#1258 should land together. The Layer 1 ticket's plan must address whether to depend on mika#1258 landing first, or to land Layer 1 with today's blocking sync_channel and migrate the write path when #1258 ships DB-as-actor.

> **DECISION — F (`EvidenceRef` kinds — closed enum vs extensible):** Closed enum. Intentional friction is the feature — every new evidence kind is a deliberate decision requiring migration + code change. Avoids the structured-data-becomes-string-bag failure mode.

---

## 8. Read surfaces

Five surfaces consume the `OperationalItem` ranking:

| Surface | Query | Output |
|---|---|---|
| CLI: `mika next` | top 5 by priority | Plain-text list with one-line rationale per item (LLM narration) |
| CLI: `mika status` | filter by status, group | Bucketed view: Now / Waiting / Delegated / Scheduled / AtRisk / Done |
| Dashboard: Operational inbox | filter + sort with UI controls | Web view with editable fields (priority override, status flip, blocked_by link) |
| Daily brief | top 10 by priority + AtRisk summary | Markdown brief delivered via configured channel (Telegram, email) |
| Agent prompt | top 20 by priority, scoped to current conversation | Injected into system prompt at turn start — Mika knows the user's current operational load when responding |

The **agent-prompt surface is the load-bearing one**: it's how Mika BECOMES an operational partner instead of a stateless responder. Today's agent prompt injects core memory + active skills + retrieved context. Adding the operational ranking means every turn, Mika has the user's current world in view — and can proactively surface "by the way, you have 3 At-Risk items" without being asked.

### Identity transition

This surface is what shifts Mika from "wait for the user to tell me what to do" to "know what the user is in the middle of, and surface what changed." That's the difference between assistant and operational partner stated concretely.

> **DECISION — I (Layer 1 feature flag):** Layer 1 lands behind `MIKA_OPERATIONAL_PARTNER=1` for gradual rollout. Writes happen on every operationally-relevant subsystem; reads are gated by the flag until Layer 2 is ready. Each of the 5 read surfaces enables progressively (CLI first, dashboard second, agent prompt last). Flag removed when all surfaces are live and stable.

---

## 9. Settled decisions — index

| ID | Decision | Section | Settled choice |
|---|---|---|---|
| **A** | Schema migration path | §3 | Fresh start from this PR forward; optional backfill as a separate ticket |
| **B** | Confidence units | §5 | Continuous `f32` in [0.0, 1.0] |
| **C** | Term weight calibration | §5 | Named constants in `crates/mika-agent/src/operational/calibration.rs`; ongoing operational concern, not a v1 gate |
| **D** | Write atomicity | §7 | Single transaction in v1; Layer 1 + mika#1258 sequence together |
| **E** | `Owner` enum granularity | §3 | `User / Mika / Person(String) / Agent(String)`; semantic categorization via separate `Owner::category()` derivation |
| **F** | `EvidenceRef` kinds | §7 | Closed enum; new kinds require migration |
| **G** | Status derivation precedence | §4 | AtRisk overrides all non-terminal statuses; Done is terminal, excluded from re-derivation |
| **H** | LLM-explainer model + cost budget | §5 | `mika-dev` default model; ~$0.0005 per daily brief |
| **I** | Layer 1 feature flag | §8 | `MIKA_OPERATIONAL_PARTNER=1`; reads enable progressively per surface |

All nine settled 2026-05-24 by operator. Decisions are revisable via a follow-up rev to this doc; until then, downstream implementations treat them as load-bearing.

---

## 10. Out of scope / deferred

- **Sub-tasks and task hierarchies** — `OperationalItem` is flat; relationships expressed via `blocked_by`, `next_action`, `evidence_refs`. Deep hierarchies (task trees) are out of v1 scope. Complex sub-structure lives in the GitHub Issue layer or external project tools; OperationalItem captures the top-level operational surface.
- **Cross-user/team operational state** — v1 is single-user (the operator's view). Multi-user shared operational state lands in v2 if/when Mika becomes a team tool.
- **Time-machine / undo** — out of scope for the foundation. Eventual integration with [mika#1116](https://github.com/senara-solutions/mika/issues/1116) (btrfs time-machine, currently paused) could surface OperationalItem snapshots, but not a v1 concern.
- **Calendar/email integration** — write paths are sketched in §7 but the integrations themselves are post-Layer-1.
- **Voice / mobile surfaces** — read surfaces are CLI + Dashboard + Daily Brief + Agent Prompt. Voice and mobile come later.

---

## 11. Verification

This doc is the contract for Layer 1 ([mika#1262](https://github.com/senara-solutions/mika/issues/1262)), Layer 2 ([mika#1263](https://github.com/senara-solutions/mika/issues/1263)), and Layer 3 ([mika#1259](https://github.com/senara-solutions/mika/issues/1259)). Each implementation ticket's plan must cite this doc and demonstrate fidelity to the model:

1. **Layer 1 plan** must show how the `operational_items` table maps to the schema in §3, how the write paths in §7 are implemented per subsystem, and how the augmentation-not-replacement rule (§3) is enforced at the write layer.
2. **Layer 2 plan** must show the scoring formula in §5 implemented term-by-term with the three-case `urgency` handling and the `confidence_penalty` down-ranking, and the LLM-explainer running downstream of (never deciding) the ranking.
3. **Layer 3 plan** must show the domain boundaries in §6 mapped to the post-refactor module structure, with the loader-engine parity check ([mika#1253](https://github.com/senara-solutions/mika/issues/1253)) landing in `tool_execution/` and the fabrication-guard predicates ([mika#1254](https://github.com/senara-solutions/mika/issues/1254)) landing in `evidence/`.

Subsequent revisions to this doc follow the rhythm established for [`mika/os/FOUNDATION.md`](../../os/FOUNDATION.md): editor pass for clarity, architect second-review for technical soundness, operator stamp for decisions. The changelog records each pass.

---

## Changelog

- **2026-05-24 (rev 5 — committed)** — mika-arch second-pass review emitted `Verdict: GROOMED` (session `67d79e6b`) with five non-blocking findings (F1–F5). Two folded into the doc as polish: **F1** — added `user_importance: f32` field to `OperationalItem` struct in §3 (the term was referenced in the §5 scoring formula but missing from the schema); **F2** — added a "Terminal-state alignment with source-of-truth tables" paragraph in §4 making explicit that `OperationalItem.Done` is slaved to the source's terminal state (the alignment is defended by existing schema constraints but was previously unstated). **F3 / F4 / F5** are downstream-implementation concerns flagged for Layer 1 / Layer 2 / Layer 3 plans (agent-prompt token budget, narrate-once-vs-per-surface cost model, mika#1253/#1254 landing-order relative to Layer 3). Doc committed to `mika/docs/architecture/operational-partner-frame.md`.
- **2026-05-24 (rev 4 — operator-stamped)** — All nine decisions (A–I) settled by operator with default proposals accepted. Doc hermeticized: removed editor-pass-summary preamble, replaced every `DECISION NEEDED` marker with `DECISION` blockquote stating the settled choice, restructured §9 from a default-proposal index to a settled-choice index, replaced §11 verification-gate list with a contract-for-downstream-layers list. Length reduced from 347 lines (rev 3) to ~270 lines. Doc is the canonical reference for Layer 1, Layer 2, and Layer 3 implementation planning.
- **2026-05-24 (rev 3)** — CC resolved all 5 of samidarko's editor flags from rev 2. (1) inline `DECISION NEEDED — E` blockquote added in §3 matching the §9 index entry; (2) §4 declared Done terminal, marker G updated to exclude Done from re-derivation; (3) §5 `urgency` formula restated as three explicit cases (None / past-due / normal); (4) §7 write-paths table extended with 6 new rows covering Goal, Commitment (user + Mika), Decision (user + arch), Blocker, NextAction; (5) §7 dev-pilot row expanded inline to acknowledge AtRisk as catch-all for non-clean-Done outcomes.
- **2026-05-24 (rev 2)** — Editor pass by samidarko-Claude. Two inline polish edits (§3 temporal anchor removal; §4 Blocker / `blocked_by` clarification). Five substantive flags surfaced for CC's resolution. Cross-references, internal consistency (7 kinds, 6 statuses), markdown formatting verified clean.
- **2026-05-24 (rev 1)** — Initial draft by CC (orchestrator-Claude) per [mika#1261](https://github.com/senara-solutions/mika/issues/1261). Ten sections derived from operator's 2026-05-24 strategic frame + grounding in `agent.rs`/`db.rs` internals from the same day's mika#1251/#1255/#1257 cascade investigation. Nine `DECISION NEEDED` markers surfaced with default proposals.
