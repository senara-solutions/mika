---
title: Permission-decision request stream — wire protocol + config surface
issue: mika#1733 (sub-C of mika#1727)
status: design-locked (Vincent ratification 2026-07-06 17:00 CEST)
authored: 2026-07-06
authors: orchestrator-CC (MPC) via samidarko relay of Vincent + Prime ratifications
---

# Permission-decision request stream — design and implementation contract

Companion design doc for `senara-solutions/mika#1733` (sub-C of mika#1727 TUI thin-client fan-out). This document is the load-bearing implementation contract that all 8 acceptance criteria compile against. Implementation PRs cite this doc and follow the sections below verbatim.

Six sharpenings and Vincent's Q2 (a)+(b) ratifications are FROZEN. Implementers do not re-litigate; they implement.

## Ratification chain (for the record)

- **Prime AMEND** 2026-07-05 relayed by samidarko-claude — ratified plumbing (Q1/Q3/Q4).
- **§12 pass 1 outsideness catch** 2026-07-06 — chat seat's LLM-fragility distinction accepted; window resets to 3 fresh passes from that point.
- **Vincent Q2 (a) + (b)** 2026-07-06 17:00 CEST (mika#1733 comment 4896742892) — locks Sharpening 4's three pre-registered conditions AS-IS. Structural-only guarantee (a) resolved via §12 pass 1 convergence, locked into AC2. Override ceremony (b) resolved via this ratification, locked into AC5.
- **Sub-C proceeds under AC8 unchanged**: default STRICT + override OFF regardless.

## Executive summary

When mika-spirit's permission classifier defers a tool call to the operator for approval (or escalates a structured question), the TUI needs to receive the request, prompt the operator, and reply. This document specifies the wire protocol (SSE request channel + POST-back reply), the server-side config surface for the `decision_authority` control, and the async best-effort emit path from mika-spirit to cm's `event_log` for override observability.

**Load-bearing property (Sharpening 1 → AC2):** "This agent structurally cannot do X" applies to **pre-classifier engine gates only, NEVER to LLM classifier decisions.** LLM false-denies are a documented failure class (compound corpus below). Making them final-at-wire converts a known failure class into a wedge — the exact failure that elevated mika#1727.

**Shipped default (AC8):** STRICT + override OFF. No flip toggle exposed even for testing. Enable-flip lands only when Sharpening 4's three ratified conditions demonstrably trigger.

## AC1 — SSE request channel + POST-back reply

**Prime-ratified plumbing (Q1/Q3/Q4).** The wire shape has no doctrinal debate; the sharpenings apply to *content*, not *transport*.

### Endpoints

- **`GET /dashboard/permissions/stream`** — Server-Sent Events channel; long-lived; token-authenticated (bearer via `MIKA_INTERNAL_TOKEN`; same auth surface as sibling dashboard SSE endpoints). One connection per TUI process. Slow-consumer discipline: bounded per-client channel with drop-oldest-with-marker on overflow (matches D5 dual-audit / cm `event_bus` discipline; do NOT block emission on slow consumer).

- **`POST /dashboard/permissions/{request_id}/decide`** — accepts `{"decision": "approve" | "deny", "reason": Option<String>}`. Correlated by `request_id`. Idempotent: the same `request_id` decided twice returns 409 with the first decision's record.

- **Held-request timeout**: server default 5 minutes (configurable, `MIKA_PERMISSION_HOLD_TIMEOUT_SECS`). Timeout produces an internal `Deny{reason: "operator-timeout"}` result — fail-closed. Matches cpp#20 joint 2 discipline (denial paths halt honestly).

### SSE frame shapes

Discriminated by `event:` field. Two variants:

```jsonc
// event: permission_request
{
  "event": "permission_request",
  "request_id": "01H...",           // UUIDv7 for time-ordering + correlation
  "tool_name": "Bash",
  "args_summary": "git status",     // truncated + secret-scrubbed per _summarize_input
  "classifier_verdict": "held",      // approved | denied | held
  "held_reason": "requires operator review — no policy match"
}

// event: ask_user_question
{
  "event": "ask_user_question",
  "request_id": "01H...",
  "questions": [
    {
      "question": "Which library should we use?",
      "options": [
        {"label": "Serde", "description": "..."},
        {"label": "Bincode", "description": "..."}
      ],
      "multiSelect": false
    }
  ]
}
```

The discriminated union means AC1 (permission decisions) and sibling sub-D (AskUserQuestion callback bridge, `mika#1734`) share a single wire channel, single auth surface, single subscriber connection — reduces connection count. See sub-D for the answer POST-back endpoint (`POST /dashboard/permissions/{request_id}/answer`).

### Reconnect / cursor semantics

Consumers reconnect with `Last-Event-Id: <last-seen>`; server replays undelivered frames from an in-memory ring buffer (bounded 100 frames per client). Deeper resume (across process restart) is out of scope; the classifier retries the tool call on the pilot side if the frame was in flight when the TUI dropped.

### Test contract (AC1)

- **AC1.1** — SSE handshake: TUI connects; server sends `retry: 30000` + heartbeat comment every 30s.
- **AC1.2** — decision round-trip: mock classifier defers → SSE frame arrives → TUI POSTs decision → classifier receives result within 200ms.
- **AC1.3** — held-request timeout: no decision within `MIKA_PERMISSION_HOLD_TIMEOUT_SECS` → server materializes internal `Deny{reason: "operator-timeout"}`; classifier receives it.
- **AC1.4** — slow-consumer drop: fill client channel past cap; assert oldest frame dropped with `event: overflow_marker` frame sent. Server emission never blocks.

## AC2 — Structural-only guarantee (Sharpening 1)

**Rule to carry as a doctrine anchor at every classifier-gate call site:**

> **"This agent structurally cannot do X" applies to pre-classifier engine gates only, NEVER to LLM classifier decisions.**

### Where this rule is enforced structurally

- **Pre-classifier engine gates** (allowed guarantee-shape): tier1 fast-path checks, path validation (`validate_and_resolve_path`), tool-registry membership, identity-driven tool denylist (`ToolsIdentityConfig`), skill allowlists (`SkillRegistry::apply_identity_allowlist`), request-body limits, structural regex vetoes (bare-`&`, `<<<` here-strings, `${ ... }` funsub, etc.).
- **LLM classifier decisions** (NEVER final-at-wire): any decision produced by the mika-relay / permission-policy skill, mika-arch verdicts, mika-qa acceptance judgments. These have a documented false-deny failure class; they inform, they do not structurally block.

### Compound corpus grounding

- **mika#935** — `mika ask --agent mika-arch` denied by LLM permission-policy despite being a legitimate platform-agent dispatch. `mika/docs/solutions/935-intra-platform-agent-dispatch-structural-pre-classifier.md`.
- **`mika-platform/docs/solutions/agent-quality/2026-04-09-fabricated-cantool-denial-citations.md`** — fabricated cantool-denial post-mortem.
- **Memory `feedback_prompt_enforcement_fragile`** — structural constraints beat prompt-level.

### Code-side anchor (implementer follow-up)

The verbatim rule above lives as a comment at each classifier-gate call site. Concretely, that means annotating (a) the tier1 auto-approve path, (b) the tier2 policy-evaluation path, (c) the tier3 dangerous-command veto, and (d) the pre-execution assertion site. Each call site's comment says: "See permission-decision-protocol-2026-07-06.md § AC2 — structural-only guarantee applies here; LLM classifier verdicts are NOT structural gates." The comment lives at the call site; the rule lives here.

### Test contract (AC2)

- **AC2.1** — Regression: assert LLM-classifier verdicts always populate `classifier_verdict` on the decision record. Never elided. Never overridden by structural code paths.
- **AC2.2** — Regression: assert pre-classifier engine gates that structurally block (path escape, control-plane denylist) run BEFORE the classifier and short-circuit without emitting a `permission_request` frame. Structural denials never reach the operator — that's the point of "structural".

## AC3 — `decision_authority` is server-side config, NOT wire-carried input (Sharpening 2)

**Enforcement:** `decision_authority` is a **server-side config value** (env var + config-file field). It is NEVER an envelope field on the request wire. Attempts to POST a decision with `decision_authority` in the body reject with 400 `unknown_field: decision_authority`.

Same rule as cm-api's unconditional field-copy discipline / internal-flag-absent-from-input-schema pattern.

### Config surface

Environment variables and identity-file fields, resolved with the standard mika `MIKA_` prefix + config-rs override chain:

```
MIKA_DECISION_AUTHORITY = "strict" | "operator_override" | "founder_override"
                          (default: "strict")

MIKA_PERMISSION_HOLD_TIMEOUT_SECS = <positive int>
                                    (default: 300)
```

The value is loaded once at server startup. Runtime rotation requires restart (matches other decision-authority-adjacent config).

### Wire schema rejection

The `PermissionDecideRequest` struct uses `#[serde(deny_unknown_fields)]`. `decision_authority` is not a field. Any POST body containing that key returns 400 with a discriminated error variant.

### Test contract (AC3)

- **AC3.1** — Wire rejection: POST with `{"decision":"approve", "decision_authority":"operator_override"}` → 400 `unknown_field`.
- **AC3.2** — Env-load: startup with `MIKA_DECISION_AUTHORITY=invalid_value` → hard error at parse (fail-loud).
- **AC3.3** — Runtime immutability: no runtime endpoint accepts a `decision_authority` write. Test: grep the codebase; assert `decision_authority` appears only in config-load + read paths, never in a POST handler's write path.

## AC4 — Provenance fields day-one (Sharpening 3)

**Fields present on every decision record, populated on every decision, even when `override_used = false`:**

- `classifier_verdict: "approved" | "denied" | "held"` — what the classifier said.
- `operator_decision: "approve" | "deny" | null` — what the operator ratified (`null` when no decision was required, i.e., classifier auto-approved without escalation).
- `override_used: bool` — did `operator_decision` differ from `classifier_verdict`?

Reversibility asymmetry: cheap to add now, expensive to add later. Same shape as D5's dual-audit discipline.

### Persistence

Decision records live in a new mika-agent-side table `permission_decisions`:

```sql
CREATE TABLE permission_decisions (
    id BLOB PRIMARY KEY,           -- UUIDv7
    request_id BLOB NOT NULL,
    tool_name TEXT NOT NULL,
    args_summary TEXT,             -- secret-scrubbed
    tenant_id TEXT,                -- for AC6 scope
    agent_id TEXT,                 -- for AC6 scope
    classifier_verdict TEXT NOT NULL CHECK(classifier_verdict IN ('approved','denied','held')),
    operator_decision TEXT CHECK(operator_decision IN ('approve','deny')),
    override_used INTEGER NOT NULL,  -- 0 | 1
    decision_authority TEXT NOT NULL,  -- snapshot of server config at decision time
    created_at TEXT NOT NULL,
    resolved_at TEXT
);
```

Schema-version bump. Migration is additive.

### Test contract (AC4)

- **AC4.1** — Persistence: query record store after a decision; assert all three provenance fields populated.
- **AC4.2** — Snapshot: `decision_authority` column captures the config VALUE AT DECISION TIME, not a live pointer — protects the audit trail against post-hoc config rotation.

## AC5 — Pre-registered flip conditions (Sharpening 4)

**Ratified verbatim by Vincent (2026-07-06 17:00 CEST, mika#1733 comment 4896742892):**

1. **Documented false-deny reproduced ≥ 3× on the same tool-arg shape across ≥ 2 model providers** → override flag becomes admissible for that specific tool-arg pattern, gated to that scope.

2. **Operator-time interruption from a documented true-positive deny reaches ≥ N incidents/week** → escalation review; NOT automatic flip, but review-triggered by pre-committed threshold.

3. **Founder authorization for a specific scoped override** (per tenant / per agent / per tool-arg pattern) — always admissible, recorded verbatim.

**These three become the pre-registered flip conditions.** No case-by-case adjudication under flip-time pressure. Any future enable-flip ticket must cite this ratification + demonstrate the specific condition triggered.

### Implementer-scope open item

**Condition 2's `N incidents/week` threshold is unspecified.** Implementer proposes N in this design doc at grooming time; Vincent ratifies the numeric N. Vincent's "accept as-is" ratifies the CONDITION SHAPE, not a numeric N.

**Provisional proposal for grooming**: `N = 3`. Rationale: aligned with the `n=3` failure-aggregation threshold documented in `feedback_aggregate_failures_at_n3_not_silo`; three-incident-per-week is roughly one operator interruption every second workday, which reasonably crosses "sustained enough to warrant review". Vincent ratifies (or overrides) at grooming.

## AC6 — Scoped config (Sharpening 5)

**Scope axis** as a first-class config dimension from day one. Per-tenant / per-agent / global.

### Config storage

Config keys include scope (comma-separated fallback per the standard mika `MIKA_` chain):

```
MIKA_DECISION_AUTHORITY__TENANT__acme      = "operator_override"   # scope: tenant=acme
MIKA_DECISION_AUTHORITY__AGENT__mika-dev   = "strict"              # scope: agent=mika-dev
MIKA_DECISION_AUTHORITY                    = "strict"              # scope: global (fallback)
```

Resolution is **closest-match**: agent-scope > tenant-scope > global-fallback. When a decision request arrives, the handler resolves the scope by inspecting the classifier context (`agent_id`, `tenant_id`) and selects the closest-matching config value.

Family-customer-1 substrate needs this structurally: a tenant enabling override for their agent MUST NOT cascade to Vincent's agents or other tenants.

### Test contract (AC6)

- **AC6.1** — Isolation: config for tenant T1 with override enabled does NOT affect tenant T2's decisions.
- **AC6.2** — Precedence: agent-scope config wins over tenant-scope wins over global-fallback.
- **AC6.3** — No cross-scope pollution: config for one scope key does not leak into other scope keys' resolution.

## AC7 — `override_event` D6 class + fire-and-forget async emit (Sharpening 6)

`override_event` becomes a new event class in cm's `event_class_policy` — so future surfaces (audit UI, MikaWood visibility, compliance-facing surfaces) can render override history.

### Migration (cm side — separate PR on control-monitor repo)

```sql
-- migrations/YYYYMMDDHHMMSS_seed_override_event_class.up.sql
INSERT INTO event_class_policy (event_class, hot_days, warm_days, terminal_disposition)
VALUES ('override_event', 30, 150, 'cold_indefinite')
ON CONFLICT DO NOTHING;
```

Retention shape matches sibling classes: `pr_lifecycle` (cm#88), `framing_translation` (cm#67), `permission_decision` (proposed under cm#99 for cpp permission events).

### Emit path (mika-agent side)

Fire-and-forget async emit from mika-spirit → cm-api's `POST /api/v1/webhooks/permission-event` (or cm#99's async-emit path when it lands, whichever is available first). **Mirrors cm#99's discipline exactly:**

- **Buffered**: in-process bounded queue with backpressure (drop-oldest on overflow).
- **Non-blocking**: dispatched off the decision code path (background tokio task).
- **Drop-with-marker on cm-unreachable**: when cm is down / slow / returns non-2xx, drop silently but increment an in-process counter for observability. Do NOT retry (retry re-introduces coupling).
- **Time-bounded transport**: request timeout ≤ 500ms.
- **Env-gated**: `MIKA_CM_OVERRIDE_EMIT_ENABLED=1` (unset = disabled). Same discipline as `MIKA_ORCHESTRATOR_INBOX_ENABLED`.

### Code pattern reference

The exact fire-and-forget shape has landed once already in PR#1739 (mika-gateway `forward_to_cm_api` in `crates/mika-gateway/src/github.rs`). Implementers of AC7 SHOULD reuse that pattern verbatim (tokio::spawn + reqwest with per-request timeout + DEBUG on 2xx + WARN on error + drop on failure). See `PR#1739` for the reference implementation.

### Payload

```jsonc
{
  "record_id": "01H...",              // matches permission_decisions.id
  "tenant_id": "acme",                // or null
  "agent_id": "mika-dev",
  "tool_name": "Bash",
  "classifier_verdict": "denied",
  "operator_decision": "approve",
  "override_used": true,
  "decision_authority": "operator_override",  // snapshot
  "created_at": "2026-07-06T22:15:00Z"
}
```

### Test contract (AC7)

- **AC7.1** — Emit dispatched from every decision resolution site. Mock the emit surface; assert call-count matches decision-count when `MIKA_CM_OVERRIDE_EMIT_ENABLED=1`.
- **AC7.2** — **Load-bearing "cannot smuggle coupling" property**: point emit at a black-hole HTTP endpoint (e.g., `240.0.0.1:65535` TEST-NET-3), dispatch 100 decisions, assert total elapsed time is within noise of the same run with `MIKA_CM_OVERRIDE_EMIT_ENABLED=0`. This is the same test shape as PR#1739's `test_forward_to_cm_api_is_fire_and_forget_on_unreachable`.
- **AC7.3** — Buffered queue drops-oldest on overflow with a surfaced counter. Fill queue past capacity; assert oldest entries dropped; assert `overflow_count > 0` at teardown.
- **AC7.4** — Env-gated default: with `MIKA_CM_OVERRIDE_EMIT_ENABLED` unset, mika-agent makes zero HTTP calls to cm. Network-inspection test.

## AC8 — Shipped default: STRICT + override OFF

Regardless of Vincent's Q2 (a)/(b) answers (both now ratified), the **shipped default is `decision_authority = strict` + `override_used = false` in production configs**.

No flip toggle exposed. Not even for testing — an explicit `MIKA_DECISION_AUTHORITY=operator_override` set at operator command-line is the only path to non-strict; there is no runtime API to modify it, no `flip=true` query param, no admin endpoint.

Enable-flip lands as a **separate future ticket** citing this ratification + demonstrating one of Sharpening 4's three ratified conditions has triggered.

### Test contract (AC8)

- **AC8.1** — Config defaults: fresh install with no `MIKA_DECISION_AUTHORITY` env → resolved value is `strict`.
- **AC8.2** — Grep discipline: `grep -rn 'override_used = true' crates/mika-agent/src/` returns only test code paths — production code paths never construct `override_used = true` inline; the value comes from operator-decision runtime resolution.

## Blast-radius protection (Prime's veto axis, reproduced)

Sub-C MUST NOT re-introduce coupling. Failure modes to guard against explicitly:

- **Sync HTTP on the permission-decision return path** — smuggles the coupling in through logging. AC7.2 catches this.
- **Retry on cm-unreachable** — even bounded retries create tail-latency correlation with cm availability. AC7.2 catches this.
- **Blocking on queue-full** — queue must drop-oldest, never block enqueue. AC7.3 catches this.
- **Un-gated default** — if `MIKA_CM_OVERRIDE_EMIT_ENABLED` defaults to on, every operator running mika-agent against an unreachable/not-yet-existing cm pays the (bounded) tail cost. AC7.4 keeps it off by default; enable per-deployment.

## Implementation call-sites map

| AC | Files to touch | Estimated size |
|---|---|---|
| AC1 | `crates/mika-agent/src/server/mod.rs` (new routes) + `crates/mika-agent/src/server/permissions_stream.rs` (new file — SSE + POST-back handlers + in-memory ring buffer) | ~300 lines Rust |
| AC2 | Doctrine anchor comments at classifier-gate call sites (grep for tier1/tier2/tier3 gate; find pre-execution assert site). Anchor points to this doc. | ~10 lines |
| AC3 | `crates/mika-agent/src/config_keys.rs` (new setting), `crates/mika-common/src/config.rs` (Settings field), wire-schema struct with `#[serde(deny_unknown_fields)]` | ~40 lines |
| AC4 | Migration: `crates/mika-agent/migrations/*` (new SQL) + `crates/mika-agent/src/db/schema.rs` (schema version bump v39→v40) + `crates/mika-agent/src/db.rs` (CRUD helpers) | ~120 lines |
| AC5 | This design doc's § AC5 (already landed) + implementer's provisional N proposal for condition 2 (default: N=3 pending Vincent ratification) | ~0 (doc-only) |
| AC6 | `crates/mika-common/src/config.rs` (scope-resolution helper) + `crates/mika-agent/src/server/permissions_stream.rs` (scope-lookup at decision time) | ~80 lines |
| AC7 | `crates/mika-agent/src/server/permissions_stream.rs` (async emit dispatch after decision resolves) + `crates/mika-agent/src/cm_emit.rs` (new file — mirrors PR#1739 pattern). Companion migration on control-monitor repo. | ~150 lines Rust + 10 lines SQL |
| AC8 | Default value in AC3 config field. | ~0 (comes free from AC3 default) |

**Estimated total**: ~700 lines Rust + migration + tests. Comparable to sub-E's `/healthz` overshoot (which was ~120 lines including tests) but multi-file.

## What this PR (feat/1733/agent-permission-decision-protocol) delivers

**Overnight-bounded ship (this PR):**

- ✅ This design doc (`crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md`) — locks the full architecture in writing.
- ✅ AC5 pre-registered conditions (in this doc, verbatim from Vincent's comment 4896742892).
- ✅ AC5 provisional N proposal (`N=3 pending Vincent ratification`).
- ✅ AC8 shipped-default doctrine (in this doc).
- ✅ Compound corpus citations for AC2 (in this doc).

**Deferred to follow-up PRs (clean handoff — call sites named above):**

- ⏭ AC1 runtime code (SSE endpoints + POST-back + tests).
- ⏭ AC2 in-code doctrine anchor comments at classifier gate sites.
- ⏭ AC3 config plumbing + wire-schema rejection test.
- ⏭ AC4 migration + `permission_decisions` table + CRUD.
- ⏭ AC6 scope-resolution helper.
- ⏭ AC7 mika-agent emit path + companion cm migration for `override_event` event_class_policy.

**Cross-repo companion PRs when Vincent's morning window opens:**

- Control-monitor migration for `override_event` event_class_policy row (small, self-contained; landing separately keeps the two repos' PRs reviewable independently).

## Implementation status

**AC1 (shipped, PR#1741):** SSE request channel + POST-back handler at
`crates/mika-agent/src/server/permissions_stream.rs`.

**AC2-AC6, AC8 (shipping in this PR, feat/1733/permission-decision-request-stream):**

| AC | Status | Landing site |
|---|---|---|
| AC2 | Partial — in-repo anchors landed | `crates/mika-agent/src/skills/executor.rs::validate_dispatch_readiness`, `crates/mika-agent/src/webhook_dispatch.rs::is_unauthorized_webhook_dispatch`. Claude-pilot-py companion anchors tracked as a follow-up (see PR body). |
| AC3 | Landed | `Settings.decision_authority` + `Settings.permission_hold_timeout_secs` in `crates/mika-common/src/config.rs`; `MIKA_DECISION_AUTHORITY` / `MIKA_PERMISSION_HOLD_TIMEOUT_SECS` env vars; `PermissionDecideRequest` wire-schema rejection preserved from PR#1741. |
| AC4 | Landed | Schema v43→v44 migration + `permission_decisions` table in `crates/mika-agent/src/db.rs`; `AsyncDatabase::insert_permission_decision` helper; `PermissionsChannel::resolve_decision` full-signature refactor with oneshot-first, DB-write-in-spawn ordering. |
| AC5 | No new content | Pre-registered flip conditions shipped in PR#1740; this PR references verbatim. |
| AC6 | Landed | `crates/mika-common/src/permission_authority.rs` (`DecisionScope` + `resolve_authority`); startup validation via `validate_env_authority_vars`; three-tier tests (agent > tenant > global > compile-time default). |
| AC7 | Deferred (14-day / P1) | Follow-up ticket filed alongside this PR — cm-side ingest endpoint blocking. |
| AC8 | Landed | `DecisionAuthority::default() == Strict` asserted by unit test; grep-discipline test in `crates/mika-agent/tests/ac8_grep_discipline.rs` enforces production emit paths never hard-set `override_used = true`. |

**Signature note (F1 architect sharpening):** the plan called for a
`resolve_decision_legacy` thin wrapper; the architect first-pass flagged
it as a silent-data-loss seam and directed a full-signature refactor
with mechanical test edits. The shipped shape reflects that: single
`resolve_decision(db, request_id, classifier_verdict, decision,
tool_name, args_summary, authority, scope)` function; no legacy variant;
the eight existing unit tests were rewritten in-place with the extra
provenance arguments plus new tests for the Strict-vs-Override
`override_used` derivation matrix.

## Ratification-preservation clause

This doc is the SOURCE OF TRUTH for sub-C's design. Implementation PRs cite section numbers here (e.g., "implements AC1 per permission-decision-protocol-2026-07-06.md § AC1"). If an implementer discovers a genuine architectural blocker requiring a design amendment, the implementer:

1. Halts implementation.
2. Drafts a proposed amendment to this doc.
3. Routes through samidarko outbox for Vincent + Prime re-ratification.
4. Does NOT ship implementation code that diverges from this doc without ratification.

The six sharpenings + Vincent's Q2 ratifications are frozen. Implementation-only. See samidarko's dispatch note 2026-07-06 22:45 CEST: "Ticket is fully specified: 8 ACs, six sharpenings landed, override ceremony locked. No fresh architectural decisions from me required — implement against the ticket text."

## Cross-links

- **Parent ticket**: `senara-solutions/mika#1733` (sub-C of mika#1727 fan-out).
- **Prime AMEND**: samidarko relay 2026-07-05 evening + Vincent's 17:00 CEST ratifying comment (mika#1733 comment 4896742892).
- **Reference implementation for AC7 emit shape**: `senara-solutions/mika#1739` (mika-gateway → cm-api fire-and-forget forward). Implementers of AC7 SHOULD study and mimic `forward_to_cm_api` in `crates/mika-gateway/src/github.rs`.
- **Companion cm ticket** (async-emit path infrastructure): `senara-solutions/control-monitor#99`.
- **Compound corpus** (AC2 grounding):
  - `senara-solutions/mika#935` (canary)
  - `mika-platform/docs/solutions/agent-quality/2026-04-09-fabricated-cantool-denial-citations.md`
  - Memory `feedback_prompt_enforcement_fragile`
- **Sibling sub-tickets**: sub-A (`mika#1731`), sub-B (`mika#1732`), sub-D (`mika#1734`, shares AC1's wire channel via discriminated event type), sub-E (`mika#1735`, PR#1738 open), sub-F (`mika#1736`), sub-G (`mika#1737`).
