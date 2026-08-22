---
title: "feat(notifications): severity-tiered operator notifications — P0 'loop down' on a never-muted surface"
type: feat
status: active
date: 2026-08-22
ticket: senara-solutions/mika#1381
branch: feat/1381/notifications-severity-tiered-operator
milestone: senara-solutions/mika#30 — Loop Trustworthiness (10/11 done; #1381 last unstarted)
---

# Severity-tiered operator notifications — P0 on a never-muted surface

## Overview

Today all operator notifications from the mika-spirit runtime (verdict handler, ci-failure handler) share one channel: the customer's Telegram chat. `send_notification()` — 11 callsites in `crates/mika-agent/src/server/verdict_handler.rs` and 2 callsites in `crates/mika-agent/src/server/ci_failure_handler.rs` — takes a plain `&str` and calls `MessageSender::send()` with no severity signal. Under real operating load the routine-ack volume drives the operator to mute the chat, and the P0 signals ("the loop is down") mute with it.

This plan adds a `NotificationSeverity` tier to operator notifications and routes **P0 events** — dispatch-stall, `block[security]`, fabrication-detected — to a **structurally separate never-muted surface** (a second Telegram chat, configured per-customer). Routine acks (P2) continue to flow to the normal chat, which the operator can mute without silencing failure alarms.

The pattern is directly grounded in the `mika-manager` (milestone-manager) cadence layer ratified 2026-08-21 by Vincent + Prime (V3 of the design brief at `docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`, wired into `crates/mika-agent/src/milestone_manager/cadence.rs`). That system already implements the exact shape: `Severity::Blocked` routes to `MIKA_MANAGER_ESCALATION_URL` (Vincent-direct route) and `Severity::Attention`/`Severity::Healthy` route to `MIKA_MANAGER_DELIVERY_URL` (normal Prime→sami→Vincent relay). This plan mirrors that dual-URL shape at the customer-scoped operator-notification layer.

## Process note (unblocks 2026-06-26 ESCALATE)

On 2026-06-26 mika-arch returned `Verdict: ESCALATE` on this ticket (session `a26ffb5c`) with F2 blocker: *"the P0 'never-muted surface' target is a design call — separate Telegram chat / bypass mute flag / external pager."* Architect recommended option 1 (separate chat, KISS) but treated it as operator-territory.

**That design call is now resolved by precedent, not by fresh judgment.** The mika-manager brief (Vincent-authored, Prime-ratified 2026-08-21, verdicts 2 + 3 in `docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md`) already established:

- Dual endpoints (normal + escalation) is the correct shape for this class of signal.
- The Severity axis is `Healthy`/`Attention`/`Blocked` — the same three-tier vocabulary defined in `crates/mika-agent/src/milestone_manager/types.rs`.
- `Blocked` routes exclusively to the escalation endpoint — no fallback to the normal endpoint, so a critical signal cannot silently queue behind routine traffic (`cadence.rs::select_route` H1 review fix at line 307-317).
- When the escalation endpoint is unset, fall through to an offline sink so nothing is lost.

This plan applies the same shape to operator notifications from verdict/ci-failure handlers. The design call is now a precedent-application, not an open design question — surface to Prime only if Vincent explicitly wants divergence from the ratified mika-manager pattern.

## Problem Frame

