---
module: merge_gate
tags: [merge-gate, verdict, role-separation, forge-identity, read-side]
problem_type: incomplete-enforcement
category: best-practices
---

# A read-side rule must cover every reader of the verdict

## Problem

mika#2667 asked for one condition: in the `pass` arm of `verdict_handler`,
reach `run_gh_merge` only when the review author is the reviewer identity
(`REVIEWER_FORGE_LOGIN`). The ticket named one site, and that site was real.

But the merge path consumes a `VERDICT: pass` at **two** places, not one:

| reader | trigger | how it selects the verdict |
|---|---|---|
| `server::verdict_handler` | `pull_request_review.submitted` | parses the event's own body |
| `server::ci_success_handler::find_pass_verdict` | `check_suite.completed(success)` | lists the PR's reviews and keeps the latest APPROVED one carrying `VERDICT: pass` |

The second one did not read the author either. A `pass` approved under the
dispatcher identity would be refused by the first reader, then picked up by the
second the moment CI went green, turned into a merge-ready signal, and merged
by `merge_ready_handler` — whose own belt (`would_merge_as_reviewer`) only
stops the *reviewer* from merging, never a non-reviewer verdict from being
consumed. Closing the named site alone would have produced a gate that reads as
closed and is not.

## Solution

- **One reader of the reviewer login**:
  `mika_common::forge_identity::is_reviewer_forge_login` (trim, leading `@`,
  case, `[bot]` suffix). Both readers call it; no literal is copied.
- `ci_success_handler`'s selection is extracted into a pure function,
  `select_pass_verdict(&[Value])`, so the rule is testable without a network
  call. A non-reviewer `pass` is skipped, a review with no author is skipped,
  and an older reviewer verdict still wins over a newer non-reviewer one.

## How to apply

Before closing a ticket that adds a condition "at the merge gate", enumerate
every consumer of the verdict, not only the one the ticket names:

```bash
grep -rn "parse_verdict\|Verdict::Pass" crates/mika-agent/src --include='*.rs' \
  | grep -v '/tests/\|#\[test\]'
```

Each hit that leads to `run_gh_merge` or to a `MergeReadySignal` is a reader
the rule must cover. A rule enforced at one reader and absent at another is a
rule with a side door, and a side door reads exactly like a closed gate.
