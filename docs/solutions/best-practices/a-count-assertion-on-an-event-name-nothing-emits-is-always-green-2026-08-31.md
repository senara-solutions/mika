---
title: A count assertion on an event name nothing emits is always green — pin the name to the source
date: 2026-08-31
category: best-practices
module: crates/mika-agent/tests/eval, crates/mika-agent/src/perimeter
problem_type: best_practice
component: testing
severity: high
applies_when:
  - "Writing a test that asserts an audit event, log line, or metric was NOT emitted"
  - "Asserting a count of rows selected by a free-form string key (tool_name, event name, label)"
  - "Reviewing a test whose only failure mode would be a row appearing where none can"
related_components:
  - testing_framework
  - observability
tags:
  - anti-vacuity
  - audit-events
  - negative-assertion
  - structural-guard
  - free-form-key
  - forge-gate
---

# A count assertion on an event name nothing emits is always green

## The class

`audit_events.tool_name` is free-form TEXT. Nothing — not the compiler, not a schema,
not a lint — connects the string a test asks for to the strings the code writes. So a
test can assert

```rust
assert_eq!(db.count_audit_events_by_tool_name("ci_success_handler_merge_initiated").await?, 0,
           "no merge may be initiated");
```

and pass forever, in every possible state of the system, because
`ci_success_handler_merge_initiated` is not a name any code emits. The row it counts
cannot exist. The assertion is not weak — it is empty. It reads like a safety check
and is a decoration.

This is worse than no test. A negative assertion is exactly where a reviewer's eye
relaxes: absence is the expected result, a green is the expected outcome, and there is
no moment where the test's own correctness is put under load. A positive assertion
(`count == 1`) fails loudly the first time you get the name wrong; a negative one never
does. **The failure mode of a wrong name is silent in one direction only, and it is the
direction that matters.**

Caught in mika#1947 review. The intent — *the CI-success path must not merge a
DECISION-CORE PR* — was right; the invariant it was guarding is load-bearing
(mika#1851 auto-merged four DECISION-CORE files through exactly that path). The
assertion protecting it proved nothing.

## The fix: make the name checkable

Pin the cited name against the source that emits it, at compile time, before counting:

```rust
const HANDLER_SRC: &str = include_str!("../../src/server/ci_success_handler.rs");

pub fn assert_audit_event_name_is_real(name: &str) {
    assert!(
        HANDLER_SRC.contains(&format!("\"{name}\"")),
        "audit-event name `{name}` appears nowhere in ci_success_handler.rs — \
         a count assertion on it is vacuous."
    );
}
```

Then every negative assertion becomes two: the name is real, *and* the count is zero.
`include_str!` rather than `fs::read_to_string` so the test is pinned to the file the
binary was built from — a stale copy on disk cannot make it pass against rules that are
not the ones in force.

Verify the guard the way you verify any guard: re-inject the invented name and watch it
go red. A guard you have not seen fail is a guard you are guessing about.

## Where else this bites

Any assertion keyed on a free-form string the compiler does not check:

- `tool_name` / `target_key` in `audit_events` (`phantom_aged_out`, `wip_rescue`,
  `destructive_action_grounding`, …) — no enum, no CHECK, deliberately.
- Structured-log `event = "..."` names that operator greps and monitors stand on.
- Metric and span names.
- Label strings in `gh` assertions.

The rule generalises: **when a test names something the compiler will not check, the
test must check it.** Otherwise the test is asserting about a world it invented.

## The tell, for review

A negative assertion whose subject appears nowhere else in the diff and nowhere in the
non-test source. Grep the name across `src/`. If the only hits are the test file, the
assertion is vacuous — regardless of how well the comment above it reads.

## See also

- `docs/solutions/best-practices/verification-claims-with-expected-output-shape-2026-04-28.md`
- `feedback_verify_pipeline_passes_without_the_fix` — the same discipline on the other
  side: a test that passes without the fix is not testing the fix.
- `feedback_a_stub_built_from_the_doc_cannot_falsify_the_doc` — the sibling shape:
  a test built from the premise cannot test the premise.
