---
title: "Gateway truncation caps must be calibrated per event type, not globally"
date: 2026-05-01
category: best-practices
module: mika-gateway
problem_type: workflow_issue
component: github-event-forwarding
applies_when:
  - Adding a new GitHub webhook event type to `mika-gateway`'s `format_event_text` path
  - Raising or lowering an existing truncation cap on a body surface (issue, PR, comment, review)
  - Diagnosing `verdict_classification_failed` with `body_truncated: true` in `mika-agent` logs
  - Designing transport-layer caps for any agent-emitted structured surface that the engine parses by token (VERDICT, dispositions, suffix lines)
tags:
  - mika-gateway
  - github-webhooks
  - truncation
  - qa-verdict
  - per-event-calibration
  - transport-vs-workflow
  - mika-909
  - mika-898
  - mika-911
  - defense-in-depth
---

# Gateway truncation caps must be calibrated per event type, not globally

## Context

`mika-gateway/src/github.rs` calls `truncate_body(...)` from four call sites in the `format_event_text` path — one for each GitHub webhook event type the gateway forwards: issue body, PR body, issue comment, and pull-request review. Pre-mika#911, all four call sites used a hard-coded 2 KB cap.

That uniform cap was correct for three of the four surfaces and wrong for the fourth. The two incidents that surfaced the problem — mika#909 and mika#898 on 2026-04-30 — produced silent verdict-classification failures because mika-qa's review body, which is structured-and-long, exceeded 2 KB and the VERDICT token at the bottom got clipped.

## Incident summary

| PR | Body size | VERDICT line offset | Failure shape |
|---|---|---|---|
| mika#909 | 3,302 chars | 2,758 | `state=APPROVED + VERDICT: pass` clipped at offset 2,000 → engine emitted `verdict_classification_failed` with `body_truncated: true` → mika-dev parked the PR awaiting operator unblock; manual merge needed |
| mika#898 | similar | similar | same `body_truncated: true` shape, server.log 2026-04-30 09:23:57 |

Both incidents were the same root cause: gateway transport cap chosen for the smallest body surface, applied uniformly, breaking the contract on the largest.

## Root cause

```rust
// pre-fix at crates/mika-gateway/src/github.rs (4 call sites, all 2000)
let body = truncate_body(review.and_then(|r| r.body.as_deref()).unwrap_or(""), 2000);
```

The engine's `verdict_handler` parses `(?mi)^VERDICT:\s*(.+)$` against the gateway-truncated body. mika-qa's body shape is:

```
DIFF ANALYSIS
PLAN-AC VERIFICATION
BUILD VERIFICATION
(FINDINGS)
VERDICT: <pass|hold[*]|block[*]>
REASON: ...
```

The VERDICT line lives at the bottom — exactly the part the truncation eats first. Any non-trivial review (3-5 KB typical) crossed the cap and lost its routing token.

The single 2 KB cap was correct for issue/PR/comment surfaces (operator-curated, mostly human-authored, mostly short) and wrong for the review surface (agent-emitted, structurally long, with a load-bearing token at a non-deterministic position).

## The principle: per-event-type calibration

Caps should be sized to the **expected body shape and growth pressure of each event type**, not to a single global value.

| Event type | Author | Body shape | Growth pressure | Cap rationale |
|---|---|---|---|---|
| Issue body | Operator (human) | Free-form prose, occasional templates | Low — humans naturally bound | 2 KB suffices |
| PR body | Operator or agent | Templated, sometimes with diff excerpts | Low-medium | 2 KB suffices |
| Issue comment | Operator (human) | Short reactions, occasional links | Very low | 2 KB suffices |
| PR review (qa-review) | Agent (`mika-qa`) | Structured (DIFF + AC + BUILD + VERDICT) | Medium-high — grows with PR size | 16 KB needed |

Operator-curated surfaces (issue/PR/comment) tolerate small caps because humans naturally bound their writing. Agent-emitted structured surfaces (`pull_request_review`) need larger caps OR position-independent parsing, because the agent emits a fixed-shape structure whose total length grows with the work being reviewed.

The fix in mika#911 makes this asymmetry explicit via two named constants:

```rust
const DEFAULT_GITHUB_BODY_TRUNCATION_CHARS: usize = 2_000;
const GITHUB_REVIEW_BODY_TRUNCATION_CHARS: usize = 16_000;
```

…and applies the review constant only at the `pull_request_review` branch.

## 16 KB derivation rationale