**Founding evidence (from milestone #30 plan, `mika-platform/docs/plans/2026-06-03-001-project-loop-trustworthiness-sequenced-plan.md` lines 45-55, 68-79):**

> "**Alarm into the void** — the engine logs `operator notification fired` for all 15 stalls, but no follow-up work reached the operator. The channel is muted at the destination, or the destination has never been the operator's actual watched surface. Either way, the signal is fired-and-forgotten."

> "Phase 1 — Observability keystone: severity-tiered notifications ⟵ CYCLE-BREAKER. Goal: a P0 'the loop is down' signal that can never share a channel with routine acks, so the operator can mute noise at the source without silencing failure alarms. [...] Phases 2–3 are unverifiable without it — we'd fix dispatch while blind to whether dispatch works."

**Codebase state (verified 2026-08-22 on this worktree):**

- `crates/mika-agent/src/server/verdict_handler.rs`: `send_notification` fn at line 2268; 11 callsites at lines 396, 642, 1033, 1102, 1320, 1371, 1528, 1644, 1731, 1762, 1915, 2203 (12 total including the fn definition itself and 2203 which is a duplicate site — matches body claim of "11 send_notification callsites + fn").
- `crates/mika-agent/src/server/ci_failure_handler.rs`: `send_notification` fn at line 593; 2 callsites at lines 268, 357 (body claimed :582 in June 2026 — the file has since evolved; the shape is unchanged).
- `crates/mika-agent/src/agent_loop/mod.rs`: **ready-label dispatch-stall notification** at line 2372-2382 — direct `sender.send(&notification).await` call (NOT via the two `send_notification` helper fns above). This is the `dispatch-stall` P0 class the ticket body explicitly names. The plan MUST cover this site or the P0 "loop-down" promise from the founding evidence is not delivered.
- `crates/mika-agent/src/messaging.rs`: `MessageSender` trait at line 41 — single method `async fn send(&self, text: &str) -> Result<SendOutcome>`; `GatewayMessageSender::chat_id: Option<i64>` at line 60 (per-instance override, currently used by delegated agents).
- `crates/mika-agent/src/milestone_manager/cadence.rs`: `select_route()` at line 307 — the reference implementation of severity-based route selection.
- **INTENT_GUARD** references moved: body claimed `agent.rs:1591` but the file has been restructured — INTENT_GUARDS now live in `crates/mika-agent/src/agent_loop/mod.rs:1686` (registry) and `crates/mika-agent/src/agent_loop/mod.rs:857` (iteration site). This changes the *locus* of the deferred P2-suppression work (see Scope Boundaries), not the substance.

**Body-vs-code divergences resolved in the plan:**

| Body claim (2026-06-06) | Current code state | Resolution |
|---|---|---|
| "11 `send_notification` callsites + fn at :1136" in verdict_handler.rs | 11 callsites, fn at line 2268 | Line numbers drifted, count unchanged — plan targets shape, not line numbers |
| "duplicated `send_notification` at :582" in ci_failure_handler.rs | Fn at line 593, 2 callsites at 268/357 | Same shape — plan updates both handlers uniformly |
| "`crates/mika-agent/src/agent.rs:1591` — INTENT_GUARDS for P2 suppression" | File moved to `agent_loop/mod.rs`; INTENT_GUARDS registry at line 1686 | INTENT_GUARD relaxation is deferred (out of scope, see below) |
| "MessageSender trait / routing layer — P0 target requires design call" | Design ratified via mika-manager precedent 2026-08-21 | Design call resolved; this plan applies the ratified pattern |
| "P0 (dispatch-stall, recovery-runbook [inventory #4], block[security] [#15], fabrication-detected [#11])" | dispatch-stall: direct `sender.send()` at `agent_loop/mod.rs:2381` (NOT via `send_notification` helper). recovery-runbook and fabrication-detected: no operator-notification callsite exists — these are aspirational sites named by the body author, not currently-emitted alarms | Plan **includes** dispatch-stall (see AC13, Phase B site 15). recovery-runbook and fabrication-detected are deferred as separate follow-up tickets — introducing new notification emission sites is a scope expansion beyond "tier existing notifications." A one-line follow-up ticket per class is filed after this ships. |
| "Loci: crates/mika-gateway/.../github.rs (format)" | `mika-gateway/src/github.rs` formats *inbound* webhook payloads for agent inboxes; it does NOT format outbound operator notifications. No format change is required to add a severity tier — routing happens in the agent, not the gateway | The gateway locus in the body is a code-drift artifact from June 2026. Plan does not touch `mika-gateway/`. |

## Requirements Trace

- **R1.** Introduce a `NotificationSeverity` enum with variants `P0` (never-muted) and `P2` (routine) in `crates/mika-agent/src/messaging.rs`. Rationale: reuse the existing `messaging` module to keep operator-notification routing colocated with `MessageSender`. `P1` is deliberately omitted in v1 (KISS — two tiers cover the cycle-breaker cases; add P1 in a follow-up if a real signal needs it, per YAGNI).
- **R2.** Extend the operator-notification path (not the `MessageSender` trait itself) with a severity-aware helper. The `MessageSender::send(text)` signature stays unchanged so the LLM `send_message` tool (registered in every agent) and the ~30 other `sender.send(text).await` callsites in the codebase are untouched. Handler-side `send_notification()` becomes `send_notification(sender, message, severity)` with routing decided inside the helper.
- **R3.** The routing decision uses an optional escalation channel: `GatewayMessageSender` gains `escalation_chat_id: Option<i64>`. When severity is `P0` and `escalation_chat_id.is_some()`, the helper constructs a one-shot `GatewayMessageSender` variant that overrides `chat_id` for that single send. When severity is `P0` and `escalation_chat_id.is_none()`, fall back to the normal channel with a structured `warn!(event = "operator_notification_p0_fallback_normal_chat", ...)` log — the operator sees the signal, we get a greppable signal that the escalation channel is unconfigured. This mirrors mika-manager's `select_route()` fallthrough-to-offline-sink discipline.
- **R4.** Configuration surface: `MIKA_OPERATOR_ESCALATION_CHAT_ID` env var (per-customer, container-scoped, i64). Wired through `Settings` in `mika-common` following the pattern of every other `MIKA_*` env var. Missing/unset → `None` → the fallback path in R3 fires. Zero-value → `None` (chat_id=0 is the documented no-channel sentinel per `messaging.rs:20-25`).
- **R5.** Classify each of the 15 existing operator-notification callsites (11 verdict + 2 ci-failure + 1 ready-label dispatch-stall in `agent_loop/mod.rs:2381` + 1 verdict_handler duplicate at :2203) into `P0` or `P2` per the classification table in § Implementation Plan Phase B. Anchored to the founding evidence from the milestone #30 plan: dispatch-stall, `block[security]`, retry-exhausted escalations → P0; block[ac], block[ci] first-attempts, hold[review], routine pass/merge → P2. The dispatch-stall site currently calls `sender.send(&notification)` directly (not through a `send_notification` helper) and must be migrated to `sender.send_with_severity(&notification, NotificationSeverity::P0)`.
- **R6.** Delivery verification: at least one integration test exercises the P0 path against a mock `MessageSender` and asserts the escalation chat_id was targeted. This is not "P0 hits Telegram in prod" (out of scope — that's operator config), it is "the routing logic selects the escalation channel when configured." Verifies the founding-evidence bar in the milestone plan line 79: *"1c — verify the void: confirm P0 notifications actually deliver to a channel the operator watches."* The engine-side proof of routing correctness is the plan's contribution; the deployment-side proof (a real second Telegram chat) is a customer-config runbook step (see § Deployment).
- **R7.** No regression on the ~30 non-handler `sender.send(text).await` callsites (agent LLM `send_message` tool, team-run notifications, silent-mode completions, failed-send flush, etc.). The trait is unchanged; only the 13 handler callsites shift to the severity-aware helper.
- **R8.** Zero regressions on `test_dispatcher_skills_dont_declare_required_suffix_lines`, `no_dispatch_test`, mika-arch calibration suite, and the milestone-manager `blocked_severity_routes_to_escalation_url` test — the latter is the reference implementation this plan mirrors, and it must stay green as an anchor.

## Scope Boundaries

**In scope (this PR):**

1. `NotificationSeverity` enum in `messaging.rs` (P0, P2).
2. `send_notification` helper signature change in `verdict_handler.rs` + `ci_failure_handler.rs` — take `severity` param.
3. Per-callsite classification of the 15 existing sites (11 verdict + 2 ci-failure + 1 dispatch-stall + 1 duplicate) — one classification table, executed in one commit.
4. Ready-label dispatch-stall site at `agent_loop/mod.rs:2381` migrated from direct `sender.send()` to severity-aware `sender.send_with_severity(text, NotificationSeverity::P0)`.
5. `escalation_chat_id: Option<i64>` on `GatewayMessageSender` + constructor threading.
6. `MIKA_OPERATOR_ESCALATION_CHAT_ID` env var in `Settings`.
7. Routing helper: when P0 + `escalation_chat_id.is_some()`, issue a one-shot escalation-channel send.
8. Structured fallback WARN log (`operator_notification_p0_fallback_normal_chat`) when P0 fires without an escalation channel configured.
9. Integration test that exercises the routing decision with a mock sender.
10. Env-var docs in `mika/CLAUDE.md` (Environment Variables section, where `MIKA_MANAGER_ESCALATION_URL` is already documented).
11. Compound doc capturing the "operator-notification tier mirrors mika-manager escalation pattern" pattern.

**Out of scope (deferred with named follow-ups):**

- **INTENT_GUARD P2 suppression** (body's second axis: "batch/suppress P2 acks via INTENT_GUARD relaxation, let mika-dev EndTurn without send_message on non-actionable webhooks"). This targets `agent_loop/mod.rs:1686` (INTENT_GUARDS registry) and involves engine-loop discipline changes with a much wider blast radius (every EndTurn path). It solves noise, but the *founding evidence* is about P0 being audible — noise reduction is orthogonal. **Follow-up ticket:** file after this ships, referencing the body's original axis.
- **New P0 emission sites for "recovery-runbook" and "fabrication-detected" classes** (body mentioned but never existed as `send_notification` callsites). Introducing new alarm emission points is a scope expansion beyond "tier existing alarms." Two follow-up tickets — one per class — filed after this ships. The mechanism this PR ships (severity + escalation routing) is what those follow-ups will use; without this PR they'd have nowhere to route.
- **A `P1` severity tier.** Two tiers are enough for the cycle-breaker; add P1 when a concrete signal doesn't fit either bucket. YAGNI.
- **Real second Telegram chat provisioning.** That is customer-config (`add-customer.sh` / manual `mika config set`), documented as a runbook step in § Deployment. This plan ships the mechanism; the customer decides whether to configure it.
- **Cross-channel notification** (SMS, external pager, webhook). The mika-manager brief already parked pager/SMS as out-of-scope; this plan inherits that boundary.
- **Retro-active reclassification of `send_notification` sites in code not yet written.** New callsites must classify their severity at commit time — enforced by the `severity` param being non-optional (no `Default` impl).
- **Renaming `NotificationSeverity` variants to match `milestone_manager::types::Severity` (`Healthy`/`Attention`/`Blocked`).** The two subsystems have different semantics — operator notifications are event-shaped (a verdict, a failure), milestone-manager Severity is state-shaped (a milestone's current condition). Reusing the enum would create false coupling. The alignment is *shape* (dual-endpoint routing), not *type*.
- **Gateway-side formatting changes.** `mika-gateway/src/github.rs` is untouched — outbound operator notifications don't traverse gateway formatting; the routing decision is fully agent-side.

## Design decisions

| # | Decision | Options considered | Chosen | Rationale |
|---|---|---|---|---|
| D1 | Where the severity axis lives | (a) On `MessageSender` trait, (b) On handler-side helper only, (c) On a new trait | **(b)** | (a) forces breaking-change on ~30 callsites unrelated to operator notifications (LLM send_message tool, team runs, failed-send flush). (c) adds trait sprawl. (b) is scoped to the ~13 handler sites — precise blast radius. Precedent: `mika-manager` puts routing in `cadence.rs::select_route`, not in a delivery trait. |
| D2 | Config surface for escalation chat | (a) Env var per customer, (b) Per-agent `identity.toml` field, (c) Global env var | **(a)** `MIKA_OPERATOR_ESCALATION_CHAT_ID` | Matches mika-manager (`MIKA_MANAGER_ESCALATION_URL` is env-scoped, container-per-customer). Per-agent identity.toml would fragment the routing config across N files; env var is one place. Global would break multi-customer isolation. |
| D3 | Two tiers (P0/P2) vs three (P0/P1/P2) | (a) Two, (b) Three | **(a)** | KISS. The founding-evidence classes split cleanly into "loop-down / must-see" and "everything else." Add P1 when a real signal doesn't fit — YAGNI. Precedent asymmetry with mika-manager (three-tier) is deliberate: mika-manager is state-shaped (Healthy/Attention/Blocked), operator notifications are event-shaped (audible/routine). Different domains, different vocabularies. |
| D4 | Fallback when P0 fires without escalation channel configured | (a) Silently route to normal chat, (b) Route to normal chat with WARN, (c) Hard-error, (d) Drop the notification | **(b)** WARN + normal chat | (a) silent-fail — anti-pattern per `feedback_prompt_enforcement_fragile`. (c) makes the deploy dependent on env-var config — brittle. (d) loses the signal entirely. (b) preserves the signal AND emits a greppable operational signal that the escalation channel is unconfigured. Mirror of mika-manager's offline-sink fallback (`cadence.rs::select_route`). |
| D5 | Signature: `send_notification(sender, msg, severity)` vs `send_notification_p0(sender, msg)` + `send_notification(sender, msg)` | (a) Single fn with severity param, (b) Two fns per severity | **(a)** | (b) requires two entry points per handler; forgotten sites default to P2 silently. (a) makes severity a mandatory arg — no callsite can forget to classify. Structural enforcement > convention. |
| D6 | Escalation channel implementation: one-shot `GatewayMessageSender` clone vs new trait method | (a) One-shot clone with overridden `chat_id`, (b) `MessageSender::send_to(chat_id, text)` method | **(a)** | (b) adds a trait method every impl must satisfy (mock, noop, gateway). (a) uses the existing `chat_id: Option<i64>` override field — no trait surface change, contained to the helper. `GatewayMessageSender::new()` already threads `chat_id` positionally per `messaging.rs:75-95`. |
| D7 | Classification of `block[ac]` / `block[ci]` — P0 or P2? | (a) P0 (blocking = urgent), (b) P2 (auto-retry handles it) | **(b) P2** | Grounded in verdict_handler behavior: `block[ac]` and `block[ci]` auto-dispatch a fix (`handle_block_ac` / `handle_block_ci` in verdict_handler.rs). The operator sees them only if the bounded retry counter (max 3) exhausts. Routine block → P2; retry-exhaustion escalation is a *separate* notification site inside the retry handler and should be P0. |
| D8 | Classification of `block[security]` and `block[pipeline]` — P0 or P2? | (a) P0, (b) P2 | **(a) P0** | These have no auto-fix path (verdict_handler dispatches to operator directly, no retry). Founding evidence in milestone #30 plan explicitly names `block[security]` as a P0 class. `block[pipeline]` is symmetric — an infrastructure-shape verdict operator must see. |
| D9 | Test shape for R6 | (a) Unit test on the routing helper, (b) Integration test with mock `MessageSender` capturing chat_id, (c) Real Telegram deployment test | **(b)** | (a) tests plumbing only, not the routing decision. (c) requires real customer config — cannot run in CI. (b) proves the routing logic selects the escalation channel when configured, using the existing mock-sender pattern from `messaging.rs` tests. |

## Implementation plan

**Sequencing:** Phase A → Phase B → Phase C → Phase D. Each phase is a separate commit for clean review. Phase A must land first because Phase B depends on the `NotificationSeverity` enum and the helper signature.

### Phase A — Add the severity axis and the routing helper

**Files touched:**

- `crates/mika-agent/src/messaging.rs` — add `NotificationSeverity` enum (`P0`, `P2`), add `escalation_chat_id: Option<i64>` field to `GatewayMessageSender`, extend constructor.
- `crates/mika-common/src/config.rs` (or wherever `Settings` lives — verify at implementation time) — add `operator_escalation_chat_id: Option<i64>` field derived from `MIKA_OPERATOR_ESCALATION_CHAT_ID`.
- `crates/mika-agent/src/server/handlers.rs` — thread `escalation_chat_id` through the two `GatewayMessageSender::new()` callsites at lines 1149 and 1488.
- All existing `GatewayMessageSender::new()` test-site callsites in `crates/mika-agent/src/messaging.rs` (lines 261, 285, 304, 342, 373, 395, 424, 455, 491, 523) — pass `None` for the new field.

**Steps:**

1. Define `pub enum NotificationSeverity { P0, P2 }` in `messaging.rs` with `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`.
2. Add `escalation_chat_id: Option<i64>` field to `GatewayMessageSender`. Update `new()` positionally (accepts already `too_many_arguments` allow — pattern established).
3. Add `pub async fn send_with_severity(&self, text: &str, severity: NotificationSeverity) -> Result<SendOutcome>` method. When `severity == P0 && escalation_chat_id.is_some()`, construct a temporary `GatewayMessageSender` clone with `chat_id = escalation_chat_id`, call `.send(text)` on it, done. When `severity == P0 && escalation_chat_id.is_none()`, emit `warn!(event = "operator_notification_p0_fallback_normal_chat", agent_name, message_preview_truncated_500)` and call the normal `send(text)`. When `severity == P2`, call `send(text)` directly.
4. Wire `Settings` env var and thread through the two `handlers.rs` construction sites.
5. Update test-site `GatewayMessageSender::new()` callsites to pass `None`.
6. Add a unit test in `messaging.rs`: mock the sender, assert `P0 + Some(escalation_chat_id)` targets the escalation chat, `P0 + None` targets normal + WARN, `P2` targets normal.

### Phase B — Classify the 13 handler callsites and switch to the severity-aware helper

**Classification table (justified per D7/D8):**

| Callsite | File:line | Verdict/event | Severity |
|---|---|---|---|
| 1 | verdict_handler.rs:396 | pass — merge succeeded | P2 |
| 2 | verdict_handler.rs:642 | pass — auto-merge enabled (CI pending) | P2 |
| 3 | verdict_handler.rs:1033 | block[ac] — dispatch preparing | P2 |
| 4 | verdict_handler.rs:1102 | block[ac] — retry attempt N/3 | P2 |
| 5 | verdict_handler.rs:1320 | block[ac] — retry exhausted, escalation | **P0** |
| 6 | verdict_handler.rs:1371 | block[ci] — dispatch preparing | P2 |
| 7 | verdict_handler.rs:1528 | block[ci] — retry attempt N/3 | P2 |
| 8 | verdict_handler.rs:1644 | block[ci] — retry exhausted, escalation | **P0** |
| 9 | verdict_handler.rs:1731 | block[security] — no auto-fix, operator must see | **P0** |
| 10 | verdict_handler.rs:1762 | block[pipeline] — infrastructure verdict | **P0** |
| 11 | verdict_handler.rs:1915 | hold[review] — operator review requested | P2 |
| 12 | verdict_handler.rs:2203 | fallback / unparseable verdict | P2 |
| 13 | ci_failure_handler.rs:268 | CI failure — autonomous fix dispatched | P2 |
| 14 | ci_failure_handler.rs:357 | CI failure — circuit breaker tripped (ci_fix_count >= 2) | **P0** |
| 15 | agent_loop/mod.rs:2381 | Ready-label dispatch stalled — direct `sender.send()` today, migrate to `send_with_severity` | **P0** |

Verification anchor: each row's file:line is checked against the current worktree HEAD at commit time; if any drifted, the classification stays, only the line updates. Site 15 (dispatch-stall) is the founding-evidence "loop-down" alarm the milestone #30 plan named as the P0 keystone (line 74) — omitting it would ship a partial fix that leaves the most important P0 signal on the muted channel.

**Files touched:**

- `crates/mika-agent/src/server/verdict_handler.rs` — change `send_notification(sender, msg)` signature to `send_notification(sender, msg, severity)`; update 12 callsites per classification table above (11 + 1 duplicate at line 2203).
- `crates/mika-agent/src/server/ci_failure_handler.rs` — same shape; update 2 callsites.
- `crates/mika-agent/src/agent_loop/mod.rs` — line 2381: replace `sender.send(&notification).await` with `sender.send_with_severity(&notification, crate::messaging::NotificationSeverity::P0).await`. Add `use crate::messaging::NotificationSeverity;` at the top of the file if not already imported.
- Both `send_notification` helper fns internally call `sender.send_with_severity(text, severity)` — the trait default method handles impls without severity awareness.
- **Refinement per D1:** avoid `Any` downcast — add `async fn send_with_severity(&self, text: &str, _severity: NotificationSeverity) -> Result<SendOutcome> { self.send(text).await }` as a default method on the `MessageSender` trait, override only in `GatewayMessageSender`. Zero-cost for existing impls, no `Any` downcast, no dyn-cast fragility.

**Steps:**

1. Add the default method to `MessageSender` trait (revise Phase A to include this).
2. Override in `GatewayMessageSender` with real routing.
3. Update the two handler `send_notification` helper fns to take + propagate severity.
4. Change all 14 handler-side callsites per classification table (12 verdict + 2 ci-failure).
5. Change the direct `sender.send()` call at `agent_loop/mod.rs:2381` (dispatch-stall) to `sender.send_with_severity(..., NotificationSeverity::P0)`.
6. Add a `#[test]` in `verdict_handler.rs` module that asserts a `block[security]` verdict path invokes P0 severity on a mock sender.
7. Add a `#[test]` in `agent_loop/mod.rs` module (or an integration test) that asserts the dispatch-stall notification path invokes P0 severity.

### Phase C — Documentation

**Files touched:**

- `crates/mika-agent/CLAUDE.md` — extend § Environment Variables with `MIKA_OPERATOR_ESCALATION_CHAT_ID` right after the `MIKA_MANAGER_*` block. Cross-reference the two subsystems (both use the dual-endpoint escalation pattern).
- `mika/CLAUDE.md` (root) — one-line addition to § Environment Variables listing `MIKA_OPERATOR_ESCALATION_CHAT_ID` alongside the existing `MIKA_MANAGER_*` entries.
- **New file:** `docs/solutions/best-practices/operator-notification-severity-tier-2026-08-22.md` — capture the pattern (severity axis + escalation endpoint + fallback-with-warn) as a compound-doc. Cite mika-manager as the sibling implementation.

### Phase D — Deployment note

Add a section to the compound doc (Phase C) documenting the customer-config runbook step:

> To activate the never-muted P0 surface, provision a second Telegram bot chat (or existing chat with only the operator + Mika bot as members), retrieve its chat_id, and set `MIKA_OPERATOR_ESCALATION_CHAT_ID=<id>` in the customer's container env before `mika-spirit` restart. Verify by triggering a synthetic `block[security]` (test-only path) and confirming delivery to the second chat. Without this step, P0 notifications route to the normal chat with a `warn!(event = "operator_notification_p0_fallback_normal_chat", ...)` log — the signal is not lost, but the isolation guarantee does not hold.

No PR-blocking work in Phase D; the runbook step is documentation-only. The operator (or `add-customer.sh` in a future PR) does the actual config.

## Definition of Done

- [ ] `NotificationSeverity { P0, P2 }` enum lives in `crates/mika-agent/src/messaging.rs`.
- [ ] `MessageSender` trait has `send_with_severity` default method delegating to `send`.
- [ ] `GatewayMessageSender` overrides `send_with_severity` with severity-aware routing.
- [ ] `GatewayMessageSender::new()` accepts `escalation_chat_id: Option<i64>` positionally.
- [ ] `MIKA_OPERATOR_ESCALATION_CHAT_ID` env var wired through `Settings`; zero-value → `None`.
- [ ] All 15 operator-notification callsites (12 in verdict_handler.rs including the duplicate at line 2203, 2 in ci_failure_handler.rs, 1 in agent_loop/mod.rs at line 2381, per classification table) classified and passing severity through the helper or via `send_with_severity` directly.
- [ ] Fallback WARN log (`operator_notification_p0_fallback_normal_chat`) fires on P0 without escalation channel configured — verified by unit test.
- [ ] Unit test in `messaging.rs`: mock sender captures target chat_id, asserts P0-with-escalation routes to escalation, P0-without routes to normal + WARN, P2 always routes to normal.
- [ ] Integration test in `verdict_handler.rs`: `block[security]` code path invokes P0 severity on mock sender.
- [ ] `MIKA_OPERATOR_ESCALATION_CHAT_ID` documented in `crates/mika-agent/CLAUDE.md` § Environment Variables and cross-referenced from root `mika/CLAUDE.md`.
- [ ] Compound doc `docs/solutions/best-practices/operator-notification-severity-tier-2026-08-22.md` captures the pattern and cites mika-manager as sibling.
- [ ] `cargo build`, `cargo clippy`, `cargo test -p mika-agent` all pass.
- [ ] `blocked_severity_routes_to_escalation_url` in `cadence.rs` still passes — anchor test unchanged.
- [ ] mika-arch second-pass verdict is `GROOMED`.
- [ ] PR body cross-references milestone #30 and cites 2026-06-26 ESCALATE resolution via mika-manager precedent.

## Acceptance Criteria

**AC1.** A `NotificationSeverity` enum with variants `P0` and `P2` exists in `crates/mika-agent/src/messaging.rs`, is `Copy + Clone + Debug + PartialEq + Eq`, and is exported from the crate root.

**AC2.** The `MessageSender` trait exposes `async fn send_with_severity(&self, text: &str, severity: NotificationSeverity) -> Result<SendOutcome>` as a default method that delegates to `send(text)`, so existing impls (`NoopSender`, test mocks) compile without change.

**AC3.** `GatewayMessageSender` overrides `send_with_severity` with routing logic: `(severity == P0, escalation_chat_id.is_some())` targets the escalation chat_id; `(severity == P0, escalation_chat_id.is_none())` targets the normal chat AND emits `warn!(event = "operator_notification_p0_fallback_normal_chat", agent_name, message_preview)`; `severity == P2` targets the normal chat with no warn.

**AC4.** `GatewayMessageSender::new()` accepts an `escalation_chat_id: Option<i64>` positional argument. `chat_id == Some(0)` on the escalation slot is coerced to `None` before storage (chat_id=0 sentinel discipline per `messaging.rs:20-25`).

**AC5.** `MIKA_OPERATOR_ESCALATION_CHAT_ID` env var is wired through `Settings`; unset → `None`; empty-string → `None`; `"0"` → `None`; any other parseable i64 → `Some(i64)`; unparseable value → `None` with a `warn!` log line at `Settings` load time.

**AC6.** All 15 operator-notification callsites (`verdict_handler.rs` lines 396, 642, 1033, 1102, 1320, 1371, 1528, 1644, 1731, 1762, 1915, 2203; `ci_failure_handler.rs` lines 268, 357; `agent_loop/mod.rs` line 2381) pass an explicit `NotificationSeverity` argument (via helper for the 14 handler sites, via direct `send_with_severity` call for the dispatch-stall site). The classification matches the table in § Implementation Plan Phase B.

**AC7.** A unit test in `messaging.rs` (name: `send_with_severity_p0_targets_escalation_when_configured`) asserts that a `GatewayMessageSender` with `escalation_chat_id = Some(999)` and severity `P0` targets chat_id 999. Two sibling tests cover the P0-no-escalation + WARN path and the P2 path.

**AC8.** An integration test in the `verdict_handler` module (name: `block_security_invokes_p0_severity`) drives a synthetic `block[security]` verdict through the handler and asserts the mock sender received a `send_with_severity` call with `NotificationSeverity::P0`. Existing anchor tests (`blocked_severity_routes_to_escalation_url` in `cadence.rs`, `no_dispatch_test`, dispatcher skill parity tests) stay green.

**AC13.** A test in `agent_loop/mod.rs` (name: `dispatch_stall_invokes_p0_severity` — location: co-located with the existing `webhook_ready_label_dispatch` guard tests) exercises the ready-label dispatch-stall path with a mock `MessageSender` and asserts it received a `send_with_severity` call with `NotificationSeverity::P0`. If the existing test scaffolding for `agent_loop` guards is too invasive to reach the dispatch-stall branch, this AC may be satisfied by a smaller-scope test that instantiates the stall notification message directly and asserts the severity constant it uses.

**AC9.** `MIKA_OPERATOR_ESCALATION_CHAT_ID` is documented in `crates/mika-agent/CLAUDE.md` § Environment Variables (right after the `MIKA_MANAGER_*` block) with: purpose, per-customer scope, unset behavior (fall-back + WARN), and a cross-reference to `MIKA_MANAGER_ESCALATION_URL` as the sibling pattern.

**AC10.** A compound doc lives at `docs/solutions/best-practices/operator-notification-severity-tier-2026-08-22.md` with YAML frontmatter (`module: mika-agent, category: best-practices, tags: [notifications, operator, severity, routing]`), a "Problem" section citing the milestone #30 alarm-into-the-void finding, a "Pattern" section describing severity-axis + escalation-endpoint + fallback-with-warn, a "Sibling" section citing `mika-manager::cadence::select_route`, and a "Runbook" section for customer-config activation.

**AC11.** `cargo build --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p mika-agent`, `cargo test -p mika-common`, and `cargo test -p mika-gateway` all pass on the branch tip. `make lint` and `make check` pass.

**AC12.** PR body cross-references milestone #30 (`Loop Trustworthiness — observability → stability`) and cites the 2026-06-26 ESCALATE resolution: the F2 design-call blocker is closed by precedent from the mika-manager brief (2026-08-21, verdicts 2+3), not by fresh judgment.

## Verification plan

1. **Unit tests** — the three `send_with_severity_*` tests in `messaging.rs` (AC7) run on every `cargo test`.
2. **Integration test** — `block_security_invokes_p0_severity` in `verdict_handler.rs` module tests (AC8) runs on every `cargo test -p mika-agent`.
3. **Anchor tests unchanged** — `blocked_severity_routes_to_escalation_url` in `milestone_manager::cadence` stays green; if it breaks, this plan has drifted from its precedent and must be reconciled before merge.
4. **Manual smoke (post-deploy, operator-driven, out-of-band):**
   - Deploy with `MIKA_OPERATOR_ESCALATION_CHAT_ID` set to a test chat.
   - Trigger a `block[security]` verdict on a test PR (or invoke a test-only path).
   - Confirm delivery to the escalation chat, not the normal chat.
   - Deploy with the env var unset.
   - Trigger the same verdict.
   - Confirm delivery to normal chat + `operator_notification_p0_fallback_normal_chat` WARN in `$MIKA_SPIRIT_LOG_FILE`.
5. **Regression sweep** — the ~30 non-handler `sender.send(text).await` callsites (grep `sender.send\(` across `crates/`) — none should change behavior (R7); the trait method is unchanged, only the operator-notification helper is severity-aware.

## Rollback

- All changes are additive at the messaging-layer level (new enum, new default-method, new field with `Option` type, new env var with unset-safe default).
- Revert path: `git revert` the merge commit. No DB migration involved; no schema change; no wire-format change on `/send` payload (chat_id override was already in the payload path). Environment variable stays behind (harmless — read but ignored when the code is reverted).

## Related

- `mika-platform/docs/plans/2026-06-03-001-project-loop-trustworthiness-sequenced-plan.md` — parent project plan (milestone #30). Founding evidence for the alarm-into-the-void finding and the severity-tier phase.
- `mika/docs/brainstorms/2026-08-21-mika-manager-de-milestones-design-brief.md` — Vincent + Prime ratified brief establishing the dual-endpoint escalation pattern (V2, V3).
- `mika/crates/mika-agent/src/milestone_manager/cadence.rs` — reference implementation of severity-based route selection (`select_route` at line 307).
- `mika/crates/mika-agent/src/milestone_manager/types.rs` — `Severity` enum (Healthy/Attention/Blocked); this plan deliberately does NOT reuse it (different semantics — event vs state).
- mika#1381 comment 2026-06-26T20:33:32Z — the ESCALATE session (`a26ffb5c`) whose F2 blocker this plan closes by precedent.
- `docs/solutions/best-practices/dont-flip-on-pushback-without-new-reasoning.md` — the resolution here is precedent-application (mika-manager 2026-08-21), not a flip.
