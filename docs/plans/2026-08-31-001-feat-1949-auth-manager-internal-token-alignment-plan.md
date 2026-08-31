---
issue: mika#1949
title: INTERNAL_TOKEN Alignment Across the Manager Write Path - Plan
type: feat
scope_repo: mika
companion_repo: control-monitor
priority: p1-important
date: 2026-08-31
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
product_contract_source: ce-plan-bootstrap
execution: code
---

# INTERNAL_TOKEN Alignment Across the Manager Write Path - Plan

## Goal Capsule

**Objective.** When a cross-boundary call in the manager write path fails to
authenticate, the operator can tell *which* token failed, *at which boundary*,
and *why* — from the report and from `audit_events`, without reading a log or
attaching a debugger. Today that failure is indistinguishable from a network
error.

**Means.** Add an observation layer — a shared `AuthBoundaryError` shape and an
`auth_boundary` audit row at each of the four boundary sites — plus a rotation
runbook. Keep four distinct tokens (KTD1).

**Authority hierarchy.** Issue ACs > this plan > implementer judgment. AC9 is
inviolable: no write authority is added to `milestone_manager`, and
`no_dispatch_test.rs` FORBIDDEN_TOKENS is unchanged.

**Stop conditions.**
- Stop and escalate if any change would add a token-minting or
  token-transformation path (see KTD4 and the escalation table in Planning
  Contract § Classifier exposure).
- Stop if `cargo test -p mika-agent no_dispatch` fails: AC9 has been breached.
- Stop if the status-code arbitration (KTD2) cannot be settled without changing
  an existing caller's behavior.

**Execution profile.** Cross-repo, primary `mika`, direct secondary change in
`control-monitor`, doc in `mika-platform`. Sequential units; U5 and U6 are the
only ones expected to need operator escalation.

**Tail ownership.** PR on `mika` first, companion change on `control-monitor`
second, cross-referenced per the meta-repo convention.

## Product Contract

### Summary

Four env-var-backed tokens guard the manager write path. None of them leaves a
trace when it fails. This plan makes each cross-boundary authentication attempt
observable — a typed error naming the token and the boundary, and an
`audit_events` row — and writes the operator rotation procedure. It does not
unify the tokens and does not grant the manager any new authority.

### Problem Frame

Phase 2 dispatch authority for `mika-manager` is gated behind three portes.
This is Porte 3.

The manager's write path crosses four boundaries, each guarded by a different
env var with its own rotation, scope, and audit semantics:

| Token | Read at | Guards |
|---|---|---|
| `MIKA_INTERNAL_TOKEN` | `crates/mika-gateway/src/settings.rs:32`, hex-validated `:171` | gateway to spirit |
| `INTERNAL_TOKEN` | `control-monitor/backend/crates/cm-api/src/sink_builder.rs:106`, read `:122` | cm to spirit (A2A) |
| `CM_FULL_ACCESS_TOKEN` | `control-monitor/backend/crates/cm-api/src/scope.rs:95`, read `:131` | cm content-plane scope |
| `MIKA_MANAGER_DELIVERY_TOKEN` | `crates/mika-agent/src/milestone_manager/spawn.rs:76` | manager report delivery |

The failure is not that there are four. It is that a failure at any of them is
silent and shapeless. `mika#2013` is the measured precedent for the class: a
GitHub App installation token froze at spawn, the manager cycled
`manager_cycle_error auth_class=401` **sixteen times in one night**, and
nothing surfaced which credential had expired. That was a different token; the
observability gap is the same one, and it is still open on all four.

Two facts found in the code shape this plan more than the issue text does.

**First: the two sides already disagree on the refusal status code, by
decision on each side.** `control-monitor/backend/crates/cm-api/src/routes/permission_events.rs:104`
states it outright — "cm#99 specifies 401 for bad auth. This answers 403,
matching every other token refusal in this codebase." Meanwhile
`crates/mika-agent/src/server/auth.rs:33` answers `401` with
`{"error": "unauthorized"}`. "Failure-mode parity" is therefore an
arbitration, not a refactor (KTD2).

**Second: the work is asymmetric.** cm already distinguishes *unset* from
*empty*, names the entity, and names the file to edit
(`sink_builder.rs:131-146`). mika's `auth.rs:33` names nothing. The gap is
mostly on the mika side, and the cm pattern is the model to copy — not a
new invention.

