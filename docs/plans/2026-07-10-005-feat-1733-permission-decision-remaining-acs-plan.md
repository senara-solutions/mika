---
ticket: mika#1733
branch: feat/1733/permission-decision-request-stream-remaining-acs
type: feat
scope: crates/mika-agent, crates/mika-common
grooming: /mika-groom-ticket
parent: mika#1727 (Phase 1 audit sub-ticket C)
---

# Plan — mika#1733 permission-decision request stream, remaining ACs

## Problem

Sub-ticket C of mika#1727. Prime's Q1/Q3/Q4 plumbing (SSE request stream + POST-back for the decision) was ratified 2026-07-06 and **AC1 shipped in PR#1741** as `crates/mika-agent/src/server/permissions_stream.rs` (376 lines, 8 unit tests). Q2 (structural shape) was refined by the two-seat §12 pass, producing six sharpenings that MUST land in this ticket rather than as separate follow-ups.

This ticket lands the AC2-AC8 delta on top of AC1's wire.

## Verification result

Fresh audit against `origin/main @ d53df2bc` (verification pass on branch `feat/1733/...`). Comprehensive delta below (details cross-referenced from the pre-groom investigation):

| AC | Status today | Location if shipped |
|---|---|---|
| AC1 | ✅ Shipped | `permissions_stream.rs` (PR#1741) |
| AC2 | ⚠️ Doc-only | Design doc `permission-decision-protocol-2026-07-06.md §AC2`. Doctrine-anchor comments have no in-mika-agent call sites because `permission-policy` skill was retired in mika#1193 — the classifier tiers moved to `claude-pilot-py`. Remaining in-mika-agent structural gates (`validate_dispatch_readiness`, `webhook_dispatch::is_unauthorized_webhook_dispatch`) can still carry the doctrine anchor for the "structural pre-classifier gates" side of the rule. |
| AC3 | 🟡 Partial | Wire-schema rejection via `#[serde(deny_unknown_fields)]` already at `permissions_stream.rs:88`. `Settings.decision_authority` and `MIKA_DECISION_AUTHORITY` env var — NOT FOUND. `Settings.permission_hold_timeout_secs` — NOT FOUND (only the runtime `DEFAULT_HOLD_TIMEOUT_SECS = 300` const at `permissions_stream.rs:47`). |
| AC4 | ❌ Not started | `permission_decisions` table — NOT FOUND. Schema is at v43; no migration exists. `resolve_decision` (`permissions_stream.rs:152`) fires the classifier's oneshot only; no DB write. |
| AC5 | ✅ Shipped | Design doc `§AC5` (lines 181-197) has the pre-register subsection ratified. |
| AC6 | ❌ Not started | `MIKA_DECISION_AUTHORITY__TENANT__*` / `__AGENT__*` scoping resolver — NOT FOUND. No tenant/agent scope model in `Settings`. |
| AC7 | ❌ Not started (both ends) | cm-side seed migration for `override_event` class shipped on `origin/main` at `control-monitor/backend/migrations/20260706220000_seed_override_event_event_class.up.sql`. cm-side ingest endpoint (`POST /webhooks/permission-event` or cm#99 async-emit surface) — NOT FOUND. Mika-side `cm_emit.rs` module + `MIKA_CM_OVERRIDE_EMIT_ENABLED` env — NOT FOUND. |
| AC8 | 🟢 Vacuous until AC3 lands | Compile-time default is a one-line assertion once `Settings.decision_authority: DecisionAuthority` exists with `Strict` as `Default`. |

## Coordination with mika#1731 / mika#1732

Both of my prior wire-first PRs are open on GitHub (PR#1756 mika#1731, PR#1759 mika#1732). This ticket touches `crates/mika-agent/src/server/permissions_stream.rs` + `crates/mika-common/src/config.rs` + new files. **No conflict with PR#1756 (mika-a2a crate) or PR#1759 (mika-agent server/tasks_stream.rs).** All three PRs can land in any order via the qa-chain.

## Scope

### In scope for v1 (this PR)

Same wire-first split samidarko-claude endorsed on mika#1731 and mika#1732. Ship the AC2-AC6 + AC8 delta from a coherent config+provenance boundary. Defer AC7 (async emit to cm) to a follow-up ticket because the cm-side ingest endpoint does not exist yet — shipping an emit path with no receiver is dead-code-worse than deferring.

**AC2 partial — Doctrine anchor comments at remaining in-mika-agent structural gates.** The AC2 rule verbatim: *"This agent structurally cannot do X applies to pre-classifier engine gates only, NEVER to LLM classifier decisions."* Land as inline `//` comment blocks at the remaining structural gate sites in this repo:

- `crates/mika-agent/src/skills/executor.rs` — `validate_dispatch_readiness()` (the dispatch-readiness guard).
- `crates/mika-agent/src/webhook_dispatch.rs` — `is_unauthorized_webhook_dispatch()`.

Both are pre-classifier engine gates that enforce structural bounds (agent authority, webhook source). Per architect first-pass — the comment MUST be **precise about the cross-repo boundary**, not claim "AC2 complete":

```rust
// DOCTRINE: pre-classifier structural gate
// Applies per permission-decision-protocol-2026-07-06.md §AC2
// NOTE: tier1/tier2/tier3 classifier gates live in claude-pilot-py;
//       companion anchor PR tracked in mika#<follow-up> for the cross-repo
//       completion of AC2. This annotation covers the in-mika-agent
//       structural gates only.
```

This documents the local anchor, references the protocol, and explicitly notes the cross-repo gap. AC2 is *partially* satisfied by this PR (in-mika-agent gates anchored); *fully* satisfied when the claude-pilot-py follow-up PR lands. The follow-up ticket is filed alongside this PR (mika#new-cpp) rather than blocking.

**AC3 — Server-side `decision_authority` config in `Settings`.** Adds to `crates/mika-common/src/config.rs`:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAuthority {
    /// Strict mode — operator decisions are advisory only; classifier verdict wins.
    /// This is the compile-time default per AC8.
    #[default]
    Strict,
    /// Override mode — operator decisions can flip a classifier deny. Only enabled
    /// via explicit config; NEVER via wire input (AC3 discipline).
    Override,
}
```

`Settings` gains:

```rust
pub decision_authority: DecisionAuthority,
pub permission_hold_timeout_secs: u64,  // default DEFAULT_HOLD_TIMEOUT_SECS (300)
```

Both loaded from env via `config-rs`: `MIKA_DECISION_AUTHORITY`, `MIKA_PERMISSION_HOLD_TIMEOUT_SECS`. Env parse errors are hard errors at startup (fail-closed on misconfig; matches the design doc's "fail-closed deny" discipline). The wire schema on `PermissionDecideRequest` (`#[serde(deny_unknown_fields)]`) already rejects `decision_authority` on POST — no new work there.

**AC4 — `permission_decisions` DB table + provenance write.** New schema v44 migration. Table shape:

```sql
CREATE TABLE permission_decisions (
    id TEXT PRIMARY KEY,             -- ULID or UUID
    request_id TEXT NOT NULL,        -- correlates to PermissionRequest wire frame
    tool_name TEXT NOT NULL,
    args_summary TEXT,
    classifier_verdict TEXT NOT NULL CHECK (classifier_verdict IN ('approved','denied','held')),
    operator_decision TEXT           CHECK (operator_decision IN ('approve','deny')),
    override_used INTEGER NOT NULL DEFAULT 0,  -- 0/1
    decision_authority TEXT NOT NULL CHECK (decision_authority IN ('strict','override')),
    tenant_id TEXT,                  -- null if global-scoped
    agent_id TEXT,                   -- null if tenant/global-scoped
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX idx_permission_decisions_request_id ON permission_decisions(request_id);
CREATE INDEX idx_permission_decisions_created_at ON permission_decisions(created_at DESC);
```

Additive; no rebuild of existing tables. Migration follows the `v42->v43` shape (additive ALTER / CREATE TABLE, no rebuild). Companion `AsyncDatabase::insert_permission_decision(...)` helper.

`PermissionsChannel::resolve_decision` is extended to record the provenance:

```rust
pub async fn resolve_decision(
    &self,
    db: &AsyncDatabase,
    request_id: Uuid,
    classifier_verdict: ClassifierVerdict,
    decision: OperatorDecision,
    decision_authority: DecisionAuthority,
    scope: DecisionScope,  // tenant_id + agent_id resolved at call time
) -> Result<(), ResolveError>
```

`override_used` is derived: `true` iff `classifier_verdict == Denied && decision == Approve` under `override` authority. **Under `strict` authority `override_used` is always `false` — the operator's decision is advisory only and never flips the classifier verdict** (this is the AC8 default behavior). The DB write is fire-and-forget from the classifier's perspective — the oneshot fires first, DB write runs in a `tokio::spawn` so a slow DB never blocks the classifier's continue-or-deny path.

**AC5 — Design doc already shipped.** No new content needed — the doc landed in PR#1740 with the pre-registered flip conditions subsection intact. This PR references it verbatim in code comments (AC2).

**AC6 — Scoped config resolver.** New module `crates/mika-common/src/permission_authority.rs`:

```rust
pub struct DecisionScope {
    pub tenant_id: Option<String>,
    pub agent_id: Option<String>,
}

pub fn resolve_authority(
    settings: &Settings,
    scope: &DecisionScope,
    env: &HashMap<String, String>,  // stubbed accessor; production reads from std::env
) -> DecisionAuthority {
    // 1. per-agent env: MIKA_DECISION_AUTHORITY__AGENT__<agent_id>
    // 2. per-tenant env: MIKA_DECISION_AUTHORITY__TENANT__<tenant_id>
    // 3. global setting: settings.decision_authority
    // Closest match wins.
}
```

Env-var key format uses double-underscore separators per `config-rs` convention. Agent-id and tenant-id keys are lowercase, hyphens preserved. Invalid enum values fail loud at startup. Unit tests cover the three-tier fallback chain.

**AC8 — Compile-time default STRICT + assertion test.** `Default for DecisionAuthority` returns `Strict`. Add a compile-time invariant test:

```rust
#[test]
fn default_authority_is_strict() {
    assert_eq!(DecisionAuthority::default(), DecisionAuthority::Strict);
}
```

Also a grep-discipline test (per AC8.2) asserting `override_used = true` appears only in test code and code comments — no production emit path defaults to `true`.

**AC-general tests.** Extend `permissions_stream.rs` tests:

- Serde round-trip for the extended `resolve_decision` shape.
- Fire an `Approve` decision under `Strict` authority against a `Denied` verdict — assert `override_used = false` in the DB row.
- Fire the same combination under `Override` authority — assert `override_used = true`.
- Assert the DB write is asynchronous (does not block on a `tokio::sync::Mutex` held for >10ms — matches the design doc's ≤500ms end-to-end timing).

**Docs.**

- Extend `crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md` with a §Implementation-status section referencing this PR's SHA. Bump the "Implementation" heading to reflect AC2-AC6/AC8 landed.
- Update `crates/mika-agent/CLAUDE.md` with the new `Settings.decision_authority` field, the `MIKA_DECISION_AUTHORITY` env, and the `permission_decisions` schema line.
- Update `docs/architecture.md` §14.2 SSE Frame Catalog note (introduced in mika#1732) to add a fifth axis: **decision persistence** — dashboard SSE surfaces may write to a companion DB table (`permission_decisions` for `PermissionStreamFrame`; task-event has no persistence yet).

### Out of scope for v1 (deferred)

**AC7 — `override_event` async emit to cm.** Cm-side ingest endpoint does not exist (audit confirmed: cm#99's async-emit surface not implemented; no `POST /api/v1/webhooks/permission-event`). Filing an emit path with no receiver is worse than deferring — the code is dead until cm ships. **Follow-up ticket mika#new to file alongside this PR** with the same 14-day-or-P1 time-bound commitment samidarko-claude endorsed on mika#1732 (mika#1758 mirror). Architect first-pass sharpened the follow-up ticket's shape — it MUST carry a **preparation note** for the cm-side endpoint so cm has a clear contract to implement against:

```
AC7 Follow-up (mika#<new>):
- Blocked on: cm-side ingest endpoint (cm#<TBD>)
- Reference emit shape: forward_to_cm_api @ crates/mika-gateway/src/github.rs:1132
- Acceptance: Real cm-side smoke test with 200/4xx/5xx response handling
- Emit contract: buffered queue with drop-oldest, tokio::spawn non-blocking,
  ≤500ms transport timeout, drop-with-marker counter on cm-unreachable,
  MIKA_CM_OVERRIDE_EMIT_ENABLED=0 default
- Payload shape: {record_id, tenant_id, agent_id, tool_name,
  classifier_verdict, operator_decision, override_used,
  decision_authority, created_at}
- Timeout: 14 days from this ticket or escalate to P1
```

Reference implementation: `crates/mika-gateway/src/github.rs:1132-1213` (`forward_to_cm_api`).

**AC2 companion in `claude-pilot-py`.** The tier1/tier2/tier3 classifier code is in `claude-pilot-py` — a separate repo. Doctrine anchors at those sites land in a companion PR against that repo. **Filed as follow-up mika#new-cpp** (or an equivalent `claude-pilot-py#new`).

**Enable-flip toggle.** Per ticket §Not-in-scope, the actual `override` mode is not exposed even for testing — the config field exists, the enum variant exists, but no runtime path exercises `Override` mode. Awaits Vincent's formal ratification per the design doc's §Founder-question-restated. This preserves the AC8 "shipped default STRICT + override OFF regardless" invariant.

**Cursor replay** on the SSE stream. Not part of the design doc's contract.

**Vincent's numeric `N` for pre-register condition 2.** Design doc leaves this as provisional `N=3`; formal ratification happens separately (not blocking this PR).

## Implementation guardrails

### File and function targets

| Change | File | Notes |
|---|---|---|
| `DecisionAuthority` enum + `Default` impl | `crates/mika-common/src/config.rs` | Near `Settings` struct |
| `Settings.decision_authority: DecisionAuthority` field | `crates/mika-common/src/config.rs` | Alongside existing fields |
| `Settings.permission_hold_timeout_secs: u64` field | Same | |
| Config keys registry entries | `crates/mika-agent/src/config_keys.rs` | If it enumerates keys |
| Scoped resolver module | `crates/mika-common/src/permission_authority.rs` | New file |
| `permission_decisions` v44 migration | `crates/mika-agent/src/db.rs` | Follows the v42→v43 additive shape |
| `AsyncDatabase::insert_permission_decision` helper | `crates/mika-agent/src/async_db.rs` (or equivalent) | Async wrapper around `Database::insert_permission_decision` |
| Extend `PermissionsChannel::resolve_decision` signature | `crates/mika-agent/src/server/permissions_stream.rs` | Threads through classifier verdict + authority + scope; writes DB record |
| AC2 doctrine anchor comment | `crates/mika-agent/src/skills/executor.rs` | On `validate_dispatch_readiness()` — inline `//` comment block |
| AC2 doctrine anchor comment | `crates/mika-agent/src/webhook_dispatch.rs` | On `is_unauthorized_webhook_dispatch()` — inline `//` comment block |
| Design doc §Implementation-status update | `crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md` | Append PR SHA + AC status table |
| `crates/mika-agent/CLAUDE.md` update | Same | Settings + schema line |
| Architecture §14.2 update | `docs/architecture.md` + doc-synced | Fifth axis (persistence) |

### `PermissionsChannel::resolve_decision` new signature (no wrapper — mika-arch first-pass F1)

```rust
pub async fn resolve_decision(
    &self,
    db: Option<&AsyncDatabase>,           // None in tests where DB is not wired
    request_id: Uuid,
    classifier_verdict: ClassifierVerdict, // From the request the operator is deciding
    decision: OperatorDecision,
    tool_name: &str,
    args_summary: Option<&str>,
    authority: DecisionAuthority,
    scope: DecisionScope,
) -> Result<(), ResolveError>
```

**No wrapper.** Per architect first-pass F1 (BLOCKING): eliminate the two-path seam. A thin `resolve_decision_legacy` wrapper would create a silent-data-loss footgun — a future caller who reaches for the "convenient" 2-arg version thinking they get provenance logging silently gets none. The 8 existing unit tests at `permissions_stream.rs:274-374` will be updated mechanically as part of this PR: pass `Strict` authority, empty `DecisionScope`, and either a test `AsyncDatabase` fixture or `None` for tests that genuinely don't care about persistence. Mechanical refactor path per architect:

1. Rewrite `resolve_decision` with the full provenance signature.
2. Update `handle_permission_decide` (`permissions_stream.rs:231-266`) to construct the full call — this handler is where the classifier_verdict lookup + config resolution happens.
3. Update the 8 existing unit tests in-place. `channel_resolves_pending_decision`, `channel_rejects_unknown_request_id`, `channel_reports_classifier_dropped` all pass `Strict` + empty scope + `None` DB.
4. No `resolve_decision_legacy`; no compat surface.

Estimated added mechanical churn from removing the wrapper: ~30 minutes of straightforward test edits; saves permanent footgun.

### Migration discipline

- Additive ALTER + CREATE TABLE. No rebuild of existing tables.
- Follows the `v42->v43` shape (additive columns, no rebuild).
- `column_exists`/`table_exists` guards for crash-recovery safety per the mika-agent convention.
- Bumps `CURRENT_SCHEMA_VERSION` from 43 to 44 at `crates/mika-agent/src/db.rs:30`.

### Config precedence order (per AC6)

1. `MIKA_DECISION_AUTHORITY__AGENT__<agent_id>=strict|override`
2. `MIKA_DECISION_AUTHORITY__TENANT__<tenant_id>=strict|override`
3. `MIKA_DECISION_AUTHORITY=strict|override` (global; also `decision_authority` in config.toml)
4. Compile-time default: `DecisionAuthority::Strict`

`config-rs` supports double-underscore separators via `Config::with_prefix("MIKA").separator("__")`. The resolver reads the env vars directly (not via `Settings`) to preserve dynamism — Settings is a snapshot at startup. Startup validation asserts all `MIKA_DECISION_AUTHORITY*` env vars parse successfully (per AC3.2 hard error).

### DB-write ordering discipline

The classifier's `oneshot::Sender` fires FIRST (unblocks the classifier's continue-or-deny path). The DB write runs in a spawned task afterwards — the design doc's ≤500ms budget applies to the classifier's oneshot, not to the DB write. If the DB is slow, the classifier is unaffected; if the DB is unreachable, the write fails silently (matches AC7's fire-and-forget spirit even though AC7 targets cm, not the local DB).

## Acceptance criteria

**AC1.** No change (already shipped in PR#1741).

**AC2 (partial for this PR).** Doctrine-anchor comment blocks land at `crates/mika-agent/src/skills/executor.rs::validate_dispatch_readiness` and `crates/mika-agent/src/webhook_dispatch.rs::is_unauthorized_webhook_dispatch`, cross-referencing `permission-decision-protocol-2026-07-06.md §AC2` and `mika#1193`. The claude-pilot-py-side companion PR is filed as a follow-up ticket.

**AC3.** `Settings.decision_authority: DecisionAuthority` (default `Strict`) and `Settings.permission_hold_timeout_secs: u64` (default 300) are added. Env vars `MIKA_DECISION_AUTHORITY` / `MIKA_PERMISSION_HOLD_TIMEOUT_SECS` load them; invalid values fail startup. `PermissionDecideRequest` (already `#[serde(deny_unknown_fields)]`) continues to reject `decision_authority` on the POST body — regression-tested.

**AC4.** Schema v43→v44 migration creates the `permission_decisions` table with the columns above. `AsyncDatabase::insert_permission_decision` helper writes the provenance record. `PermissionsChannel::resolve_decision` calls the helper AFTER firing the classifier oneshot. `override_used` is derived correctly: `true` iff classifier=`denied` && decision=`approve` && authority=`override`; `false` otherwise.

**AC5.** Design doc `permission-decision-protocol-2026-07-06.md` gains an §Implementation-status subsection referencing this PR's SHA. No new content in the pre-register subsection (already shipped in PR#1740).

**AC6.** New `permission_authority::resolve_authority` reads env vars in the precedence order agent > tenant > global; unit tests cover all three tiers plus the compile-time-default fallback. Config for `tenant=T1 override` does NOT affect tenant `T2` in the tests.

**AC7 (deferred).** Follow-up ticket filed with the 14-day-or-P1 commitment. Reference implementation pointed at `crates/mika-gateway/src/github.rs::forward_to_cm_api` for the emit shape; ingest endpoint blocked on cm side.

**AC8.** `DecisionAuthority::default() == Strict` is asserted by a unit test. A grep-discipline test asserts `override_used = true` appears only in test code and code comments (rg-based; test in `tests/`).

**AC-general.** `cargo build`, `cargo test -p mika-agent --lib permissions_stream permission_authority`, and `cargo clippy --workspace --all-targets -- -D warnings` all pass.

## Verification steps (post-implementation)

1. `cargo test -p mika-agent --lib permissions_stream` — extended tests + regression on existing 8 tests green.
2. `cargo test -p mika-common --lib permission_authority` — new resolver tests green.
3. `cargo test -p mika-agent --lib default_authority_is_strict` — AC8 invariant.
4. `cargo clippy --workspace --all-targets -- -D warnings` — clean.
5. Manual (post-merge): with `MIKA_DECISION_AUTHORITY=strict` set, run the full permission decision flow (SSE subscribe + POST decide) locally; verify `permission_decisions` DB row is written with `override_used=0`, `decision_authority='strict'`. Repeat with `MIKA_DECISION_AUTHORITY=override` — verify `override_used=1` when the operator approves a classifier deny. **Test with `override` mode requires a temporary config bump; the shipped default remains `strict` per AC8.**

## Rollout

- Merge to `main` → next `make deploy` picks it up. No cluster ops.
- No consumer yet — the `permission_decisions` table starts empty until a classifier goes through the operator-decide path (which requires a live consumer connecting to the SSE; the TUI thin-client refactor in mika#1727 is the intended one).
- Cm-side emit remains dark (AC7 follow-up).
- Watch: after mika#1727 lands, grep the `permission_decisions` table by `override_used=1` to verify that the flip is exercised only under `override` authority (should be zero rows in shipped `strict` default).

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| `Settings` struct field cascade — many `Settings::new()` construction sites need updating. | Grep-verify all construction sites (test factories, integration test scaffolds); mechanical `decision_authority: DecisionAuthority::Strict` at each. |
| v44 migration on production DB fails with lock contention. | Migration is additive-only (CREATE TABLE + CREATE INDEX). No ALTER on existing rows. Fail-closed at startup per convention. |
| AC7 follow-up is filed but never landed → dead-code accumulation. | 14-day-or-P1 time-bound commitment on the follow-up ticket, mirrored from mika#1758 architect sharpening. Cross-linked in this PR body via `Tracked in:`. |
| AC2 doctrine anchor comments feel weak because the actual classifier is in another repo. | The `skills/executor.rs` + `webhook_dispatch.rs` gates ARE structural pre-classifier gates — the doctrine rule applies to them regardless of where the LLM classifier lives. The comments cross-reference the design doc + mika#1193 for full context. Additional cross-repo anchors go in the claude-pilot-py follow-up. |
| Enabling `override` mode later without a config-migration path. | The env var + `Settings` field is a switch, not a data-structure change. Flip is a config edit + restart. Ratification lands as a separate ticket per AC8. |
| Scope resolver adds complexity for a single-tenant deploy. | Global default is `Strict`; agent/tenant tiers stay unused until multi-tenant becomes real. Complexity is 100 lines of resolver + tests. YAGNI vs pre-committed scope-as-first-class-axis (Sharpening 5) resolves toward pre-commit. |

## Files changed (expected)

- `crates/mika-common/src/config.rs` — `DecisionAuthority` enum + `Settings` fields. ~40 lines added.
- `crates/mika-common/src/permission_authority.rs` — new module (resolver + tests). ~150 lines.
- `crates/mika-common/src/lib.rs` — `pub mod permission_authority;`. 1 line.
- `crates/mika-agent/src/db.rs` — v44 migration (`CURRENT_SCHEMA_VERSION` bump + CREATE TABLE + CREATE INDEX + `insert_permission_decision`). ~80 lines.
- `crates/mika-agent/src/async_db.rs` — `AsyncDatabase::insert_permission_decision` helper. ~20 lines.
- `crates/mika-agent/src/server/permissions_stream.rs` — extended `resolve_decision` signature + DB write + tests. ~120 lines added.
- `crates/mika-agent/src/skills/executor.rs` — doctrine anchor comment (~10 lines).
- `crates/mika-agent/src/webhook_dispatch.rs` — doctrine anchor comment (~10 lines).
- `crates/mika-agent/docs/permission-decision-protocol-2026-07-06.md` — §Implementation-status. ~20 lines.
- `crates/mika-agent/CLAUDE.md` — Settings + schema. ~10 lines.
- `docs/architecture.md` — §14.2 fifth-axis addition. ~15 lines.
- `crates/mika-agent/src/config_keys.rs` — env key entries. ~10 lines.
- Update all `Settings::new()` / `Settings::test_defaults()` construction sites — mechanical `decision_authority: DecisionAuthority::Strict, permission_hold_timeout_secs: 300`. Grep-driven; ~30 lines across ~5-10 sites.

Estimated diff: ~500 net lines added.

## Grooming history

- 2026-07-10 — `/ce:plan` draft (with pre-groom verification pass covering all 8 ACs + cm-side dependency check).
- 2026-07-10 — `mika-arch` first-pass review (session `d8597217-476d-4e76-8c9d-69b943e5f9b2`): **Disposition: ITERATE**. One BLOCKING F1 concern — the `resolve_decision_legacy` thin wrapper introduces a two-path silent-data-loss seam. Three revisions applied to the plan:
  1. Eliminated the wrapper — full-signature refactor with mechanical test edits per architect's step-by-step path (rewrite → update `handle_permission_decide` → update 8 unit tests in-place → no legacy variant).
  2. AC2 doctrine anchor comment format sharpened to be explicit about the cross-repo boundary (NOTE line naming the claude-pilot-py gap + follow-up ticket).
  3. AC7 follow-up ticket shape sharpened to carry a preparation note (contract for the cm-side endpoint: reference emit shape, acceptance criteria, payload shape, 14-day timeout).
- 2026-07-10 — `mika-arch` second-pass review (same session): **Verdict: GROOMED**. All three revisions confirmed satisfying pass-1 concerns. Gates green: Unresolved-Decision Gate (no TBD/placeholder tokens), Acceptance-Criteria Gate (AC1-AC8 + AC-general present). Architect: "The plan is cleanly implementable as-written."
