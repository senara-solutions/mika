# Plan: mika#760 — Clarify default=None as deliberate design

## Overview

Doc-only change. Add a subsection to `docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md` explaining that `None` as the default for `MIKA_KG_EXTRACTION_MODEL` and `MIKA_KG_RESOLUTION_MODEL` is a deliberate policy choice, not a deferred improvement.

## Motivation

The compound doc captures the #757 incident, the fix, and the pattern, but does not explicitly call out **why** the KG model env vars default to `None` instead of falling back to a cheap provider. A future reviewer could reasonably assume this is a gap and add a silent fallback — reintroducing the exact class of cost surprise the doc was written to prevent.

The reasoning: silent fallback to a default provider hides misconfiguration and increases the probability of cost surprises. A failing startup when no KG model is configured is the *desired* behavior — fail loudly, force an explicit operator decision.

## Changes

### 1. Add design-note subsection

**File:** `docs/solutions/best-practices/first-boot-cost-spike-after-tracking-table-migration-2026-04-23.md`

**Location:** After the "Provider choice is a structural cost lever" subsection (under `## Guidance`), before the "Fan-out on shared source data" subsection. This placement groups all provider/cost-related guidance together.

**Content:** A new `### Design note: default=None is deliberate` subsection containing:

- Statement that `MIKA_KG_EXTRACTION_MODEL` and `MIKA_KG_RESOLUTION_MODEL` defaulting to `None` (KG features disabled when unset) is a deliberate policy, not a gap.
- Rationale: silent fallback to any provider — even a cheap one — hides misconfiguration and shifts cost discovery from deploy-time (explicit operator decision) to bill-time (surprise). The `kg_anthropic_provider` WARN is the compensating signal for the specific incident class where an operator *has* configured a model but chose an expensive provider.
- Cross-reference to `CLAUDE.md` env var docs where the "If unset, KG features requiring LLM calls are disabled" behavior is documented.
- Explicit "do not add a code-level fallback" directive to prevent well-intentioned future changes.

### 2. No code changes

This is pipeline-exempt, doc-only scope per the ticket's own declaration.

## Acceptance criteria mapping

| Ticket AC | Plan step |
|-----------|-----------|
| Add "Design note: default=None is deliberate" subsection | Step 1 |
| Cross-reference `kg_anthropic_provider` WARN | Step 1 (included in subsection) |
| Do not add a code-level fallback | Step 1 (explicit directive in subsection) + Step 2 (no code changes) |

## Risks

None. Single-file doc edit with no behavioral changes.
