# Decomposition Plan — mika#1259 sub-issue breakdown (Layer 3 domain refactor)

## Phase 0 — Pin

**A. Foundation doc §6** (`mika/docs/architecture/operational-partner-frame.md:165-184`) defines 9 target modules with explicit operational responsibilities + read/write ownership of `OperationalItem`:

| Module | Operational responsibility |
|---|---|
| `task_state/` | Task lifecycle: created → in_progress → blocked → done. Status transition rules. |
| `commitments/` | Promise tracking, follow-ups, due-date reminders |
| `planning/` | Plan-doc invariants, dispatch-readiness predicates, agent-loop policy |
| `agent_loop/` | Iteration itself: retrieve-context → build-prompt → LLM → match stop_reason → execute tools |
| `tool_execution/` | Tool dispatch, MCP integration, exec handlers, dispatch gates |
| `memory/` | Core memory, structured facts, search, KG bridges |
| `notifications/` | Outbound messages, webhook callbacks, channel-specific formatting |
| `evidence/` | Grounding-rule enforcement, fabrication-guard predicates, tool-call audit trail |
| `dashboard_queries/` | Read-side aggregation for dashboard surfaces |

**B. Current file sizes:**
- `crates/mika-agent/src/agent.rs` = 11,401 lines (grew from 10k cited in ticket)
- `crates/mika-agent/src/db.rs` = 17,645 lines (grew from 17k)
- Total: 29,046 lines targeted for decomposition.

**C. Existing module landscape** (`crates/mika-agent/src/`):
- **Already extracted, likely incorporates into Foundation §6 modules**: `task_engine/`, `skills/`, `tools/`, `teams/`, `operational/`, `db/`, `server/`, `messaging.rs`, `compaction.rs`, `post_condition.rs`, `prompt.rs`, `bundled_skills.rs`
- **Orthogonal infrastructure (likely stays outside Foundation §6)**: `bin/`, `calibration/`, `kg/`, `mcp/`, `a2a_card.rs`, `a2a_db.rs`, `async_db.rs`, `config_keys.rs`, `github_graphql.rs`, `panic_hook.rs`, `pricing.rs`, `rewind.rs`, `secret_scrubber.rs`, `startup.rs`, `task_metadata.rs`, `test_utils.rs`

**D. Sister-ticket sequencing — dependencies cleared:**
- mika#1262 (Layer 1 / Task Ledger) — CLOSED ✓
- mika#1261 (Foundation doc) — CLOSED ✓ (artifact at `mika/docs/architecture/operational-partner-frame.md`)
- mika#1253 (loader-engine parity assertion) — CLOSED ✓
- mika#1254 (fabrication guard predicate audit) — CLOSED ✓
- mika#1263 (Layer 2 / What's Next engine) — OPEN, p1 (lands BEFORE or AFTER #1259; not a hard sequencing dep per Foundation §6)

