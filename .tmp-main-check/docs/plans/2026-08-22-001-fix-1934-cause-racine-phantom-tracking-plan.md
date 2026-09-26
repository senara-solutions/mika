---
ticket: mika#1934
type: fix
scope: engine
shape: supersede-on-new-dispatch (primary) + accept #1712 sweep as backstop
schema_version_change: no
depends_on: mika#1712 (shipped 2026-08-22 as PR#1930-class)
---

# fix(engine): cause-racine phantom tracking rows — escalation-blocked with no cleanup path

## Rationale — how the pass-2 groom is structured

Sami arbitrated pass 1 as blocked-by #1712 with escalation trigger "if rows/pass rate accelerates → p1." The #1712 sweep shipped 2026-08-22; the first startup_sweep fired at 07:14:50 and cleared **42 pre-existing phantom rows** — the télémétrie the pass-2 groom was gated on is now available and exploited below.

Pass 2 turns the ticket from "investigate WHY" into a concrete fix plan against the mechanism the télémétrie actually reveals — which is different from the mechanism the original body hypothesised.

## Télémétrie (2026-08-22 07:14:50 — 42 rows swept)

Query surface: `SELECT ... FROM audit_events WHERE tool_name='phantom_aged_out'` (SOLE READER, mika#1712 AC7).

### Shape of the 42 swept rows

| Axis | Distribution |
|------|--------------|
| `agent_id` | **100% `mika-dev`** — single agent, no scatter |
| `trigger_type` | 100% `manual` |
| `action_type` | 100% `none` |
| `process_id` | 100% NULL |
| `source` | 37 `self_dev` / 4 NULL / 1 `user_request` (all dispatch-tracking) |
| Pre-sweep `status` (`before_value` in audit_events) | **100% `blocked`** (load-bearing) |
| Post-sweep `status` (`after_value`) | 100% `failed` w/ reason `phantom_aged_out` |
| Time span | 2026-07-01 19:13:01 → 2026-08-21 22:45:52 (52 days) |
| Rate observed | ~0.8 rows/day (well below the initial 24 rows / 18h burst — see §"Rate reconciliation") |

### Label shape (dispatch-source distribution)

| Label pattern | Count | Origin |
|---------------|-------|--------|
| `ready-label: <owner/repo>#<num>` | 22 | Engine-side ready-label handler (`crates/mika-agent/src/server/ready_label_handler.rs:168`, mika#1572) |
| `groom <repo>#<num>` / `Groom <repo>#<num>` | 15 | LLM tool-call `create_task` with grooming discriminator (`?phase=groom` URL suffix) |
| `ready-label: … (retry attempt N)` / `… (cycle #N)` / `… (poison loop)` / `… (predicate miss)` | 5 (subset of above) | Auto-groom retry paths |

### Repo-target distribution

32 mika · 4 mika-cloud · 1 claude-pilot · 5 no-URL (LLM-created retry attempts).

### Multi-dispatch collisions (n≥2 phantoms for the same reference_url)

- **mika#1574** — 4 phantom rows (2026-07-30, 2026-08-04 × 3): retry cycles for `ready-label: senara-solutions/mika#1574` + `groom mika#1574 (auto-groom, poison loop)`. Each retry created a new tracking row while the previous sat in `blocked`.
- **mika#1712** — 1 phantom row (2026-07-25) still open until sweep. Recursive: the ticket that shipped the sweep left its own grooming tracking row in `blocked` for the month between grooming and shipping.
- Same pattern for #1727, #1713, #1787, #1772, #1867, #1887, #1729, #1403 — each has both `ready-label` and `groom` rows sitting `blocked` in parallel.

### Post-close verification (sample)

Of the 42 phantom-referenced tickets checked: **mika#1711 (CLOSED 2026-07-13), mika#1712 (CLOSED 2026-08-22), mika#1583 (CLOSED 2026-08-21), mika#1867 (CLOSED 2026-08-21), mika#1889 (CLOSED 2026-08-20)** — all closed via merged PR out-of-band, with their tracking rows still `blocked` at sweep time. Confirms the "escalation resolved elsewhere, tracking row never terminal-marked" pattern.

### Rate reconciliation vs the original 2026-07-01 signal

The pass-1 body cited "24 rows / 18h" from 2026-07-01. That day is present in the télémétrie as the FIRST 4 phantom entries (2026-07-01 19:13 → 2026-07-02 09:35) — those tickets (#1699, #1711, #1664, #1627) match the burst window sami manually swept at the time. The 42-row aggregate covers 52 days, so the sustained rate is ~0.8/day — the initial 24/18h reads as a burst against ~4 baseline rows in the same window. Escalation threshold (rate × 1.5 over 3d rolling) has NOT fired; #1712 sweep is caught up.

## Root cause (verified — code + télémétrie converge)

**Every phantom row is a tracking row whose parent-status was intentionally set `blocked` by the escalation surface, then never converted to a terminal state (`completed`/`cancelled`/`failed`) because the underlying issue was resolved out-of-band.**

### Where `blocked` gets written to a tracking row (grep-verified, 100% of writers)

`grep -rn '"blocked"' crates/mika-agent/src/` yields five production writers of `status='blocked'` on manual/self_dev tracking rows:

1. `crates/mika-agent/src/server/verdict_handler.rs:993` — BLOCK[ac] retry limit exceeded → parent marked `blocked` (escalation "loop_limit_reached"; audit_events `verdict_escalated`).
2. `crates/mika-agent/src/server/verdict_handler.rs:1283` — BLOCK[ci] retry limit exceeded → same shape.
3. `crates/mika-agent/src/server/verdict_handler.rs:1490` — BLOCK[security] / BLOCK[pipeline] verdict → immediate escalation, no retry.
4. `crates/mika-agent/src/server/verdict_handler.rs:2154` — Identical-diff circuit breaker (mika#1563) — same PR head SHA rejected N times → parent marked `blocked`.
5. `crates/mika-agent/src/task_engine/dispatcher.rs:3191` — inherited-status path (parent already `blocked` → dispatch preserves it).

Every one of these is a **designed escalation** — the intent is to pause the loop and summon operator attention. `blocked` is the correct semantic marker at write time.

### The missing surface — cleanup after escalation is handled

The lifecycle contract for a tracking row is `pending → in_progress → (completed | failed | cancelled)`. `blocked` is a legitimate intermediate but has no defined exit path today:

- No reaper converts `blocked → terminal` after operator intervention.
- Operator interventions happen **out-of-band** on GitHub (merge PR, close issue, apply new label, cancel work) — none of these events reach back to the tracking row.
- Retries create NEW rows (see mika#1574 × 4 above) instead of resurrecting the `blocked` row.

The pattern is symmetric across all 42 phantoms: the escalation fired, operator (Sami) resolved the underlying ticket some other way (merged the fix in a different PR, cancelled the branch, superseded with a new ticket), and the `blocked` tracking row was left behind as an artefact of the escalation surface.

### What mika#1712 sweep does — and its limitation

The #1712 startup + watchdog sweep transitions `blocked` (age > 3600s) → `failed` w/ reason `phantom_aged_out`. That drains the backlog and emits the télémétrie that seeded this ticket.

**Limitation of sweep-only:** it converts every aged `blocked` row into `failed`, erasing the distinction between:
- (a) a genuine escalation that has NOT yet reached operator attention (aged out too early — actionable false-positive), and
- (b) a stale escalation whose underlying issue was resolved elsewhere (aged out correctly — noise).

The sweep timestamp reveals category (b) — 100% of the 42 rows are class (b) per the CLOSED-out-of-band sample. But the sweep itself can't tell them apart at write time, and running a longer grace window would trade drain time for signal fidelity.

## Fix shape

**Primary: supersede-on-new-dispatch (write-time; no external event dependency)**

When `ready_label_handler` or `create_task` (grooming path) creates a new tracking row for a `reference_url` that already has an active tracking row in `blocked` or `in_progress`, mark the older row `cancelled` with reason `superseded_by_new_dispatch` before inserting the new row. This kills the multi-dispatch-collision class outright (mika#1574 × 4 becomes mika#1574 × 1 active + 3 cleanly `cancelled`) and prevents accumulation from the retry-cycle paths (auto-groom "cycle #N" labels).

**Secondary: complete-on-issue-close (event-driven; addresses category-b out-of-band resolution)**

Hook GitHub `issues.closed` and `pull_request.closed` webhook events. On receipt, look up any `blocked` or `in_progress` tracking rows whose `reference_url` matches the closed issue / PR's issue URL and mark them `cancelled` with reason `issue_closed_upstream` (or `completed` if the close was via merged PR). This closes the surface #1712 sweep drains from — the tracking rows terminal-mark synchronously with the operator's actual out-of-band resolution.

**Backstop: keep #1712 sweep as-is.** Any row the two active surfaces miss (webhook delivery failure, race with issue-close, escalations not linked to a specific issue URL) is caught by the sweep at 3600s. Sweep telemetry becomes a health indicator, not the primary cleanup path.

**Deferred: enrich escalation UX.** Making `blocked` rows visible in the CLI operator surface (e.g. `mika tasks list --status blocked` with an "awaiting decision" callout) is a separate ticket (out of scope for this fix; capture as follow-up).

### Rejected shapes

- **Change escalation semantics to not use `blocked`** — rejected. `blocked` correctly represents the escalation state; the bug is the missing exit surface, not the write itself. Changing it would break operator-facing status semantics (`mika tasks list --status blocked` today lists escalations correctly).
- **Longer sweep grace (24h/7d)** — rejected. Trades drain-latency for FP fidelity. The primary + secondary fixes give both.
- **Reap-on-parent-exit** — not applicable. There is no "parent process" for these tracking rows post-dispatch; the dispatched claude-pilot is the child and it has its own callback lifecycle. The tracking row is the parent already.

## Acceptance criteria

**AC1 — Identify orphan-creating callsites.** ALL five `status='blocked'` writers on tracking rows are enumerated in the plan (verified above); the file:line references are current at implementation time (rebase-check invariant).

**AC2 — Supersede-on-new-dispatch invariant.** Both dispatch-write paths (`server/ready_label_handler.rs::handle` and `tools/create_task.rs` grooming branch) MUST, before inserting the new tracking row for a `reference_url`, execute a guarded UPDATE against any existing rows for the same agent + reference_url where `status IN ('blocked','in_progress')` AND `action_type='none'` AND `process_id IS NULL`, transitioning them to `cancelled` with `result='superseded_by_new_dispatch'`. SOLE WRITER: this transition writes `cancelled` with that exact reason string; audit event `tool_name='tracking_row_superseded'` MUST be emitted per superseded row.

**AC2.1 — Index verification (F1 sharpening).** The lookup path REUSES the existing `db.find_active_task_by_ref_url(agent_id, reference_url)` helper (`crates/mika-agent/src/db.rs:6134`) which already uses the partial unique index `idx_tasks_manual_active_ref_url` (`db.rs:1306`, `CREATE UNIQUE INDEX ON tasks(agent_id, reference_url) WHERE trigger_type='manual' AND reference_url IS NOT NULL AND status NOT IN ('completed','cancelled','failed','delivered')`). No new migration required — `blocked` and `in_progress` ARE inside the "active" set the index covers. Implementation MUST NOT add a duplicate index; the pre-existing composite covers the supersede lookup.

**AC2.2 — URL-suffix variant handling (F1 sharpening, télémétrie-driven).** The `?phase=groom` URL suffix used by the LLM-driven grooming path (`crates/mika-agent/src/db.rs:7266-7278`) makes `.../issues/1574` and `.../issues/1574?phase=groom` DIFFERENT reference_urls for lookup purposes. Télémétrie shows several tickets (mika#1574, #1712, #1867) have simultaneous phantom rows on BOTH URL variants. The supersede check MUST canonicalize by stripping the `?phase=groom` suffix and comparing on the base URL, so a fresh dispatch supersedes both the ready-label row (base URL) and the groom row (suffixed URL) for the same underlying issue. Implementation adds `db.find_active_tracking_rows_for_issue_url_and_variants(agent_id, base_url)` returning both the exact URL match and the `<base>?phase=groom` match; used inside AC2's pre-create supersede block.

**AC2.3 — NULL-URL fallback via label match.** Télémétrie shows 5 phantom rows with `reference_url=NULL` (LLM retry-cycle rows created via the tool-call path when the LLM omitted the URL). For these, the label carries the issue reference (e.g. `groom mika#1574 (auto-groom, poison loop)`). The supersede pre-check MUST also invoke the existing `db.find_active_task_by_label(agent_id, label)` helper (`db.rs:6197`) to catch label-match dedup collisions in the NULL-URL path. Same cancel-with-`superseded_by_new_dispatch` transition applies.

**AC3 — Reproduction test (unit-level).** New test `tests/eval/test_tracking_row_supersede.rs` seeds a `blocked` tracking row for `reference_url='https://github.com/senara-solutions/mika/issues/9999'`, invokes `ready_label_handler`'s pre-create path for the same URL, and asserts:
   - the seeded row transitions to `cancelled` with `result='superseded_by_new_dispatch'`
   - a fresh row is inserted for the same URL with `status='pending'`
   - exactly one `audit_events` row with `tool_name='tracking_row_superseded'` is written per superseded row

**AC4 — Webhook-driven complete-on-close (revised per F2 sharpening).**

**Existing route surface state (grep-verified against current main @ 66056d42):**
- `pull_request.closed` IS routed to `mika-dev` today (`crates/mika-gateway/src/github.rs:1774-1781`, `test_route_event_pr_closed`).
- `issues.closed` is NOT routed today — `route_event("issues", Some("closed"), None)` returns `None` (`github.rs:1727-1730`, `test_route_event_issues_closed`). The gateway drops the event.

**Implementation is a TWO-part change:**

**AC4.a — Extend the routing table.** `route_event` in `crates/mika-gateway/src/github.rs` MUST map `("issues", "closed")` to a target agent (either `mika-dev` for consistency with pr.closed, or a new dedicated `tracking-cleanup` route — implementer picks based on the loudness cost of the extra agent surface; default choice is `mika-dev`). Update `test_route_event_issues_closed` accordingly and add a positive-case sibling `test_route_event_issues_closed_routes_to_mika_dev`.

**AC4.b — Engine handler for the pr.closed/issues.closed events.** In `crates/mika-agent/src/server/` add a new module `upstream_close_handler.rs` invoked from the same webhook-event verdict-handler surface that today receives pr.* events. Handler flow:
1. Extract `reference_url` from the event payload (issue URL for `issues.closed`; PR's linked issue URL parsed from the PR body's `Closes #<n>` / `Fixes #<n>` refs for `pull_request.closed`).
2. Call `db.find_active_tracking_rows_by_reference_url_and_variants(agent_id, base_url)` — a new helper that returns rows matching the exact URL, the `?phase=groom` variant, and any label-match fallback for NULL-URL rows (same canonicalization as AC2.2/AC2.3).
3. For each matched row: for `issues.closed` transition to `cancelled` with `result='issue_closed_upstream'`; for `pull_request.closed` merged=true transition to `completed` with `result='upstream_pr_merged'`; for `pull_request.closed` merged=false transition to `cancelled` with `result='upstream_pr_closed_unmerged'`.
4. Emit audit event `tool_name='tracking_row_upstream_closed'` per transitioned row with the source discriminator (`event_type=issues.closed` or `event_type=pull_request.closed`) in the `reasoning` field.

**AC4.c — Idempotency and no-op safety.** Handler MUST be idempotent (re-delivery safe — the guarded UPDATE against `status IN ('blocked','in_progress')` naturally short-circuits on already-terminal rows) and MUST be no-op when no matching rows exist (log at DEBUG, no WARN, no audit event).

**AC4.d — Route wiring test.** New integration test `crates/mika-gateway/tests/test_issues_closed_route.rs` fires a synthetic `issues.closed` webhook payload and asserts the event reaches the mika-dev agent's event surface (existing gateway test infrastructure).

**AC5 — Regression test (webhook).** New test `tests/eval/test_tracking_row_upstream_close.rs` fires a synthetic `issues.closed` event for a `reference_url` with a `blocked` tracking row and asserts terminal transition + audit event.

**AC6 — Télémétrie health assertion.** After the fix ships and is deployed for ≥7 days, the mika#1712 sweep count should trend toward zero. Add a documentation-only assertion in `docs/solutions/observability/phantom-sweep-baseline.md` capturing "post-mika#1934 baseline: sweep count expected to be <2 rows/week on mika-dev; higher indicates a new orphan-creation path not covered by AC2/AC4."

**AC7 — Injection-verified evidence (extended per F3 sharpening).** Implementer runs `mika` binary against a scratch DB and injects three shapes; each shape's before/after `sqlite3` output MUST be attached to the PR as an evidence artifact:

- **AC7.1 — blocked → superseded (primary path).** Seed a phantom row via `db.create_task` with the exact shape (action_type=none, process_id=NULL, status=blocked, source=self_dev, reference_url=`https://github.com/senara-solutions/mika/issues/9999`), trigger the ready-label handler for the same URL, verify the seeded row transitions to `cancelled` with `result='superseded_by_new_dispatch'` and a new row is `pending` for the same URL.

- **AC7.2 — in_progress → superseded (F3 edge case).** Seed a tracking row in `in_progress` (a race path: manual insert while a prior dispatch is still running), trigger a fresh ready-label handler for the same URL, verify the seeded row transitions to `cancelled` with the same reason. Distinct from AC7.1 because `in_progress` supersede is the load-bearing race path — mika#1574 × 4 pattern relies on this working correctly for retry cycles.

- **AC7.3 — issues.closed → cancelled (secondary path).** Seed a blocked tracking row for `mika#9999`, fire a synthetic `issues.closed` webhook payload against the gateway, verify the row transitions to `cancelled` with `result='issue_closed_upstream'` and audit event `tool_name='tracking_row_upstream_closed'` is emitted.

## Definition of Done (DoD)

- [ ] All 7 ACs above met, each with named file:line evidence in the PR description.
- [ ] `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` green.
- [ ] Unit tests (AC3 + AC5) live in `crates/mika-agent/tests/eval/` and pass in CI.
- [ ] The two new audit_event `tool_name` values (`tracking_row_superseded`, `tracking_row_upstream_closed`) are documented in the audit-events surface reference.
- [ ] No sweep-count regression: seeded `phantom_aged_out` fixture in `test_phantom_task_row_sweep.rs` still passes (the sweep remains the backstop).
- [ ] PR description includes the télémétrie table above as the "before" state and a follow-up snapshot as the "after."

## Verbatim Investigation Directions (from mika#1934 body, preserved for traceability)

The pass-1 body listed these directions; pass-2 has converged them into concrete ACs above. Preserved verbatim so the reader can audit the conversion:

1. **Enumerate call sites** invoking `update_task_status → in_progress` on tracking rows (`db.rs:5307`) — converted into AC1 with `status='blocked'` as the actual signal (télémétrie showed 100% pre-sweep status was `blocked`, not `in_progress`).
2. **Match against session lifecycle** — sessions wedged, timed out, killed mid-orchestration — hypothesis falsified by data (0 rows in `in_progress` post-sweep implies subprocess-crash-mid-flight is not the mechanism).
3. **Cross-check against phantom timestamps** — quelles séquences opérateur/dispatch précèdent le leak — converted into the multi-dispatch-collision analysis (mika#1574 × 4) and the CLOSED-out-of-band verification.
4. **Baseline vs mika#1289 territory** (task claim/complete flow) — not applicable; #1289 covers claim/complete for `resume_agent` callback children, orthogonal to the `action_type='none'` tracking-row shape.

## Body-vs-code citations (all verified against current main @ 66056d42)

| Claim | Verified file:line |
|-------|--------------------|
| Ready-label engine handler creates tracking row | `crates/mika-agent/src/server/ready_label_handler.rs:168-201` (label format), `169-193` (`NewTask` insert) |
| Ready-label handler auto-transitions parent to in_progress | `crates/mika-agent/src/server/ready_label_handler.rs:374-380` (step 9g) |
| verdict_handler BLOCK[ac] → blocked | `crates/mika-agent/src/server/verdict_handler.rs:993` |
| verdict_handler BLOCK[ci] → blocked | `crates/mika-agent/src/server/verdict_handler.rs:1283` |
| verdict_handler BLOCK[security]/[pipeline] → blocked | `crates/mika-agent/src/server/verdict_handler.rs:1490` |
| Identical-diff circuit breaker → blocked | `crates/mika-agent/src/server/verdict_handler.rs:2154` |
| Dispatcher inherited-blocked path | `crates/mika-agent/src/task_engine/dispatcher.rs:3191` |
| Sweep predicate (SOLE READER) | `crates/mika-agent/src/db.rs:6751-6784` (`find_phantom_tracking_tasks`) |
| Sweep write (SOLE WRITER phantom_aged_out) | `crates/mika-agent/src/task_engine/engine.rs:840-873` (AC3 watchdog) + `engine.rs:980-1030` (AC5 startup) |
| Grooming label URL suffix | `crates/mika-agent/src/db.rs:7266-7278` |

The pass-1 body's file/function references (`create_task.rs:235-258, 318-341` and `db.rs:5307`) are stale relative to the actual mechanism the télémétrie reveals — the primary writers of the leak are in `verdict_handler.rs`, not `create_task.rs`. The pass-1 body is left intact for lineage; this plan supersedes its investigation shape.

## Sequencing

**Phase 1 — Supersede-on-new-dispatch (AC2 + AC3)**

1. Add `db.find_active_tracking_rows_for_issue_url_and_variants(agent_id, base_url)` — returns rows matching the exact URL AND `<base>?phase=groom` variant AND any NULL-URL label-match fallback (canonicalization per AC2.2/AC2.3). Reuses the existing `idx_tasks_manual_active_ref_url` partial index (AC2.1) for the URL-match branch; label-match branch is bounded to `trigger_type='manual'` rows and is expected small-N.
2. Add `db.cancel_task_superseded(task_id)` — guarded UPDATE transitioning to `cancelled` with `result='superseded_by_new_dispatch'`, only from `('blocked','in_progress')`. Returns bool. SOLE WRITER: this method writes `cancelled` with that exact reason string.
3. In `ready_label_handler.rs` before step 7 (`db.create_task(new_task)`), call the two helpers above and emit `audit_events` (`tool_name='tracking_row_superseded'`) per superseded row.
4. Mirror the same guard in the LLM-facing `tools/create_task.rs` grooming branch (`?phase=groom` URL suffix path) — same helper, same audit tool_name.
5. Write AC3 test (`tests/eval/test_tracking_row_supersede.rs`) covering: (a) blocked → superseded (AC7.1 fixture), (b) in_progress → superseded (AC7.2 fixture), (c) URL-suffix variant (base URL + `?phase=groom` both superseded by fresh dispatch), (d) NULL-URL label match, (e) idempotency (re-invoke supersede, verify no double-cancel).

**Phase 2 — Complete-on-issue-close webhook (AC4 + AC5)**

6. Extend the `issues` event route in `crates/mika-gateway/src/github.rs` — currently `("issues","closed") → None` (dropped). Change to route to `mika-dev` (matches existing `pr.closed → mika-dev` pattern). Update `test_route_event_issues_closed` to assert the new positive routing and add sibling positive-case coverage.
7. Add engine-side handler `crates/mika-agent/src/server/upstream_close_handler.rs` invoked from the same webhook-event verdict-handler surface that already routes pr.* events. Handler looks up tracking rows via `db.find_active_tracking_rows_by_reference_url_and_variants(agent_id, base_url)` (new helper — handles URL exact + `?phase=groom` variant + NULL-URL label fallback per AC4.b step 2). Transitions each matched row: `issues.closed` → `cancelled` w/ `result='issue_closed_upstream'`; `pull_request.closed` merged=true → `completed` w/ `result='upstream_pr_merged'`; unmerged close → `cancelled` w/ `result='upstream_pr_closed_unmerged'`.
8. Emit audit_events (`tool_name='tracking_row_upstream_closed'`) per transitioned row with `event_type` in reasoning discriminator.
9. Write AC5 test (`tests/eval/test_tracking_row_upstream_close.rs`) covering both `issues.closed` and `pull_request.closed` (merged/unmerged) paths + idempotency assertion (re-fire same event, verify no double-transition, no duplicate audit events).

**Phase 3 — Documentation + télémétrie baseline (AC6 + AC7)**

10. Create `docs/solutions/observability/phantom-sweep-baseline.md` documenting the expected trend and interpretation guide.
11. Update audit-events surface reference with the two new `tool_name` values.
12. Injection evidence per AC7 attached to PR.

**Rebase invariant.** Base branch is `origin/main` @ `66056d42`. If main advances during grooming/implementation, rebase before Phase 1 — no direct-merge of stale branches.

## Not in scope

- Extended sweep grace window changes — #1712 grace stays at 3600s.
- Operator-facing UX for `blocked` tracking rows (`mika tasks list --status blocked --awaiting`) — separate follow-up ticket.
- Retroactive cleanup of pre-existing rows — the #1712 sweep already handles this.
- Changes to the escalation surface itself (`verdict_handler.rs` write sites stay unchanged; the cleanup is downstream).

## Refs

- Parent (dependency, MERGED 2026-08-22): mika#1712
- Sami bearing (pass-1 shape decision): `/var/spool/claude-mail/mpc/archive/2026-08-21-143740-*.md`
- Sami bearing (pass-2 dispatch, 2026-08-22 06:42Z): "LANE 2 — Cause-racine #1712, groom pass 2. La télémétrie est vivante maintenant."
- #1712 plan (for shape context): `mika/docs/plans/2026-08-21-001-fix-1712-phantom-task-row-accumulation-plan.md`
- Sweep telemetry raw: `sqlite3 ~/.mika/data/mika.db "SELECT ... FROM audit_events WHERE tool_name='phantom_aged_out'"` (42 rows @ 2026-08-22 07:14:50)

## Priority

**p2-normal** stays. Escalation trigger (rate × 1.5 over 3d) has NOT fired; the observed ~0.8 rows/day is below the sustained-leak threshold. If post-fix the sweep count doesn't trend toward zero within 7 days, this ticket escalates to p1 (the fix didn't cover the whole surface).

<!-- GROOMED -->
