# Plan conventions

Conventions for groomed plans in this directory. Established during milestone #16 (Evaluation) grooming (2026-04-22 / 2026-04-23). Applies to any plan with cross-plan dependencies.

## Amendment protocol for SHA-pinned plans

When a plan cites an upstream plan's decisions by pinning a specific commit SHA (e.g., `**Blocked by:** #338 at plan commit \`fa54d950\``), the pinned SHA acts as a **frozen public contract** at dispatch time. The pin exists so upstream drift surfaces as a grep-findable version bump in the downstream plan, not as silent breakage.

**If an upstream plan amends a shared-surface decision after dispatch:**

1. Amend the upstream plan doc; commit; push. The commit produces a **new plan SHA**.
2. Update every downstream plan that pins the old SHA. Bump the pin to the new SHA in the `Blocked by:` header.
3. Re-review each downstream plan for fit against the amended upstream surface. If signatures, vocabulary, or semantics changed, the downstream plan may need its own amendment.
4. If a downstream ticket has already dispatched (implementation is in flight), the amendment propagation is a PR review conversation — the implementer sees the SHA bump as an explicit signal that upstream moved.

**What counts as a shared-surface amendment:**
- Any change to helper module signatures that downstream plans import (e.g., `kg_fixtures::seed_resolution` in milestone #16)
- Any change to ticket-namespaced vocabulary that downstream scenarios emit tags into
- Any change to acceptance-criteria shape that downstream plans cite ("per #338 D9 frozen-fixture pattern")
- Any change to execution-model decisions that downstream plans inherit (three-tier execution, registration idioms)

**What does NOT count (no SHA bump required):**
- Clarifying prose in rationale sections
- Typo fixes
- Adding examples that don't change the decision
- Review-log updates recording the amendment itself

The discipline: **amend upstream → bump downstream pins → review fit**. Cheap when followed. The cost of skipping it is exactly the silent-drift problem SHA-pinning was built to prevent. This is the implementation-time analog of the grooming-time Socratic review discipline — the pin is the structural equivalent of "peer review before dispatch," enforced at amendment time rather than at authoring time.

## Origin

Milestone #16 (Evaluation) introduced SHA-pinning as the cross-plan dependency mechanism during its Socratic grooming pass. The five plans (`#340` / `#338` / `#339` / `#740` / `#741`) each cite upstream plan commits explicitly. During the milestone-level friend review on 2026-04-23, both `#740` and `#339` received amendments that bumped their SHAs; `#741` re-pinned to the new SHAs as an explicit version bump. This protocol documents the discipline that made those amendments structural rather than silent.

## Related conventions

Other grooming patterns established in milestone #16 that aren't formalized here but are worth knowing:

- **Ticket-namespaced soft-assertion vocabulary** (e.g., `quality:*` owned by `#339`, `self-knowledge:*` by `#740`, `grounding:*` by `#741`). Each namespace is owned by the ticket that defines it; calibration artifacts preserve namespace structure so aggregation works at the tooling layer.
- **Tag attribution follows cause-location, not symptom** (`#740` D4 + `#741` D4). When a failure could be classified two ways, attribute the tag to the code path where the failure originated, not the user-visible symptom.
- **Design-time cost envelopes** (`#740` D2 fixed fixture shape + priced; `#741` D7 per-scenario; `#339` class-average for large tickets). Small tickets price per-scenario; large tickets price per-class. No plan-level dollar figures without commitment to their accuracy; workflow-timeout + scenario caps are the structural enforcement.
- **Frozen regression fixtures for retro-validation claims** (`#338` D9 pattern). A plan claiming "would have caught X" must ship a committed fixture reproducing X's pre-fix state, with a test that demonstrably fails against the fixture without the fix.