**E. Ticket ACs** (mika#1259):
- AC1: Plan adopts boundaries from Layer 1's foundation doc (§6 above); explain divergences.
- AC2: `cargo test -p mika-agent` passes unchanged after refactor.
- AC3: No behavior change. Pure module split; logic identical.
- AC4: Each new module has `mod.rs` with one-paragraph doc-comment naming operational responsibility.
- AC5: `agent.rs` and `db.rs` each drop below ~2k lines.
- AC6: mika#1253 lands AT or AFTER (per ticket — but #1253 already shipped; AC moot).
- AC7: mika#1254 lands AT or AFTER (per ticket — but #1254 already shipped; AC moot).

## Hypothesis (committed)

**Decomposition produces 9 sub-issues, one per Foundation §6 module.** Each sub-issue is a single-canvass-groomable unit of work scoped to:
- Create `crates/mika-agent/src/<module>/mod.rs` with operational-responsibility doc-comment (AC4)
- Move logic from `agent.rs` + `db.rs` + relevant existing subdirs into the new module
- Update `lib.rs` to declare the new module
- Verify `cargo test -p mika-agent` + `cargo clippy --tests --no-deps -- -D warnings` clean
- Verify behavior unchanged (no new tests; existing coverage is the regression)

Each sub-issue is independently dispatchable through the autonomous loop post-freeze. Within-PR scope: one module per PR. Cross-PR ordering matters because of import resolution (see §Sequencing below).

The 9 sub-issues, with rough scope estimates (lines-of-code moving):

| Sub-issue | Module | LoC estimate | Hard deps on other sub-issues |
|---|---|---|---|
| #1259-A | `evidence/` (fabrication guards, grounding, tool-call audit) | ~1,500 (mostly from agent.rs) | None — leaf module |
| #1259-B | `tool_execution/` (dispatch, MCP, exec handlers, loader-engine parity, gates) | ~3,500 (agent.rs + tools/ + bundled_skills.rs) | Depends on evidence/ for guard imports |
| #1259-C | `memory/` (core memory, structured facts, search, KG bridges) | ~1,500 (agent.rs + kg/) | None significant |
| #1259-D | `notifications/` (outbound messages, webhook callbacks) | ~1,000 (agent.rs + messaging.rs + server/) | None significant |
| #1259-E | `task_state/` (task lifecycle, status transitions) | ~2,500 (agent.rs + task_engine/ + db.rs) | None — but incorporates task_engine/ |
| #1259-F | `commitments/` (promise tracking, follow-ups, due-date reminders) | ~800 (db.rs primarily — light coverage today) | Depends on task_state/ (Commitment relates-to Task) |
| #1259-G | `planning/` (plan-doc invariants, dispatch-readiness, agent-loop policy) | ~1,500 (agent.rs primarily) | Depends on evidence/ + tool_execution/. **Coupling note (F2):** absorbs `is_groomed(body)` predicate from `auto_pull.rs` if mika#1363 ships first. |
| #1259-H | `agent_loop/` (iteration: retrieve-context → build-prompt → LLM → match → execute) | ~2,500 (agent.rs core loop) | Depends on planning/ + tool_execution/ |
| #1259-I | `dashboard_queries/` (read-side aggregation) | ~1,200 (db.rs read methods + server/dashboard*) | None — leaf module |

Total LoC moved: ~16,000 (lower-bound estimate). The 29k → 4k AC5 target requires ~25k LoC moved; the ~9k gap reflects:
- **Estimate conservatism**: per-module estimates are lower bounds. Actual extraction will likely reach 20-22k LoC moved as functions imported from modules like `task_engine/`, `tools/`, `bundled_skills.rs` get relocated under their new operational domain owners.
- **Some lines are shared utilities** (small helper functions used by 3+ modules) that consolidate into a single new module owner, not double-counted.
- **AC5's ~2k target is aspirational**, not the gate. Final residual measured after all 9 sub-issues ship — if residual is materially above ~2k each, the parent ticket gets a follow-up sub-issue (#1259-J or similar) for the rest. Decomposition-progress matters more than hitting exact line counts.

**The ~16k estimate is the plan-time baseline for sub-issue scoping; actual extraction may be larger. Sub-issue grooms will refine the per-module LoC estimates at canvass time.**

## Sequencing

**Leaf-first, dep-last.** Recommended order (lower-bound dependency satisfaction):

1. **#1259-A `evidence/`** — leaf module; many dependents need its guards
2. **#1259-I `dashboard_queries/`** — leaf module; read-only, no behavior coupling
3. **#1259-C `memory/`** — leaf module; reads OperationalItem, writes Evidence (via evidence/ once it exists)
4. **#1259-D `notifications/`** — leaf module; depends only on lib infrastructure
5. **#1259-E `task_state/`** — incorporates task_engine/; foundational for #1259-F
6. **#1259-F `commitments/`** — depends on task_state/
7. **#1259-B `tool_execution/`** — depends on evidence/; large extraction
8. **#1259-G `planning/`** — depends on evidence/ + tool_execution/
9. **#1259-H `agent_loop/`** — depends on planning/ + tool_execution/; core iteration

Each sub-issue groomed independently. Post-groom dispatch order can deviate based on operator priority, but plan-time order matters for the architecture review per sub-issue.

## Acceptance Criteria (for THIS decomposition meta-ticket)

1. **AC1:** 9 sub-issues filed on mika repo, each with title `refactor(mika#1259): extract <module>/ into its own module dir`.
2. **AC2:** Each sub-issue body cites Foundation doc §6, names the operational responsibility, and provides the LoC estimate + hard-dep list.
3. **AC3:** Each sub-issue tagged as sub-issue of mika#1259 via GraphQL `addSubIssue` mutation.
4. **AC4:** Sequencing recommendation (above) documented in the parent #1259 body.
5. **AC5:** Each sub-issue is single-canvass-groomable post-freeze (scope < 1 working day per ticket, with clear single-module focus).

## Files this decomposition meta-ticket changes

- This plan doc only (no code changes; that's the sub-issues' job)
- mika#1259 body update: append §Sub-issue breakdown with the 9 issue refs
- 9 new sub-issues filed on mika

## Out of scope (for THIS meta-ticket)

- Actually performing the refactor (each sub-issue does its slice)
- Layer 2 (#1263 What's Next engine) — separate ticket, parallel-not-blocking
- Closing #1259 itself — closure is on completion of all 9 sub-issues

## Risk

Medium-low.
- **Decomposition boundaries are doc-driven** (Foundation §6) — low ambiguity risk.
- **Sub-issue order may need adjustment** at implementation time if sequencing assumptions wrong. Each canvass at post-freeze groom-time can override.
- **Existing module overlap** (task_engine, skills, tools, kg, server) — some content migrates into Foundation §6 modules; others stay outside. Each sub-issue plan handles its own boundaries.

## Implementation order (for this decomposition meta-ticket)

1. Architect canvass on this decomposition plan
2. If GROOMED, file the 9 sub-issues via `gh issue create` + GraphQL `addSubIssue` for parent linkage
3. Update mika#1259 body with the §Sub-issue breakdown
4. Surface URLs to Mika Prime for bearing-check before next milestone (#1247)