### Key Decisions

- **Keep four tokens; add a ledger and a runbook.** Unification to a derived
  per-boundary secret is deferred to a Phase 3 refactor. Governs R1, R2, R7.
- **Observation only; no authority change.** Every unit adds reads, error
  shapes, audit rows, or docs. None adds a write capability to the manager.
  Governs R8.

### Requirements

**Error shape and parity**

- R1. A shared `AuthBoundaryError { token_name, from, to, kind }` type exists,
  with `kind` one of `Missing | Empty | Invalid | Rejected | Unreachable`, and
  is serializable.
- R2. Each of the four boundary sites returns `AuthBoundaryError` on
  authentication failure, naming the token by **name**, never by value.
- R3. A refusal response body names the failing token and boundary in
  structured JSON. It never echoes a token value, a prefix, or a length.

**Observability**

- R4. Every cross-boundary authentication attempt on the manager write path
  writes an `audit_events` row with `tool_name = 'auth_boundary'`,
  `target_key = '<from>_to_<to>'`, and an `event_reason` carrying the token
  name and the outcome.
- R5. The audit write is fire-and-forget: an audit failure never changes the
  request's outcome, and an authentication failure still drops the request
  exactly as today.
- R6. The manager `Reporter` renders an `AuthBoundaryError` at severity
  `Attention`, or `Blocked` on repetition, naming the token and the boundary.

**Operations**

- R7. An operator runbook covers rotation of all four tokens: order, which
  service restarts, which check confirms the new value took effect, and the
  visible symptom of each skipped step.

**Invariants**

- R8. `milestone_manager` gains no write authority. `no_dispatch_test.rs`
  FORBIDDEN_TOKENS is byte-identical after this work.

### Scope Boundaries

In scope: the four boundaries the manager write path crosses; the error shape;
the audit rows; the rotation runbook; the cm role decision.

Out of scope, per the issue: token unification to a derived secret; automated
rotation via a secrets manager; hardware attestation; extending
`AuthBoundaryError` to skill-exec handlers or MCP servers.

Out of scope, added here: changing the status code any *existing* caller
receives. KTD2 settles the convention going forward; it does not retrofit
callers, because `permission_events.rs:104` records that its 403 was chosen
deliberately and a caller is documented not to read the status.

### Acceptance Examples

- AE1. `INTERNAL_TOKEN` is unset. cm attempts an A2A dispatch. The caller
  receives an `AuthBoundaryError` with `kind = Missing`,
  `token_name = "INTERNAL_TOKEN"`, `from = "cm"`, `to = "spirit"`. An
  `audit_events` row exists with `target_key = 'cm_to_spirit'`. Covers R1, R2, R4.
- AE2. `MIKA_INTERNAL_TOKEN` is set to a wrong value. The gateway calls the
  spirit. The spirit refuses. The body names `MIKA_INTERNAL_TOKEN` and the
  `gateway_to_spirit` boundary, and contains no fragment of either token
  value. Covers R2, R3.
- AE3. The audit-events writer is made to fail. The authentication path still
  refuses the request, and no panic and no changed status code result.
  Covers R5.
- AE4. The manager cycle takes three consecutive `Rejected` outcomes on
  `MIKA_MANAGER_DELIVERY_TOKEN`. The report renders at `Blocked`, naming the
  token. Covers R6.

### Sources

