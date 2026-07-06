**First-pass review — mika#1723: DashMap self-deadlock fix**

Plan reviewed against `docs/architecture/review-guide.md` principles and the ticket context fetched via `gh_read`.

**Issue state sanity check:** mika#1723 is OPEN with labels `bug`, `p1-important`, `ready`. mika#1719 is OPEN as the parent substrate ticket. The plan's root-cause analysis matches the issue body verbatim.

---

### Principle verification

**Single Responsibility** — PASS. The helper extraction (`should_emit_rate_limit_audit`) isolates the throttle decision from the Axum handler. The plan touches exactly one file (`handlers.rs`) and makes no schema or public API changes.

**KISS** — PASS. The fix is the minimal change: extract the `Copy` `Instant` via `.get().map(|r| *r.value())` before the match, dropping the `Ref` guard. No `.entry()` API, no restructuring beyond the guard-drop discipline. The accepted TOCTOU race is explicitly documented with a "DO NOT fix" warning.

**Orthogonality** — PASS. Scope is rigorously contained:
- mika#1719 invariant 4 deferred.
- Clippy structural enforcement deferred to companion PR.
- Prime task #446 unblock is operational, not code deliverable.
- No schema migration, no dependency addition.

**YAGNI** — PASS. The test suite is proportionate: one concurrency regression test, two semantic correctness tests. The 10-line repro is verification-only (not committed). No extra abstractions.

---

### Gate checks

**Unresolved-Decision Gate (mika#1244)** — PASS. No TBD tokens, placeholder paths, unspecified pins, or deferred-load-bearing decisions. Every design choice is committed.

**Acceptance-Criteria Gate (mika#1559)** — PASS. The `## Acceptance criteria` section has 6 concrete, testable criteria derived from the ticket's verification contract.

---

### Findings

None blocking. The plan correctly:
- Identifies the match-scrutinee temporary lifetime as the root cause.
- Uses guard-drop (not guard-hold) as the fix shape.
- Documents the benign TOCTOU race with a future-reviewer warning.
- Keeps the concurrency test focused on completion (not exact count, which would be flaky under the accepted race).

All conventions and safety requirements are met.

Disposition: READY