QA verdict bodies run 3–5 KB typical. 16 KB gives roughly 3× headroom over typical. The multiplier shape matches mika#864's `MAX_REQUIRED_SUFFIX_LINES = 8` precedent: observed-typical 1–2 lines, cap at 8 lines = ~4× headroom. Same shape — bound the cap at a small multiple of observed-typical, not at infinity.

The cap is finite for transport sanity. Unbounded review bodies would expand the gateway's per-event memory footprint and the agent's per-message processing cost without bound.

## Defense-in-depth pattern

The fix has two layers, retained permanently:

1. **Structural cap raise** (`mika#911`, this fix) — gateway forwards up to 16 KB per review body. Durable bound. Engine regex parses position-independently within the forwarded region.
2. **Prompt-side VERDICT-on-top** (`170148a2`, hot-fix shipped 2026-04-30) — `qa-review/system_prompt.md` restructured to emit `VERDICT: <verdict>` and `REASON: <reason>` as the first two body lines. The engine regex captures first-match (per `crates/mika-agent/src/server/verdict.rs:203`), so VERDICT-at-top survives any cap.

Both layers are kept permanently. Per `feedback_prompt_enforcement_fragile.md`, prompt rules drift over time as model versions change; the structural cap is durable. Per `feedback_transport_vs_workflow.md`, transport caps shouldn't compensate for workflow concerns; the prompt-side defense is resilience against future model-output drift. Together they form belt-and-braces against:

- Body shapes that exceed 16 KB in the future (prompt-side first-match still works)
- Model output drift away from VERDICT-on-top (structural cap still preserves long bodies)

Near-zero ongoing maintenance cost for both layers — both already shipped, neither requires per-deploy attention.

## Forward-pointer: config-flag escalation

If 3 documented incidents post-deploy involve QA verdicts exceeding the 16 KB cap, escalate to a configurable env var:

```
MIKA_GATEWAY_REVIEW_BODY_CAP=16000   # default, override per environment
```

The named-constant refactor in mika#911 makes this a 1-line change when the threshold is hit. The N=3 recurrence threshold matches the structural-escalation pattern from `engine-guards-vs-prompt-rules-for-agent-behavior-2026-04-19.md` — recurrence establishes a real signal vs. a one-off.

Until the threshold is hit, the named constant suffices. Don't escalate prematurely.

## Cross-references

- **Anti-pattern parents:**
  - `feedback_transport_vs_workflow.md` — transport caps shouldn't break workflow contracts
  - `feedback_prompt_enforcement_fragile.md` — prompt rules drift; structural defense is durable
- **Related compound docs:**
  - `mika/docs/solutions/best-practices/required-tools-gate-transport-contract-thin-final-turn-2026-04-29.md` — sibling pattern: transport-layer assumptions silently breaking agent contracts
  - `mika/docs/solutions/best-practices/operator-db-evidence-disconfirmation-when-architect-cant-surface-premise-2026-04-30.md` — operator-side recovery when transport breaks emission
- **Issue history:**
  - mika#487 — original incident establishing `state ≠ CHANGES_REQUESTED + VERDICT-token-in-body` contract
  - mika-skills#55 (2026-03-30) — established `pass → --approve` for branch-protection-approval gate
  - mika-skills#119 (2026-04-11) — `pr_merge_with_gate` depends on the approval being there
  - mika#909 — first incident (live, manual-merge unblock applied)
  - mika#898 — second incident (same `body_truncated` pattern)
  - mika#911 — this fix
- **Commit history:**
  - `170148a2` — prompt hot-fix (VERDICT-on-top)
  - alceops's PR #912 — community contributor's gateway cap raise + docs reconciliation, cherry-picked onto `fix/911/review-truncation-cap` as `f7691ccb` and `575044eb`
- **Plan:** `docs/plans/2026-05-01-001-fix-gateway-truncation-cap-and-docs-plan.md` (this fix)

## Adjacent precedents to apply this lesson to

When designing or auditing the gateway:

- New webhook event types should pick a cap based on the surface's expected body shape, not the existing default.
- If the engine parses a body by **token at non-deterministic position**, the surface needs a generous cap OR a prompt-side discipline guaranteeing the token's position.
- Per-event-type caps are explicit knobs; document them as named constants in `crates/mika-gateway/src/github.rs` so future calibration is a 1-line change.
- The N=3 recurrence threshold for promoting a hard-coded cap to a config flag applies generally — don't add config flags speculatively.