- Issue: `senara-solutions/mika#1949`
- Brief: `mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md § 3 Porte 3`
- Precedent for the class: `docs/solutions/logic-errors/2013-token-resolved-once-at-spawn-freezes-a-renewable-credential.md` (mika#2013, closed 2026-08-29)
- The cm pattern to copy: `control-monitor/backend/crates/cm-api/src/sink_builder.rs:121-146`
- The status-code divergence: `control-monitor/backend/crates/cm-api/src/routes/permission_events.rs:104-109` vs `crates/mika-agent/src/server/auth.rs:27-40`
- The four sites: table under Problem Frame

## Planning Contract

### Key Technical Decisions

- KTD1. **Four tokens stay; add a ledger.** Deriving four tokens from one root
  secret would put a key-derivation path into the classifier-blockable surface
  for no Porte-3 benefit. Governs R1, R2, R7.
- KTD2. **This plan delivers distinguishability, not status-code parity — and
  claims that as the decision.** The issue asks for both: point 1 asks for a
  coherent auth surface, point 4 asks for distinguishable errors. Where they
  pull apart, distinguishability wins and the divergence is documented. Stated
  plainly so no reader mistakes it for an oversight: **cm answers 403 and mika
  answers 401, and after this work they still will.** What changes is that both
  bodies name the token and the boundary. The refusal convention is arbitrated,
  not retrofitted. New and
  changed refusal paths in this work answer with the structured
  `AuthBoundaryError` body. Existing callers keep the status code they get
  today — cm's 403 and mika's 401 both stand. The runbook records the
  divergence and its two rationales so the next reader does not re-derive it.
  Rationale: `permission_events.rs:104` records the 403 as a deliberate
  house-convention choice; silently flipping it would change a shipped
  contract for a documentation benefit.
- KTD3. **`AuthBoundaryError` lives in `mika-common`, and cm gets a mirror,
  not a dependency.** `control-monitor` is a separate repo and must not take a
  build dependency on `mika-common` for one error enum. cm keeps its
  `ApiError` variants and gains the same four fields in its message shape; a
  serialization test in each repo pins the two to one JSON shape.
- KTD4. **The token-comparison code is not touched.** `token_matches` and
  `extract_bearer` in `crates/mika-agent/src/server/auth.rs` keep their
  constant-time comparison and their current signatures. This work wraps the
  *refusal branch*, never the comparison. This is both a safety property and
  the way the classifier exposure is kept to one unit.
- KTD5. **The manager stays `Role::CommandPlane`. No new role variant.**
  Measured, not deferred: `control-monitor/backend/crates/cm-api/src/scope.rs`
  `check()` already returns `Ok(())` for `["messages", "dispatch"]` — the test
  `the_command_plane_still_works` pins it. The same function returns `Err` for
  `["permission-events"]`, and the code states why: *"permission events are an
  audit corpus, written only by the pilot"*, because *"a caller that could forge
  permission decisions could make a denial look like an allow after the fact"*.
  The test `check(Role::CommandPlane, "/permission-events").is_err()` pins that
  too. The issue's proposed `Role::Manager` allowlist is exactly those two
  routes — one already permitted, one deliberately forbidden as an anti-forgery
  boundary. Creating the role would reopen a hole cm closed on purpose, to gain
  a route the manager already has. So: no new variant. The manager's audit trail
  runs through mika's own `audit_events` (U2), never through cm's
  `/permission-events`. Governs R2, R4, and issue AC1.
- KTD6. **Fail-closed on an auth failure is the correct behaviour, not a gap.**
  When a boundary token fails, the manager's write does not happen. That is the
  property Porte 3 protects, not a defect this plan leaves open. What Porte 3
  adds is that the operator learns *which* token failed instead of reading a
  network error. A plan that made writes proceed past a failed credential would
  breach the gate it is meant to discharge.
- KTD7. **Audit rows are written by the existing `audit_events` writer.** No
  new table, no new migration. `tool_name` is a free-text column already
  carrying distinct values; `'auth_boundary'` joins them.

### Classifier exposure — where escalation is expected

This ticket edits authentication code. The Claude Code permission classifier
firm-blocks MITM-shaped edits, and measured evidence shows a session that takes
a denial is a session that was doing real work. The exposure is therefore
concentrated deliberately, not spread:

| Unit | Touches auth decision code | Expected disposition |
|---|---|---|
| U1 error type | no | auto |
| U2 audit writer | no | auto |
| U3 manager delivery site | no — reads `cfg.delivery_token`, does not validate | auto |
| U4 reporter rendering | no | auto |
| U5 spirit refusal branch | **yes** — `server/auth.rs` refusal arm | **escalate to operator** |
| U6 cm role test + docstring | no — adds a test, does not touch `check()` (KTD5) | auto |
| U7 runbook | no | auto |
| U8 doc callouts | no | auto |

An implementer who is denied on U5 must stop, report the denial with the file
and the intended arm, and let the operator decide. Do not restructure the change
to slip past the classifier. U5 is the **only** unit that edits an
authentication decision path, so a denial there blocks one unit, not nine.

### High-Level Technical Design

```
manager cycle ──deliver_report──> delivery endpoint     [MIKA_MANAGER_DELIVERY_TOKEN]  U3
       cm ─────A2aAdapter────────> spirit /a2a          [INTERNAL_TOKEN]               U2
  gateway ─────HTTP──────────────> spirit /message      [MIKA_INTERNAL_TOKEN]          U5
       cm ─────scope guard───────> content plane        [CM_FULL_ACCESS_TOKEN]         U6

each arrow, on failure:  AuthBoundaryError (U1)  +  audit_events row (U2)
                                   │
                                   └──> Reporter renders severity (U4)
```

### Assumptions

- The `audit_events` schema accepts a new `tool_name` value with no migration.
  Verify before U2 with `grep -n 'INSERT INTO audit_events' crates/mika-agent/src/db.rs:2338`.
- `control-monitor` builds independently of `mika`. Confirmed: separate repo,
  separate workspace.

### Corrected premises

The issue's design section B names `A2aSink` as the cm outbound site. **No such
type exists** — `grep -rl 'A2aSink' --include='*.rs' control-monitor/` returns
0 files, against a positive control of `ApiError` = 21 files. The real type is
`A2aAdapter` at `control-monitor/backend/crates/cm-adapter/src/a2a.rs:80`,
constructed at `cm-api/src/sink_builder.rs:68`. Units name the real site.

The issue cites `MIKA_MANAGER_DELIVERY_TOKEN` at `spawn.rs:46`; it is at
`spawn.rs:76`. Symbol correct, line drifted.

### Sequencing

U1 first — every other unit depends on the type. U2 next — the audit helper is
used by U3, U5, U6. U3 and U4 then run without further dependency. U5 and U6 are
the escalation-exposed pair and run last among code units, so a block there
leaves seven units landed. U7 and U8 are documentation and close the ACs.

## Implementation Units

### U1. `AuthBoundaryError` type

**Goal.** One serializable error shape naming a token, a boundary, and a kind.

**Requirements.** R1, R2.

**Files.** `crates/mika-common/src/auth_boundary.rs` (new);
`crates/mika-common/src/lib.rs` (module declaration).

The issue's AC3 says `mika-common/src/errors.rs` "or equivalent shared
location". There is no `errors.rs` in that crate — the module list is
`agent, build_info, claude, config, dotenv, embedding, github_app,
github_event_format, home, llm, logging, mcp_config_path, oauth,
permission_authority, telemetry, text, trace, validation`. A dedicated
`auth_boundary.rs` is the equivalent location; do not create `errors.rs` for
one type.

**Approach.** A struct with four fields and a `kind` enum of five variants per
R1. Derive `Serialize`/`Deserialize`. `Display` renders
`"<kind> at <from>-><to> boundary (token: <token_name>)"`. No field holds a
token value; add a unit test asserting the `Display` and JSON output of an
instance built with a token name that is also a plausible secret contain the
**name** only.

**Test scenarios.** Round-trip serialization for all five kinds. `Display`
shape. Negative: an instance never renders a value it was not given.

**Verification.** `cargo test -p mika-common auth_boundary`.

### U2. `auth_boundary` audit helper

**Goal.** One fire-and-forget helper that writes the audit row, so the four
call sites do not each re-derive the row shape, and one documented row shape.

**Requirements.** R4, R5, and issue AC2's schema-documentation clause.

**Files.** `crates/mika-agent/src/db.rs` (helper next to the existing
`audit_events` writer at `:2338`); a new call-site-facing wrapper in the module
that owns boundary calls; `crates/mika-agent/CLAUDE.md § Audit Log` (existing
section at `:749`).

**Approach.** `fn record_auth_boundary(&self, err: &AuthBoundaryError) ->
Result<()>` writing `tool_name = "auth_boundary"`,
`target_key = format!("{}_to_{}", err.from, err.to)`, and an `event_reason`
carrying token name and kind. Call sites ignore the `Result` deliberately and
log at `warn` on error; document why on the function.

Document the row shape in `crates/mika-agent/CLAUDE.md § Audit Log`
(`:749`): the `tool_name` value, the `target_key` grammar
`<from>_to_<to>`, the four boundary pairs it can take, and the `event_reason`
fields. AC2 requires this documentation; the unit is not done without it.

**Test scenarios.** A row is written with the exact `tool_name` and
`target_key`. An injected writer failure does not propagate. `event_reason`
contains the token name and never a value.

**Verification.** `cargo test -p mika-agent auth_boundary`.

### U3. Manager delivery boundary

**Goal.** The manager's report POST reports its auth outcome.

**Requirements.** R2, R4.

**Files.** `crates/mika-agent/src/milestone_manager/spawn.rs` around `:958`
(`sink.send(url, cfg.delivery_token.as_deref(), &body)`);
`crates/mika-agent/src/milestone_manager/cadence.rs`.

**Approach.** Map the send outcome to an `AuthBoundaryError` kind: absent token
to `Missing`, empty to `Empty`, HTTP 401/403 to `Rejected`, connection refused
to `Unreachable`, other transport to a non-auth error left as today. Call the
U2 helper. This site **reads** a token from config; it does not validate one,
so it is outside KTD4's frozen surface.

**Test scenarios.** Each of the five kinds produced from a mocked sink. AC9
guard: the diff adds no token from `no_dispatch_test.rs` FORBIDDEN_TOKENS.

**Verification.** `cargo test -p mika-agent milestone_manager`, then
`cargo test -p mika-agent no_dispatch`.

### U4. Reporter severity rendering

**Goal.** An auth failure reaches the operator's report in words.

**Requirements.** R6.

**Files.** `crates/mika-agent/src/milestone_manager/reporter.rs`.

**Approach.** Render an `AuthBoundaryError` at `Severity::Attention`; on a
third consecutive occurrence of the same `(token_name, boundary)` pair within
one cycle window, render at `Blocked`. The repetition counter is per-cycle
in-memory state; it does not persist.

**Test scenarios.** One occurrence renders `Attention`. Three consecutive
render `Blocked`. Two occurrences on *different* boundaries do not escalate.

**Verification.** `cargo test -p mika-agent reporter`.

### U5. Spirit refusal branch — ESCALATION EXPECTED

**Goal.** The spirit's 401 names the token and the boundary.

**Requirements.** R2, R3, R4.

**Files.** `crates/mika-agent/src/server/auth.rs`, refusal arms at `:33` and
in `require_dashboard_or_internal_token`.

**Approach.** Replace the `{"error": "unauthorized"}` body with the
`AuthBoundaryError` JSON. **Keep the 401 status** (KTD2). **Do not touch
`token_matches` or `extract_bearer`** (KTD4) — the change is confined to the
`_ =>` refusal arm and its response construction. Call the U2 helper before
returning.

This unit edits authentication-decision code and is expected to draw a
permission denial. On denial: stop, report the file, the arm, and the intended
body shape, and hand to the operator. Do not restructure to avoid the
classifier.

**Test scenarios.** A wrong token yields 401 with a body naming
`MIKA_INTERNAL_TOKEN` and `gateway_to_spirit`. The body contains no substring
of either the presented or the expected token. An audit row is written. A
*correct* token still passes with the response unchanged — the negative
control that proves the refusal path, not the accept path, was changed.

**Verification.** `cargo test -p mika-agent auth`, plus the negative control
above stated as its own named test.

### U6. Pin the cm role decision with a negative regression test

**Goal.** The role question is settled by KTD5 — `CommandPlane`, no new variant.
This unit makes that decision hold against a future Phase 2 change, and records
it where the next reader finds it.

**Requirements.** R2, R4.

**Files.** `control-monitor/backend/crates/cm-api/src/scope.rs` (enum at `:67`,
guard around `:131`).

**Approach.** Add a named test —
`manager_write_path_needs_no_new_role` — asserting both halves of KTD5 in one
place: `check(Role::CommandPlane, "/messages/dispatch").is_ok()` and
`check(Role::CommandPlane, "/permission-events").is_err()`. The existing tests
assert each separately; this one states the *decision*, so a future change that
widens the boundary to give the manager an audit route goes red against a test
whose name says why.

Then extend the `scope.rs` module docstring: the manager authenticates as
`CommandPlane`; it does not get `/permission-events`; its audit trail is mika's
`audit_events`, not cm's audit corpus.

**Do not add a `Role` variant.** **Do not modify `check()`.** This unit adds a
test and a docstring only, which is why it is no longer on the escalation-
exposed list — the guard itself is untouched.

**Test scenarios.** The new test passes on today's code. Verify it is not
vacuous: temporarily add `["permission-events"] => Ok(())` to `check()` and
confirm the test goes red, then revert. Record that check in the PR body.

**Verification.** `cargo test -p cm-api scope` in `control-monitor/`.

### U7. Rotation runbook

**Goal.** An operator can rotate any of the four tokens without guessing.

**Requirements.** R7.

**Files.** `mika-platform/docs/operator/token-rotation-procedure.md` (new).

**Approach.** One section per token: where it is set, what reads it, rotation
order, which service restarts, which check confirms the new value took effect.
Then a table: **if you skip step N, the visible symptom is X**. Record the
KTD2 status-code divergence and both rationales. Cite `verify-deploy` for the
`INTERNAL_TOKEN` check that already exists.

**Test scenarios.** None — documentation. Reviewer confirms each of the four
tokens has a named confirming check, and that no token *value* appears.

**Verification.** Manual review against the four rows of the Problem Frame table.

### U8. Doc callouts and porte discharge

**Goal.** The Phase 2 gate note points at the proof.

**Requirements.** R7, and issue AC6, AC7.

**Files.** `crates/mika-agent/src/milestone_manager/mod.rs` docstring;
`mika-platform/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md § 3 Porte 3`.

**Approach.** In `mod.rs`, name mika#1949 as the Porte 3 discharge condition and
give the post-deploy audit query. In the brief, add
`**Statut : DISCHARGED**` naming the ticket, the PR, the role decision from U6,
and the runbook path.

**Test scenarios.** None.

**Verification.** `cargo test -p mika-agent no_dispatch` still passes — the
docstring edit must not disturb the structural test that reads this file.

## Verification Contract

- `cargo test -p mika-common auth_boundary`
- `cargo test -p mika-agent auth_boundary milestone_manager reporter auth`
- `cargo test -p mika-agent no_dispatch` — **AC9 gate.** Must pass, and the
  diff for `crates/mika-agent/src/milestone_manager/no_dispatch_test.rs`
  FORBIDDEN_TOKENS must be empty. Verify with
  `git diff main -- crates/mika-agent/src/milestone_manager/no_dispatch_test.rs`.
- `cargo clippy --all-targets -- -D warnings`
- In `control-monitor/`: `cargo test -p cm-api scope`
- Secret hygiene: `scripts/check-secrets.sh` must pass. No test fixture may
  contain a real-shaped 64-hex token.

**Post-deploy verification (operator-driven, per issue AC8).** Run this in a
**rehearsal environment, never in production.** Unsetting a live boundary token
suspends the manager write path for as long as it is unset, and the issue does
not name a rollback. Procedure: bring up the manager against a scratch DB with
the four tokens set, then unset each one at a time — restoring it before the
next — and after each, run one manager cycle and confirm a distinct
`audit_events` row with the correct `token_name`, plus the report rendering at
the expected severity. Exit criterion: four rows, four distinct `target_key`
values, no token value present in any of them. Then:
`SELECT target_key, count(*) FROM audit_events WHERE tool_name = 'auth_boundary' GROUP BY 1;`
This is listed in the PR body; it is not a merge gate.

## Definition of Done

**Global.**
- R1 through R8 satisfied, each traced to a landed unit.
- AC9 verified by an empty diff on `no_dispatch_test.rs` FORBIDDEN_TOKENS.
- No token value appears in any source file, test fixture, doc, or audit row.
- Both PRs opened and cross-referenced (`Companion PR: senara-solutions/<repo>#<n>`).
- Abandoned approaches removed from the diff.

**Per unit.** Each unit's Verification passes, and its named negative control
(where it has one — U1, U2, U5) passes.

**Blocked-unit protocol.** If U5 is blocked by the permission classifier, the
PR ships U1-U4 and U6-U8, states in its body which unit is blocked and on which
file and arm, and the issue stays open with the remaining ACs named. A partial
Porte 3 is honest; a Porte 3 marked DISCHARGED with U5 blocked is not — U8's
brief edit must not be written until U5 has landed or been explicitly waived by
the operator.
